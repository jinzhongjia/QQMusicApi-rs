//! TUI 状态与交互逻辑。

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use qqmusic_api::error::LoginErrorKind;
use qqmusic_api::models::login::{QrCodeLoginEvent, QrLoginType};
use qqmusic_api::modules::lyric::LyricOptions;
use qqmusic_api::modules::song::Quality;
use qqmusic_api::modules::songlist::SonglistWrite;
use qqmusic_api::{ApiErrorKind, Client, Credential, Error};
use ratatui::text::Line;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::data::{self, Action, Batch, Failure, Feed, Item, SearchKind, Source};
use super::input::TextInput;
use super::lyric::{self, LyricLine};
use super::qr;
use crate::paths::Paths;
use crate::protocol::{
    Activity, Command, ErrorKind, Event, PROTOCOL_VERSION, PlayerState, RecommendationSource, Status, Track,
    quality_label,
};

/// 选中项距离已加载内容末尾不足这么多行时继续加载。
const LOAD_AHEAD: usize = 15;
/// 搜索补全的防抖时间。
const SUGGEST_DEBOUNCE: Duration = Duration::from_millis(300);
/// 保留的搜索结果页数（切换类型、换关键词后可以原样恢复）。
const SEARCH_CACHE: usize = 6;
/// 通知列表的长度上限。
const NOTICE_LIMIT: usize = 100;
/// 普通提示在底栏停留的时间。
pub const TOAST_SECS: u64 = 4;

/// 发给主循环的消息。
pub enum Msg {
    Player(Event),
    Disconnected,
    /// 断线期间积压、重连后被丢弃的指令数。
    Dropped(usize),
    Page {
        id: u64,
        generation: u64,
        result: Result<data::Opened, Failure>,
    },
    More {
        id: u64,
        generation: u64,
        feed: Feed,
        result: Option<Result<Batch, Failure>>,
    },
    Lyric {
        mid: String,
        result: Result<Vec<LyricLine>, Failure>,
    },
    Liked {
        account: u64,
        result: Result<HashSet<i64>, Failure>,
    },
    Nick {
        account: u64,
        name: String,
    },
    LikeDone {
        account: u64,
        track: Box<Track>,
        target: bool,
        result: Result<SonglistWrite, Failure>,
    },
    Suggest {
        generation: u64,
        items: Vec<String>,
    },
    Session {
        account: u64,
        session: Session,
    },
    Login(LoginMsg),
    Error(ErrorKind, String),
}

/// 登录流程消息。
pub enum LoginMsg {
    Qr {
        lines: Vec<Line<'static>>,
        path: Option<String>,
    },
    Status(String),
    /// 扫码登录成功（可能换了账号）。
    Done(Box<Credential>),
    /// 同一账号的凭证已刷新。
    Refreshed {
        account: u64,
        credential: Box<Credential>,
    },
    Failed(String),
}

/// 登录状态：区分「凭证已保存」与「服务端确认有效」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    LoggedOut,
    /// 有凭证，正在向服务端确认。
    Checking,
    /// 服务端确认有效。
    Valid,
    /// 有凭证，但暂时无法确认（多为网络问题）；不等于退出登录。
    Unverified(String),
    /// 登录已过期，需要重新登录。
    Expired,
}

/// 续页状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum More {
    /// 还有后续内容，尚未请求。
    Idle,
    Loading,
    /// 续页失败：已加载的内容保留，按 r 重试。
    Failed(String),
    /// 没有更多，或已达到上限。
    Done,
}

/// 一级页面。
pub struct Page {
    pub id: u64,
    pub title: String,
    pub source: Source,
    pub items: Vec<Item>,
    pub selected: usize,
    /// 列表滚动位置（渲染时调整以保证选中项可见）。
    pub offset: Cell<usize>,
    /// 首次加载 / 刷新进行中。
    pub loading: bool,
    /// 首次加载 / 刷新失败的原因；已有内容时旧内容保留。
    pub error: Option<String>,
    /// 每次加载递增，旧请求的结果据此丢弃。
    generation: u64,
    feed: Option<Feed>,
    pub more: More,
    /// 服务端报告的总数。
    pub total: Option<usize>,
    /// 已达到 [`data::MAX_ITEMS`]，没有继续加载。
    pub capped: bool,
    pub recommendation: Option<RecommendationSource>,
}

impl Page {
    /// 页面里的歌曲数。
    pub fn song_count(&self) -> usize {
        self.items.iter().filter(|i| matches!(i, Item::Song(_))).count()
    }

    /// 是否还有没加载的内容。
    pub fn incomplete(&self) -> bool {
        self.capped || !matches!(self.more, More::Done)
    }
}

/// 登录弹窗。
pub struct LoginView {
    pub kind: QrLoginType,
    pub qr: Vec<Line<'static>>,
    pub path: Option<String>,
    pub status: String,
}

/// 搜索框。
pub struct SearchBox {
    pub input: TextInput,
    pub kind: SearchKind,
    /// 补全建议；输入为空时是搜索历史。
    pub options: Vec<String>,
    pub picked: Option<usize>,
    pub from_history: bool,
}

/// 菜单项触发的动作。
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    PlayReplace,
    PlayNext(Box<Track>),
    Append(Box<Track>),
    ToggleLike(Box<Track>),
    OpenAlbum { mid: String, name: String },
    OpenSinger { mid: String, name: String },
    PickSinger(Box<Track>),
    RemoveFromQueue(usize),
    SetQuality(Quality),
    ClearQueue,
    Cancel,
}

/// 选择菜单（歌曲操作、音质、确认）。
pub struct Menu {
    pub title: String,
    pub items: Vec<(String, MenuAction)>,
    pub selected: usize,
}

/// 输入 / 弹窗模式。弹窗打开时按键只交给弹窗。
pub enum Mode {
    Normal,
    Search(SearchBox),
    Menu(Menu),
    Login(LoginView),
    Help { scroll: usize },
    Notices { scroll: usize },
}

/// 扫码登录失败的提示：只有重试可能成功时才提示按 r。
fn login_failure(e: &Error, kind: QrLoginType) -> String {
    let Error::Api(api) = e else {
        return format!("登录失败: {e}，按 r 重试");
    };
    match api.kind {
        ApiErrorKind::Login(LoginErrorKind::DeviceLimit) => {
            let alt = if kind == QrLoginType::Mobile { "" } else { "，或改用 QQ 音乐 App 扫码登录" };
            format!("登录失败: 登录设备已达上限，请在 QQ 音乐 App 的登录设备管理中移除不用的设备后按 r 重试{alt}")
        }
        ApiErrorKind::Login(LoginErrorKind::AccountRestricted) => {
            format!("登录失败: {}，请在 QQ 音乐 App 中处理后再登录", api.message)
        }
        ApiErrorKind::Login(LoginErrorKind::RateLimit) | ApiErrorKind::Ratelimited { .. } => {
            format!("登录失败: {}，请稍后再按 r 重试", api.message)
        }
        _ => format!("登录失败: {e}，按 r 重试"),
    }
}

pub const QUALITIES: [Quality; 3] = [Quality::Lossless, Quality::High, Quality::Standard];

/// 当前接收按键的视图，由当前页面决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Queue,
    Lyrics,
}

/// 渲染时根据窗口大小算出的布局，按键处理据此决定翻页步长。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// 列表一屏的行数。
    pub rows: usize,
}

/// 歌词加载状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LyricState {
    /// 没有正在播放的歌曲。
    Idle,
    Loading,
    Ready(Vec<LyricLine>),
    /// 这首歌确实没有歌词。
    Empty,
    /// 加载失败，可重试。
    Failed(String),
}

/// 通知级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

/// 一条通知。
pub struct Notice {
    pub level: Level,
    pub text: String,
    pub at: Instant,
}

/// 按错误分类给出的恢复提示。
fn recovery(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Login => " → 请到「账号」重新登录",
        ErrorKind::Rights => " → 当前账号无法播放，可换一首或更换账号",
        ErrorKind::Network => " → 网络或服务暂时不可用，稍后按 r 或 Space 重试",
        ErrorKind::Local => "",
        ErrorKind::Other => "",
    }
}

