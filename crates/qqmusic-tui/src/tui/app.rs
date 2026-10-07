//! TUI 状态与交互逻辑。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures::StreamExt;
use qqmusic_api::models::login::{QrCodeLoginEvent, QrLoginType};
use qqmusic_api::modules::lyric::LyricOptions;
use qqmusic_api::modules::song::Quality;
use qqmusic_api::{Client, Credential};
use ratatui::text::Line;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use super::data::{self, Action, Item, SearchKind, Source};
use super::lyric::{self, LyricLine};
use super::qr;
use crate::paths::Paths;
use crate::protocol::{Command, Event, PlayerState, Status, Track};

/// 发给主循环的消息。
pub enum Msg {
    Player(Event),
    Disconnected,
    Loaded { id: u64, result: Result<Vec<Item>, String> },
    Lyric { mid: String, lines: Vec<LyricLine> },
    Liked(HashSet<i64>),
    Nick(String),
    Login(LoginMsg),
    Toast(String),
}

/// 登录流程消息。
pub enum LoginMsg {
    Qr { lines: Vec<Line<'static>>, path: Option<String> },
    Status(String),
    Done(Box<Credential>),
    Failed(String),
}

/// 一级页面。
pub struct Page {
    pub id: u64,
    pub title: String,
    pub source: Source,
    pub items: Vec<Item>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
}

/// 登录弹窗。
pub struct LoginView {
    pub kind: QrLoginType,
    pub qr: Vec<Line<'static>>,
    pub path: Option<String>,
    pub status: String,
}

/// 输入 / 弹窗模式。
pub enum Mode {
    Normal,
    Search { input: String, kind: SearchKind },
    Quality { selected: usize },
    Login(LoginView),
    Help,
}

pub const QUALITIES: [Quality; 3] = [Quality::Lossless, Quality::High, Quality::Standard];

/// 菜单分页布局（由渲染时根据窗口大小计算）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuLayout {
    /// 每页条目数。
    pub page_size: usize,
    /// 列数（1 或 2）。
    pub columns: usize,
}

pub struct App {
    pub layout: std::cell::Cell<MenuLayout>,
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
    pub lyrics: Vec<LyricLine>,
    lyric_mid: String,
    pub liked: HashSet<i64>,
    pub nick: Option<String>,
    pub toast: Option<(String, Instant)>,
    pub connected: bool,
    /// `Some(stop_player)` 表示退出。
    pub quit: Option<bool>,
    login_task: Option<JoinHandle<()>>,
}

impl App {
    pub fn new(paths: Paths, client: Client, tx: UnboundedSender<Msg>, commands: UnboundedSender<Command>) -> Self {
        let mut app = Self {
            layout: std::cell::Cell::new(MenuLayout { page_size: 10, columns: 1 }),
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
            lyrics: Vec::new(),
            lyric_mid: String::new(),
            liked: HashSet::new(),
            nick: None,
            toast: None,
            connected: false,
            quit: None,
            login_task: None,
        };
        app.push("主菜单".into(), Source::Root);
        app.on_login_changed();
        app
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
    }

    pub fn logged_in(&self) -> bool {
        self.client.credential().is_valid()
    }

    /// 估算的当前播放位置（两次状态推送之间插值）。
    pub fn position_ms(&self) -> u64 {
        let mut position = self.state.position_ms;
        if self.state.status == Status::Playing {
            position += self.state_at.elapsed().as_millis() as u64;
        }
        if self.state.duration_ms > 0 { position.min(self.state.duration_ms) } else { position }
    }

    pub fn page(&self) -> &Page {
        self.stack.last().expect("page stack is never empty")
    }

    fn page_mut(&mut self) -> &mut Page {
        self.stack.last_mut().expect("page stack is never empty")
    }

    fn local_items(&self, source: &Source) -> Vec<Item> {
        match source {
            Source::Root => data::root_items(),
            Source::Queue => self.queue.iter().cloned().map(Item::Song).collect(),
            Source::Account => {
                let credential = self.client.credential();
                let nick = self.nick.clone().unwrap_or_default();
                data::account_items(credential.is_valid().then_some((credential.musicid, nick.as_str())))
            }
            _ => Vec::new(),
        }
    }

    fn push(&mut self, title: String, source: Source) {
        self.next_id += 1;
        let items = self.local_items(&source);
        let page = Page {
            id: self.next_id,
            title,
            selected: first_selectable(&items, 0),
            items,
            source,
            loading: false,
            error: None,
        };
        self.stack.push(page);
        self.reload();
    }

