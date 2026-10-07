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

use self::stream::{StreamHandle, StreamReader};
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
        output: Output::default(),
        idle_ticks: 0,
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
}

struct Loaded {
    generation: u64,
    result: Result<Prepared, String>,
}

struct Engine {
    client: Client,
    paths: Paths,
    config: Config,
    output: Output,
    /// 连续空闲的 tick 数，用于关闭音频设备。
    idle_ticks: u32,
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
                    self.on_tick(ticks);
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
                Status::Playing | Status::Loading => self.pause(),
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
                if let Some(sink) = self.current.as_ref().and_then(|c| c.sink.as_ref()) {
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
        match &self.current {
            Some(Current { sink: Some(sink), .. }) => sink.get_pos(),
            Some(current) => current.paused_at,
            None => Duration::ZERO,
        }
    }

    fn on_tick(&mut self, ticks: u64) {
        enum Action {
            None,
            End,
            StartBuffering,
            StopBuffering,
        }
        let action = match &self.current {
            Some(Current { sink: Some(sink), stream, buffering, .. }) => {
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
            Action::End => return self.on_track_end(),
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
        if self.status == Status::Playing && ticks.is_multiple_of(2) {
            self.publish();
        }
        let idle = matches!(self.status, Status::Stopped | Status::Paused)
            && self.current.as_ref().is_none_or(|c| c.sink.is_none());
        self.idle_ticks = if idle { self.idle_ticks + 1 } else { 0 };
        if self.idle_ticks == IDLE_CLOSE_TICKS {
            self.output.close();
        }
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

        let mixer = match self.output.mixer() {
            Ok(mixer) => mixer,
            Err(message) => {
                let _ = self.events.send(Event::Error { message });
                return self.stop();
            }
        };
        let generation = self.generation;
        let client = self.client.clone();
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
                if let Some(duration) = prepared.duration {
                    self.duration_ms = duration.as_millis() as u64;
                }
                self.playing_quality = Some(prepared.quality);
                // 新曲目：建立 Current，需要时跳到 resume_at；重建的音源已在加载时跳转过。
                let mut seek_to = None;
                if let Some(stream) = prepared.stream {
                    let paused_at = self.resume_at.take().unwrap_or_default();
                    seek_to = (!paused_at.is_zero()).then_some(paused_at);
                    self.current =
                        Some(Current { sink: None, stream, hint: prepared.hint, paused_at, buffering: false });
                }
                // 加载期间被暂停：不保留音源，恢复时再重建。
                if self.status == Status::Paused {
                    return self.publish();
                }
                let volume = self.volume_factor();
                let Some(current) = &mut self.current else { return };
                let sink = Arc::new(prepared.sink);
                sink.set_volume(volume);
                if let Some(position) = seek_to {
                    seek_blocking(sink.clone(), position);
                }
                sink.play();
                current.sink = Some(sink);
                current.buffering = false;
                self.status = Status::Playing;
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
        if !matches!(self.status, Status::Playing | Status::Loading) {
            return;
        }
        // 丢弃音源（停止解码与混音），只保留内存中的下载数据与位置。
        if let Some(current) = &mut self.current {
            if let Some(sink) = current.sink.take() {
                current.paused_at = sink.get_pos();
            }
            current.buffering = false;
        }
        self.status = Status::Paused;
        self.publish();
    }

    fn resume(&mut self) {
        if self.status != Status::Paused {
            return;
        }
        match &self.current {
            Some(Current { sink: Some(sink), .. }) => {
                sink.play();
                self.status = Status::Playing;
                self.publish();
            }
            Some(_) => {
                self.status = Status::Loading;
                self.publish();
                self.rebuild();
            }
            // 加载尚未完成，完成后自动播放。
            None => {
                self.status = Status::Loading;
                self.publish();
            }
        }
    }

    /// 暂停后恢复：从内存缓冲重建解码器并跳回暂停位置。
    fn rebuild(&mut self) {
        let mixer = match self.output.mixer() {
            Ok(mixer) => mixer,
            Err(message) => {
                let _ = self.events.send(Event::Error { message });
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
        let tx = self.load_tx.clone();
        tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let (sink, duration) = build_sink(&mixer, reader, len, &hint)?;
                if !position.is_zero() {
                    sink.try_seek(position).map_err(|e| format!("恢复播放位置失败: {e}"))?;
                }
                Ok(Prepared { sink, stream: None, hint, duration, quality })
            })
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r);
            let _ = tx.send(Loaded { generation, result });
        });
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
        match &mut self.current {
            Some(Current { sink: Some(sink), .. }) => seek_blocking(sink.clone(), position),
            Some(current) => {
                current.paused_at = position;
                self.publish();
            }
            None if self.status == Status::Loading => self.resume_at = Some(position),
            None => {}
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
    let build_hint = hint.clone();
    let (sink, duration) = tokio::task::spawn_blocking(move || build_sink(&mixer, reader, len, &build_hint))
        .await
        .map_err(|e| e.to_string())??;
    Ok(Prepared { sink, stream: Some(stream), hint, duration, quality: url.quality })
}

/// 创建解码器并接入混音器（保持暂停，由调用方决定何时播放）。
fn build_sink(
    mixer: &rodio::mixer::Mixer,
    reader: StreamReader,
    len: Option<u64>,
    hint: &str,
) -> Result<(rodio::Player, Option<Duration>), String> {
    let mut builder = Decoder::builder().with_data(reader).with_seekable(true).with_hint(hint);
    if let Some(len) = len {
        builder = builder.with_byte_len(len);
    }
    let decoder = builder.build().map_err(|e| format!("解码失败: {e}"))?;
    let duration = decoder.total_duration();
    let sink = rodio::Player::connect_new(mixer);
    sink.pause();
    sink.append(decoder);
    Ok((sink, duration))
}