/// 断线时仍然接受的指令：设置类只保留最新意图，重连后生效；其余一律拒绝，避免重连后突然执行。
pub fn allowed_offline(command: &Command) -> bool {
    matches!(
        command,
        Command::SetVolume { .. }
            | Command::SetMode { .. }
            | Command::SetQuality { .. }
            | Command::ReloadCredential { .. }
            | Command::Hello { .. }
            | Command::GetQueue
            | Command::Shutdown
    )
}

pub struct App {
    pub viewport: Cell<Viewport>,
    pub paths: Paths,
    pub client: Client,
    tx: UnboundedSender<Msg>,
    commands: UnboundedSender<Command>,
    pub stack: Vec<Page>,
    next_id: u64,
    pub mode: Mode,
    pub state: PlayerState,
    state_at: Instant,
    pub queue: Vec<Track>,
    /// 与 `queue` 一一对应的条目 id；旧版守护进程不提供时为空。
    queue_ids: Vec<u64>,
    queue_version: Option<u64>,
    pub queue_sel: usize,
    pub queue_offset: Cell<usize>,
    /// 帮助 / 通知弹窗可滚动的最大行数（渲染时写入）。
    pub scroll_max: Cell<usize>,
    pub lyric: LyricState,
    lyric_mid: String,
    /// 歌词视图里手动选中的行；`None` 表示跟随播放位置。
    pub lyric_sel: Option<usize>,
    pub show_trans: bool,
    pub liked: HashSet<i64>,
    /// 喜欢 / 取消喜欢的请求尚未返回的歌曲。
    like_pending: HashSet<i64>,
    /// 每次登录状态变化递增，旧账号的请求结果据此丢弃。
    account: u64,
    pub nick: Option<String>,
    pub session: Session,
    pub notices: VecDeque<Notice>,
    pub toast: Option<(String, Instant)>,
    /// 尚未关闭的错误提示（不会自动消失，也不会被普通提示顶掉）。
    pub alert: Option<String>,
    /// 还没看过的错误数。
    pub unread: usize,
    pub connected: bool,
    was_connected: bool,
    old_daemon: bool,
    /// 已发给后台、等待确认的操作：tag → 确认后显示的文字。
    pending: HashMap<u64, String>,
    next_tag: u64,
    pub history: Vec<String>,
    search_kind: SearchKind,
    search_cache: Vec<Page>,
    suggest_generation: u64,
    suggest_task: Option<JoinHandle<()>>,
    /// `Some(stop_player)` 表示退出。
    pub quit: Option<bool>,
    login_task: Option<JoinHandle<()>>,
}

impl App {
    pub fn new(paths: Paths, client: Client, tx: UnboundedSender<Msg>, commands: UnboundedSender<Command>) -> Self {
        let history = data::load_history(&paths);
        let mut app = Self {
            viewport: Cell::new(Viewport { rows: 10 }),
            paths,
            client,
            tx,
            commands,
            stack: Vec::new(),
            next_id: 0,
            mode: Mode::Normal,
            state: PlayerState::default(),
            state_at: Instant::now(),
            queue: Vec::new(),
            queue_ids: Vec::new(),
            queue_version: None,
            queue_sel: 0,
            queue_offset: Cell::new(0),
            scroll_max: Cell::new(0),
            lyric: LyricState::Idle,
            lyric_mid: String::new(),
            lyric_sel: None,
            show_trans: true,
            liked: HashSet::new(),
            like_pending: HashSet::new(),
            account: 0,
            nick: None,
            session: Session::LoggedOut,
            notices: VecDeque::new(),
            toast: None,
            alert: None,
            unread: 0,
            connected: false,
            was_connected: false,
            old_daemon: false,
            pending: HashMap::new(),
            // 随机起点，避免与其他客户端的确认标记相撞。
            next_tag: fastrand::u64(..u64::MAX / 2),
            history,
            search_kind: SearchKind::Song,
            search_cache: Vec::new(),
            suggest_generation: 0,
            suggest_task: None,
            quit: None,
            login_task: None,
        };
        app.push("首页".into(), Source::Root);
        app.on_login_changed();
        app
    }

    // ---- 通知 ----