    /// 重新加载当前页。
    fn reload(&mut self) {
        let source = self.page().source.clone();
        if !source.is_remote() {
            let items = self.local_items(&source);
            let page = self.page_mut();
            page.selected = page.selected.min(items.len().saturating_sub(1));
            page.items = items;
            return;
        }
        self.next_id += 1;
        let id = self.next_id;
        let page = self.page_mut();
        page.id = id;
        page.loading = true;
        page.error = None;
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = data::load(&client, &source).await;
            let _ = tx.send(Msg::Loaded { id, result });
        });
    }

    fn refresh_local(&mut self, source: &Source) {
        for i in 0..self.stack.len() {
            if &self.stack[i].source == source {
                let items = self.local_items(source);
                let page = &mut self.stack[i];
                page.selected = first_selectable(&items, page.selected.min(items.len().saturating_sub(1)));
                page.items = items;
            }
        }
    }

    /// 登录状态变化：刷新昵称、喜欢列表、账号页，并通知守护进程。
    fn on_login_changed(&mut self) {
        self.nick = None;
        self.liked.clear();
        self.refresh_local(&Source::Account);
        if !self.logged_in() {
            return;
        }
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let credential = client.credential();
            if let Ok(home) = client.user().get_homepage(&credential.encrypt_uin, None).await {
                let _ = tx.send(Msg::Nick(home.base_info.name));
            }
            match data::liked_ids(&client).await {
                Ok(ids) => {
                    let _ = tx.send(Msg::Liked(ids));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Toast(format!("获取喜欢列表失败: {e}")));
                }
            }
        });
    }

    /// 启动时检查凭证是否过期，过期则刷新。
    pub fn refresh_credential_if_needed(&self) {
        if !self.logged_in() {
            return;
        }
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if !matches!(client.login().check_expired(None).await, Ok(true)) {
                return;
            }
            let msg = match client.login().refresh_credential(None).await {
                Ok(credential) => LoginMsg::Done(Box::new(credential)),
                Err(e) => LoginMsg::Failed(format!("登录已过期且刷新失败，请重新登录: {e}")),
            };
            let _ = tx.send(Msg::Login(msg));
        });
    }

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Player(event) => self.on_player_event(event),
            Msg::Disconnected => {
                self.connected = false;
                self.toast("与播放进程的连接已断开，正在重连…");
            }
            Msg::Loaded { id, result } => {
                if let Some(page) = self.stack.iter_mut().find(|p| p.id == id) {
                    page.loading = false;
                    match result {
                        Ok(items) => {
                            page.selected = page.selected.min(items.len().saturating_sub(1));
                            page.items = items;
                            page.selected = first_selectable(&page.items, page.selected);
                        }
                        Err(e) => page.error = Some(e),
                    }
                }
            }
            Msg::Lyric { mid, lines } => {
                if mid == self.lyric_mid {
                    self.lyrics = lines;
                }
            }
            Msg::Liked(ids) => self.liked = ids,
            Msg::Nick(nick) => {
                self.nick = Some(nick);
                self.refresh_local(&Source::Account);
            }
            Msg::Login(msg) => self.on_login_msg(msg),
            Msg::Toast(message) => self.toast(message),
        }
    }

    fn on_player_event(&mut self, event: Event) {
        match event {
            Event::State(state) => {
                if !self.connected {
                    self.connected = true;
                }
                let mid = state.track.as_ref().map(|t| t.mid.clone()).unwrap_or_default();
                if mid != self.lyric_mid {
                    self.lyric_mid = mid.clone();
                    self.lyrics.clear();
                    if !mid.is_empty() {
                        self.fetch_lyric(mid);
                    }
                }
                if state.queue_version != self.state.queue_version {
                    self.send(Command::GetQueue);
                }
                self.state = *state;
                self.state_at = Instant::now();
            }
            Event::Queue { tracks } => {
                self.queue = tracks;
                self.refresh_local(&Source::Queue);
            }
            Event::Error { message } => self.toast(message),
            Event::Shutdown => {
                self.state = PlayerState::default();
                self.toast("播放进程已退出");
            }
        }
    }

    fn fetch_lyric(&self, mid: String) {
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let options = LyricOptions { trans: true, ..Default::default() };
            let lines = match client.lyric().get_lyric_with(mid.as_str(), options).await {
                Ok(resp) => lyric::parse(&resp.lyric, &resp.trans),
                Err(_) => Vec::new(),
            };
            let _ = tx.send(Msg::Lyric { mid, lines });
        });
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
                if let Err(e) = crate::save_credential(&self.paths, &credential) {
                    self.toast(format!("保存登录凭证失败: {e}"));
                }
                self.client.set_credential(*credential);
                self.send(Command::ReloadCredential);
                if matches!(self.mode, Mode::Login(_)) {
                    self.mode = Mode::Normal;
                    self.toast("登录成功");
                }
                self.on_login_changed();
            }
            LoginMsg::Failed(e) => {
                if let Mode::Login(view) = &mut self.mode {
                    view.status = e;
                } else {
                    self.toast(e);
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
                    Err(e) => return send(LoginMsg::Failed(format!("登录失败: {e}，按 r 重试"))),
                }
            }
        }));
    }

    fn logout(&mut self) {
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = client.login().logout(None).await;
        });
        if let Err(e) = crate::remove_credential(&self.paths) {
            self.toast(format!("删除凭证失败: {e}"));
        }
        self.client.set_credential(Credential::default());
        self.send(Command::ReloadCredential);
        self.on_login_changed();
        self.toast("已退出登录");
    }

    fn songs_of_page(&self) -> (Vec<Track>, Option<usize>) {
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
        (tracks, index)
    }

    fn selected_item(&self) -> Option<&Item> {
        let page = self.page();
        page.items.get(page.selected)
    }

    fn selected_track(&self) -> Option<Track> {
        match self.selected_item() {
            Some(Item::Song(track)) => Some(track.clone()),
            _ => None,
        }
    }

    fn activate(&mut self) {
        let Some(item) = self.selected_item().cloned() else { return };
        match item {
            Item::Link { title, source, .. } => {
                let title = format!("{} › {}", self.page().title, title);
                self.push(title, source);
            }
            Item::Song(_) => {
                if self.page().source == Source::Queue {
                    self.send(Command::PlayIndex { index: self.page().selected });
                } else {
                    let (tracks, index) = self.songs_of_page();
                    if let Some(index) = index {
                        self.send(Command::Play { tracks, index });
                    }
                }
            }
            Item::Action { action, .. } => match action {
                Action::Login(kind) => self.start_login(kind),
                Action::Logout => self.logout(),
                Action::Search => self.mode = Mode::Search { input: String::new(), kind: SearchKind::Song },
            },
            Item::Text(_) => {}
        }
    }

    fn back(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn move_by(&mut self, delta: isize) {
        let page = self.page_mut();
        let len = page.items.len() as isize;
        if len == 0 {
            return;
        }
        let target = (page.selected as isize).saturating_add(delta).clamp(0, len - 1);
        // 跳过不可选的文本行。
        let selectable = |i: &isize| !matches!(page.items[*i as usize], Item::Text(_));
        let forward = (target..len).find(selectable);
        let backward = (0..=target).rev().find(selectable);
        let pick = if delta >= 0 { forward.or(backward) } else { backward.or(forward) };
        if let Some(i) = pick {
            page.selected = i as usize;
        }
    }

    fn move_page(&mut self, pages: isize) {
        self.move_by(pages * self.layout.get().page_size as isize);
    }

    /// 左右移动：双列时在两列之间切换，到边缘翻页；单列时直接翻页。
    fn move_horizontal(&mut self, right: bool) {
        let MenuLayout { page_size, columns } = self.layout.get();
        let selected = self.page().selected;
        let page_size = page_size as isize;
        let delta = match (columns, right, selected % 2) {
            (1, true, _) => page_size,
            (1, false, _) => -page_size,
            (_, true, 0) => 1,
            (_, true, _) => page_size - 1,
            (_, false, 1) => -1,
            (_, false, _) if (selected as isize) < page_size => return,
            (_, false, _) => -(page_size - 1),
        };
        self.move_by(delta);
    }

    fn toggle_like(&mut self, track: Option<Track>) {
        let Some(track) = track else { return };
        if !self.logged_in() {
            self.toast("请先登录");
            return;
        }
        let liked = self.liked.contains(&track.id);
        if liked {
            self.liked.remove(&track.id);
        } else {
            self.liked.insert(track.id);
        }
        let client = self.client.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let songs = [(track.id, track.song_type)];
            let result = if liked {
                client.songlist().unlike_song(&songs, None).await
            } else {
                client.songlist().like_song(&songs, None).await
            };
            let msg = match result {
                Ok(_) if liked => format!("已取消喜欢《{}》", track.name),
                Ok(_) => format!("已喜欢《{}》", track.name),
                Err(e) => format!("操作失败: {e}"),
            };
            let _ = tx.send(Msg::Toast(msg));
        });
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = Some(false);
            return;
        }
        match &mut self.mode {
            Mode::Normal => self.handle_normal_key(key),
            Mode::Search { input, kind } => match key.code {
                KeyCode::Esc => self.mode = Mode::Normal,
                KeyCode::Tab => *kind = kind.next(),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Enter => {
                    let keyword = input.trim().to_string();
                    let kind = *kind;
                    self.mode = Mode::Normal;
                    if !keyword.is_empty() {
                        self.push(format!("搜索 › {keyword}（{}）", kind.label()), Source::Search { keyword, kind });
                    }
                }
                KeyCode::Char(c) => input.push(c),
                _ => {}
            },
            Mode::Quality { selected } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('v') => self.mode = Mode::Normal,
                KeyCode::Up | KeyCode::Char('k') => *selected = selected.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => *selected = (*selected + 1).min(QUALITIES.len() - 1),
                KeyCode::Enter | KeyCode::Char('l') => {
                    let quality = QUALITIES[*selected];
                    self.mode = Mode::Normal;
                    self.send(Command::SetQuality { quality });
                }
                _ => {}
            },
            Mode::Help => self.mode = Mode::Normal,
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
        match key.code {
            KeyCode::Char('q') => self.quit = Some(false),
            KeyCode::Char('Q') => self.quit = Some(true),
            KeyCode::Char('d') if ctrl => self.move_page(1),
            KeyCode::Char('u') if ctrl => self.move_page(-1),
            KeyCode::PageDown => self.move_page(1),
            KeyCode::PageUp => self.move_page(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_by(self.layout.get().columns as isize),
            KeyCode::Up | KeyCode::Char('k') => self.move_by(-(self.layout.get().columns as isize)),
            KeyCode::Left | KeyCode::Char('h') => self.move_horizontal(false),
            KeyCode::Right | KeyCode::Char('l') => self.move_horizontal(true),
            KeyCode::Home | KeyCode::Char('g') => self.move_by(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_by(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Char('n') => self.activate(),
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('b') => self.back(),
            KeyCode::Char(' ') => self.send(Command::Toggle),
            KeyCode::Char('[') => self.send(Command::Prev),
            KeyCode::Char(']') => self.send(Command::Next),
            KeyCode::Char(',') => self.send(Command::SeekBy { offset_ms: -5000 }),
            KeyCode::Char('.') => self.send(Command::SeekBy { offset_ms: 5000 }),
            KeyCode::Char('-') => self.send(Command::SetVolume { volume: self.state.volume.saturating_sub(5) }),
            KeyCode::Char('=') | KeyCode::Char('+') => {
                self.send(Command::SetVolume { volume: self.state.volume.saturating_add(5).min(100) });
            }
            KeyCode::Char('m') => self.send(Command::SetMode { mode: self.state.mode.next() }),
            KeyCode::Char('v') => {
                let selected = QUALITIES.iter().position(|q| *q == self.state.quality).unwrap_or(1);
                self.mode = Mode::Quality { selected };
            }
            KeyCode::Char('/') => self.mode = Mode::Search { input: String::new(), kind: SearchKind::Song },
            KeyCode::Tab => {
                if let Source::Search { keyword, kind } = self.page().source.clone() {
                    let kind = kind.next();
                    self.stack.pop();
                    self.push(format!("搜索 › {keyword}（{}）", kind.label()), Source::Search { keyword, kind });
                }
            }
            KeyCode::Char('f') => self.toggle_like(self.state.track.clone()),
            KeyCode::Char('F') => self.toggle_like(self.selected_track()),
            KeyCode::Char('a') => {
                if let Some(track) = self.selected_track() {
                    self.toast(format!("已加入播放列表：{}", track.name));
                    self.send(Command::Append { tracks: vec![track] });
                }
            }
            KeyCode::Char('A') => {
                if let Some(track) = self.selected_track() {
                    self.toast(format!("下一首播放：{}", track.name));
                    self.send(Command::PlayNext { tracks: vec![track] });
                }
            }
            KeyCode::Char('c') => {
                if self.page().source != Source::Queue {
                    self.push("播放列表".into(), Source::Queue);
                    if let Some(index) = self.state.index {
                        self.page_mut().selected = index;
                    }
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if self.page().source == Source::Queue && !self.page().items.is_empty() {
                    self.send(Command::Remove { index: self.page().selected });
                }
            }
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            _ => {}
        }
    }
}

fn first_selectable(items: &[Item], from: usize) -> usize {
    items.iter().skip(from).position(|i| !matches!(i, Item::Text(_))).map_or(from, |p| from + p)
}
