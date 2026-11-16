//! 界面场景测试：用 MockTransport 充当服务端、TestBackend 充当终端，
//! 不访问网络、不需要真实账号，也不启动后台播放进程。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use qqmusic_api::Credential;
use qqmusic_api::modules::search::SearchType;
use qqmusic_api::transport::mock::MockTransport;
use qqmusic_api::transport::{Request, Response, TransportError};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Line;
use serde_json::{Value, json};
use tokio::sync::mpsc::{self, UnboundedReceiver};

use super::app::{App, Level, LoginMsg, LoginView, LyricState, Mode, More, Msg, Session};
use super::data::{self, Item, Source};
use super::input::width;
use super::{lyric, ui};
use crate::paths::Paths;
use crate::protocol::{Activity, Command, ErrorKind, Event, PlayerState, SingerRef, Status, Track};
use qqmusic_api::models::login::QrLoginType;

struct Harness {
    app: App,
    rx: UnboundedReceiver<Msg>,
    commands: UnboundedReceiver<Command>,
    mock: MockTransport,
    dir: tempfile::TempDir,
}

fn method_of(request: &Request) -> String {
    request.json_body().and_then(|b| b["req_0"]["method"].as_str().map(str::to_string)).unwrap_or_default()
}

fn param_of(request: &Request) -> Value {
    request.json_body().map(|b| b["req_0"]["param"].clone()).unwrap_or_default()
}

fn ok(data: Value) -> Result<Response, TransportError> {
    Ok(Response::json(&json!({"code": 0, "req_0": {"code": 0, "data": data}})))
}

fn code(code: i64) -> Result<Response, TransportError> {
    Ok(Response::json(&json!({"code": 0, "req_0": {"code": code, "data": {}}})))
}

/// 为某个接口方法注册应答。
fn on(
    mock: &MockTransport,
    method: &'static str,
    handler: impl Fn(&Value) -> Result<Response, TransportError> + Send + Sync + 'static,
) {
    mock.route(move |r| method_of(r) == method, move |r| handler(&param_of(r)));
}

fn song_json(id: i64, name: &str) -> Value {
    json!({
        "id": id, "mid": format!("m{id}"), "name": name, "interval": 215,
        "singer": [{"mid": "s1", "name": "周杰伦"}, {"mid": "s2", "name": "费玉清"}],
        "album": {"mid": "a1", "name": "依然范特西"},
    })
}

fn track(i: usize) -> Track {
    Track {
        id: i as i64,
        mid: format!("m{i}"),
        media_mid: String::new(),
        song_type: 0,
        name: format!("歌曲 {i}"),
        singers: "周杰伦".into(),
        album: "叶惠美".into(),
        cover: String::new(),
        duration: 269,
        vip: i.is_multiple_of(3),
        album_mid: "a1".into(),
        singer_list: vec![SingerRef { mid: "s1".into(), name: "周杰伦".into() }],
    }
}

impl Harness {
    /// 未登录的界面；没有注册应答的接口一律返回空数据。
    fn new() -> Self {
        Self::build(None)
    }

    fn logged_in() -> Self {
        Self::build(Some(Credential::new(1001, "key")))
    }

    fn build(credential: Option<Credential>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path()).unwrap();
        let mock = MockTransport::with_handler(|_| ok(json!({})));
        let client = crate::testing::client(&mock);
        if let Some(credential) = credential {
            crate::save_credential(&paths, &credential).unwrap();
            client.set_credential(credential);
        }
        let (tx, rx) = mpsc::unbounded_channel();
        let (commands_tx, commands) = mpsc::unbounded_channel();
        let app = App::new(paths, client, tx, commands_tx);
        Self { app, rx, commands, mock, dir }
    }

    /// 处理界面收到的异步消息，直到一小段时间内不再有新消息。
    async fn settle(&mut self) {
        while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_millis(40), self.rx.recv()).await {
            self.app.handle_msg(msg);
        }
    }

    fn key(&mut self, code: KeyCode) {
        self.app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn ctrl(&mut self, c: char) {
        self.app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    }

    /// 依次按下每个字符键。
    fn keys(&mut self, keys: &str) {
        for c in keys.chars() {
            self.key(KeyCode::Char(c));
        }
    }

    fn render(&self, width: u16, height: u16) -> String {
        render(&self.app, width, height)
    }

    /// 取走界面发给后台的指令（GetQueue / Hello 这类握手不算）。
    fn sent(&mut self) -> Vec<Command> {
        let mut out = Vec::new();
        while let Ok(command) = self.commands.try_recv() {
            if !matches!(command, Command::GetQueue | Command::Hello { .. }) {
                out.push(command);
            }
        }
        out
    }

    fn player(&mut self, event: Event) {
        self.app.handle_msg(Msg::Player(event));
    }

    /// 模拟后台连上并推送队列与状态。
    fn connect(&mut self, count: usize, index: Option<usize>, status: Status) {
        let tracks: Vec<Track> = (1..=count).map(track).collect();
        let ids: Vec<u64> = (1..=count as u64).collect();
        let state = PlayerState {
            status,
            index,
            track: index.map(|i| tracks[i].clone()),
            position_ms: 61_000,
            duration_ms: 269_000,
            volume: 80,
            queue_len: count,
            queue_version: 1,
            entry: index.map(|i| ids[i]),
            next: index.map(|i| (i + 1) % count.max(1)),
            activity: if status == Status::Playing { Activity::Output } else { Activity::Idle },
            protocol: 2,
            ..Default::default()
        };
        self.player(Event::Queue { tracks, version: 1, ids });
        self.player(Event::State(Box::new(state)));
        self.sent();
    }

    /// 输入关键词并搜索。
    async fn search(&mut self, keyword: &str) {
        self.key(KeyCode::Char('/'));
        self.ctrl('u');
        self.app.handle_paste(keyword);
        self.key(KeyCode::Enter);
        self.settle().await;
    }

    fn songs(&self) -> usize {
        self.app.page().song_count()
    }

    fn infos(&self) -> Vec<&str> {
        self.app.notices.iter().filter(|n| n.level == Level::Info).map(|n| n.text.as_str()).collect()
    }

    fn errors(&self) -> Vec<&str> {
        self.app.notices.iter().filter(|n| n.level == Level::Error).map(|n| n.text.as_str()).collect()
    }
}

