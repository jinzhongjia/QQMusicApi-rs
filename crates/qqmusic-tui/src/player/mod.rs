//! 播放引擎：队列、播放模式、音质与音频输出。
//!
//! 引擎是一个 tokio 任务，通过 [`PlayerHandle`] 收发 [`Command`] / [`Event`]，
//! 可以独立嵌入到其他程序中使用（守护进程只是在外面包了一层 IPC）。

pub mod cdn;
#[cfg(test)]
mod engine_tests;
mod radio;
#[cfg(test)]
mod recommendation_tests;
pub mod stream;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use qqmusic_api::Client;
use qqmusic_api::modules::song::Quality;
use rodio::{Decoder, Source};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::AbortHandle;

use self::stream::{StreamHandle, StreamReader};
use crate::config::Config;
use crate::paths::Paths;
use crate::protocol::{Activity, Command, ErrorKind, Event, PROTOCOL_VERSION, PlayMode, PlayerState, Status, Track};
use crate::recommendation::Batch;

/// 队列快照：曲目、条目 id 与版本号保持一致。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueueSnapshot {
    /// 曲目。
    pub tracks: Vec<Track>,
    /// 与 `tracks` 一一对应的条目 id。
    pub ids: Vec<u64>,
    /// 队列版本号。
    pub version: u64,
}

impl QueueSnapshot {
    /// 转成推送给客户端的事件。
    pub fn to_event(&self) -> Event {
        Event::Queue { tracks: self.tracks.clone(), version: self.version, ids: self.ids.clone() }
    }
}

/// 播放引擎句柄（可克隆）。
#[derive(Clone)]
pub struct PlayerHandle {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<PlayerState>,
    queue: watch::Receiver<Arc<QueueSnapshot>>,
    events: broadcast::Sender<Event>,
}

impl PlayerHandle {
    /// 发送指令；引擎已退出时返回 `false`。
    pub fn send(&self, command: Command) -> bool {
        self.commands.send(command).is_ok()
    }

    /// 当前状态。
    pub fn state(&self) -> PlayerState {
        self.state.borrow().clone()
    }

    /// 状态监听（每次变化都会通知）。
    pub fn watch_state(&self) -> watch::Receiver<PlayerState> {
        self.state.clone()
    }

    /// 当前队列。
    pub fn queue(&self) -> Arc<QueueSnapshot> {
        self.queue.borrow().clone()
    }

    /// 订阅事件。
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// 等待引擎退出。
    pub async fn closed(&self) {
        self.commands.closed().await;
    }
}

/// 启动播放引擎（需要在 tokio 运行时中调用）。
///
/// `paths` 用于持久化配置与重新读取凭证。
pub fn spawn(client: Client, paths: Paths) -> Result<PlayerHandle, String> {
    let config = Config::load(&paths);
    let (commands, command_rx) = mpsc::unbounded_channel();
    let (events, _) = broadcast::channel(64);
    let (state_tx, state) = watch::channel(PlayerState::default());
    let (queue_tx, queue) = watch::channel(Arc::new(QueueSnapshot::default()));
    let (tx, internal_rx) = mpsc::unbounded_channel();
    let cdn = Arc::new(cdn::CdnSelector::new(&client));
    let engine = Engine::new(client, paths, config, cdn, Output::default(), tx, events.clone(), state_tx, queue_tx);
    tokio::spawn(engine.run(command_rx, internal_rx));
    Ok(PlayerHandle { commands, state, queue, events })
}

/// 音频回调缓冲帧数：音乐播放不需要低延迟，较大的缓冲能减少回调次数。
const BUFFER_FRAMES: u32 = 4096;
/// 空闲（停止 / 暂停）多少个 tick（250ms）后关闭音频设备。
const IDLE_CLOSE_TICKS: u32 = 40;

/// 音频输出设备：按需打开，空闲一段时间后关闭，避免停止 / 暂停时音频回调空转。
#[derive(Default)]
struct Output {
    mixer: Option<rodio::mixer::Mixer>,
    /// drop 时通知输出线程关闭设备。
    close: Option<std::sync::mpsc::Sender<()>>,
}

impl Output {
    fn mixer(&mut self) -> Result<rodio::mixer::Mixer, String> {
        if let Some(mixer) = &self.mixer {
            return Ok(mixer.clone());
        }
        let (mixer, close) = open_output()?;
        self.mixer = Some(mixer.clone());
        self.close = Some(close);
        Ok(mixer)
    }

    fn close(&mut self) {
        self.mixer = None;
        self.close = None;
    }
}

