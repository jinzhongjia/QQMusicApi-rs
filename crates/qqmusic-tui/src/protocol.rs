//! TUI ⇄ 播放守护进程的消息协议（每行一个 JSON）。

use qqmusic_api::models::base::{CoverSize, Song};
use qqmusic_api::modules::song::Quality;
use serde::{Deserialize, Serialize};

/// 队列中的一首歌。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    /// 歌曲 id。
    pub id: i64,
    /// 歌曲 mid。
    pub mid: String,
    /// 媒体文件 mid（为空时使用 `mid`）。
    pub media_mid: String,
    /// 歌曲类型。
    pub song_type: i64,
    /// 歌名。
    pub name: String,
    /// 歌手（多个以 ` / ` 分隔）。
    pub singers: String,
    /// 专辑名。
    pub album: String,
    /// 封面 URL。
    pub cover: String,
    /// 时长（秒）。
    pub duration: u64,
    /// 是否需要会员。
    pub vip: bool,
}

impl From<&Song> for Track {
    fn from(song: &Song) -> Self {
        let name = if song.name.is_empty() { song.title.clone() } else { song.name.clone() };
        Self {
            id: song.id,
            mid: song.mid.clone(),
            media_mid: song.file.media_mid.clone(),
            song_type: song.r#type,
            name,
            singers: song.singer.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(" / "),
            album: song.album.name.clone(),
            cover: if song.album.mid.is_empty() { String::new() } else { song.album.cover_url(CoverSize::S500) },
            duration: u64::try_from(song.interval).unwrap_or(0),
            vip: song.requires_vip(),
        }
    }
}

/// 播放模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayMode {
    /// 顺序播放，到末尾停止。
    Order,
    /// 列表循环。
    #[default]
    Loop,
    /// 单曲循环。
    Single,
    /// 随机播放。
    Shuffle,
}

impl PlayMode {
    /// 中文名。
    pub fn label(self) -> &'static str {
        match self {
            Self::Order => "顺序播放",
            Self::Loop => "列表循环",
            Self::Single => "单曲循环",
            Self::Shuffle => "随机播放",
        }
    }

    /// 下一个模式。
    pub fn next(self) -> Self {
        match self {
            Self::Order => Self::Loop,
            Self::Loop => Self::Single,
            Self::Single => Self::Shuffle,
            Self::Shuffle => Self::Order,
        }
    }
}

/// 音质中文名。
pub fn quality_label(quality: Quality) -> &'static str {
    match quality {
        Quality::Lossless => "无损",
        Quality::High => "高品质",
        Quality::Standard => "标准",
    }
}

/// 播放状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// 停止。
    #[default]
    Stopped,
    /// 正在解析链接 / 缓冲。
    Loading,
    /// 播放中。
    Playing,
    /// 暂停。
    Paused,
}

/// 播放器状态快照。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PlayerState {
    /// 状态。
    pub status: Status,
    /// 当前曲目在队列中的下标。
    pub index: Option<usize>,
    /// 当前曲目。
    pub track: Option<Track>,
    /// 播放位置（毫秒）。
    pub position_ms: u64,
    /// 总时长（毫秒）。
    pub duration_ms: u64,
    /// 音量 0..=100。
    pub volume: u8,
    /// 播放模式。
    pub mode: PlayMode,
    /// 期望的最高音质。
    pub quality: Quality,
    /// 当前曲目实际获取到的音质。
    pub playing_quality: Option<Quality>,
    /// 队列长度。
    pub queue_len: usize,
    /// 队列版本号，变化时客户端应重新获取队列。
    pub queue_version: u64,
}

/// 客户端发给守护进程的指令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// 替换队列并从 `index` 开始播放。
    Play { tracks: Vec<Track>, index: usize },
    /// 追加到队列末尾（队列为空时开始播放）。
    Append { tracks: Vec<Track> },
    /// 插入到当前曲目之后。
    PlayNext { tracks: Vec<Track> },
    /// 播放队列中的第 `index` 首。
    PlayIndex { index: usize },
    /// 从队列移除。
    Remove { index: usize },
    /// 清空队列并停止。
    Clear,
    /// 播放 / 暂停。
    Toggle,
    /// 暂停。
    Pause,
    /// 继续。
    Resume,
    /// 停止。
    Stop,
    /// 下一首。
    Next,
    /// 上一首。
    Prev,
    /// 跳转到绝对位置。
    Seek { position_ms: u64 },
    /// 相对跳转。
    SeekBy { offset_ms: i64 },
    /// 设置音量 0..=100。
    SetVolume { volume: u8 },
    /// 设置播放模式。
    SetMode { mode: PlayMode },
    /// 设置最高音质（正在播放的曲目会按新音质重新加载）。
    SetQuality { quality: Quality },
    /// 重新读取登录凭证（登录 / 退出后发送）。
    ReloadCredential,
    /// 请求一次完整的队列（以 [`Event::Queue`] 回复）。
    GetQueue,
    /// 退出守护进程。
    Shutdown,
}

/// 守护进程推送给客户端的事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// 状态快照（状态变化时及播放中每 500ms 推送）。
    State(Box<PlayerState>),
    /// 完整队列。
    Queue { tracks: Vec<Track> },
    /// 错误提示。
    Error { message: String },
    /// 守护进程即将退出。
    Shutdown,
}