/// 渲染成文本；宽字符后面的占位格跳过。顺带检查每一行都没有超出终端宽度。
fn render(app: &App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut lines = Vec::new();
    for row in buffer.content.chunks(w as usize) {
        let mut line = String::new();
        let mut skip = 0;
        for cell in row {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            line.push_str(cell.symbol());
            skip = Line::from(cell.symbol()).width().saturating_sub(1);
        }
        assert!(width(&line) <= w as usize, "行宽超出终端: {line:?}");
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

/// 搜索接口：单曲每页 30 首共 80 首；`fail` 为真时第二页及之后失败。
fn search_server(mock: &MockTransport) -> Arc<AtomicBool> {
    let fail = Arc::new(AtomicBool::new(false));
    let flag = fail.clone();
    on(mock, "DoSearchForQQMusicMobile", move |param| {
        let page = param["page_num"].as_i64().unwrap();
        let keyword = param["query"].as_str().unwrap().to_string();
        if keyword == "空" {
            return ok(json!({"meta": {"nextpage": -1, "sum": 0}, "body": {}}));
        }
        if param["search_type"] == SearchType::Singer.code() {
            let singers = json!([{"singerMID": "s1", "singerName": "周杰伦", "songNum": 500, "albumNum": 30}]);
            return ok(json!({"meta": {"nextpage": -1, "sum": 1}, "body": {"singer": singers}}));
        }
        if param["search_type"] != SearchType::Song.code() {
            return ok(json!({"meta": {"nextpage": -1, "sum": 0}, "body": {}}));
        }
        if page > 1 && flag.load(Ordering::Relaxed) {
            return Ok(Response::new(502, "bad gateway"));
        }
        let count = if page < 3 { 30 } else { 20 };
        let songs: Vec<Value> =
            (0..count).map(|i| song_json((page - 1) * 30 + i + 1, &format!("第{page}页-{i} {keyword}"))).collect();
        let next = if page < 3 { page + 1 } else { -1 };
        ok(json!({"meta": {"searchid": "sid", "nextpage": next, "sum": 80}, "body": {"item_song": songs}}))
    });
    fail
}

fn search_requests(mock: &MockTransport, search_type: SearchType) -> Vec<Value> {
    mock.requests()
        .iter()
        .filter(|r| method_of(r) == "DoSearchForQQMusicMobile")
        .map(param_of)
        .filter(|p| p["search_type"] == search_type.code())
        .collect()
}

#[tokio::test]
async fn search_loads_real_pages_and_survives_a_failed_page() {
    let mut h = Harness::new();
    let fail = search_server(&h.mock);
    h.connect(0, None, Status::Stopped);

    // 中文输入 + 粘贴；首批到达即展示，并标明服务端总量。
    h.search("周杰伦").await;
    assert_eq!(h.songs(), 30);
    assert!(h.render(100, 30).contains("已加载 30 / 共 80 首"), "{}", h.render(100, 30));
    assert_eq!(search_requests(&h.mock, SearchType::Song).len(), 1, "不提前拉后面的页");

    // 走到末尾触发第二页；失败时第一页保留，并提示可重试。
    fail.store(true, Ordering::Relaxed);
    h.key(KeyCode::Char('G'));
    h.settle().await;
    assert_eq!(h.songs(), 30);
    assert!(matches!(h.app.page().more, More::Failed(_)));
    assert!(h.render(100, 30).contains("加载更多失败（r 重试）"), "{}", h.render(100, 30));
    let before = search_requests(&h.mock, SearchType::Song).len();
    h.key(KeyCode::Char('j'));
    h.settle().await;
    assert_eq!(search_requests(&h.mock, SearchType::Song).len(), before, "失败后不自动反复请求");

    // r 重试的是同一页，沿用同一个 searchid。
    fail.store(false, Ordering::Relaxed);
    h.key(KeyCode::Char('r'));
    h.settle().await;
    assert_eq!(h.songs(), 60);
    let requests = search_requests(&h.mock, SearchType::Song);
    assert_eq!(requests.last().unwrap()["page_num"], 2);
    assert_eq!(requests.last().unwrap()["searchid"], requests[0]["searchid"]);

    // 第三页是最后一页：按服务端的结束标记停止，而不是按条数猜。
    h.key(KeyCode::Char('G'));
    h.settle().await;
    assert_eq!((h.songs(), &h.app.page().more), (80, &More::Done));
    assert!(h.render(100, 30).contains("共 80 首"));
    let total = search_requests(&h.mock, SearchType::Song).len();
    h.key(KeyCode::Char('G'));
    h.settle().await;
    assert_eq!(search_requests(&h.mock, SearchType::Song).len(), total, "没有更多时不再请求");
}

#[tokio::test]
async fn search_types_keep_their_own_position_and_singers_can_be_opened() {
    let mut h = Harness::new();
    search_server(&h.mock);
    on(&h.mock, "GetSingerSongList", |_| {
        ok(
            json!({"totalNum": 2, "songList": [{"songInfo": song_json(901, "晴天")}, {"songInfo": song_json(902, "稻香")}]}),
        )
    });
    h.connect(0, None, Status::Stopped);
    h.search("周杰伦").await;
    h.keys("jjjj");
    assert_eq!(h.app.page().selected, 4);
    let depth = h.app.stack.len();

    // 切到歌手：复用歌手搜索，能进入歌手的歌曲列表。
    h.key(KeyCode::Char('t'));
    h.settle().await;
    assert_eq!(h.app.stack.len(), depth, "切换类型不会越叠越深");
    assert!(h.render(100, 30).contains("周杰伦 500 首 · 30 张专辑"), "{}", h.render(100, 30));
    h.key(KeyCode::Enter);
    h.settle().await;
    assert!(matches!(h.app.page().source, Source::Singer(ref mid) if mid == "s1"));
    assert_eq!(h.songs(), 2);
    assert!(matches!(&h.app.page().items[0], Item::Link { source: Source::SingerAlbums(_), .. }));

    // 返回后停在原来的歌手结果上。
    h.key(KeyCode::Esc);
    assert!(matches!(h.app.page().source, Source::Search { kind: data::SearchKind::Singer, .. }));

    // 转一圈回到单曲：位置和已加载的内容都还在，没有重新请求。
    let requests = search_requests(&h.mock, SearchType::Song).len();
    h.keys("ttt");
    h.settle().await;
    assert!(matches!(h.app.page().source, Source::Search { kind: data::SearchKind::Song, .. }));
    assert_eq!((h.app.page().selected, h.songs()), (4, 30));
    assert_eq!(search_requests(&h.mock, SearchType::Song).len(), requests);

    // 空结果有明确说明。
    h.search("空").await;
    assert!(h.render(100, 30).contains("没有找到相关结果"));

    // 快速换关键词：先发出的请求晚到也不会盖掉后一次的结果。
    h.key(KeyCode::Char('/'));
    h.ctrl('u');
    h.app.handle_paste("甲");
    h.key(KeyCode::Enter);
    h.key(KeyCode::Char('/'));
    h.ctrl('u');
    h.app.handle_paste("乙");
    h.key(KeyCode::Enter);
    h.settle().await;
    assert!(matches!(h.app.page().source, Source::Search { ref keyword, .. } if keyword == "乙"));
    assert!(matches!(&h.app.page().items[0], Item::Song(t) if t.name.ends_with("乙")));
    assert_eq!(h.songs(), 30);
}

#[tokio::test(start_paused = true)]
async fn search_box_edits_text_debounces_suggestions_and_keeps_history() {
    let mut h = Harness::new();
    search_server(&h.mock);
    let broken = Arc::new(AtomicBool::new(false));
    let flag = broken.clone();
    on(&h.mock, "GetSmartBoxResult", move |param| {
        if flag.load(Ordering::Relaxed) {
            return Ok(Response::new(500, "boom"));
        }
        let query = param["query"].as_str().unwrap();
        ok(json!({"items": [{"hint": format!("{query}的歌")}, {"hint": format!("{query}演唱会")}]}))
    });
    let smartbox = |h: &Harness| h.mock.requests().iter().filter(|r| method_of(r) == "GetSmartBoxResult").count();
    h.connect(0, None, Status::Stopped);

    // 连续输入只在停顿后请求一次，用的是最新的内容。
    h.key(KeyCode::Char('/'));
    for c in "周杰".chars() {
        h.key(KeyCode::Char(c));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(smartbox(&h), 0, "防抖时间内不请求");
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.settle().await;
    assert_eq!(smartbox(&h), 1);
    let Mode::Search(search) = &h.app.mode else { panic!("搜索框应保持打开") };
    assert_eq!(search.options, ["周杰的歌", "周杰演唱会"]);

    // 光标移动、中间插入、删除；q / Q 这些字符只是文本。
    h.key(KeyCode::Home);
    h.key(KeyCode::Right);
    h.keys("Qq");
    h.key(KeyCode::Backspace);
    h.key(KeyCode::End);
    h.key(KeyCode::Char('伦'));
    let Mode::Search(search) = &h.app.mode else { panic!() };
    assert_eq!(search.input.text(), "周Q杰伦");
    assert_eq!(h.app.quit, None);

    // 选中一条建议后回车：按建议搜索，并记入历史。
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.settle().await;
    h.key(KeyCode::Down);
    h.key(KeyCode::Down);
    h.key(KeyCode::Enter);
    h.settle().await;
    assert!(matches!(h.app.page().source, Source::Search { ref keyword, .. } if keyword == "周Q杰伦演唱会"));
    assert_eq!(h.app.history, ["周Q杰伦演唱会"]);

    // 关闭搜索框后，迟到的建议不会再弹出来。
    h.key(KeyCode::Char('/'));
    h.key(KeyCode::Char('x'));
    h.key(KeyCode::Esc);
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.settle().await;
    assert!(matches!(h.app.mode, Mode::Normal));

    // 补全接口坏了不影响手动搜索。
    broken.store(true, Ordering::Relaxed);
    h.key(KeyCode::Char('/'));
    h.ctrl('u');
    h.keys("abc");
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.settle().await;
    h.key(KeyCode::Enter);
    h.settle().await;
    assert!(matches!(h.app.page().source, Source::Search { ref keyword, .. } if keyword == "abc"));
    assert!(h.errors().is_empty(), "{:?}", h.errors());

    // 历史持久化；空输入时显示历史，可以清除。
    assert_eq!(data::load_history(&h.app.paths), ["abc", "周Q杰伦演唱会"]);
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('/'));
    let Mode::Search(search) = &h.app.mode else { panic!() };
    assert!(search.from_history && search.options.len() == 2);
    assert!(h.render(80, 24).contains("历史"));
    h.ctrl('x');
    assert!(h.app.history.is_empty() && data::load_history(&h.app.paths).is_empty());

    // 很长的输入横向滚动：光标处的内容可见，弹窗不越界。
    h.app.handle_paste(&"很长的关键词".repeat(20));
    let screen = h.render(60, 20);
    assert!(screen.contains("很长的关键词"), "{screen}");
}

#[tokio::test]
async fn paste_outside_the_search_box_is_not_treated_as_shortcuts() {
    let mut h = Harness::new();
    h.connect(3, Some(0), Status::Playing);
    h.app.handle_paste("Q ]] d /abc\n");
    assert_eq!(h.app.quit, None);
    assert!(h.sent().is_empty());
    assert!(matches!(h.app.mode, Mode::Normal));
}

/// 喜欢接口的应答由测试逐次指定。
fn like_server(mock: &MockTransport) -> Arc<std::sync::Mutex<Vec<Result<Response, TransportError>>>> {
    let replies = Arc::new(std::sync::Mutex::new(Vec::new()));
    for method in ["AddSonglist", "DelSonglist"] {
        let replies = replies.clone();
        on(mock, method, move |_| {
            let mut replies: std::sync::MutexGuard<Vec<_>> = replies.lock().unwrap();
            if replies.is_empty() { ok(json!({"retCode": 0})) } else { replies.remove(0) }
        });
    }
    replies
}

fn like_requests(mock: &MockTransport) -> Vec<String> {
    mock.requests().iter().map(method_of).filter(|m| m == "AddSonglist" || m == "DelSonglist").collect()
}

#[tokio::test]
async fn like_is_optimistic_but_only_confirmed_results_stick() {
    let mut h = Harness::logged_in();
    let replies = like_server(&h.mock);
    h.settle().await;
    h.connect(3, Some(0), Status::Playing);
    let id = track(1).id;

    // 成功：先乐观显示，确认后保持。
    h.key(KeyCode::Char('f'));
    assert!(h.app.liked.contains(&id), "乐观更新");
    h.settle().await;
    assert!(h.app.liked.contains(&id));
    assert_eq!(h.infos().last().copied(), Some("已喜欢《歌曲 1》"));

    // 网络失败：回滚，并留下错误。
    replies.lock().unwrap().push(Err(TransportError::new("connection reset")));
    h.key(KeyCode::Char('f'));
    assert!(!h.app.liked.contains(&id));
    h.settle().await;
    assert!(h.app.liked.contains(&id), "取消喜欢失败后恢复为喜欢");
    assert!(h.errors().last().unwrap().contains("取消喜欢《歌曲 1》失败"), "{:?}", h.errors());

    // 非零 retCode：不是成功，回滚。
    replies.lock().unwrap().push(ok(json!({"retCode": 3})));
    h.key(KeyCode::Char('f'));
    h.settle().await;
    assert!(h.app.liked.contains(&id));
    assert!(h.errors().last().unwrap().contains("服务端拒绝（3）"), "{:?}", h.errors());

    // 「无变化」：目标状态已经成立，算确认，不算失败。
    replies.lock().unwrap().push(code(80092));
    let errors = h.errors().len();
    h.key(KeyCode::Char('f'));
    h.settle().await;
    assert!(!h.app.liked.contains(&id));
    assert_eq!(h.errors().len(), errors);
    assert!(h.infos().last().unwrap().contains("本来就不在"));

    // 快速连按：请求没回来之前不叠加第二个。
    let before = like_requests(&h.mock).len();
    h.keys("fff");
    h.settle().await;
    assert_eq!(like_requests(&h.mock).len(), before + 1);
    assert!(h.app.liked.contains(&id));
    assert!(h.infos().iter().any(|m| m.contains("还在处理中")));
}

#[tokio::test]
async fn like_updates_the_open_liked_page_and_ignores_results_from_the_old_account() {
    let mut h = Harness::logged_in();
    like_server(&h.mock);
    on(&h.mock, "CgiGetDiss", |_| {
        ok(json!({"hasmore": 0, "total_song_num": 2, "songlist": [song_json(10, "七里香"), song_json(11, "稻香")]}))
    });
    h.settle().await;
    h.connect(3, Some(0), Status::Playing);
    assert!(h.app.liked.contains(&10), "启动时取回喜欢列表");
    h.app.push("我喜欢的音乐".into(), Source::Liked);
    h.settle().await;
    assert_eq!(h.songs(), 2);

    // 喜欢正在播放的歌：确认后出现在「我喜欢」页，光标仍在原来那首上。
    h.key(KeyCode::Char('j'));
    h.key(KeyCode::Char('f'));
    h.settle().await;
    assert_eq!((h.songs(), h.app.page().total), (3, Some(3)));
    assert!(matches!(&h.app.page().items[0], Item::Song(t) if t.id == 1));
    assert!(matches!(&h.app.page().items[h.app.page().selected], Item::Song(t) if t.name == "稻香"));

    // 取消喜欢选中的歌：确认后从页面移除。
    h.key(KeyCode::Char('F'));
    h.settle().await;
    assert_eq!(h.songs(), 2);
    assert!(!h.app.liked.contains(&11));

    // 请求途中换了账号：旧账号的结果、昵称、喜欢列表都不能落到新账号上。
    h.key(KeyCode::Char('f'));
    h.app.handle_msg(Msg::Login(LoginMsg::Done(Box::new(Credential::new(2002, "other")))));
    assert!(h.app.liked.is_empty());
    assert!(h.app.stack.iter().all(|p| p.source != Source::Liked), "账号相关的页面随账号丢弃");
    let infos = h.infos().len();
    h.app.handle_msg(Msg::Nick { account: 1, name: "旧昵称".into() });
    h.app.handle_msg(Msg::Liked { account: 1, result: Ok([777].into()) });
    assert!(h.app.nick.is_none() && !h.app.liked.contains(&777));
    h.settle().await;
    assert!(!h.infos()[infos..].iter().any(|m| m.contains("取消喜欢")), "{:?}", &h.infos()[infos..]);
    assert_eq!(h.app.client.credential().musicid, 2002);
}

#[tokio::test]
async fn like_requires_login() {
    let mut h = Harness::new();
    h.connect(3, Some(0), Status::Playing);
    h.key(KeyCode::Char('f'));
    assert!(h.app.liked.is_empty());
    assert!(h.errors().last().unwrap().contains("需要登录"));
    assert!(like_requests(&h.mock).is_empty());
}

#[tokio::test]
async fn login_does_not_report_success_when_saving_or_syncing_fails() {
    let mut h = Harness::new();
    h.connect(0, None, Status::Stopped);

    // 正常登录：保存、通知后台重新读取；后台确认前不说「已同步」。
    h.app.handle_msg(Msg::Login(LoginMsg::Done(Box::new(Credential::new(1001, "key")))));
    assert_eq!(crate::read_credential(&h.app.paths).unwrap().unwrap().musicid, 1001);
    let sent = h.sent();
    let [Command::ReloadCredential { tag: Some(tag) }] = sent[..] else { panic!("{sent:?}") };
    assert!(h.infos().contains(&"登录成功") && !h.infos().iter().any(|m| m.contains("已同步")));
    assert_eq!(h.app.session, Session::Valid);

    // 后台重新读取失败：如实报告。
    h.player(Event::Done { tag, ok: false, message: "读取登录凭证失败: denied".into() });
    assert!(h.errors().last().unwrap().contains("读取登录凭证失败"), "{:?}", h.errors());

    // 凭证写不进去（目标是个目录）：不报登录成功，也不让后台去读旧文件。
    crate::remove_credential(&h.app.paths).unwrap();
    std::fs::create_dir(h.app.paths.credential()).unwrap();
    let infos = h.infos().len();
    h.app.handle_msg(Msg::Login(LoginMsg::Done(Box::new(Credential::new(3003, "new")))));
    assert!(!h.infos()[infos..].contains(&"登录成功"));
    assert!(h.errors().last().unwrap().contains("未能保存"), "{:?}", h.errors());
    assert!(h.sent().is_empty(), "保存失败时不通知后台");
    assert_eq!(h.app.client.credential().musicid, 3003, "本次界面仍可使用");

    // 凭证删不掉：退出登录失败，登录状态不变。
    std::fs::write(h.app.paths.credential().join("keep"), "x").unwrap();
    h.app.push("账号".into(), Source::Account);
    let logout = h.app.page().items.iter().position(|i| matches!(i, Item::Action { title, .. } if title == "退出登录"));
    h.app.stack.last_mut().unwrap().selected = logout.unwrap();
    h.key(KeyCode::Enter);
    assert!(h.app.logged_in());
    assert!(h.errors().last().unwrap().contains("退出登录失败"));
    assert!(h.sent().is_empty());

    // 能删掉时：退出成功，并通知后台。
    std::fs::remove_dir_all(h.app.paths.credential()).unwrap();
    h.key(KeyCode::Enter);
    assert!(!h.app.logged_in() && h.app.session == Session::LoggedOut);
    assert!(matches!(h.sent()[..], [Command::ReloadCredential { .. }]));
    assert_eq!(h.infos().last().copied(), Some("已退出登录"));
}

#[tokio::test]
async fn session_check_distinguishes_unverified_from_expired_and_syncs_across_clients() {
    // 网络检查失败：保留登录，只标记为「未确认」。
    let mut h = Harness::logged_in();
    on(&h.mock, "GetLoginUserInfo", |_| Err(TransportError::new("offline")));
    h.settle().await;
    h.app.check_session();
    assert_eq!(h.app.session, Session::Checking);
    h.settle().await;
    assert!(matches!(h.app.session, Session::Unverified(_)), "{:?}", h.app.session);
    assert!(h.app.logged_in());
    assert!(h.app.account_label().contains("未确认"));

    // 服务端确认有效。
    let mut h = Harness::logged_in();
    h.settle().await;
    h.app.check_session();
    h.settle().await;
    assert_eq!(h.app.session, Session::Valid);

    // 接口报告登录失效：标记为过期，账号页给出重新登录的入口。
    h.connect(0, None, Status::Stopped);
    h.player(Event::Error {
        message: "《x》获取播放链接失败: 登录已失效".into(), kind: ErrorKind::Login
    });
    assert_eq!(h.app.session, Session::Expired);
    assert!(h.app.alert.as_ref().unwrap().contains("重新登录"));
    h.app.push("账号".into(), Source::Account);
    assert!(h.render(80, 24).contains("登录已过期") && h.render(80, 24).contains("QQ 扫码登录"));
    // 旧账号的检查结果不能改写状态。
    h.app.handle_msg(Msg::Session { account: 0, session: Session::Valid });
    assert_eq!(h.app.session, Session::Expired);

    // 另一个界面换了账号：后台广播后本界面跟着读取新凭证。
    crate::save_credential(&h.app.paths, &Credential::new(4004, "k")).unwrap();
    h.player(Event::CredentialChanged { logged_in: true, musicid: 4004 });
    assert_eq!(h.app.client.credential().musicid, 4004);
    h.settle().await;
    // 自己触发的广播（账号一致）不重复处理。
    let notices = h.app.notices.len();
    h.player(Event::CredentialChanged { logged_in: true, musicid: 4004 });
    assert_eq!(h.app.notices.len(), notices);
    // 别处退出登录。
    crate::remove_credential(&h.app.paths).unwrap();
    h.player(Event::CredentialChanged { logged_in: false, musicid: 0 });
    assert!(!h.app.logged_in());
}

/// 歌单接口：每页 100 首，共 5000 首；`fail` 为真时请求失败。
fn playlist_server(mock: &MockTransport) -> Arc<AtomicBool> {
    let fail = Arc::new(AtomicBool::new(false));
    let flag = fail.clone();
    on(mock, "CgiGetDiss", move |param| {
        if flag.load(Ordering::Relaxed) {
            return Ok(Response::new(503, "busy"));
        }
        let begin = param["song_begin"].as_i64().unwrap();
        let songs: Vec<Value> =
            (begin..begin + 100).map(|i| song_json(i + 1, &format!("歌单曲目 {}", i + 1))).collect();
        ok(json!({"hasmore": 1, "total_song_num": 5000, "songlist": songs}))
    });
    fail
}

#[tokio::test]
async fn large_playlist_shows_the_first_batch_and_states_what_is_loaded() {
    let mut h = Harness::new();
    let fail = playlist_server(&h.mock);
    h.connect(0, None, Status::Stopped);
    h.app.push("歌单 › 大歌单".into(), Source::Playlist(7));
    assert!(h.render(100, 30).contains("加载中…"));
    h.settle().await;
    let requests = |h: &Harness| h.mock.requests().iter().filter(|r| method_of(r) == "CgiGetDiss").count();

    // 第一批到了就能看、能操作，不等后面的。
    assert_eq!((h.songs(), requests(&h)), (100, 1));
    assert!(h.render(100, 30).contains("已加载 100 / 共 5000 首"));

    // Enter 播放的是「已加载的 N 首」，并且如实说明。
    h.keys("jj");
    h.key(KeyCode::Enter);
    let sent = h.sent();
    let [Command::Play { tracks, index, .. }] = &sent[..] else { panic!("{sent:?}") };
    assert_eq!((tracks.len(), *index), (100, 2));
    assert_eq!(h.infos().last().copied(), Some("播放已加载的 100 首（共 5000 首，其余尚未加载），已替换队列"));

    // 进入专辑再返回：位置与已加载的内容原样保留，不重新请求。
    h.key(KeyCode::Char('o'));
    assert!(h.render(100, 30).contains("播放并替换队列（本页已加载的 100 首）"), "{}", h.render(100, 30));
    let Mode::Menu(menu) = &h.app.mode else { panic!() };
    let album = menu.items.iter().position(|(label, _)| label.starts_with("查看专辑")).unwrap();
    for _ in 0..album {
        h.key(KeyCode::Char('j'));
    }
    h.key(KeyCode::Enter);
    assert!(matches!(h.app.page().source, Source::Album(ref mid) if mid == "a1"));
    h.key(KeyCode::Esc);
    assert_eq!((h.app.page().selected, h.songs(), requests(&h)), (2, 100, 1));

    // 多歌手的歌先让用户选是哪一位，而不是按名字去猜。
    h.key(KeyCode::Char('o'));
    let Mode::Menu(menu) = &h.app.mode else { panic!() };
    let singers = menu.items.iter().position(|(label, _)| label == "查看歌手…").unwrap();
    for _ in 0..singers {
        h.key(KeyCode::Char('j'));
    }
    h.key(KeyCode::Enter);
    assert!(h.render(100, 30).contains("选择歌手") && h.render(100, 30).contains("费玉清"));
    h.key(KeyCode::Char('j'));
    h.key(KeyCode::Enter);
    assert!(matches!(h.app.page().source, Source::Singer(ref mid) if mid == "s2"));
    h.key(KeyCode::Esc);

    // 刷新失败：旧内容保留，并明确显示失败。
    fail.store(true, Ordering::Relaxed);
    h.key(KeyCode::Char('r'));
    h.settle().await;
    assert_eq!(h.songs(), 100);
    assert!(h.render(120, 30).contains("刷新失败，显示的是旧内容"), "{}", h.render(120, 30));
    fail.store(false, Ordering::Relaxed);
    h.key(KeyCode::Char('r'));
    h.settle().await;
    assert!(h.app.page().error.is_none());

    // 一直往下：到达安全上限后停止，并注明没有全部加载。
    for _ in 0..40 {
        h.key(KeyCode::Char('G'));
        h.settle().await;
    }
    assert_eq!(h.songs(), data::MAX_ITEMS);
    assert!(h.app.page().capped);
    let screen = h.render(120, 30);
    assert!(screen.contains("已加载 2000 / 共 5000 首") && screen.contains("已达上限 2000 条"), "{screen}");
    let loaded = requests(&h);
    h.key(KeyCode::Char('G'));
    h.settle().await;
    assert_eq!(requests(&h), loaded, "达到上限后不再请求");
}

#[tokio::test]
async fn queue_actions_wait_for_confirmation_and_never_replace_the_queue() {
    let mut h = Harness::new();
    search_server(&h.mock);
    h.connect(3, Some(0), Status::Playing);
    h.search("晴天").await;

    // a：追加到队尾。先显示处理中，后台确认后才显示完成。
    h.key(KeyCode::Char('a'));
    let sent = h.sent();
    let [Command::Append { tracks, tag: Some(append) }] = &sent[..] else { panic!("{sent:?}") };
    assert_eq!(tracks.len(), 1, "只入队选中的一首");
    assert!(h.infos().last().unwrap().starts_with("正在追加到队尾"));
    // 别的客户端的确认不算数。
    h.player(Event::Done { tag: append + 12345, ok: true, message: String::new() });
    assert!(!h.infos().iter().any(|m| m.starts_with("已追加")));
    h.player(Event::Done { tag: *append, ok: true, message: String::new() });
    assert!(h.infos().last().unwrap().starts_with("已追加到队尾"));

    // A：下一首播放；失败时报告失败而不是成功。
    h.key(KeyCode::Char('A'));
    let sent = h.sent();
    let [Command::PlayNext { tag: Some(next), .. }] = &sent[..] else { panic!("{sent:?}") };
    h.player(Event::Done { tag: *next, ok: false, message: "没有可添加的歌曲".into() });
    assert!(h.errors().last().unwrap().contains("操作没有完成"));
    assert!(!h.infos().iter().any(|m| m.starts_with("下一首播放：")));

    // l 在歌曲上只开菜单，不会替换队列；Enter 才是「播放并替换队列」。
    h.key(KeyCode::Char('l'));
    assert!(matches!(h.app.mode, Mode::Menu(_)) && h.sent().is_empty());
    h.key(KeyCode::Esc);
    assert!(matches!(h.app.mode, Mode::Normal), "Esc 先关弹窗");
    assert!(matches!(h.app.page().source, Source::Search { .. }), "而不是返回上一页");
    h.key(KeyCode::Enter);
    assert!(matches!(h.sent()[..], [Command::Play { .. }]));

    // 队列视图：定位到当前歌曲；删除 / 播放带条目 id，避免队列变化后误操作。
    h.key(KeyCode::Char('c'));
    assert_eq!((h.app.page().source.clone(), h.app.queue_sel), (Source::Queue, 0));
    h.keys("jj");
    h.key(KeyCode::Char('d'));
    let sent = h.sent();
    assert!(matches!(sent[..], [Command::Remove { index: 2, entry: Some(3), tag: Some(_) }]), "{sent:?}");
    h.key(KeyCode::Enter);
    assert!(matches!(h.sent()[..], [Command::PlayIndex { index: 2, entry: Some(3) }]));
    h.key(KeyCode::Char('K'));
    assert!(matches!(h.sent()[..], [Command::Move { entry: 3, to: 1, .. }]));
    h.key(KeyCode::Char('c'));
    assert_eq!(h.app.queue_sel, 0);

    // 队列更新后光标跟着条目走；比已知版本旧的快照被忽略。
    h.keys("j");
    let tracks: Vec<Track> = [3, 1, 2].into_iter().map(track).collect();
    h.player(Event::Queue { tracks, version: 5, ids: vec![3, 1, 2] });
    assert_eq!((h.app.queue_sel, h.app.queue[0].id), (2, 3), "原来选中的是条目 2");
    h.player(Event::Queue { tracks: vec![track(9)], version: 4, ids: vec![9] });
    assert_eq!(h.app.queue.len(), 3, "旧快照不能把队列改回去");

    // 清空需要确认，默认停在「取消」上。
    h.key(KeyCode::Char('D'));
    h.key(KeyCode::Enter);
    assert!(h.sent().is_empty());
    h.key(KeyCode::Char('D'));
    h.key(KeyCode::Char('j'));
    h.key(KeyCode::Enter);
    assert!(matches!(h.sent()[..], [Command::Clear]));
}

#[tokio::test]
async fn disconnected_ui_rejects_playback_commands_and_reports_unconfirmed_ones() {
    let mut h = Harness::new();
    search_server(&h.mock);
    h.connect(3, Some(0), Status::Playing);
    h.search("晴天").await;
    h.key(KeyCode::Char('a'));
    h.sent();

    h.app.handle_msg(Msg::Disconnected);
    assert!(h.errors().iter().any(|m| m.contains("1 个操作没有得到后台确认")), "{:?}", h.errors());
    assert!(h.render(100, 30).contains("后台未连接"));

    // 断线期间：切歌、入队、暂停一律拒绝并提示；音量这类设置保留最新值。
    for key in [']', '[', ' ', 'a', 'A', '.'] {
        h.key(KeyCode::Char(key));
    }
    h.key(KeyCode::Enter);
    assert!(h.sent().is_empty(), "不能留到重连后执行");
    assert!(h.errors().last().unwrap().contains("操作没有执行"));
    h.keys("-m");
    assert!(matches!(h.sent()[..], [Command::SetVolume { volume: 75 }, Command::SetMode { .. }]));

    // 断线时进度不再前进，避免假装还在播放。
    let frozen = h.app.position_ms();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(h.app.position_ms(), frozen);

    // 重连：以新的快照为准（后台重启后版本号从头开始也能接上）。
    h.player(Event::Queue { tracks: vec![track(8)], version: 1, ids: vec![1] });
    h.connect(1, Some(0), Status::Paused);
    assert!(h.app.connected && h.app.queue.len() == 1);
    assert!(h.infos().iter().any(|m| m.contains("已重新连接")));
    h.app.handle_msg(Msg::Dropped(2));
    assert!(h.errors().last().unwrap().contains("2 个操作已丢弃"));

    // 旧版后台：不支持确认时明说，并提示如何升级。
    let state = PlayerState { status: Status::Paused, queue_version: 1, ..h.app.state.clone() };
    h.player(Event::State(Box::new(PlayerState { protocol: 0, ..state })));
    assert!(h.errors().iter().any(|m| m.contains("旧版本")));
    h.app.push("搜索".into(), Source::Search { keyword: "晴天".into(), kind: data::SearchKind::Song });
    h.settle().await;
    h.sent();
    h.key(KeyCode::Char('a'));
    assert!(matches!(h.sent()[..], [Command::Append { tag: None, .. }]));
    assert!(h.infos().last().unwrap().contains("无法确认结果"));
}

#[tokio::test]
async fn lyric_states_are_distinct_and_the_lyric_view_can_seek() {
    let mut h = Harness::new();
    let fail = Arc::new(AtomicBool::new(true));
    let flag = fail.clone();
    on(&h.mock, "GetPlayLyricInfo", move |_| {
        if flag.load(Ordering::Relaxed) { Ok(Response::new(500, "boom")) } else { ok(json!({})) }
    });

    // 没有歌曲 / 加载中 / 失败 / 确实没有歌词，各有各的说法。
    h.key(KeyCode::Char('L'));
    assert!(h.render(80, 24).contains("没有正在播放的歌曲"));
    h.connect(3, Some(0), Status::Playing);
    assert_eq!(h.app.lyric, LyricState::Loading);
    assert!(h.render(80, 24).contains("歌词加载中"));
    h.settle().await;
    assert!(matches!(h.app.lyric, LyricState::Failed(_)), "{:?}", h.app.lyric);
    assert!(h.render(80, 24).contains("歌词加载失败"));
    fail.store(false, Ordering::Relaxed);
    h.key(KeyCode::Char('r'));
    h.settle().await;
    assert_eq!(h.app.lyric, LyricState::Empty);
    assert!(h.render(80, 24).contains("这首歌没有歌词"));

    // 有歌词：跟随播放位置（61 秒处是第二行），翻译可以开关。
    let lines = lyric::parse(
        "[00:50.00]故事的小黄花\n[01:00.00]从出生那年就飘着\n[01:10.00]童年的荡秋千\n[01:20.00]随记忆一直晃到现在",
        "[01:00.00]Drifting since the year I was born",
    );
    h.app.handle_msg(Msg::Lyric { mid: "m1".into(), result: Ok(lines) });
    let screen = h.render(80, 24);
    assert!(screen.contains("♪ 从出生那年就飘着") && screen.contains("Drifting since"), "{screen}");
    h.key(KeyCode::Char('t'));
    assert!(!h.render(80, 24).contains("Drifting since"));
    // 切歌后迟到的旧歌词不会显示到新歌上。
    h.app.handle_msg(Msg::Lyric { mid: "other".into(), result: Ok(Vec::new()) });
    assert!(matches!(h.app.lyric, LyricState::Ready(_)));

    // 手动滚动：选中行有自己的标记，并提示如何恢复跟随。
    h.keys("jj");
    assert_eq!(h.app.lyric_sel, Some(3));
    let screen = h.render(80, 24);
    assert!(screen.contains("› 随记忆一直晃到现在") && screen.contains("z 恢复跟随"), "{screen}");
    // Enter 跳到选中的那一行，随后恢复跟随。
    h.key(KeyCode::Enter);
    assert!(matches!(h.sent()[..], [Command::Seek { position_ms: 80_000 }]));
    assert_eq!(h.app.lyric_sel, None);
    h.key(KeyCode::Char('k'));
    h.key(KeyCode::Char('z'));
    assert_eq!(h.app.lyric_sel, None);

    // 窄窗口里长歌词折行显示，而不是被截掉。
    let long = lyric::parse("[01:00.00]这是一句特别特别特别特别特别特别特别特别长的歌词结尾在这里", "");
    h.app.handle_msg(Msg::Lyric { mid: "m1".into(), result: Ok(long) });
    let screen = h.render(40, 16);
    assert!(screen.contains("这是一句") && screen.contains("长的歌词结尾在这"), "{screen}");
}

#[tokio::test]
async fn errors_stay_visible_until_dismissed_and_explain_recovery() {
    let mut h = Harness::new();
    h.connect(3, Some(0), Status::Playing);
    h.player(Event::Error { message: "《歌曲 1》没有可用的播放链接".into(), kind: ErrorKind::Rights });
    h.player(Event::Error { message: "《歌曲 2》下载失败: timeout".into(), kind: ErrorKind::Network });

    // 普通提示显示时盖在上面，但错误不会因此丢掉。
    h.keys("-");
    h.app.info("音量 75%");
    assert!(h.render(100, 30).contains("音量 75%"));
    assert!(h.app.alert.as_ref().unwrap().contains("稍后按 r 或 Space 重试"));
    h.app.toast = None;
    let screen = h.render(100, 30);
    assert!(screen.contains("下载失败: timeout") && screen.contains("x 关闭"), "{screen}");
    assert!(screen.contains("! 2 条错误"), "{screen}");

    // 重复的错误不刷屏。
    h.player(Event::Error { message: "《歌曲 2》下载失败: timeout".into(), kind: ErrorKind::Network });
    assert_eq!(h.errors().len(), 2);

    // e 打开列表：两条都在，带各自的恢复提示；小窗口里可以滚动。
    h.key(KeyCode::Char('e'));
    let screen = h.render(100, 30);
    assert!(screen.contains("可换一首或更换账号") && screen.contains("下载失败"), "{screen}");
    assert_eq!(h.app.unread, 0);
    let small = h.render(50, 6);
    assert!(small.contains("j/k 滚动"), "{small}");
    h.key(KeyCode::Char('g'));
    assert!(h.render(50, 6).contains("没有可用的播放链接"), "{}", h.render(50, 6));
    h.key(KeyCode::Esc);
    assert!(matches!(h.app.mode, Mode::Normal));

    // x 关闭底部的错误提示。
    h.key(KeyCode::Char('x'));
    assert!(h.app.alert.is_none() && !h.render(100, 30).contains("x 关闭"));
}

#[tokio::test]
async fn playbar_tells_paused_stopped_loading_and_buffering_apart() {
    let mut h = Harness::new();
    h.connect(3, Some(0), Status::Playing);
    let show = |h: &mut Harness, status, activity| {
        let state = PlayerState { status, activity, ..h.app.state.clone() };
        h.player(Event::State(Box::new(state)));
        h.render(120, 30)
    };
    assert!(show(&mut h, Status::Playing, Activity::Output).contains("♫ ♪ ♫ ♪ ♥"));
    assert!(show(&mut h, Status::Paused, Activity::Idle).contains("_ z Z Z 已暂停 ♥"));
    assert!(show(&mut h, Status::Paused, Activity::Loading).contains("已暂停（加载中）"));
    assert!(show(&mut h, Status::Stopped, Activity::Idle).contains("■ 已停止"));
    assert!(show(&mut h, Status::Loading, Activity::Loading).contains("· · · · 加载中"));
    assert!(show(&mut h, Status::Loading, Activity::Buffering).contains("· · · · 缓冲中"));

    // 缓冲 / 加载时进度不前进。
    let frozen = h.app.position_ms();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(h.app.position_ms(), frozen);

    // 期望音质与实际音质不同时都显示出来。
    use qqmusic_api::modules::song::Quality;
    let state = PlayerState {
        status: Status::Playing,
        activity: Activity::Output,
        quality: Quality::Lossless,
        playing_quality: Some(Quality::Standard),
        ..h.app.state.clone()
    };
    h.player(Event::State(Box::new(state)));
    let screen = h.render(120, 30);
    assert!(screen.contains("标准（期望无损，已降级）"), "{screen}");
    assert!(screen.contains("01:01/04:29") && screen.contains("[列表循环] 80%"), "{screen}");

    // 「接下来」与后台的播放计划一致；随机时不编造。
    assert!(screen.contains("接下来：歌曲 2"), "{screen}");
    let state = PlayerState { mode: crate::protocol::PlayMode::Shuffle, next: None, ..h.app.state.clone() };
    h.player(Event::State(Box::new(state)));
    assert!(h.render(120, 30).contains("接下来：随机挑选"));
}

#[tokio::test]
async fn keys_mean_the_same_and_lists_use_the_available_height_at_every_width() {
    // j / k / h 的含义不随宽度变化：始终是上下移动、返回。
    for (w, rows) in [(140u16, 40u16), (80, 24), (60, 20)] {
        let mut h = Harness::new();
        search_server(&h.mock);
        h.connect(5, Some(1), Status::Playing);
        // 首页内容少：像菜单一样留白，不贴着顶部。
        let home = h.render(w, rows);
        let first = home.lines().position(|l| l.contains("首页")).unwrap();
        assert!(first >= 2 && home.contains(" =>  1. 搜索"), "{w}x{rows}\n{home}");
        if rows >= 30 {
            assert!(first >= 6, "{w}x{rows}: 高窗口里菜单从约 1/3 处开始\n{home}");
        }
        h.search("晴天").await;
        h.render(w, rows);
        h.keys("jjj");
        assert_eq!(h.app.page().selected, 3, "{w}x{rows}");
        h.key(KeyCode::Char('k'));
        assert_eq!(h.app.page().selected, 2, "{w}x{rows}");
        // 长列表上移并占满可用高度；翻页步长跟着可见行数走，而不是固定 10。
        let visible = h.app.viewport.get().rows;
        assert!(visible >= 8 && visible + 12 >= rows as usize, "{w}x{rows}: {visible}");
        h.ctrl('d');
        assert_eq!(h.app.page().selected, (2 + visible).min(29), "{w}x{rows}");
        h.key(KeyCode::Char('h'));
        assert!(!matches!(h.app.page().source, Source::Search { .. }), "{w}x{rows}: h 返回");
    }
}

#[tokio::test]
async fn lyrics_show_below_the_list_and_follow_playback() {
    let mut h = Harness::new();
    h.connect(5, Some(0), Status::Playing);
    let lines = lyric::parse("[00:50.00]故事的小黄花\n[01:00.00]从出生那年就飘着\n[01:10.00]童年的荡秋千", "");
    h.app.handle_msg(Msg::Lyric { mid: "m1".into(), result: Ok(lines) });
    for (w, rows) in [(120, 36), (80, 24), (60, 20)] {
        let screen = h.render(w, rows);
        let menu = screen.lines().position(|l| l.contains("1. 搜索")).unwrap();
        let lyric = screen.lines().position(|l| l.contains("从出生那年就飘着")).unwrap();
        let info = screen.lines().position(|l| l.contains("♫ ♪ ♫ ♪")).unwrap();
        assert!(menu < lyric && lyric < info, "{w}x{rows}\n{screen}");
    }
}

/// 在给定尺寸下走完：搜索 → 播放 → 入队 → 暂停 → 返回。
async fn core_flow(w: u16, rows: u16) -> String {
    let mut h = Harness::new();
    search_server(&h.mock);
    h.connect(5, Some(1), Status::Playing);
    h.render(w, rows);
    h.search("一首名字特别特别特别特别特别长的歌曲").await;
    let screen = h.render(w, rows);
    // 中文长标题按显示宽度截断，不会把后面的列挤乱。
    assert!(screen.contains("…"), "{w}x{rows}\n{screen}");
    // 选中项与正在播放的标记不同，且都不只靠颜色。
    assert!(screen.contains(" => "), "{w}x{rows}\n{screen}");
    assert!(screen.contains("周杰伦"), "{w}x{rows}: 歌手列保留\n{screen}");
    h.key(KeyCode::Char('j'));
    h.key(KeyCode::Enter);
    h.key(KeyCode::Char('a'));
    h.key(KeyCode::Char(' '));
    let sent = h.sent();
    assert!(
        matches!(sent[..], [Command::Play { index: 1, .. }, Command::Append { .. }, Command::Toggle]),
        "{w}x{rows}: {sent:?}"
    );
    // 播放后：列表里正在播放的那首有 ♪，光标在另一行时两者同时可见。
    let playing = PlayerState { track: Some(Track { mid: "m2".into(), ..track(2) }), ..h.app.state.clone() };
    h.player(Event::State(Box::new(playing)));
    h.key(KeyCode::Char('j'));
    let screen = h.render(w, rows);
    let marked: Vec<&str> = screen.lines().filter(|l| l.contains("♪ ") && l.contains("第1页-1")).collect();
    let selected: Vec<&str> = screen.lines().filter(|l| l.contains(" => ") && l.contains("第1页-2")).collect();
    assert!(marked.len() == 1 && selected.len() == 1, "{w}x{rows}\n{screen}");
    // 帮助可以打开，并能滚到最后一项。
    h.key(KeyCode::Char('?'));
    assert!(h.render(w, rows).contains("帮助"), "{w}x{rows}");
    h.key(KeyCode::Char('G'));
    assert!(h.render(w, rows).contains("退出并停止后台播放"), "{w}x{rows}\n{}", h.render(w, rows));
    h.key(KeyCode::Esc);
    h.key(KeyCode::Esc);
    assert!(!matches!(h.app.page().source, Source::Search { .. }), "{w}x{rows}: 返回");
    h.search("晴天").await;
    h.render(w, rows)
}

#[tokio::test]
async fn core_flow_works_at_common_terminal_sizes() {
    for (w, rows) in [(120, 36), (80, 24), (60, 20)] {
        let screen = core_flow(w, rows).await;
        if std::env::var_os("QQM_PRINT_SCREENS").is_some() {
            println!("===== TestBackend 测试渲染 {w}x{rows}（不是运行截图）=====\n{screen}\n");
        }
    }
}

#[tokio::test]
async fn tiny_terminals_never_panic() {
    let mut h = Harness::logged_in();
    search_server(&h.mock);
    h.settle().await;
    h.connect(30, Some(4), Status::Playing);
    let lines = lyric::parse("[00:50.00]故事的小黄花\n[01:00.00]从出生那年就飘着", "[01:00.00]translation");
    h.app.handle_msg(Msg::Lyric { mid: "m5".into(), result: Ok(lines) });
    h.search("晴天").await;
    let sizes = || (1..=12u16).chain([20, 40, 61, 99, 100, 119, 120, 200]);
    let all = |h: &Harness| {
        for w in sizes() {
            for rows in (1..=8u16).chain([14, 24, 50]) {
                render(&h.app, w, rows);
            }
        }
    };
    all(&h);
    h.key(KeyCode::Char('o'));
    all(&h);
    h.key(KeyCode::Esc);
    h.key(KeyCode::Char('/'));
    h.app.handle_paste("很长的关键词很长的关键词很长的关键词");
    all(&h);
    h.key(KeyCode::Esc);
    for key in ['?', 'e'] {
        h.key(KeyCode::Char(key));
        all(&h);
        h.key(KeyCode::Esc);
    }
    for key in ['c', 'L'] {
        h.key(KeyCode::Char(key));
        all(&h);
    }
    h.app.push("账号".into(), Source::Account);
    all(&h);
    h.app.error(ErrorKind::Network, "一条很长很长很长很长很长很长很长很长很长很长很长很长很长很长的错误");
    all(&h);
}

#[tokio::test]
async fn browsing_does_not_disturb_playback_and_quit_keys_keep_their_meaning() {
    let mut h = Harness::new();
    search_server(&h.mock);
    h.connect(5, Some(1), Status::Playing);
    // 搜索、翻页、进出页面、看队列和歌词：都不向后台发任何播放指令。
    h.search("晴天").await;
    h.keys("jjkGg");
    h.ctrl('d');
    h.key(KeyCode::Char('t'));
    h.settle().await;
    h.key(KeyCode::Esc);
    for key in ['c', 'j', 'L', '?'] {
        h.key(KeyCode::Char(key));
    }
    h.key(KeyCode::Esc);
    assert!(h.sent().is_empty(), "浏览不影响后台播放");

    // q：只退出界面；Q：退出并停止后台。
    h.key(KeyCode::Char('q'));
    assert_eq!(h.app.quit, Some(false));
    assert!(h.sent().is_empty(), "q 不停止播放");
    h.app.quit = None;
    h.key(KeyCode::Char('Q'));
    assert_eq!(h.app.quit, Some(true));

    // 重新打开界面：接上后台当前的队列与状态。
    let mut reopened = Harness::new();
    reopened.connect(5, Some(1), Status::Playing);
    assert_eq!(reopened.app.queue.len(), 5);
    let screen = reopened.render(120, 36);
    assert!(screen.contains("♫ ♪ ♫ ♪") && screen.contains("歌曲 2"), "{screen}");
}

#[tokio::test]
async fn recommendation_pages_pass_their_cursor_to_the_daemon() {
    use crate::protocol::RecommendationSource;
    let mut h = Harness::new();
    on(&h.mock, "get_radar_recommend", |_| ok(json!({})));
    h.mock.route(
        |r| param_of(r).get("Page").is_some(),
        |_| ok(json!({"VecSongs": [{"Track": song_json(1, "雷达一")}, {"Track": song_json(2, "雷达二")}], "HasMore": true})),
    );
    h.connect(0, None, Status::Stopped);
    h.app.push("雷达推荐".into(), Source::Radar);
    h.settle().await;
    assert_eq!(h.songs(), 2);
    h.key(KeyCode::Enter);
    let sent = h.sent();
    assert!(
        matches!(sent[..], [Command::Play { recommendation: Some(RecommendationSource::Radar { next_page: 3 }), .. }]),
        "{sent:?}"
    );
    assert_eq!(h.infos().last().copied(), Some("播放本页 2 首，已替换队列"));
}

#[tokio::test]
async fn login_popup_wraps_long_text_and_hides_a_dead_qr_code() {
    let mut h = Harness::new();
    // 登录弹窗：窄屏下长路径折行显示完整；失败后不再显示旧二维码。
    let qr = vec![Line::from("█▀▀▀▀▀█ QR"); 10];
    let path = format!("{}/qrcode/qq-5e024ef5-1f74-435f-9b33-c1abac5cbac1.png", h.dir.path().display());
    h.app.mode = Mode::Login(LoginView { kind: QrLoginType::Qq, qr, path: Some(path), status: String::new() });
    let screen = h.render(60, 30);
    assert!(screen.contains("█▀▀▀▀▀█ QR") && screen.contains("cbac1.png"), "{screen}");
    assert!(!screen.contains("首页"), "{screen}");
    h.app.handle_msg(Msg::Login(LoginMsg::Failed("登录失败: 登录设备已达上限，请在 QQ 音乐 App 中处理".into())));
    let screen = h.render(40, 30);
    assert!(!screen.contains("QR") && !screen.contains("二维码图片"), "{screen}");
    assert!(screen.contains("设备已达上限") && screen.contains("App 中处理"), "{screen}");
}