/// 在专用线程中打开默认音频输出（`cpal::Stream` 不能跨线程移动），返回混音器与关闭句柄。
fn open_output() -> Result<(rodio::mixer::Mixer, std::sync::mpsc::Sender<()>), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (close_tx, close_rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("audio-output".into())
        .spawn(move || {
            let sink = rodio::DeviceSinkBuilder::from_default_device().and_then(|builder| {
                builder.with_buffer_size(rodio::cpal::BufferSize::Fixed(BUFFER_FRAMES)).open_sink_or_fallback()
            });
            match sink {
                Ok(mut sink) => {
                    sink.log_on_drop(false);
                    let _ = tx.send(Ok(sink.mixer().clone()));
                    // 关闭句柄被 drop 后返回，随后 drop sink 关闭设备。
                    let _ = close_rx.recv();
                }
                Err(e) => {
                    let _ = tx.send(Err(format!("无法打开音频输出设备: {e}")));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    let mixer = rx.recv().map_err(|e| e.to_string())??;
    Ok((mixer, close_tx))
}

struct Current {
    /// 正在输出的音源；暂停时为 `None`，释放整条解码管线，恢复时从内存缓冲重建。
    sink: Option<Arc<rodio::Player>>,
    stream: StreamHandle,
    hint: String,
    /// 暂停时的播放位置。
    paused_at: Duration,
    /// 下载跟不上播放时主动暂停，攒够数据再继续，避免解码器阻塞系统音频线程。
    buffering: bool,
}

struct Prepared {
    sink: rodio::Player,
    /// 新曲目的下载句柄；暂停后重建音源时为 `None`。
    stream: Option<StreamHandle>,
    hint: String,
    duration: Option<Duration>,
    quality: Quality,
    /// 加载时已经跳转到的位置。
    position: Duration,
}

/// 加载失败的原因与分类。
struct LoadError {
    kind: ErrorKind,
    message: String,
}

impl LoadError {
    fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

struct Loaded {
    generation: u64,
    result: Result<Prepared, LoadError>,
}

/// 后台任务回报给引擎的结果。
enum Internal {
    Loaded(Loaded),
    Seeked { seq: u64, result: Result<(), String> },
}

/// 在阻塞线程里对音源执行跳转（测试中替换成不依赖音频设备的实现）。
type SeekFn = Arc<dyn Fn(&rodio::Player, Duration) -> Result<(), String> + Send + Sync>;

/// 连续多少首加载失败后停止，避免整张歌单都不可播时无限跳歌。
const MAX_FAIL_STREAK: usize = 10;
/// 网络类失败换一首多半也不行，更早停止。
const MAX_NETWORK_FAIL_STREAK: usize = 3;

struct Engine {
    client: Client,
    paths: Paths,
    config: Config,
    output: Output,
    /// 渐入任务的取消令牌：每次开始新的渐入或手动调音量时递增，旧任务随即停止。
    fade_token: Arc<AtomicU64>,
    /// 连续空闲的 tick 数，用于关闭音频设备。
    idle_ticks: u32,
    /// 队列与条目 id 一一对应；`index` 为 `Some` 时一定是有效下标，空队列时一定为 `None`。
    queue: Vec<Track>,
    ids: Vec<u64>,
    next_entry: u64,
    index: Option<usize>,
    /// 播放历史（条目 id），用于随机模式的「上一首」；用 id 而不是下标，队列增删后依然有效。
    history: Vec<u64>,
    /// 通过「下一首播放」插入、尚未播放的条目，按播放顺序排列；随机模式下优先于随机挑选。
    up_next: Vec<u64>,
    radio: Option<radio::Radio>,
    /// 用户意图：`Paused` 表示用户要暂停，即使此刻仍在加载；只有用户操作能离开 `Paused`。
    status: Status,
    current: Option<Current>,
    /// 每次切歌 / 停止时递增，旧的加载与跳转结果据此丢弃。
    generation: u64,
    /// 在途的加载任务，切歌 / 停止时中止，不让它继续占用网络。
    load_task: Option<AbortHandle>,
    /// 是否有加载（新曲目或暂停后重建）在途。
    loading: bool,
    /// 在途加载会跳转到的位置。
    load_at: Duration,
    /// 尚未应用的最新跳转目标（加载或上一次跳转还没结束时先记下，只保留最新的）。
    target: Option<Duration>,
    /// 正在执行的跳转：序号与目标位置。
    seeking: Option<(u64, Duration)>,
    seek_seq: u64,
    /// 音源已就绪，等跳转完成后再开始出声。
    starting: bool,
    /// 当前曲目已因下载中断重新加载过一次。
    retried: bool,
    /// 最近一次观察到的播放位置，下载中断后从这里重试。
    last_pos: Duration,
    playing_quality: Option<Quality>,
    duration_ms: u64,
    queue_version: u64,
    /// 连续加载失败次数。
    fail_streak: usize,
    cdn: Arc<cdn::CdnSelector>,
    seek_fn: SeekFn,
    tx: mpsc::UnboundedSender<Internal>,
    events: broadcast::Sender<Event>,
    state_tx: watch::Sender<PlayerState>,
    queue_tx: watch::Sender<Arc<QueueSnapshot>>,
}

impl Engine {
    #[allow(clippy::too_many_arguments)]
    fn new(
        client: Client,
        paths: Paths,
        config: Config,
        cdn: Arc<cdn::CdnSelector>,
        output: Output,
        tx: mpsc::UnboundedSender<Internal>,
        events: broadcast::Sender<Event>,
        state_tx: watch::Sender<PlayerState>,
        queue_tx: watch::Sender<Arc<QueueSnapshot>>,
    ) -> Self {
        Self {
            client,
            paths,
            config,
            output,
            fade_token: Arc::default(),
            idle_ticks: 0,
            queue: Vec::new(),
            ids: Vec::new(),
            next_entry: 0,
            index: None,
            history: Vec::new(),
            up_next: Vec::new(),
            radio: None,
            status: Status::Stopped,
            current: None,
            generation: 0,
            load_task: None,
            loading: false,
            load_at: Duration::ZERO,
            target: None,
            seeking: None,
            seek_seq: 0,
            starting: false,
            retried: false,
            last_pos: Duration::ZERO,
            playing_quality: None,
            duration_ms: 0,
            queue_version: 0,
            fail_streak: 0,
            cdn,
            seek_fn: Arc::new(|sink, position| sink.try_seek(position).map_err(|e| e.to_string())),
            tx,
            events,
            state_tx,
            queue_tx,
        }
    }

    async fn run(
        mut self,
        mut commands: mpsc::UnboundedReceiver<Command>,
        mut internal: mpsc::UnboundedReceiver<Internal>,
    ) {
        self.publish();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut ticks = 0u64;
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => self.handle(command),
                },
                Some(message) = internal.recv() => self.on_internal(message),
                batch = radio::next_batch(&mut self.radio) => self.on_recommendation(batch),
                _ = tick.tick() => {
                    ticks += 1;
                    self.on_tick(ticks);
                }
            }
        }
        self.halt();
        let _ = self.events.send(Event::Shutdown);
    }

    fn on_internal(&mut self, message: Internal) {
        match message {
            Internal::Loaded(loaded) => self.on_loaded(loaded),
            Internal::Seeked { seq, result } => self.on_seeked(seq, result),
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Play { tracks, index, recommendation } => {
                // 非空列表配越界下标是非法指令：拒绝执行，队列保持不变。
                if !tracks.is_empty() && index >= tracks.len() {
                    return self.error(ErrorKind::Other, "播放位置超出列表范围，队列未改变");
                }
                self.radio = recommendation.map(|source| radio::Radio::new(source, &tracks));
                self.fail_streak = 0;
                self.history.clear();
                self.up_next.clear();
                self.halt();
                self.index = None;
                self.ids = self.fresh_ids(tracks.len());
                self.queue = tracks;
                self.queue_changed();
                if self.queue.is_empty() {
                    // 空列表等同于清空。
                    self.radio = None;
                    self.status = Status::Stopped;
                    self.publish();
                } else {
                    self.play_index(index);
                }
            }
            Command::Append { tracks, tag } => {
                if tracks.is_empty() {
                    return self.done(tag, Err("没有可添加的歌曲".into()));
                }
                let start = self.queue.len();
                let ids = self.fresh_ids(tracks.len());
                self.ids.extend(ids);
                self.queue.extend(tracks);
                self.queue_changed();
                self.done(tag, Ok(()));
                self.after_insert(start);
            }
            Command::PlayNext { tracks, tag } => {
                if tracks.is_empty() {
                    return self.done(tag, Err("没有可添加的歌曲".into()));
                }
                let at = self.index.map_or(0, |i| i + 1).min(self.queue.len());
                let ids = self.fresh_ids(tracks.len());
                // 后插入的排在前面：最后一次设置的最先播放。
                self.up_next.splice(0..0, ids.iter().copied());
                self.ids.splice(at..at, ids);
                self.queue.splice(at..at, tracks);
                self.queue_changed();
                self.done(tag, Ok(()));
                self.after_insert(at);
            }
            Command::PlayIndex { index, entry } => match self.resolve(index, entry) {
                Some(index) => {
                    self.push_history();
                    self.play_index(index);
                }
                None => self.error(ErrorKind::Other, "播放队列已变化，未切换曲目"),
            },
            Command::Remove { index, entry, tag } => match self.resolve(index, entry) {
                Some(index) => {
                    self.remove(index);
                    self.done(tag, Ok(()));
                }
                None => self.done(tag, Err("播放队列已变化，未删除".into())),
            },
            Command::Move { entry, to, tag } => {
                let Some(from) = self.position_of(entry) else {
                    return self.done(tag, Err("播放队列已变化，未移动".into()));
                };
                let to = to.min(self.queue.len() - 1);
                if from != to {
                    let current = self.index.map(|i| self.ids[i]);
                    let id = self.ids.remove(from);
                    self.ids.insert(to, id);
                    let track = self.queue.remove(from);
                    self.queue.insert(to, track);
                    self.index = current.and_then(|id| self.position_of(id));
                    self.queue_changed();
                }
                self.done(tag, Ok(()));
                self.publish();
            }
            Command::Clear => {
                self.radio = None;
                self.halt();
                self.status = Status::Stopped;
                self.index = None;
                self.history.clear();
                self.up_next.clear();
                self.queue.clear();
                self.ids.clear();
                self.queue_changed();
                self.publish();
            }
            Command::Toggle => match self.status {
                Status::Playing | Status::Loading => self.pause(),
                Status::Paused => self.resume(),
                Status::Stopped if !self.queue.is_empty() => self.play_index(self.index.unwrap_or(0)),
                Status::Stopped => {}
            },
            Command::Pause => self.pause(),
            Command::Resume => match self.status {
                Status::Paused => self.resume(),
                Status::Stopped if !self.queue.is_empty() => self.play_index(self.index.unwrap_or(0)),
                _ => {}
            },
            Command::Stop => self.stop(),
            Command::Next => self.advance(false),
            Command::Prev => self.prev(),
            Command::Seek { position_ms } => self.seek(Duration::from_millis(position_ms)),
            Command::SeekBy { offset_ms } => {
                let position = self.position().as_millis() as i64;
                self.seek(Duration::from_millis(position.saturating_add(offset_ms).max(0) as u64));
            }
            Command::SetVolume { volume } => {
                self.config.volume = volume.min(100);
                self.fade_token.fetch_add(1, Ordering::Relaxed);
                if !self.starting
                    && let Some(sink) = self.current.as_ref().and_then(|c| c.sink.as_ref())
                {
                    sink.set_volume(self.volume_factor());
                }
                self.save_config();
                self.publish();
            }
            Command::SetMode { mode } => {
                self.config.mode = mode;
                self.history.clear();
                self.save_config();
                self.publish();
            }
            Command::SetQuality { quality } => {
                self.config.quality = quality;
                self.save_config();
                // 当前曲目（播放中、暂停或仍在加载）按新音质从当前位置重新加载，暂停状态保持不变。
                match self.index {
                    Some(index) if self.status != Status::Stopped && (self.current.is_some() || self.loading) => {
                        let position = self.position();
                        let paused = self.status == Status::Paused;
                        self.start(index, position, paused);
                    }
                    _ => self.publish(),
                }
            }
            Command::ReloadCredential { tag } => match crate::read_credential(&self.paths) {
                Ok(credential) => {
                    let credential = credential.unwrap_or_default();
                    let event =
                        Event::CredentialChanged { logged_in: credential.is_valid(), musicid: credential.musicid };
                    self.client.set_credential(credential);
                    self.done(tag, Ok(()));
                    let _ = self.events.send(event);
                }
                // 读不出来时保留原来的登录状态，不把它当成「已退出」。
                Err(e) => self.done(tag, Err(format!("读取登录凭证失败: {e}"))),
            },
            Command::GetQueue => {
                let _ = self.events.send(self.snapshot().to_event());
            }
            // 由守护进程按连接处理。
            Command::Hello { .. } | Command::Shutdown => {}
        }
    }

    fn fresh_ids(&mut self, count: usize) -> Vec<u64> {
        let start = self.next_entry + 1;
        self.next_entry += count as u64;
        (start..=self.next_entry).collect()
    }

    fn position_of(&self, entry: u64) -> Option<usize> {
        self.ids.iter().position(|id| *id == entry)
    }

    /// 把客户端给的下标解析成当前队列里的下标。带条目 id 时以 id 为准：条目被移动过就跟过去，
    /// 已不在队列里则返回 `None`；不带 id（旧客户端）时只做越界检查。
    fn resolve(&self, index: usize, entry: Option<u64>) -> Option<usize> {
        match entry {
            Some(id) if self.ids.get(index) == Some(&id) => Some(index),
            Some(id) => self.position_of(id),
            None => (index < self.queue.len()).then_some(index),
        }
    }

    fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot { tracks: self.queue.clone(), ids: self.ids.clone(), version: self.queue_version }
    }

    /// 队列内容变化后调用。调用前 `index` / 历史必须已经调整完毕；状态由调用方随后发布，
    /// 这样客户端先拿到新队列，再拿到与之匹配的状态。
    fn queue_changed(&mut self) {
        debug_assert_eq!(self.queue.len(), self.ids.len());
        debug_assert!(self.index.is_none_or(|i| i < self.queue.len()));
        if let Some(radio) = &mut self.radio {
            radio.remember(&self.queue);
        }
        self.queue_version += 1;
        let snapshot = self.snapshot();
        let _ = self.events.send(snapshot.to_event());
        self.queue_tx.send_replace(Arc::new(snapshot));
    }

    /// 插入曲目之后：没有当前曲目时从 `at` 开始播放，否则不打断正在播放的曲目。
    fn after_insert(&mut self, at: usize) {
        if self.index.is_none() {
            self.play_index(at);
        } else {
            self.continue_recommendations();
            self.publish();
        }
    }

    fn remove(&mut self, index: usize) {
        let id = self.ids.remove(index);
        self.queue.remove(index);
        self.history.retain(|entry| *entry != id);
        self.up_next.retain(|entry| *entry != id);
        match self.index {
            // 删除的是当前曲目：换成顶上来的那一首，没有则停止。暂停 / 停止状态保持不变。
            Some(cur) if cur == index => {
                let status = self.status;
                self.halt();
                if index < self.queue.len() && status != Status::Stopped {
                    self.queue_changed();
                    self.start(index, Duration::ZERO, status == Status::Paused);
                } else {
                    self.index = (index < self.queue.len()).then_some(index);
                    if let Some(radio) = &mut self.radio {
                        radio.cancel();
                        radio.waiting = false;
                    }
                    self.status = Status::Stopped;
                    self.queue_changed();
                    self.publish();
                }
            }
            // 删除的在当前曲目之前：只修正下标，实际播放的曲目不变。
            Some(cur) if cur > index => {
                self.index = Some(cur - 1);
                self.queue_changed();
                self.publish();
            }
            _ => {
                self.queue_changed();
                self.publish();
            }
        }
    }

    fn error(&self, kind: ErrorKind, message: impl Into<String>) {
        let message = crate::redact(&message.into());
        eprintln!("{message}");
        let _ = self.events.send(Event::Error { message, kind });
    }

    /// 回报带 `tag` 的指令的执行结果。
    fn done(&self, tag: Option<u64>, result: Result<(), String>) {
        if let Some(tag) = tag {
            let (ok, message) = match result {
                Ok(()) => (true, String::new()),
                Err(message) => (false, message),
            };
            let _ = self.events.send(Event::Done { tag, ok, message });
        }
    }

    fn fade_duration(&self) -> Duration {
        Duration::from_millis(self.config.fade_ms)
    }

    /// 从静音渐入到当前音量并开始播放。
    fn fade_in(&self, sink: &Arc<rodio::Player>) {
        let target = self.volume_factor();
        let duration = self.fade_duration();
        let id = self.fade_token.fetch_add(1, Ordering::Relaxed) + 1;
        if duration.is_zero() {
            sink.set_volume(target);
            sink.play();
            return;
        }
        sink.set_volume(0.0);
        sink.play();
        let token = self.fade_token.clone();
        tokio::spawn(ramp(sink.clone(), 0.0, target, duration, move || token.load(Ordering::Relaxed) == id));
    }

    /// 渐出后释放音源（期间继续读取内存中已下载的数据）。
    fn fade_out(&self, sink: Arc<rodio::Player>) {
        let duration = self.fade_duration();
        if duration.is_zero() || sink.is_paused() {
            return;
        }
        let from = sink.volume();
        tokio::spawn(async move {
            ramp(sink.clone(), from, 0.0, duration, || true).await;
            sink.stop();
        });
    }

    /// 丢弃当前曲目，正在输出的音源渐出。
    fn release_current(&mut self) {
        if let Some(sink) = self.current.take().and_then(|c| c.sink) {
            self.fade_out(sink);
        }
    }

    /// 丢弃当前音源，并让在途的加载 / 跳转失效：加载任务被中止，已经发出的结果因代次不符被丢弃。
    fn halt(&mut self) {
        self.release_current();
        self.generation += 1;
        if let Some(task) = self.load_task.take() {
            task.abort();
        }
        self.loading = false;
        self.target = None;
        self.seeking = None;
        self.starting = false;
        self.playing_quality = None;
    }

    fn volume_factor(&self) -> f32 {
        let v = f32::from(self.config.volume) / 100.0;
        v * v
    }

    /// 对外报告的播放位置：有未完成的跳转时报告跳转目标。
    fn position(&self) -> Duration {
        if let Some(position) = self.target.or(self.seeking.map(|(_, position)| position)) {
            return position;
        }
        match &self.current {
            Some(Current { sink: Some(sink), .. }) => sink.get_pos(),
            Some(current) => current.paused_at,
            None if self.loading => self.load_at,
            None => Duration::ZERO,
        }
    }

    fn on_tick(&mut self, ticks: u64) {
        self.fetch_recommendations();
        enum Action {
            None,
            End,
            StartBuffering,
            StopBuffering,
        }
        let action = match &self.current {
            Some(Current { sink: Some(sink), stream, buffering, .. }) if !self.starting => {
                let (ahead, done) = stream.ahead();
                // 按平均码率换算出 2 秒 / 6 秒的数据量作为缓冲阈值。
                let rate = match (stream.content_length(), self.duration_ms) {
                    (Some(len), ms) if ms > 0 => len * 1000 / ms,
                    _ => 128 * 1024,
                };
                let (low, high) = ((rate * 2).max(64 * 1024), (rate * 6).max(256 * 1024));
                match self.status {
                    Status::Playing if sink.empty() => Action::End,
                    Status::Playing if !done && ahead < low => Action::StartBuffering,
                    Status::Loading if *buffering && (done || ahead >= high) => Action::StopBuffering,
                    _ => Action::None,
                }
            }
            _ => Action::None,
        };
        match action {
            Action::End => {
                // 解码器读到下载断点时和读到文件尾一样结束，这里区分开。
                let failure = self.current.as_ref().and_then(|c| c.stream.failure());
                return match failure {
                    Some(message) => self.on_stream_failed(message),
                    None => self.on_track_end(),
                };
            }
            Action::StartBuffering | Action::StopBuffering => {
                let start = matches!(action, Action::StartBuffering);
                if let Some(current) = &mut self.current
                    && let Some(sink) = &current.sink
                {
                    if start {
                        sink.pause()
                    } else {
                        sink.play()
                    }
                    current.buffering = start;
                    self.status = if start { Status::Loading } else { Status::Playing };
                    self.publish();
                }
                return;
            }
            Action::None => {}
        }
        if self.status == Status::Playing {
            if self.seeking.is_none() {
                self.last_pos = self.position();
            }
            if ticks.is_multiple_of(2) {
                self.publish();
            }
        }
        let idle = matches!(self.status, Status::Stopped | Status::Paused)
            && !self.loading
            && self.current.as_ref().is_none_or(|c| c.sink.is_none());
        self.idle_ticks = if idle { self.idle_ticks + 1 } else { 0 };
        if self.idle_ticks == IDLE_CLOSE_TICKS {
            self.output.close();
        }
    }

    /// 用户要求播放第 `index` 首。
    fn play_index(&mut self, index: usize) {
        self.start(index, Duration::ZERO, false);
    }

    /// 从 `at` 处加载第 `index` 首；`paused` 为真时保持暂停，加载完成后不出声。
    fn start(&mut self, index: usize, at: Duration, paused: bool) {
        let Some(track) = self.queue.get(index).cloned() else { return };
        if let Some(radio) = &mut self.radio {
            radio.played.insert(track.mid.clone());
            radio.waiting = false;
            if self.status == Status::Stopped {
                radio.failures = 0;
                radio.retry_at = tokio::time::Instant::now();
            }
        }
        let entry = self.ids[index];
        self.up_next.retain(|id| *id != entry);
        self.halt();
        self.index = Some(index);
        self.status = if paused { Status::Paused } else { Status::Loading };
        self.loading = true;
        self.load_at = at;
        self.last_pos = at;
        self.retried = false;
        self.duration_ms = track.duration * 1000;
        self.publish();

        let mixer = match self.output.mixer() {
            Ok(mixer) => mixer,
            Err(message) => {
                self.error(ErrorKind::Local, message);
                return self.stop();
            }
        };
        let generation = self.generation;
        let client = self.client.clone();
        let quality = self.config.quality;
        let tx = self.tx.clone();
        let cdn = self.cdn.clone();
        let task = tokio::spawn(async move {
            let result = load(client, mixer, &cdn, track, quality, at).await;
            let _ = tx.send(Internal::Loaded(Loaded { generation, result }));
        });
        self.load_task = Some(task.abort_handle());
    }

    fn on_loaded(&mut self, loaded: Loaded) {
        // 切歌 / 停止之后才返回的旧任务：成功或失败都不回写。
        if loaded.generation != self.generation {
            return;
        }
        self.loading = false;
        self.load_task = None;
        let prepared = match loaded.result {
            Ok(prepared) => prepared,
            Err(error) => return self.on_load_failed(error),
        };
        self.fail_streak = 0;
        if let Some(duration) = prepared.duration {
            self.duration_ms = duration.as_millis() as u64;
        }
        self.playing_quality = Some(prepared.quality);
        if let Some(stream) = prepared.stream {
            self.current = Some(Current {
                sink: None,
                stream,
                hint: prepared.hint,
                paused_at: prepared.position,
                buffering: false,
            });
        }
        let Some(current) = &mut self.current else { return };
        // 加载期间被暂停：不保留音源，恢复时再重建；期间的跳转目标记为暂停位置。
        if self.status == Status::Paused {
            if let Some(position) = self.target.take() {
                current.paused_at = position;
            }
            return self.publish();
        }
        let sink = Arc::new(prepared.sink);
        current.sink = Some(sink.clone());
        current.buffering = false;
        match self.target.take() {
            // 加载期间又跳转过：先跳到最新目标，完成后再出声。
            Some(position) if position != prepared.position => {
                self.starting = true;
                self.start_seek(sink, position);
            }
            _ => {
                self.fade_in(&sink);
                self.status = Status::Playing;
            }
        }
        self.publish();
    }

    /// 当前曲目无法播放：报告原因并换下一首；连续失败达到上限时停止。暂停状态保持不变。
    fn on_load_failed(&mut self, error: LoadError) {
        let name = self.index.and_then(|i| self.queue.get(i)).map(|t| t.name.clone()).unwrap_or_default();
        self.error(error.kind, format!("《{name}》{}", error.message));
        self.fail_streak += 1;
        let paused = self.status == Status::Paused;
        self.release_current();
        let limit = match error.kind {
            // 登录失效时换哪一首都一样。
            ErrorKind::Login => 1,
            ErrorKind::Network => MAX_NETWORK_FAIL_STREAK,
            _ => MAX_FAIL_STREAK,
        }
        .min(self.queue.len().max(1));
        if self.fail_streak < limit {
            self.advance_with(false, paused);
        } else {
            if self.fail_streak > 1 {
                self.error(error.kind, format!("连续 {} 首无法播放，已停止", self.fail_streak));
            }
            self.fail_streak = 0;
            self.stop();
        }
    }

    /// 下载中途断开：从断点重新加载一次，仍然失败则按加载失败处理。
    fn on_stream_failed(&mut self, message: String) {
        let Some(index) = self.index else { return self.stop() };
        if self.retried {
            return self.on_load_failed(LoadError::new(ErrorKind::Network, message));
        }
        let position = self.last_pos;
        let name = self.queue[index].name.clone();
        self.error(ErrorKind::Network, format!("《{name}》{message}，正在从断点重新加载"));
        self.start(index, position, false);
        self.retried = true;
    }

    fn on_track_end(&mut self) {
        self.current = None;
        self.advance(true);
    }

    /// 切到下一首；`auto` 表示自然播完（单曲循环时重播）。
    fn advance(&mut self, auto: bool) {
        self.advance_with(auto, false);
    }

    /// `paused` 为真时保持暂停：换到下一首并加载，但不出声。
    fn advance_with(&mut self, auto: bool, paused: bool) {
        match self.next_index(auto) {
            Some(index) => {
                self.push_history();
                self.start(index, Duration::ZERO, paused);
            }
            None if self.radio.as_ref().is_some_and(|r| r.failures < radio::MAX_FAILURES) => {
                self.halt();
                self.radio.as_mut().unwrap().waiting = true;
                // 暂停时不去取推荐，恢复播放时再取。
                self.status = if paused { Status::Paused } else { Status::Loading };
                self.publish();
                self.fetch_recommendations();
            }
            None => self.stop(),
        }
    }

    fn next_index(&self, auto: bool) -> Option<usize> {
        // 随机模式下「下一首播放」的条目优先，保证用户安排的顺序。
        if self.config.mode == PlayMode::Shuffle
            && let Some(index) = self.up_next.iter().find_map(|id| self.position_of(*id))
        {
            return Some(index);
        }
        if let Some(radio) = &self.radio {
            return radio.next_index(&self.queue, self.index, self.config.mode, auto);
        }
        let len = self.queue.len();
        if len == 0 {
            return None;
        }
        let Some(cur) = self.index else { return Some(0) };
        match self.config.mode {
            PlayMode::Single if auto => Some(cur),
            PlayMode::Order => (cur + 1 < len).then_some(cur + 1),
            PlayMode::Loop | PlayMode::Single => Some((cur + 1) % len),
            PlayMode::Shuffle if len == 1 => Some(0),
            PlayMode::Shuffle => {
                let next = fastrand::usize(..len - 1);
                Some(if next >= cur { next + 1 } else { next })
            }
        }
    }

    /// 当前曲目自然播完后确定会播放的下标；随机挑选时无法预知，返回 `None`。
    fn peek_next(&self) -> Option<usize> {
        let cur = self.index?;
        let len = self.queue.len();
        match self.config.mode {
            PlayMode::Single => Some(cur),
            PlayMode::Shuffle => self.up_next.iter().find_map(|id| self.position_of(*id)),
            PlayMode::Order => (cur + 1 < len).then_some(cur + 1),
            PlayMode::Loop if self.radio.is_some() => (cur + 1 < len).then_some(cur + 1),
            PlayMode::Loop => Some((cur + 1) % len),
        }
    }

    fn push_history(&mut self) {
        if let Some(cur) = self.index {
            self.history.push(self.ids[cur]);
            if self.history.len() > 200 {
                self.history.remove(0);
            }
        }
    }

    fn prev(&mut self) {
        let len = self.queue.len();
        if len == 0 {
            return;
        }
        if self.config.mode == PlayMode::Shuffle {
            // 历史里已被移出队列的条目直接跳过。
            while let Some(entry) = self.history.pop() {
                if let Some(index) = self.position_of(entry) {
                    return self.play_index(index);
                }
            }
        }
        let index = match self.index {
            Some(0) | None if self.config.mode == PlayMode::Order => 0,
            Some(0) | None => len - 1,
            Some(cur) => cur - 1,
        };
        self.play_index(index);
    }

    fn pause(&mut self) {
        if !matches!(self.status, Status::Playing | Status::Loading) {
            return;
        }
        // 丢弃音源（停止解码与混音），只保留内存中的下载数据与位置。
        let pending = self.target.or(self.seeking.map(|(_, position)| position));
        let released = self.current.as_mut().and_then(|current| {
            current.buffering = false;
            let sink = current.sink.take()?;
            current.paused_at = pending.unwrap_or_else(|| sink.get_pos());
            Some(sink)
        });
        if let Some(sink) = released {
            // 音源没了，未完成的跳转已经折算进暂停位置。
            self.target = None;
            self.seeking = None;
            if !self.starting {
                self.fade_out(sink);
            }
            self.starting = false;
        }
        self.status = Status::Paused;
        self.publish();
    }

    fn resume(&mut self) {
        if self.status != Status::Paused {
            return;
        }
        if let Some(radio) = &mut self.radio
            && radio.waiting
        {
            radio.waiting = false;
            radio.failures = 0;
            radio.retry_at = tokio::time::Instant::now();
            return self.advance(false);
        }
        match &self.current {
            Some(Current { sink: Some(sink), .. }) => {
                let sink = sink.clone();
                self.fade_in(&sink);
                self.status = Status::Playing;
                self.publish();
            }
            // 重建已经在途（暂停后马上恢复）时等它完成即可。
            Some(_) if self.loading => {
                self.status = Status::Loading;
                self.publish();
            }
            Some(_) => {
                self.status = Status::Loading;
                self.rebuild();
                self.publish();
            }
            // 加载尚未完成，完成后自动播放。
            None if self.loading => {
                self.status = Status::Loading;
                self.publish();
            }
            None => match self.index {
                Some(index) => self.play_index(index),
                None => self.stop(),
            },
        }
    }

    /// 暂停后恢复：从内存缓冲重建解码器并跳回暂停位置。
    fn rebuild(&mut self) {
        let mixer = match self.output.mixer() {
            Ok(mixer) => mixer,
            Err(message) => {
                self.error(ErrorKind::Local, message);
                return self.stop();
            }
        };
        let Some(current) = &self.current else { return };
        let reader = current.stream.reader();
        let len = current.stream.content_length();
        let hint = current.hint.clone();
        let position = current.paused_at;
        let quality = self.playing_quality.unwrap_or(self.config.quality);
        let generation = self.generation;
        let tx = self.tx.clone();
        self.loading = true;
        self.load_at = position;
        let task = tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let (sink, duration) = build_sink(&mixer, reader, len, &hint, position)?;
                Ok(Prepared { sink, stream: None, hint, duration, quality, position })
            })
            .await
            .map_err(|e| LoadError::new(ErrorKind::Local, e.to_string()))
            .and_then(|r| r);
            let _ = tx.send(Internal::Loaded(Loaded { generation, result }));
        });
        self.load_task = Some(task.abort_handle());
    }

    fn stop(&mut self) {
        if let Some(radio) = &mut self.radio {
            radio.cancel();
            radio.waiting = false;
        }
        self.halt();
        self.status = Status::Stopped;
        self.publish();
    }

    /// 跳转到 `position`。连续跳转只保留最新目标；暂停时只移动位置，不恢复播放。
    fn seek(&mut self, position: Duration) {
        if self.status == Status::Stopped {
            return;
        }
        let position = if self.duration_ms > 0 {
            position.min(Duration::from_millis(self.duration_ms.saturating_sub(500)))
        } else {
            position
        };
        match &mut self.current {
            Some(Current { sink: Some(sink), .. }) => {
                if self.seeking.is_some() {
                    self.target = Some(position);
                } else {
                    let sink = sink.clone();
                    self.start_seek(sink, position);
                }
            }
            Some(current) => {
                current.paused_at = position;
                // 重建在途：它用的是旧位置，完成后再补一次跳转。
                if self.loading {
                    self.target = Some(position);
                }
            }
            None if self.loading => self.target = Some(position),
            // 没有可跳转的曲目（如正在等待推荐歌曲）。
            None => return,
        }
        self.last_pos = position;
        self.publish();
    }

    /// `try_seek` 会等待音频线程执行，放到阻塞线程池里做，结果回报给引擎。
    fn start_seek(&mut self, sink: Arc<rodio::Player>, position: Duration) {
        self.seek_seq += 1;
        let seq = self.seek_seq;
        self.seeking = Some((seq, position));
        let seek = self.seek_fn.clone();
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = seek(&sink, position);
            let _ = tx.send(Internal::Seeked { seq, result });
        });
    }

    fn on_seeked(&mut self, seq: u64, result: Result<(), String>) {
        // 切歌、暂停之后才返回的旧跳转。
        if self.seeking.map(|(current, _)| current) != Some(seq) {
            return;
        }
        self.seeking = None;
        if let Err(e) = result {
            self.error(ErrorKind::Local, format!("跳转失败: {e}"));
        }
        let Some(sink) = self.current.as_ref().and_then(|c| c.sink.clone()) else { return self.publish() };
        // 跳转期间又有新目标：接着跳到最新的那个。
        if let Some(next) = self.target.take() {
            self.start_seek(sink, next);
            return self.publish();
        }
        if self.starting {
            self.starting = false;
            if self.status == Status::Loading {
                self.fade_in(&sink);
                self.status = Status::Playing;
            }
        }
        self.publish();
    }

    fn save_config(&self) {
        if let Err(e) = self.config.save(&self.paths) {
            eprintln!("保存配置失败: {e}");
        }
    }

    fn fetch_recommendations(&mut self) {
        if self.status != Status::Loading {
            return;
        }
        let Some(radio) = &mut self.radio else { return };
        if !radio.waiting
            || radio.pending.is_some()
            || radio.failures >= radio::MAX_FAILURES
            || tokio::time::Instant::now() < radio.retry_at
        {
            return;
        }
        let client = self.client.clone();
        let source = radio.source;
        radio.pending = Some(tokio::spawn(async move {
            tokio::time::timeout(radio::FETCH_TIMEOUT, crate::recommendation::load(&client, source))
                .await
                .map_err(|_| "获取推荐歌曲超时".to_string())?
        }));
    }

    fn on_recommendation(&mut self, result: Result<Batch, String>) {
        let Some(radio) = &mut self.radio else { return };
        radio.pending.take();
        let (tracks, error) = match result {
            Ok(batch) => {
                radio.source = batch.next;
                let tracks: Vec<_> = batch
                    .tracks
                    .into_iter()
                    .filter(|t| !t.mid.is_empty() && radio.seen.insert(t.mid.clone()))
                    .collect();
                (tracks, None)
            }
            Err(error) => (Vec::new(), Some(error)),
        };
        if tracks.is_empty() {
            radio.failures += 1;
            radio.retry_at = tokio::time::Instant::now() + Duration::from_secs(5 << (radio.failures - 1));
            let failures = radio.failures;
            let waiting = radio.waiting;
            if let Some(error) = error {
                self.error(ErrorKind::Network, format!("推荐续播失败: {error}"));
            } else if failures >= radio::MAX_FAILURES {
                self.error(ErrorKind::Other, "推荐续播暂时没有新歌曲，请稍后重试");
            }
            if waiting && failures >= radio::MAX_FAILURES && self.status == Status::Loading {
                self.stop();
            }
            return;
        }
        radio.failures = 0;
        let ids = self.fresh_ids(tracks.len());
        self.ids.extend(ids);
        self.queue.extend(tracks);
        self.queue_changed();
        self.continue_recommendations();
        self.publish();
    }

    fn continue_recommendations(&mut self) {
        if self.radio.as_ref().is_some_and(|r| r.waiting)
            && self.status == Status::Loading
            && self.next_index(false).is_some()
        {
            self.advance(false);
        }
    }

    fn activity(&self) -> Activity {
        if self.status == Status::Stopped {
            return Activity::Idle;
        }
        let waiting = self.radio.as_ref().is_some_and(|r| r.waiting) && self.status == Status::Loading;
        if self.loading || self.starting || waiting {
            return Activity::Loading;
        }
        match &self.current {
            Some(current) if current.buffering => Activity::Buffering,
            Some(Current { sink: Some(_), .. }) if self.status == Status::Playing => Activity::Output,
            _ => Activity::Idle,
        }
    }

    fn publish(&self) {
        let state = PlayerState {
            status: self.status,
            index: self.index,
            track: self.index.and_then(|i| self.queue.get(i)).cloned(),
            position_ms: self.position().as_millis() as u64,
            duration_ms: self.duration_ms,
            volume: self.config.volume,
            mode: self.config.mode,
            quality: self.config.quality,
            playing_quality: self.playing_quality,
            queue_len: self.queue.len(),
            queue_version: self.queue_version,
            entry: self.index.and_then(|i| self.ids.get(i)).copied(),
            next: self.peek_next(),
            activity: self.activity(),
            protocol: PROTOCOL_VERSION,
        };
        self.state_tx.send_replace(state.clone());
        let _ = self.events.send(Event::State(Box::new(state)));
    }
}

