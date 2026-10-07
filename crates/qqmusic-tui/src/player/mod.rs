//! 播放引擎：队列、播放模式、音质与音频输出。
//!
//! 引擎是一个 tokio 任务，通过 [`PlayerHandle`] 收发 [`Command`] / [`Event`]，
//! 可以独立嵌入到其他程序中使用（守护进程只是在外面包了一层 IPC）。

pub mod cdn;
pub mod stream;

use std::sync::Arc;
use std::time::Duration;

use qqmusic_api::Client;
use qqmusic_api::modules::song::Quality;
use rodio::{Decoder, Source};
use tokio::sync::{broadcast, mpsc, watch};

use self::stream::StreamHandle;
use crate::config::Config;
use crate::paths::Paths;
use crate::protocol::{Command, Event, PlayMode, PlayerState, Status, Track};

/// 播放引擎句柄（可克隆）。
#[derive(Clone)]
pub struct PlayerHandle {
    commands: mpsc::UnboundedSender<Command>,
    state: watch::Receiver<PlayerState>,
    queue: watch::Receiver<Arc<Vec<Track>>>,
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
    pub fn queue(&self) -> Arc<Vec<Track>> {
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
    let mixer = open_output()?;
    let config = Config::load(&paths);
    let (commands, command_rx) = mpsc::unbounded_channel();
    let (events, _) = broadcast::channel(64);
    let (state_tx, state) = watch::channel(PlayerState::default());
    let (queue_tx, queue) = watch::channel(Arc::new(Vec::new()));
    let (load_tx, load_rx) = mpsc::unbounded_channel();
    let cdn = Arc::new(cdn::CdnSelector::new(&client));
    let engine = Engine {
        client,
        paths,
        config,
        mixer,
        queue: Vec::new(),
        index: None,
        history: Vec::new(),
        status: Status::Stopped,
        current: None,
        generation: 0,
        resume_at: None,
        playing_quality: None,
        duration_ms: 0,
        queue_version: 0,
        fail_streak: 0,
        cdn,
        load_tx,
        events: events.clone(),
        state_tx,
        queue_tx,
    };
    tokio::spawn(engine.run(command_rx, load_rx));
    Ok(PlayerHandle { commands, state, queue, events })
}

/// 在专用线程中打开默认音频输出（`cpal::Stream` 不能跨线程移动），返回混音器。
fn open_output() -> Result<rodio::mixer::Mixer, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("audio-output".into())
        .spawn(move || match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut sink) => {
                sink.log_on_drop(false);
                let _ = tx.send(Ok(sink.mixer().clone()));
                loop {
                    std::thread::park();
                }
            }
            Err(e) => {
                let _ = tx.send(Err(format!("无法打开音频输出设备: {e}")));
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

struct Current {
    sink: Arc<rodio::Player>,
    _stream: StreamHandle,
}

struct Prepared {
    sink: rodio::Player,
    stream: StreamHandle,
    duration: Option<Duration>,
    quality: Quality,
}

struct Loaded {
    generation: u64,
    result: Result<Prepared, String>,
}

struct Engine {
    client: Client,
    paths: Paths,
    config: Config,
    mixer: rodio::mixer::Mixer,
    queue: Vec<Track>,
    index: Option<usize>,
    /// 随机模式下的播放历史，用于「上一首」。
    history: Vec<usize>,
    status: Status,
    current: Option<Current>,
    generation: u64,
    resume_at: Option<Duration>,
    playing_quality: Option<Quality>,
    duration_ms: u64,
    queue_version: u64,
    /// 连续加载失败次数，避免整张歌单都不可播时无限跳歌。
    fail_streak: usize,
    cdn: Arc<cdn::CdnSelector>,
    load_tx: mpsc::UnboundedSender<Loaded>,
    events: broadcast::Sender<Event>,
    state_tx: watch::Sender<PlayerState>,
    queue_tx: watch::Sender<Arc<Vec<Track>>>,
}

impl Engine {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>, mut loads: mpsc::UnboundedReceiver<Loaded>) {
        self.publish();
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let mut ticks = 0u64;
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown) | None => break,
                    Some(command) => self.handle(command),
                },
                Some(loaded) = loads.recv() => self.on_loaded(loaded),
                _ = tick.tick() => {
                    ticks += 1;
                    if self.status == Status::Playing && self.current.as_ref().is_some_and(|c| c.sink.empty()) {
                        self.on_track_end();
                    } else if self.status == Status::Playing && ticks.is_multiple_of(2) {
                        self.publish();
                    }
                }
            }
        }
        self.current = None;
        let _ = self.events.send(Event::Shutdown);
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Play { tracks, index } => {
                self.set_queue(tracks);
                self.history.clear();
                if index < self.queue.len() {
                    self.play_index(index);
                } else {
                    self.stop();
                }
            }
            Command::Append { tracks } => {
                let start = self.queue.len();
                let mut queue = std::mem::take(&mut self.queue);
                queue.extend(tracks);
                self.set_queue(queue);
                if self.index.is_none() && start < self.queue.len() {
                    self.play_index(start);
                }
            }
            Command::PlayNext { tracks } => {
                let at = self.index.map_or(0, |i| i + 1);
                let mut queue = std::mem::take(&mut self.queue);
                let count = tracks.len();
                queue.splice(at..at, tracks);
                self.set_queue(queue);
                if self.index.is_none() && count > 0 {
                    self.play_index(at);
                }
            }
            Command::PlayIndex { index } => {
                if index < self.queue.len() {
                    self.push_history();
                    self.play_index(index);
                }
            }
            Command::Remove { index } => self.remove(index),
            Command::Clear => {
                self.stop();
                self.index = None;
                self.history.clear();
                self.set_queue(Vec::new());
            }
            Command::Toggle => match self.status {
                Status::Playing => self.pause(),
                Status::Paused => self.resume(),
                Status::Stopped if !self.queue.is_empty() => self.play_index(self.index.unwrap_or(0)),
                _ => {}
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
                if let Some(current) = &self.current {
                    current.sink.set_volume(self.volume_factor());
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
                // 正在播放的曲目按新音质从当前位置重新加载。
                if let (Some(index), Status::Playing | Status::Paused) = (self.index, self.status) {
                    let position = self.position();
                    let paused = self.status == Status::Paused;
                    self.play_index(index);
                    self.resume_at = Some(position);
                    if paused {
                        self.status = Status::Paused;
                    }
                }
                self.publish();
            }
            Command::ReloadCredential => {
                let credential = crate::load_credential(&self.paths).unwrap_or_default();
                self.client.set_credential(credential);
            }
            Command::GetQueue => {
                let _ = self.events.send(Event::Queue { tracks: self.queue.clone() });
            }
            Command::Shutdown => {}
        }
    }

    fn set_queue(&mut self, queue: Vec<Track>) {
        self.queue = queue;
        self.queue_version += 1;
        self.queue_tx.send_replace(Arc::new(self.queue.clone()));
        let _ = self.events.send(Event::Queue { tracks: self.queue.clone() });
        self.publish();
    }

    fn remove(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        let mut queue = std::mem::take(&mut self.queue);
        queue.remove(index);
        self.history.clear();
        let current = self.index;
        self.set_queue(queue);
        match current {
            Some(cur) if cur == index => {
                if index < self.queue.len() {
                    self.play_index(index);
                } else {
                    self.stop();
                    self.index = None;
                    self.publish();
                }
            }
            Some(cur) if cur > index => {
                self.index = Some(cur - 1);
                self.publish();
            }
            _ => {}
        }
    }

    fn volume_factor(&self) -> f32 {
        let v = f32::from(self.config.volume) / 100.0;
        v * v
    }

    fn position(&self) -> Duration {
        self.current.as_ref().map_or(Duration::ZERO, |c| c.sink.get_pos())
    }

    fn play_index(&mut self, index: usize) {
        let Some(track) = self.queue.get(index).cloned() else { return };
        self.current = None;
        self.generation += 1;
        self.index = Some(index);
        self.status = Status::Loading;
        self.resume_at = None;
        self.playing_quality = None;
        self.duration_ms = track.duration * 1000;
        self.publish();

        let generation = self.generation;
        let client = self.client.clone();
        let mixer = self.mixer.clone();
        let quality = self.config.quality;
        let tx = self.load_tx.clone();
        let cdn = self.cdn.clone();
        tokio::spawn(async move {
            let result = load(client, mixer, &cdn, track, quality).await;
            let _ = tx.send(Loaded { generation, result });
        });
    }

    fn on_loaded(&mut self, loaded: Loaded) {
        if loaded.generation != self.generation || !matches!(self.status, Status::Loading | Status::Paused) {
            return;
        }
        match loaded.result {
            Ok(prepared) => {
                self.fail_streak = 0;
                let sink = Arc::new(prepared.sink);
                sink.set_volume(self.volume_factor());
                if let Some(duration) = prepared.duration {
                    self.duration_ms = duration.as_millis() as u64;
                }
                self.playing_quality = Some(prepared.quality);
                if let Some(position) = self.resume_at.take() {
                    seek_blocking(sink.clone(), position);
                }
                if self.status == Status::Loading {
                    sink.play();
                    self.status = Status::Playing;
                }
                self.current = Some(Current { sink, _stream: prepared.stream });
                self.publish();
            }
            Err(message) => {
                let name = self.index.and_then(|i| self.queue.get(i)).map(|t| t.name.clone()).unwrap_or_default();
                let _ = self.events.send(Event::Error { message: format!("《{name}》{message}") });
                self.fail_streak += 1;
                self.current = None;
                if self.fail_streak < self.queue.len() {
                    self.advance(false);
                } else {
                    self.fail_streak = 0;
                    self.stop();
                }
            }
        }
    }

    fn on_track_end(&mut self) {
        self.current = None;
        self.advance(true);
    }

    /// 切到下一首；`auto` 表示自然播完（单曲循环时重播）。
    fn advance(&mut self, auto: bool) {
        match self.next_index(auto) {
            Some(index) => {
                self.push_history();
                self.play_index(index);
            }
            None => self.stop(),
        }
    }

    fn next_index(&self, auto: bool) -> Option<usize> {
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

    fn push_history(&mut self) {
        if let Some(cur) = self.index {
            self.history.push(cur);
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
        if self.config.mode == PlayMode::Shuffle
            && let Some(index) = self.history.pop().filter(|i| *i < len)
        {
            self.play_index(index);
            return;
        }
        let index = match self.index {
            Some(0) | None if self.config.mode == PlayMode::Order => 0,
            Some(0) | None => len - 1,
            Some(cur) => cur - 1,
        };
        self.play_index(index);
    }

    fn pause(&mut self) {
        if self.status == Status::Playing
            && let Some(current) = &self.current
        {
            current.sink.pause();
            self.status = Status::Paused;
            self.publish();
        }
    }

    fn resume(&mut self) {
        if self.status != Status::Paused {
            return;
        }
        match &self.current {
            Some(current) => {
                current.sink.play();
                self.status = Status::Playing;
                self.publish();
            }
            // 暂停状态下切换了音质，加载完成前恢复播放。
            None => {
                self.status = Status::Loading;
                self.publish();
            }
        }
    }

    fn stop(&mut self) {
        self.current = None;
        self.generation += 1;
        self.status = Status::Stopped;
        self.playing_quality = None;
        self.publish();
    }

    fn seek(&mut self, position: Duration) {
        let position = if self.duration_ms > 0 {
            position.min(Duration::from_millis(self.duration_ms.saturating_sub(500)))
        } else {
            position
        };
        if let Some(current) = &self.current {
            seek_blocking(current.sink.clone(), position);
        } else if self.status == Status::Loading {
            self.resume_at = Some(position);
        }
    }

    fn save_config(&self) {
        if let Err(e) = self.config.save(&self.paths) {
            eprintln!("保存配置失败: {e}");
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
        };
        self.state_tx.send_replace(state.clone());
        let _ = self.events.send(Event::State(Box::new(state)));
    }
}

/// `try_seek` 会等待音频线程执行，放到阻塞线程池里做。
fn seek_blocking(sink: Arc<rodio::Player>, position: Duration) {
    tokio::task::spawn_blocking(move || {
        if let Err(e) = sink.try_seek(position) {
            eprintln!("seek 失败: {e}");
        }
    });
}

async fn load(
    client: Client,
    mixer: rodio::mixer::Mixer,
    cdn: &cdn::CdnSelector,
    track: Track,
    quality: Quality,
) -> Result<Prepared, String> {
    let url = client
        .song()
        .playable_url(&track.mid, Some(&track.media_mid), quality)
        .await
        .map_err(|e| format!("获取播放链接失败: {e}"))?
        .ok_or_else(|| "没有可用的播放链接（可能需要会员或无版权）".to_string())?;
    let full_url = cdn.resolve(&client, &url.url).await;
    let (stream, reader, len) = stream::open(cdn.transport(), &full_url).await?;
    let hint = url.file_type.extension.trim_start_matches('.').to_string();
    let (sink, duration) = tokio::task::spawn_blocking(move || {
        let mut builder = Decoder::builder().with_data(reader).with_seekable(true).with_hint(&hint);
        if let Some(len) = len {
            builder = builder.with_byte_len(len);
        }
        let decoder = builder.build().map_err(|e| format!("解码失败: {e}"))?;
        let duration = decoder.total_duration();
        let sink = rodio::Player::connect_new(&mixer);
        sink.pause();
        sink.append(decoder);
        Ok::<_, String>((sink, duration))
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(Prepared { sink, stream, duration, quality: url.quality })
}
