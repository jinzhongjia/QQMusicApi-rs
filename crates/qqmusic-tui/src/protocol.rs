//! TUI ⇄ 播放守护进程的消息协议（每行一个 JSON）。

use qqmusic_api::models::base::{CoverSize, Song};
use qqmusic_api::modules::song::Quality;
use serde::{Deserialize, Serialize};

/// 协议版本，随 [`PlayerState::protocol`] 下发。
///
/// * 1（旧版守护进程不带该字段，读作 0）：基础指令与事件；
/// * 2：队列条目 id 与版本、[`Command::Move`]、[`Command::Hello`]、带 `tag` 的指令确认
///   （[`Event::Done`]）、[`Event::CredentialChanged`]、错误分类。
///
/// 新增字段都有默认值，旧客户端可以照常解析；新增的指令 / 事件类型旧版本无法解析，
/// 因此客户端应先检查守护进程的版本，守护进程只向发过 [`Command::Hello`] 的连接推送新事件。
pub const PROTOCOL_VERSION: u32 = 2;

/// 歌手的结构化标识。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SingerRef {
    /// 歌手 mid。
    pub mid: String,
    /// 歌手名。
    pub name: String,
}

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
    /// 专辑 mid（没有时为空）。
    #[serde(default)]
    pub album_mid: String,
    /// 歌手列表（带 mid，用于跳转到歌手页）。
    #[serde(default)]
    pub singer_list: Vec<SingerRef>,
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
            album_mid: song.album.mid.clone(),
            singer_list: song
                .singer
                .iter()
                .filter(|s| !s.mid.is_empty())
                .map(|s| SingerRef { mid: s.mid.clone(), name: s.name.clone() })
                .collect(),
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

/// 引擎当前在做什么（[`Status`] 表达的是用户意图：播放 / 暂停 / 停止）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    /// 正在解析播放链接、建立解码器或等待推荐歌曲。
    Loading,
    /// 下载跟不上播放，正在缓冲。
    Buffering,
    /// 正在输出音频。
    Output,
    /// 空闲：停止，或暂停且音源已就绪。
    #[default]
    #[serde(other)]
    Idle,
}

/// 错误分类，决定客户端给出的恢复提示。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// 登录失效，需要重新登录。
    Login,
    /// 权益 / 版权 / 地域限制，当前账号拿不到播放链接。
    Rights,
    /// 网络或服务端暂时失败，可以重试。
    Network,
    /// 本地问题：音频设备、解码、文件读写。
    Local,
    /// 其他。
    #[default]
    #[serde(other)]
    Other,
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
    /// 当前曲目的队列条目 id。
    #[serde(default)]
    pub entry: Option<u64>,
    /// 当前曲目自然播完后确定会播放的下标；随机挑选或无法预知时为 `None`。
    #[serde(default)]
    pub next: Option<usize>,
    /// 引擎活动。
    #[serde(default)]
    pub activity: Activity,
    /// 守护进程的协议版本（见 [`PROTOCOL_VERSION`]；旧版本为 0）。
    #[serde(default)]
    pub protocol: u32,
}

/// 推荐队列的来源与续播位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RecommendationSource {
    /// 猜你喜欢，每次请求新一批歌曲。
    Guess,
    /// 雷达推荐，下次要请求的页码；一轮结束后回到第一页。
    Radar { next_page: i64 },
}