/// 在 `duration` 内把音量从 `from` 线性调到 `to`；`active` 返回 false 时提前停止。
async fn ramp(sink: Arc<rodio::Player>, from: f32, to: f32, duration: Duration, active: impl Fn() -> bool) {
    const STEP: Duration = Duration::from_millis(15);
    let steps = (duration.as_millis() / STEP.as_millis()).max(1) as u32;
    for i in 1..=steps {
        tokio::time::sleep(duration / steps).await;
        if !active() {
            return;
        }
        sink.set_volume(from + (to - from) * i as f32 / steps as f32);
    }
}

fn classify(error: &qqmusic_api::Error) -> ErrorKind {
    use qqmusic_api::Error;
    match error {
        _ if error.is_credential_expired() => ErrorKind::Login,
        Error::CredentialInvalid(_) => ErrorKind::Login,
        Error::Network(_) | Error::Http { .. } => ErrorKind::Network,
        _ => ErrorKind::Other,
    }
}

async fn load(
    client: Client,
    mixer: rodio::mixer::Mixer,
    cdn: &cdn::CdnSelector,
    track: Track,
    quality: Quality,
    at: Duration,
) -> Result<Prepared, LoadError> {
    let url = client
        .song()
        .playable_url(&track.mid, Some(&track.media_mid), quality)
        .await
        .map_err(|e| LoadError::new(classify(&e), format!("获取播放链接失败: {e}")))?
        .ok_or_else(|| {
            let hint = if client.credential().is_valid() { "" } else { "，登录后可能可以播放" };
            LoadError::new(ErrorKind::Rights, format!("没有可用的播放链接（可能需要会员、无版权或地区受限{hint}）"))
        })?;
    let full_url = cdn.resolve(&client, &url.url).await;
    let (stream, reader, len) =
        stream::open(cdn.transport(), &full_url).await.map_err(|e| LoadError::new(ErrorKind::Network, e))?;
    let hint = url.file_type.extension.trim_start_matches('.').to_string();
    let build_hint = hint.clone();
    let (sink, duration) = tokio::task::spawn_blocking(move || build_sink(&mixer, reader, len, &build_hint, at))
        .await
        .map_err(|e| LoadError::new(ErrorKind::Local, e.to_string()))??;
    Ok(Prepared { sink, stream: Some(stream), hint, duration, quality: url.quality, position: at })
}

