//! 播放器持久化配置。

use qqmusic_api::modules::song::Quality;
use serde::{Deserialize, Serialize};

use crate::paths::Paths;
use crate::protocol::PlayMode;

/// 播放器配置，保存在 `config.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// 最高音质。
    pub quality: Quality,
    /// 音量 0..=100。
    pub volume: u8,
    /// 播放模式。
    pub mode: PlayMode,
    /// 切歌、暂停、恢复时的渐入渐出时长（毫秒），0 表示关闭。
    pub fade_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self { quality: Quality::High, volume: 80, mode: PlayMode::Loop, fade_ms: 400 }
    }
}

impl Config {
    /// 读取配置，不存在或损坏时返回默认值。
    pub fn load(paths: &Paths) -> Self {
        std::fs::read_to_string(paths.config())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// 写入配置。
    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(paths.config(), text)
    }
}