/// 客户端发给守护进程的指令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// 替换队列并从 `index` 开始播放。
    Play {
        tracks: Vec<Track>,
        index: usize,
        /// 推荐续播来源；省略时按普通列表播放。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        recommendation: Option<RecommendationSource>,
    },
    /// 追加到队列末尾，不改变正在播放的曲目（没有当前曲目时开始播放）。
    Append {
        tracks: Vec<Track>,
        /// 确认标记，见 [`Event::Done`]。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
    /// 插入到当前曲目之后；多次调用时后插入的先播放。
    PlayNext {
        tracks: Vec<Track>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
    /// 播放队列中的第 `index` 首。
    ///
    /// 带 `entry` 时以条目 id 为准：条目被移动过就按 id 找到它，已不在队列中则拒绝执行，
    /// 避免队列被其他客户端改动后按旧下标误操作。
    PlayIndex {
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<u64>,
    },
    /// 从队列移除（`entry` 的含义同 [`Command::PlayIndex`]）。
    Remove {
        index: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
    /// 把条目 `entry` 移动到下标 `to`（协议版本 2）。
    Move {
        entry: u64,
        to: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
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
    ReloadCredential {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<u64>,
    },
    /// 声明客户端的协议版本（协议版本 2）；之后守护进程才会向该连接推送新事件类型。
    Hello { protocol: u32 },
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
    Queue {
        tracks: Vec<Track>,
        /// 对应 [`PlayerState::queue_version`]。
        #[serde(default)]
        version: u64,
        /// 与 `tracks` 一一对应的条目 id（同一首歌重复入队时 id 不同）。
        #[serde(default)]
        ids: Vec<u64>,
    },
    /// 错误提示。
    Error {
        message: String,
        #[serde(default)]
        kind: ErrorKind,
    },
    /// 带 `tag` 的指令的执行结果（协议版本 2，只发给发过 [`Command::Hello`] 的连接）。
    Done { tag: u64, ok: bool, message: String },
    /// 守护进程已重新读取登录凭证（协议版本 2）。
    CredentialChanged { logged_in: bool, musicid: i64 },
    /// 守护进程即将退出。
    Shutdown,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn play_without_recommendation_remains_wire_compatible() {
        let old = json!({"cmd": "play", "tracks": [], "index": 0});
        let command: Command = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(command, Command::Play { tracks: Vec::new(), index: 0, recommendation: None });
        assert_eq!(serde_json::to_value(command).unwrap(), old);
        let radio = Command::Play {
            tracks: Vec::new(),
            index: 0,
            recommendation: Some(RecommendationSource::Radar { next_page: 3 }),
        };
        assert_eq!(serde_json::from_value::<Command>(serde_json::to_value(&radio).unwrap()).unwrap(), radio);
    }

    #[test]
    fn new_fields_keep_old_messages_parseable() {
        // 旧客户端发来的指令（没有 tag / entry）。
        for (old, expected) in [
            (json!({"cmd": "append", "tracks": []}), Command::Append { tracks: Vec::new(), tag: None }),
            (json!({"cmd": "remove", "index": 2}), Command::Remove { index: 2, entry: None, tag: None }),
            (json!({"cmd": "play_index", "index": 1}), Command::PlayIndex { index: 1, entry: None }),
            (json!({"cmd": "reload_credential"}), Command::ReloadCredential { tag: None }),
        ] {
            let command: Command = serde_json::from_value(old.clone()).unwrap();
            assert_eq!(command, expected);
            assert_eq!(serde_json::to_value(command).unwrap(), old);
        }
        // 旧守护进程发来的事件与状态。
        let queue: Event = serde_json::from_value(json!({"event": "queue", "tracks": []})).unwrap();
        assert_eq!(queue, Event::Queue { tracks: Vec::new(), version: 0, ids: Vec::new() });
        let error: Event = serde_json::from_value(json!({"event": "error", "message": "x"})).unwrap();
        assert_eq!(error, Event::Error { message: "x".into(), kind: ErrorKind::Other });
        let state: PlayerState = serde_json::from_value(json!({
            "status": "playing", "index": 0, "track": null, "position_ms": 1, "duration_ms": 2, "volume": 3,
            "mode": "loop", "quality": serde_json::to_value(Quality::High).unwrap(), "playing_quality": null,
            "queue_len": 1, "queue_version": 4,
        }))
        .unwrap();
        assert_eq!((state.protocol, state.entry, state.next, state.activity), (0, None, None, Activity::Idle));
        // 未知的分类不会让整条消息解析失败。
        let error: Event = serde_json::from_value(json!({"event": "error", "message": "x", "kind": "future"})).unwrap();
        assert_eq!(error, Event::Error { message: "x".into(), kind: ErrorKind::Other });
        // 旧曲目没有专辑 / 歌手标识。
        let track: Track = serde_json::from_value(json!({
            "id": 1, "mid": "m", "media_mid": "", "song_type": 0, "name": "n", "singers": "s", "album": "a",
            "cover": "", "duration": 1, "vip": false,
        }))
        .unwrap();
        assert!(track.album_mid.is_empty() && track.singer_list.is_empty());
    }
}