/// 创建解码器并接入混音器，跳到 `at`（保持暂停，由调用方决定何时播放）。
fn build_sink(
    mixer: &rodio::mixer::Mixer,
    reader: StreamReader,
    len: Option<u64>,
    hint: &str,
    at: Duration,
) -> Result<(rodio::Player, Option<Duration>), LoadError> {
    let mut builder = Decoder::builder().with_data(reader).with_seekable(true).with_hint(hint);
    if let Some(len) = len {
        builder = builder.with_byte_len(len);
    }
    let decoder = builder.build().map_err(|e| LoadError::new(ErrorKind::Local, format!("解码失败: {e}")))?;
    let duration = decoder.total_duration();
    let sink = rodio::Player::connect_new(mixer);
    sink.pause();
    sink.append(decoder);
    if !at.is_zero() {
        sink.try_seek(at).map_err(|e| LoadError::new(ErrorKind::Local, format!("跳转到播放位置失败: {e}")))?;
    }
    Ok((sink, duration))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::ramp;

    #[tokio::test]
    async fn ramp_reaches_target_and_stops_when_cancelled() {
        let (sink, _output) = rodio::Player::new();
        let sink = Arc::new(sink);

        sink.set_volume(0.0);
        let mid = {
            let sink = sink.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(60)).await;
                sink.volume()
            })
        };
        ramp(sink.clone(), 0.0, 0.8, Duration::from_millis(120), || true).await;
        let mid = mid.await.unwrap();
        assert!(mid > 0.0 && mid < 0.8, "渐变过程中音量应介于两端: {mid}");
        assert!((sink.volume() - 0.8).abs() < 1e-6);

        // 取消后不再修改音量。
        let active = Arc::new(AtomicBool::new(true));
        let flag = active.clone();
        let task = tokio::spawn(ramp(sink.clone(), 0.8, 0.0, Duration::from_millis(300), move || {
            flag.load(Ordering::Relaxed)
        }));
        tokio::time::sleep(Duration::from_millis(60)).await;
        active.store(false, Ordering::Relaxed);
        task.await.unwrap();
        let frozen = sink.volume();
        assert!(frozen > 0.0 && frozen < 0.8, "{frozen}");
    }
}