    /// 普通提示：底栏显示几秒，同时记入通知列表。
    pub fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        self.toast = Some((text.clone(), Instant::now()));
        self.note(Level::Info, text);
    }

    /// 错误：附上恢复提示，留在底栏直到用户关闭。
    pub fn error(&mut self, kind: ErrorKind, text: impl Into<String>) {
        let text = format!("{}{}", text.into(), recovery(kind));
        if kind == ErrorKind::Login && self.logged_in() && self.session != Session::Expired {
            self.session = Session::Expired;
            self.refresh_local(&Source::Account);
        }
        // 短时间内重复的同一条错误只记一次。
        let last = self.notices.iter().rev().find(|n| n.level == Level::Error);
        if last.is_none_or(|n| n.text != text || n.at.elapsed() > Duration::from_secs(30)) {
            self.unread += 1;
            self.note(Level::Error, text.clone());
        }
        self.alert = Some(text);
    }

    fn note(&mut self, level: Level, text: String) {
        self.notices.push_back(Notice { level, text, at: Instant::now() });
        if self.notices.len() > NOTICE_LIMIT {
            self.notices.pop_front();
        }
    }

    // ---- 与后台通信 ----

    /// 发送指令。断线时除设置类指令外一律拒绝并提示，不留到重连后执行。
    fn send(&mut self, command: Command) -> bool {
        if !self.connected && !allowed_offline(&command) {
            self.error(ErrorKind::Local, "后台播放进程未连接，操作没有执行（重连后请重试）");
            return false;
        }
        self.commands.send(command).is_ok()
    }

    /// 为需要确认的操作分配标记；`done` 在后台确认后显示。旧版后台不支持确认时返回 `None`。
    fn tagged(&mut self, done: String) -> Option<u64> {
        if !self.connected || self.state.protocol < 2 {
            return None;
        }
        self.next_tag += 1;
        self.pending.insert(self.next_tag, done);
        Some(self.next_tag)
    }

    /// 发送带确认的操作：先提示「处理中」，确认后才提示完成。
    fn send_tagged(&mut self, sending: String, done: String, build: impl FnOnce(Option<u64>) -> Command) {
        let tag = self.tagged(done);
        if !self.send(build(tag)) {
            if let Some(tag) = tag {
                self.pending.remove(&tag);
            }
            return;
        }
        match tag {
            Some(_) => self.info(format!("{sending}…")),
            None => self.info(format!("{sending}（已发送，后台版本较旧，无法确认结果）")),
        }
    }

    pub fn logged_in(&self) -> bool {
        self.client.credential().is_valid()
    }

    /// 估算的当前播放位置（两次状态推送之间插值；加载、缓冲、断线时不前进）。
    pub fn position_ms(&self) -> u64 {
        let mut position = self.state.position_ms;
        let advancing = self.connected
            && self.state.status == Status::Playing
            && (self.state.activity == Activity::Output || self.state.protocol < 2);
        if advancing {
            position += self.state_at.elapsed().as_millis() as u64;
        }
        if self.state.duration_ms > 0 { position.min(self.state.duration_ms) } else { position }
    }

    // ---- 页面栈 ----

    pub fn page(&self) -> &Page {
        self.stack.last().expect("page stack is never empty")
    }

    fn page_mut(&mut self) -> &mut Page {
        self.stack.last_mut().expect("page stack is never empty")
    }

    fn page_by_id(&mut self, id: u64, generation: u64) -> Option<&mut Page> {
        self.stack.iter_mut().chain(self.search_cache.iter_mut()).find(|p| p.id == id && p.generation == generation)
    }

    fn bump(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    /// 当前接收按键的视图。
    pub fn view(&self) -> View {
        match self.page().source {
            Source::Queue => View::Queue,
            Source::Lyrics => View::Lyrics,
            _ => View::List,
        }
    }

    fn account_items(&self) -> Vec<Item> {
        let login = |title: &str, kind| Item::Action { title: title.into(), action: Action::Login(kind) };
        let logins = [
            login("QQ 扫码登录", QrLoginType::Qq),
            login("微信扫码登录", QrLoginType::Wx),
            login("QQ 音乐 App 扫码登录", QrLoginType::Mobile),
        ];
        if !self.logged_in() {
            return logins.into();
        }
        let credential = self.client.credential();
        let nick = self.nick.clone().unwrap_or_default();
        let status = match &self.session {
            Session::LoggedOut => "未登录".to_string(),
            Session::Checking => "正在确认登录状态…".to_string(),
            Session::Valid => "登录有效".to_string(),
            Session::Unverified(reason) => format!("凭证已保存，但暂时无法确认是否有效：{reason}"),
            Session::Expired => "登录已过期，请重新登录".to_string(),
        };
        let mut items = vec![
            Item::Text(format!("已登录：{nick} ({})", credential.musicid)),
            Item::Text(status),
            Item::Action { title: "重新确认登录状态".into(), action: Action::CheckLogin },
            Item::Action { title: "退出登录".into(), action: Action::Logout },
        ];
        if self.session == Session::Expired {
            items.push(Item::Text("— 重新登录 —".into()));
            items.extend(logins);
        }
        items
    }

    /// 顶栏显示的账号状态。
    pub fn account_label(&self) -> String {
        let nick = self.nick.clone().unwrap_or_else(|| "已登录".into());
        match &self.session {
            _ if !self.logged_in() => "未登录".into(),
            Session::Checking => format!("{nick}（确认中）"),
            Session::Unverified(_) => format!("{nick}（登录未确认）"),
            Session::Expired => format!("{nick}（登录已过期）"),
            _ => nick,
        }
    }

    fn local_items(&self, source: &Source) -> Vec<Item> {
        match source {
            Source::Root => data::root_items(),
            Source::Account => self.account_items(),
            _ => Vec::new(),
        }
    }

    /// 打开一个页面（标题原样使用）。
    pub(super) fn push(&mut self, title: String, source: Source) {
        let id = self.bump();
        let items = self.local_items(&source);
        let page = Page {
            id,
            title,
            selected: first_selectable(&items, 0),
            offset: Cell::new(0),
            items,
            source,
            loading: false,
            error: None,
            generation: 0,
            feed: None,
            more: More::Done,
            total: None,
            capped: false,
            recommendation: None,
        };
        self.stack.push(page);
        self.reload();
    }

    /// 从当前页进入下一级：标题带上来路。
    fn push_child(&mut self, title: &str, source: Source) {
        let title = if self.stack.len() > 1 { format!("{} › {title}", self.page().title) } else { title.to_string() };
        self.push(title, source);
    }

    /// 重新加载当前页。刷新期间与刷新失败后，已有内容都保留。
    fn reload(&mut self) {
        let source = self.page().source.clone();
        if !source.is_remote() {
            return self.refresh_local(&source);
        }
        let generation = self.bump();
        let page = self.page_mut();
        page.generation = generation;
        page.loading = true;
        page.error = None;
        page.feed = None;
        page.more = More::Done;
        let id = page.id;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = data::open(&client, &source).await;
            let _ = tx.send(Msg::Page { id, generation, result });
        });
    }

    /// 选中项接近已加载内容的末尾时继续加载下一批。
    fn maybe_load_more(&mut self) {
        let page = self.page();
        if page.more == More::Idle && page.selected + LOAD_AHEAD >= page.items.len() {
            self.load_more();
        }
    }

    fn load_more(&mut self) {
        let page = self.page_mut();
        if page.loading || matches!(page.more, More::Loading | More::Done) {
            return;
        }
        let Some(mut feed) = page.feed.take() else {
            page.more = More::Done;
            return;
        };
        page.more = More::Loading;
        let (id, generation) = (page.id, page.generation);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = feed.next().await;
            let _ = tx.send(Msg::More { id, generation, feed, result });
        });
    }

    fn refresh_local(&mut self, source: &Source) {
        let items = self.local_items(source);
        for page in self.stack.iter_mut().filter(|p| &p.source == source) {
            page.selected = first_selectable(&items, page.selected.min(items.len().saturating_sub(1)));
            page.items = items.clone();
        }
    }

    // ---- 登录 ----

    /// 登录状态变化：使旧账号的请求失效，丢弃随账号变化的页面，刷新昵称与喜欢列表。
    fn on_login_changed(&mut self) {
        self.account += 1;
        self.nick = None;
        self.liked.clear();
        self.like_pending.clear();
        self.stack.retain(|p| !p.source.is_personal());
        self.search_cache.clear();
        self.session = if self.logged_in() { Session::Checking } else { Session::LoggedOut };
        self.refresh_local(&Source::Account);
        if !self.logged_in() {
            return;
        }
        let client = self.client.clone();
        let tx = self.tx.clone();
        let account = self.account;
        tokio::spawn(async move {
            let credential = client.credential();
            if let Ok(home) = client.user().get_homepage(&credential.encrypt_uin, None).await {
                let _ = tx.send(Msg::Nick { account, name: home.base_info.name });
            }
            let result = data::liked_ids(&client).await;
            let _ = tx.send(Msg::Liked { account, result });
        });
    }

    /// 向服务端确认登录是否有效；过期时尝试刷新。网络失败不等于退出登录。
    pub fn check_session(&mut self) {
        if !self.logged_in() {
            self.session = Session::LoggedOut;
            return self.refresh_local(&Source::Account);
        }
        self.session = Session::Checking;
        self.refresh_local(&Source::Account);
        let client = self.client.clone();
        let tx = self.tx.clone();
        let account = self.account;
        tokio::spawn(async move {
            let unsure = |e: Error| {
                let failure = Failure::from(e);
                match failure.kind {
                    ErrorKind::Login => Session::Expired,
                    _ => Session::Unverified(failure.message),
                }
            };
            let session = match client.login().check_expired(None).await {
                Ok(false) => Session::Valid,
                Ok(true) => match client.login().refresh_credential(None).await {
                    Ok(credential) => {
                        let credential = Box::new(credential);
                        let _ = tx.send(Msg::Login(LoginMsg::Refreshed { account, credential }));
                        return;
                    }
                    // 刷新被服务端拒绝才算过期；请求没发出去只是「未确认」。
                    Err(e @ (Error::Network(_) | Error::Http { .. })) => unsure(e),
                    Err(_) => Session::Expired,
                },
                Err(e) => unsure(e),
            };
            let _ = tx.send(Msg::Session { account, session });
        });
    }

    /// 把凭证落盘并让后台重新读取；任何一步失败都如实报告，不算完整成功。
    fn persist_credential(&mut self, credential: &Credential) -> bool {
        if let Err(e) = crate::save_credential(&self.paths, credential) {
            self.error(
                ErrorKind::Local,
                format!(
                    "登录凭证未能保存（{e}）：本次界面可以继续使用，但重启后需要重新登录，后台播放仍是原来的登录状态"
                ),
            );
            return false;
        }
        self.reload_daemon_credential("后台播放已同步登录状态");
        true
    }

    fn reload_daemon_credential(&mut self, done: &str) {
        let tag = self.tagged(done.to_string());
        let _ = self.commands.send(Command::ReloadCredential { tag });
        if !self.connected {
            self.info("后台播放进程未连接，登录状态将在重连后同步");
        }
    }

    fn on_login_msg(&mut self, msg: LoginMsg) {
        match msg {
            LoginMsg::Qr { lines, path } => {
                if let Mode::Login(view) = &mut self.mode {
                    view.qr = lines;
                    view.path = path;
                    view.status = "请扫描二维码".into();
                }
            }
            LoginMsg::Status(status) => {
                if let Mode::Login(view) = &mut self.mode {
                    view.status = status;
                }
            }
            LoginMsg::Done(credential) => {
                self.client.set_credential((*credential).clone());
                let saved = self.persist_credential(&credential);
                if matches!(self.mode, Mode::Login(_)) {
                    self.mode = Mode::Normal;
                }
                self.on_login_changed();
                // 刚从服务端拿到的凭证，视为有效。
                self.session = Session::Valid;
                self.refresh_local(&Source::Account);
                if saved {
                    self.info("登录成功");
                }
            }
            LoginMsg::Refreshed { account, credential } => {
                if account != self.account {
                    return;
                }
                self.persist_credential(&credential);
                self.session = Session::Valid;
                self.refresh_local(&Source::Account);
            }
            LoginMsg::Failed(e) => {
                if let Mode::Login(view) = &mut self.mode {
                    // 二维码已失效，不再显示，免得被误以为还能扫。
                    view.qr.clear();
                    view.path = None;
                    view.status = e;
                } else {
                    self.error(ErrorKind::Login, e);
                }
            }
        }
    }

    fn start_login(&mut self, kind: QrLoginType) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.mode =
            Mode::Login(LoginView { kind, qr: Vec::new(), path: None, status: "正在获取二维码…".into() });
        let client = self.client.clone();
        let tx = self.tx.clone();
        let dir = self.paths.qrcode_dir();
        self.login_task = Some(tokio::spawn(async move {
            let send = |msg: LoginMsg| {
                let _ = tx.send(Msg::Login(msg));
            };
            let mut session = client.login().qrcode_session(kind).timeout(Duration::from_secs(180));
            let qrcode = match session.get_qrcode().await {
                Ok(qrcode) => qrcode,
                Err(e) => return send(LoginMsg::Failed(format!("获取二维码失败: {e}"))),
            };
            let path = qrcode.save(&dir).ok().flatten().map(|p| p.display().to_string());
            let lines = qr::modules(&qrcode.data).map(|grid| qr::render(&grid)).unwrap_or_default();
            send(LoginMsg::Qr { lines, path });
            let mut events = match session.events().await {
                Ok(events) => events,
                Err(e) => return send(LoginMsg::Failed(format!("登录失败: {e}"))),
            };
            while let Some(result) = events.next().await {
                match result {
                    Ok(result) => match result.event {
                        QrCodeLoginEvent::Done => {
                            if let Some(credential) = result.credential {
                                return send(LoginMsg::Done(Box::new(credential)));
                            }
                        }
                        QrCodeLoginEvent::Scan => send(LoginMsg::Status("请扫描二维码".into())),
                        QrCodeLoginEvent::Conf => send(LoginMsg::Status("已扫码，请在手机上确认".into())),
                        QrCodeLoginEvent::Timeout => return send(LoginMsg::Failed("二维码已过期，按 r 重试".into())),
                        QrCodeLoginEvent::Refuse => return send(LoginMsg::Failed("已在手机上拒绝登录".into())),
                    },
                    Err(e) => return send(LoginMsg::Failed(login_failure(&e, kind))),
                }
            }
        }));
    }

    fn logout(&mut self) {
        // 凭证删不掉就不算退出：否则下次启动又会「自动登录」。
        if let Err(e) = crate::remove_credential(&self.paths) {
            self.error(ErrorKind::Local, format!("退出登录失败：无法删除凭证文件（{e}），登录状态没有改变"));
            return;
        }
        let client = self.client.clone();
        let credential = client.credential();
        tokio::spawn(async move {
            let _ = client.login().logout(Some(credential)).await;
        });
        self.client.set_credential(Credential::default());
        self.reload_daemon_credential("后台播放已退出登录");
        self.on_login_changed();
        self.info("已退出登录");
    }

    /// 另一个界面改了登录状态（后台重新读取凭证后广播）：与本界面不一致时跟上。
    fn on_credential_changed(&mut self, logged_in: bool, musicid: i64) {
        let mine = self.client.credential();
        if mine.is_valid() == logged_in && (!logged_in || mine.musicid == musicid) {
            return;
        }
        match crate::read_credential(&self.paths) {
            Ok(credential) => {
                self.client.set_credential(credential.unwrap_or_default());
                self.on_login_changed();
                self.check_session();
                self.info("登录状态已在另一个界面更改，已同步");
            }
            Err(e) => self.error(ErrorKind::Local, format!("登录状态已在别处更改，但读取凭证失败: {e}")),
        }
    }

    // ---- 消息处理 ----

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Player(event) => self.on_player_event(event),
            Msg::Disconnected => {
                let was = self.connected;
                self.connected = false;
                self.queue_version = None;
                self.old_daemon = false;
                if !self.pending.is_empty() {
                    let count = self.pending.len();
                    self.pending.clear();
                    self.error(ErrorKind::Local, format!("连接断开，{count} 个操作没有得到后台确认，请检查后重试"));
                }
                if was {
                    self.error(ErrorKind::Local, "与后台播放进程的连接已断开，正在重连…（期间的播放操作不会执行）");
                }
            }
            Msg::Dropped(count) => {
                self.error(ErrorKind::Local, format!("断线期间的 {count} 个操作已丢弃，没有补发"));
            }
            Msg::Page { id, generation, result } => self.on_page(id, generation, result),
            Msg::More { id, generation, feed, result } => self.on_more(id, generation, feed, result),
            Msg::Lyric { mid, result } => {
                if mid == self.lyric_mid {
                    self.lyric = match result {
                        Ok(lines) if lines.is_empty() => LyricState::Empty,
                        Ok(lines) => LyricState::Ready(lines),
                        Err(failure) => LyricState::Failed(failure.message),
                    };
                }
            }
            Msg::Liked { account, result } => {
                if account != self.account {
                    return;
                }
                match result {
                    Ok(ids) => {
                        // 请求期间确认过的操作以本地为准。
                        let pending: Vec<(i64, bool)> =
                            self.like_pending.iter().map(|id| (*id, self.liked.contains(id))).collect();
                        self.liked = ids;
                        for (id, liked) in pending {
                            if liked {
                                self.liked.insert(id);
                            } else {
                                self.liked.remove(&id);
                            }
                        }
                    }
                    Err(failure) => self.error(failure.kind, format!("获取喜欢列表失败: {}", failure.message)),
                }
            }
            Msg::Nick { account, name } => {
                if account == self.account {
                    self.nick = Some(name);
                    self.refresh_local(&Source::Account);
                }
            }
            Msg::LikeDone { account, track, target, result } => self.on_like_done(account, *track, target, result),
            Msg::Suggest { generation, items } => {
                if generation == self.suggest_generation
                    && let Mode::Search(search) = &mut self.mode
                    && !search.input.is_empty()
                {
                    search.options = items;
                    search.picked = None;
                    search.from_history = false;
                }
            }
            Msg::Session { account, session } => {
                if account != self.account {
                    return;
                }
                match &session {
                    Session::Expired => self.error(ErrorKind::Login, "登录已过期"),
                    Session::Unverified(reason) => {
                        self.info(format!("暂时无法确认登录状态（{reason}），已保留登录"));
                    }
                    _ => {}
                }
                self.session = session;
                self.refresh_local(&Source::Account);
            }
            Msg::Login(msg) => self.on_login_msg(msg),
            Msg::Error(kind, message) => self.error(kind, message),
        }
    }

    fn on_page(&mut self, id: u64, generation: u64, result: Result<data::Opened, Failure>) {
        // 页面已关闭，或之后又刷新过：旧结果丢弃。
        let Some(page) = self.page_by_id(id, generation) else { return };
        page.loading = false;
        match result {
            Ok(opened) => {
                page.items = opened.items;
                page.total = opened.total;
                page.recommendation = opened.recommendation;
                page.capped = page.items.len() >= data::MAX_ITEMS && opened.feed.is_some();
                page.items.truncate(data::MAX_ITEMS);
                page.more = if opened.feed.is_some() && !page.capped { More::Idle } else { More::Done };
                page.feed = opened.feed.filter(|_| !page.capped);
                page.selected = first_selectable(&page.items, page.selected.min(page.items.len().saturating_sub(1)));
                self.maybe_load_more();
            }
            Err(failure) => {
                page.error = Some(failure.message.clone());
                if failure.kind == ErrorKind::Login {
                    self.error(failure.kind, failure.message);
                }
            }
        }
    }

    fn on_more(&mut self, id: u64, generation: u64, feed: Feed, result: Option<Result<Batch, Failure>>) {
        let Some(page) = self.page_by_id(id, generation) else { return };
        match result {
            None => page.more = More::Done,
            Some(Ok(batch)) => {
                page.items.extend(batch.items);
                page.total = batch.total.or(page.total);
                if page.items.len() >= data::MAX_ITEMS && feed.has_more() {
                    page.items.truncate(data::MAX_ITEMS);
                    page.capped = true;
                    page.more = More::Done;
                } else if feed.has_more() {
                    page.feed = Some(feed);
                    page.more = More::Idle;
                } else {
                    page.more = More::Done;
                }
            }
            // 续页失败：已加载的内容不动，分页器留着以便重试同一页。
            Some(Err(failure)) => {
                page.feed = Some(feed);
                page.more = More::Failed(failure.message.clone());
                if failure.kind == ErrorKind::Login {
                    self.error(failure.kind, failure.message);
                }
            }
        }
        if self.page().id == id {
            self.maybe_load_more();
        }
    }

    fn on_player_event(&mut self, event: Event) {
        match event {
            Event::State(state) => {
                if !self.connected {
                    self.connected = true;
                    if self.was_connected {
                        self.info("已重新连接后台播放进程");
                    }
                    self.was_connected = true;
                }
                if state.protocol < PROTOCOL_VERSION && !self.old_daemon {
                    self.old_daemon = true;
                    self.error(
                        ErrorKind::Local,
                        "后台播放进程是旧版本，操作确认、队列排序等功能不可用：按 Q 退出后重新打开（或运行 qqm ctl quit）即可更新",
                    );
                }
                let mid = state.track.as_ref().map(|t| t.mid.clone()).unwrap_or_default();
                if mid != self.lyric_mid {
                    self.lyric_mid = mid;
                    self.fetch_lyric();
                }
                if Some(state.queue_version) != self.queue_version {
                    let _ = self.commands.send(Command::GetQueue);
                }
                self.state = *state;
                self.state_at = Instant::now();
            }
            Event::Queue { tracks, version, ids } => {
                let legacy = version == 0 && ids.len() != tracks.len();
                // 比已知版本更旧的快照（多客户端、重连时的乱序）不能把队列改回去。
                if !legacy && self.queue_version.is_some_and(|known| version < known) {
                    return;
                }
                self.queue_version = Some(if legacy { self.state.queue_version } else { version });
                // 选中项跟着条目走，而不是停在原来的下标上。
                let selected = self.queue_ids.get(self.queue_sel).copied();
                self.queue = tracks;
                self.queue_ids = if legacy { Vec::new() } else { ids };
                if let Some(at) = selected.and_then(|id| self.queue_ids.iter().position(|e| *e == id)) {
                    self.queue_sel = at;
                }
                self.queue_sel = self.queue_sel.min(self.queue.len().saturating_sub(1));
            }
            Event::Error { message, kind } => self.error(kind, message),
            Event::Done { tag, ok, message } => {
                // 只认自己发出的标记，别的客户端的确认与本界面无关。
                if let Some(done) = self.pending.remove(&tag) {
                    if ok {
                        self.info(done);
                    } else {
                        self.error(ErrorKind::Other, format!("操作没有完成：{message}"));
                    }
                }
            }
            Event::CredentialChanged { logged_in, musicid } => self.on_credential_changed(logged_in, musicid),
            Event::Shutdown => {
                self.state = PlayerState::default();
                self.connected = false;
                self.queue_version = None;
                self.info("后台播放进程已退出");
            }
        }
    }

    /// 加载当前曲目的歌词（切歌时自动调用，失败后可手动重试）。
    fn fetch_lyric(&mut self) {
        self.lyric_sel = None;
        let mid = self.lyric_mid.clone();
        if mid.is_empty() {
            self.lyric = LyricState::Idle;
            return;
        }
        self.lyric = LyricState::Loading;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let options = LyricOptions { trans: true, ..Default::default() };
            let result = client
                .lyric()
                .get_lyric_with(mid.as_str(), options)
                .await
                .map(|resp| lyric::parse(&resp.lyric, &resp.trans))
                .map_err(Failure::from);
            let _ = tx.send(Msg::Lyric { mid, result });
        });
    }

    pub fn lyrics(&self) -> &[LyricLine] {
        match &self.lyric {
            LyricState::Ready(lines) => lines,
            _ => &[],
        }
    }

    // ---- 喜欢 ----

    fn toggle_like(&mut self, track: Option<Track>) {
        let Some(track) = track else { return };
        if !self.logged_in() {
            return self.error(ErrorKind::Login, "喜欢歌曲需要登录");
        }
        // 上一次请求还没回来：不叠加请求，避免先后顺序不定。
        if self.like_pending.contains(&track.id) {
            return self.info(format!("《{}》的喜欢操作还在处理中", track.name));
        }
        let target = !self.liked.contains(&track.id);
        self.set_liked(track.id, target);
        self.like_pending.insert(track.id);
        let client = self.client.clone();
        let tx = self.tx.clone();
        let account = self.account;
        tokio::spawn(async move {
            let songs = [(track.id, track.song_type)];
            let songlist = client.songlist();
            let result = if target {
                songlist.add_songs_outcome(qqmusic_api::modules::songlist::LIKE_DIRID, &songs, 0, None).await
            } else {
                songlist.del_songs_outcome(qqmusic_api::modules::songlist::LIKE_DIRID, &songs, 0, None).await
            };
            let result = result.map_err(Failure::from);
            let _ = tx.send(Msg::LikeDone { account, track: Box::new(track), target, result });
        });
    }

    fn set_liked(&mut self, id: i64, liked: bool) {
        if liked {
            self.liked.insert(id);
        } else {
            self.liked.remove(&id);
        }
    }

    fn on_like_done(&mut self, account: u64, track: Track, target: bool, result: Result<SonglistWrite, Failure>) {
        // 请求发出后换了账号：结果属于旧账号，与现在的界面无关。
        if account != self.account {
            return;
        }
        self.like_pending.remove(&track.id);
        let name = &track.name;
        match result {
            Ok(outcome) if outcome.is_confirmed() => {
                self.set_liked(track.id, target);
                self.sync_liked_pages(&track, target);
                self.info(match (target, outcome) {
                    (true, SonglistWrite::Unchanged) => format!("《{name}》已经在「我喜欢」里"),
                    (false, SonglistWrite::Unchanged) => format!("《{name}》本来就不在「我喜欢」里"),
                    (true, _) => format!("已喜欢《{name}》"),
                    (false, _) => format!("已取消喜欢《{name}》"),
                });
            }
            Ok(outcome) => {
                self.set_liked(track.id, !target);
                let code = match outcome {
                    SonglistWrite::Rejected(code) => code,
                    _ => 0,
                };
                let verb = if target { "喜欢" } else { "取消喜欢" };
                self.error(ErrorKind::Other, format!("{verb}《{name}》失败：服务端拒绝（{code}），状态没有改变"));
            }
            Err(failure) => {
                self.set_liked(track.id, !target);
                let verb = if target { "喜欢" } else { "取消喜欢" };
                self.error(failure.kind, format!("{verb}《{name}》失败: {}，状态没有改变", failure.message));
            }
        }
    }

    /// 操作确认后更新已打开的「我喜欢的音乐」页面。
    fn sync_liked_pages(&mut self, track: &Track, liked: bool) {
        for page in self.stack.iter_mut().filter(|p| p.source == Source::Liked && !p.loading) {
            let at = page.items.iter().position(|i| matches!(i, Item::Song(t) if t.id == track.id));
            match (liked, at) {
                (true, None) => {
                    let had_items = !page.items.is_empty();
                    page.items.insert(0, Item::Song(track.clone()));
                    page.total = page.total.map(|n| n + 1);
                    if had_items {
                        page.selected += 1;
                    }
                }
                (false, Some(at)) => {
                    page.items.remove(at);
                    page.total = page.total.map(|n| n.saturating_sub(1));
                    if page.selected > at {
                        page.selected -= 1;
                    }
                    page.selected = page.selected.min(page.items.len().saturating_sub(1));
                }
                _ => {}
            }
        }
    }

    // ---- 列表操作 ----

    fn selected_item(&self) -> Option<&Item> {
        let page = self.page();
        page.items.get(page.selected)
    }

    /// 当前视图里选中的歌曲。
    fn selected_track(&self) -> Option<Track> {
        match self.view() {
            View::Queue => self.queue.get(self.queue_sel).cloned(),
            View::List => match self.selected_item() {
                Some(Item::Song(track)) => Some(track.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// 播放并替换队列：用当前页已加载的全部歌曲替换队列，从选中的那首开始。
    fn play_replace(&mut self) {
        let page = self.page();
        let mut index = None;
        let mut tracks = Vec::new();
        for (i, item) in page.items.iter().enumerate() {
            if let Item::Song(track) = item {
                if i == page.selected {
                    index = Some(tracks.len());
                }
                tracks.push(track.clone());
            }
        }
        let Some(index) = index else { return };
        let count = tracks.len();
        // 只加载了一部分时如实说明，不把它说成整张歌单。
        let message = match (page.incomplete(), page.total) {
            (true, Some(total)) if total > count => {
                format!("播放已加载的 {count} 首（共 {total} 首，其余尚未加载），已替换队列")
            }
            (true, _) => format!("播放已加载的 {count} 首（列表尚未全部加载），已替换队列"),
            _ => format!("播放本页 {count} 首，已替换队列"),
        };
        let recommendation = page.recommendation;
        if self.send(Command::Play { tracks, index, recommendation }) {
            self.info(message);
        }
    }

    fn enqueue(&mut self, track: Track, next: bool) {
        let name = track.name.clone();
        let (sending, done) = if next {
            (format!("正在设为下一首播放：{name}"), format!("下一首播放：{name}"))
        } else {
            (format!("正在追加到队尾：{name}"), format!("已追加到队尾：{name}"))
        };
        self.send_tagged(sending, done, |tag| {
            let tracks = vec![track];
            if next { Command::PlayNext { tracks, tag } } else { Command::Append { tracks, tag } }
        });
    }

    fn open_song_menu(&mut self) {
        let Some(track) = self.selected_track() else { return };
        let in_queue = self.view() == View::Queue;
        let mut items = Vec::new();
        if in_queue {
            items.push(("从队列移除".to_string(), MenuAction::RemoveFromQueue(self.queue_sel)));
        } else {
            let page = self.page();
            let count = page.song_count();
            let scope = if page.incomplete() {
                format!("本页已加载的 {count} 首")
            } else {
                format!("本页 {count} 首")
            };
            items.push((format!("播放并替换队列（{scope}）"), MenuAction::PlayReplace));
        }
        items.push(("下一首播放".into(), MenuAction::PlayNext(Box::new(track.clone()))));
        items.push(("追加到队尾".into(), MenuAction::Append(Box::new(track.clone()))));
        let like = if self.liked.contains(&track.id) { "取消喜欢" } else { "喜欢" };
        items.push((like.into(), MenuAction::ToggleLike(Box::new(track.clone()))));
        if !track.album_mid.is_empty() {
            let (mid, name) = (track.album_mid.clone(), track.album.clone());
            items.push((format!("查看专辑：{name}"), MenuAction::OpenAlbum { mid, name }));
        }
        match track.singer_list.as_slice() {
            [] => {}
            [singer] => {
                let (mid, name) = (singer.mid.clone(), singer.name.clone());
                items.push((format!("查看歌手：{name}"), MenuAction::OpenSinger { mid, name }));
            }
            _ => items.push(("查看歌手…".into(), MenuAction::PickSinger(Box::new(track.clone())))),
        }
        self.mode = Mode::Menu(Menu { title: format!(" {} ", track.name), items, selected: 0 });
    }

    fn run_menu(&mut self, action: MenuAction) {
        self.mode = Mode::Normal;
        match action {
            MenuAction::PlayReplace => self.play_replace(),
            MenuAction::PlayNext(track) => self.enqueue(*track, true),
            MenuAction::Append(track) => self.enqueue(*track, false),
            MenuAction::ToggleLike(track) => self.toggle_like(Some(*track)),
            MenuAction::OpenAlbum { mid, name } => self.push(format!("专辑 › {name}"), Source::Album(mid)),
            MenuAction::OpenSinger { mid, name } => self.push(format!("歌手 › {name}"), Source::Singer(mid)),
            MenuAction::PickSinger(track) => {
                let items = track
                    .singer_list
                    .iter()
                    .map(|s| (s.name.clone(), MenuAction::OpenSinger { mid: s.mid.clone(), name: s.name.clone() }))
                    .collect();
                self.mode = Mode::Menu(Menu { title: " 选择歌手 ".into(), items, selected: 0 });
            }
            MenuAction::RemoveFromQueue(index) => self.remove_from_queue(index),
            MenuAction::SetQuality(quality) => {
                if self.send(Command::SetQuality { quality }) {
                    self.info(format!("最高音质设为{}，当前歌曲从原位置重新加载", quality_label(quality)));
                }
            }
            MenuAction::ClearQueue => {
                if self.send(Command::Clear) {
                    self.info("已清空播放队列");
                }
            }
            MenuAction::Cancel => {}
        }
    }

    fn activate(&mut self) {
        let Some(item) = self.selected_item().cloned() else { return };
        match item {
            Item::Link { source: Source::Queue, .. } => self.open_queue(),
            Item::Link { title, source, .. } => self.push_child(&title, source),
            Item::Song(_) => self.play_replace(),
            Item::Action { action, .. } => self.run_action(action),
            Item::Text(_) => {}
        }
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::Login(kind) => self.start_login(kind),
            Action::Logout => self.logout(),
            Action::CheckLogin => self.check_session(),
            Action::Search => self.open_search_box(),
        }
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn open_queue(&mut self) {
        self.queue_sel = self.state.index.unwrap_or(0).min(self.queue.len().saturating_sub(1));
        if self.page().source != Source::Queue {
            self.push("播放队列".into(), Source::Queue);
        }
    }

    fn open_lyrics(&mut self) {
        if self.page().source != Source::Lyrics {
            self.push("当前播放".into(), Source::Lyrics);
        }
    }

    fn remove_from_queue(&mut self, index: usize) {
        let Some(track) = self.queue.get(index) else { return };
        let name = track.name.clone();
        let entry = self.queue_ids.get(index).copied();
        self.send_tagged(format!("正在从队列移除：{name}"), format!("已从队列移除：{name}"), |tag| {
            Command::Remove { index, entry, tag }
        });
    }

    fn move_in_queue(&mut self, down: bool) {
        let from = self.queue_sel;
        let Some(entry) = self.queue_ids.get(from).copied() else {
            if !self.queue.is_empty() {
                self.error(ErrorKind::Local, "后台播放进程是旧版本，不支持调整队列顺序");
            }
            return;
        };
        let to = if down { from + 1 } else { from.wrapping_sub(1) };
        if to >= self.queue.len() {
            return;
        }
        // 不单独提示完成：队列更新后光标跟着被移动的条目走，结果一目了然。
        self.send(Command::Move { entry, to, tag: None });
    }

    // ---- 搜索 ----

    fn open_search_box(&mut self) {
        // 已在搜索结果页时带上原来的关键词，方便修改。
        let (text, kind) = match &self.page().source {
            Source::Search { keyword, kind } => (keyword.clone(), *kind),
            _ => (String::new(), self.search_kind),
        };
        let mut search =
            SearchBox { input: TextInput::new(&text), kind, options: Vec::new(), picked: None, from_history: false };
        if text.is_empty() {
            search.options = self.history.clone();
            search.from_history = true;
        }
        self.mode = Mode::Search(search);
        if !text.is_empty() {
            self.refresh_suggestions();
        }
    }

    /// 输入变化后更新候选：空输入显示历史；否则防抖后请求补全，旧请求取消、旧结果丢弃。
    fn refresh_suggestions(&mut self) {
        self.suggest_generation += 1;
        if let Some(task) = self.suggest_task.take() {
            task.abort();
        }
        let Mode::Search(search) = &mut self.mode else { return };
        search.picked = None;
        let keyword = search.input.text().trim().to_string();
        if keyword.is_empty() {
            search.options = self.history.clone();
            search.from_history = true;
            return;
        }
        if search.from_history {
            search.options.clear();
            search.from_history = false;
        }
        let generation = self.suggest_generation;
        let client = self.client.clone();
        let tx = self.tx.clone();
        self.suggest_task = Some(tokio::spawn(async move {
            tokio::time::sleep(SUGGEST_DEBOUNCE).await;
            // 补全失败不提示：不影响直接回车搜索。
            if let Ok(items) = data::suggestions(&client, &keyword).await {
                let _ = tx.send(Msg::Suggest { generation, items });
            }
        }));
    }

    fn close_search_box(&mut self) {
        self.suggest_generation += 1;
        if let Some(task) = self.suggest_task.take() {
            task.abort();
        }
        self.mode = Mode::Normal;
    }

    fn submit_search(&mut self) {
        let Mode::Search(search) = &self.mode else { return };
        let keyword = match search.picked.and_then(|i| search.options.get(i)) {
            Some(option) => option.clone(),
            None => search.input.text().trim().to_string(),
        };
        let kind = search.kind;
        self.close_search_box();
        if keyword.is_empty() {
            return;
        }
        self.search_kind = kind;
        self.history = data::remember_search(&self.paths, std::mem::take(&mut self.history), &keyword);
        self.show_search(keyword, kind);
    }

    /// 显示某个关键词 + 类型的结果：之前打开过就原样恢复（位置、已加载的内容），否则新建。
    fn show_search(&mut self, keyword: String, kind: SearchKind) {
        // 当前就是搜索页：换下来放进缓存，而不是越叠越深。
        if matches!(self.page().source, Source::Search { .. })
            && let Some(page) = self.stack.pop()
        {
            self.search_cache.insert(0, page);
            self.search_cache.truncate(SEARCH_CACHE);
        }
        let source = Source::Search { keyword: keyword.clone(), kind };
        match self.search_cache.iter().position(|p| p.source == source) {
            Some(at) => {
                let page = self.search_cache.remove(at);
                self.stack.push(page);
                self.maybe_load_more();
            }
            None => self.push(format!("搜索「{keyword}」"), source),
        }
    }

    fn switch_search_kind(&mut self) {
        if let Source::Search { keyword, kind } = self.page().source.clone() {
            let kind = kind.next();
            self.search_kind = kind;
            self.show_search(keyword, kind);
        }
    }

    /// 粘贴的文本只进输入框，不会被当成快捷键执行。
    pub fn handle_paste(&mut self, text: &str) {
        if let Mode::Search(search) = &mut self.mode {
            search.input.insert_str(text);
            self.refresh_suggestions();
        }
    }

    // ---- 按键 ----

    pub fn handle_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = Some(false);
            return;
        }
        match &mut self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Search(search) => {
                let mut edited = false;
                match key.code {
                    KeyCode::Esc => return self.close_search_box(),
                    KeyCode::Enter => return self.submit_search(),
                    // 弹窗打开时 Tab 属于弹窗：这里是切换搜索类型。
                    KeyCode::Tab => search.kind = search.kind.next(),
                    KeyCode::Char('t') if ctrl => search.kind = search.kind.next(),
                    KeyCode::Char('x') if ctrl => {
                        if let Err(e) = data::clear_history(&self.paths) {
                            return self.error(ErrorKind::Local, format!("清除搜索历史失败: {e}"));
                        }
                        self.history.clear();
                        if search.from_history {
                            search.options.clear();
                            search.picked = None;
                        }
                        self.info("已清除搜索历史");
                    }
                    KeyCode::Up | KeyCode::Down if !search.options.is_empty() => {
                        let len = search.options.len();
                        search.picked = match (search.picked, key.code) {
                            (None, KeyCode::Down) => Some(0),
                            (None, _) => Some(len - 1),
                            (Some(i), KeyCode::Down) if i + 1 < len => Some(i + 1),
                            (Some(i), KeyCode::Up) if i > 0 => Some(i - 1),
                            // 越过两端回到「使用输入的内容」。
                            _ => None,
                        };
                    }
                    KeyCode::Left => search.input.left(),
                    KeyCode::Right => search.input.right(),
                    KeyCode::Home => search.input.home(),
                    KeyCode::End => search.input.end(),
                    KeyCode::Char('a') if ctrl => search.input.home(),
                    KeyCode::Char('e') if ctrl => search.input.end(),
                    KeyCode::Char('u') if ctrl => {
                        search.input.kill_to_start();
                        edited = true;
                    }
                    KeyCode::Backspace => {
                        search.input.backspace();
                        edited = true;
                    }
                    KeyCode::Delete => {
                        search.input.delete();
                        edited = true;
                    }
                    KeyCode::Char(c) if !ctrl && !key.modifiers.contains(KeyModifiers::ALT) => {
                        search.input.insert(c);
                        edited = true;
                    }
                    _ => {}
                }
                if edited {
                    self.refresh_suggestions();
                }
            }
            Mode::Menu(menu) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('h') | KeyCode::Left => self.mode = Mode::Normal,
                KeyCode::Up | KeyCode::Char('k') => menu.selected = menu.selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    menu.selected = (menu.selected + 1).min(menu.items.len().saturating_sub(1));
                }
                KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                    if let Some((_, action)) = menu.items.get(menu.selected) {
                        let action = action.clone();
                        self.run_menu(action);
                    }
                }
                _ => {}
            },
            Mode::Help { scroll } | Mode::Notices { scroll } => {
                // 先落到实际位置（初始值可能是「滚到底」），再按键移动。
                *scroll = (*scroll).min(self.scroll_max.get());
                match key.code {
                    KeyCode::Down | KeyCode::Char('j') => *scroll += 1,
                    KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
                    KeyCode::PageDown | KeyCode::Char(' ') => *scroll += 10,
                    KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                    KeyCode::Char('d') if ctrl => *scroll += 10,
                    KeyCode::Char('u') if ctrl => *scroll = scroll.saturating_sub(10),
                    KeyCode::Home | KeyCode::Char('g') => *scroll = 0,
                    KeyCode::End | KeyCode::Char('G') => *scroll = usize::MAX / 2,
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q' | '?' | 'e') => self.mode = Mode::Normal,
                    _ => {}
                }
            }
            Mode::Login(view) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    if let Some(task) = self.login_task.take() {
                        task.abort();
                    }
                    self.mode = Mode::Normal;
                }
                KeyCode::Char('r') => {
                    let kind = view.kind;
                    self.start_login(kind);
                }
                _ => {}
            },
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // 全局键：在任何视图里含义都一样。
        match key.code {
            KeyCode::Char('q') => self.quit = Some(false),
            KeyCode::Char('Q') => self.quit = Some(true),
            KeyCode::Char(' ') => {
                self.send(Command::Toggle);
            }
            KeyCode::Char('[') => {
                self.send(Command::Prev);
            }
            KeyCode::Char(']') => {
                self.send(Command::Next);
            }
            KeyCode::Char(',') => {
                self.send(Command::SeekBy { offset_ms: -5000 });
            }
            KeyCode::Char('.') => {
                self.send(Command::SeekBy { offset_ms: 5000 });
            }
            KeyCode::Char('-') => {
                self.send(Command::SetVolume { volume: self.state.volume.saturating_sub(5) });
            }
            KeyCode::Char('=') | KeyCode::Char('+') => {
                self.send(Command::SetVolume { volume: self.state.volume.saturating_add(5).min(100) });
            }
            KeyCode::Char('m') => {
                self.send(Command::SetMode { mode: self.state.mode.next() });
            }
            KeyCode::Char('v') => {
                let selected = QUALITIES.iter().position(|q| *q == self.state.quality).unwrap_or(1);
                let items = QUALITIES
                    .iter()
                    .map(|q| {
                        let mark = if *q == self.state.quality { "  ✓ 当前" } else { "" };
                        (format!("{}{mark}", quality_label(*q)), MenuAction::SetQuality(*q))
                    })
                    .collect();
                self.mode =
                    Mode::Menu(Menu { title: " 最高音质（取不到时自动降级） ".into(), items, selected });
            }
            KeyCode::Char('/') => self.open_search_box(),
            KeyCode::Char('t') if ctrl => self.switch_search_kind(),
            KeyCode::Char('f') => self.toggle_like(self.state.track.clone()),
            KeyCode::Char('L') => self.open_lyrics(),
            KeyCode::Char('e') => {
                self.unread = 0;
                self.mode = Mode::Notices { scroll: usize::MAX / 2 };
            }
            KeyCode::Char('x') => {
                self.alert = None;
                self.toast = None;
            }
            KeyCode::Char('?') => self.mode = Mode::Help { scroll: 0 },
            _ => match self.view() {
                View::List => self.handle_list_key(key),
                View::Queue => self.handle_queue_key(key),
                View::Lyrics => self.handle_lyric_key(key),
            },
        }
    }

    /// 把移动类按键换算成位移；不是移动键时返回 `None`。
    fn motion(&self, key: KeyEvent) -> Option<isize> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let page = self.viewport.get().rows.max(1) as isize;
        Some(match key.code {
            KeyCode::Char('d') if ctrl => page,
            KeyCode::Char('u') if ctrl => -page,
            KeyCode::PageDown => page,
            KeyCode::PageUp => -page,
            KeyCode::Down | KeyCode::Char('j') => 1,
            KeyCode::Up | KeyCode::Char('k') => -1,
            KeyCode::Home | KeyCode::Char('g') => isize::MIN / 2,
            KeyCode::End | KeyCode::Char('G') => isize::MAX / 2,
            _ => return None,
        })
    }

    fn handle_list_key(&mut self, key: KeyEvent) {
        if let Some(delta) = self.motion(key) {
            move_selection(self.page_mut(), delta);
            return self.maybe_load_more();
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('n') => self.activate(),
            // 在歌曲上 l 只打开菜单，不会一不小心替换掉队列。
            KeyCode::Char('l') | KeyCode::Right => match self.selected_item() {
                Some(Item::Song(_)) => self.open_song_menu(),
                _ => self.activate(),
            },
            KeyCode::Char('o') => self.open_song_menu(),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('b' | 'h') | KeyCode::Left => self.back(),
            KeyCode::Char('F') => self.toggle_like(self.selected_track()),
            KeyCode::Char('a') => {
                if let Some(track) = self.selected_track() {
                    self.enqueue(track, false);
                }
            }
            KeyCode::Char('A') => {
                if let Some(track) = self.selected_track() {
                    self.enqueue(track, true);
                }
            }
            KeyCode::Char('c') => self.open_queue(),
            KeyCode::Char('t') => self.switch_search_kind(),
            KeyCode::Char('r') => {
                // 续页失败时只重试那一页，已加载的内容不动。
                if matches!(self.page().more, More::Failed(_)) {
                    self.page_mut().more = More::Idle;
                    self.load_more();
                } else {
                    self.reload();
                }
            }
            _ => {}
        }
    }

    fn handle_queue_key(&mut self, key: KeyEvent) {
        if let Some(delta) = self.motion(key) {
            let len = self.queue.len() as isize;
            if len > 0 {
                self.queue_sel = (self.queue_sel as isize).saturating_add(delta).clamp(0, len - 1) as usize;
            }
            return;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('n') => {
                let index = self.queue_sel;
                if index < self.queue.len() {
                    let entry = self.queue_ids.get(index).copied();
                    self.send(Command::PlayIndex { index, entry });
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => self.remove_from_queue(self.queue_sel),
            KeyCode::Char('D') if !self.queue.is_empty() => {
                let items = vec![
                    ("取消".to_string(), MenuAction::Cancel),
                    (format!("清空队列并停止播放（{} 首）", self.queue.len()), MenuAction::ClearQueue),
                ];
                self.mode = Mode::Menu(Menu { title: " 清空播放队列？ ".into(), items, selected: 0 });
            }
            KeyCode::Char('J') => self.move_in_queue(true),
            KeyCode::Char('K') => self.move_in_queue(false),
            // 定位到正在播放的歌曲。
            KeyCode::Char('c') => {
                self.queue_sel = self.state.index.unwrap_or(0).min(self.queue.len().saturating_sub(1))
            }
            KeyCode::Char('o' | 'l') | KeyCode::Right => self.open_song_menu(),
            KeyCode::Char('F') => self.toggle_like(self.selected_track()),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('b' | 'h') | KeyCode::Left => self.back(),
            _ => {}
        }
    }

    fn handle_lyric_key(&mut self, key: KeyEvent) {
        if let Some(delta) = self.motion(key) {
            let len = self.lyrics().len() as isize;
            if len > 0 {
                let from = self.lyric_sel.or_else(|| lyric::current(self.lyrics(), self.position_ms())).unwrap_or(0);
                self.lyric_sel = Some((from as isize).saturating_add(delta).clamp(0, len - 1) as usize);
            }
            return;
        }
        match key.code {
            // 跳到选中的那一行，然后恢复自动跟随（后台会立刻报告新的位置，高亮不会跳回去）。
            KeyCode::Enter | KeyCode::Char('n') => {
                if let Some(line) = self.lyric_sel.and_then(|i| self.lyrics().get(i)) {
                    let position_ms = line.time_ms;
                    if self.send(Command::Seek { position_ms }) {
                        self.lyric_sel = None;
                    }
                }
            }
            KeyCode::Char('z') => self.lyric_sel = None,
            KeyCode::Char('t') => {
                self.show_trans = !self.show_trans;
                self.info(if self.show_trans { "显示歌词翻译" } else { "隐藏歌词翻译" });
            }
            KeyCode::Char('r') => self.fetch_lyric(),
            KeyCode::Char('c') => self.open_queue(),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('b' | 'h') | KeyCode::Left => self.back(),
            _ => {}
        }
    }
}

/// 移动选中项，跳过不可选的文本行。
fn move_selection(page: &mut Page, delta: isize) {
    let len = page.items.len() as isize;
    if len == 0 {
        return;
    }
    let target = (page.selected as isize).saturating_add(delta).clamp(0, len - 1);
    let selectable = |i: &isize| !matches!(page.items[*i as usize], Item::Text(_));
    let forward = (target..len).find(selectable);
    let backward = (0..=target).rev().find(selectable);
    let pick = if delta >= 0 { forward.or(backward) } else { backward.or(forward) };
    if let Some(i) = pick {
        page.selected = i as usize;
    }
}

fn first_selectable(items: &[Item], from: usize) -> usize {
    items.iter().skip(from).position(|i| !matches!(i, Item::Text(_))).map_or(from, |p| from + p)
}

#[cfg(test)]
mod tests {
    use qqmusic_api::ApiError;
    use serde_json::Value;

    use super::*;

    #[test]
    fn login_failure_hints() {
        let api = |code| Error::from(ApiError::login(code, Value::Null));
        let device = login_failure(&api(20_279), QrLoginType::Qq);
        assert!(device.contains("登录设备管理") && device.contains("QQ 音乐 App 扫码登录"), "{device}");
        assert!(!login_failure(&api(20_279), QrLoginType::Mobile).contains("改用"));
        let restricted = login_failure(&api(20_450), QrLoginType::Qq);
        assert!(restricted.contains("账号受限") && !restricted.contains("按 r"), "{restricted}");
        assert!(login_failure(&api(104_604), QrLoginType::Wx).contains("稍后"));
        assert!(login_failure(&api(5), QrLoginType::Qq).ends_with("按 r 重试"));
    }
}
