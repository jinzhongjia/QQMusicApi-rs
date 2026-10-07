//! go-musicfox 风格的 QQ 音乐终端播放器。
//!
//! 由两个进程组成，通过系统本地 IPC（Unix domain socket / Windows 命名管道）通信：
//!
//! * **播放守护进程**（[`daemon`]，feature `player`）：持有播放队列、解析播放链接、
//!   解码输出音频，并接入系统媒体控制（feature `media-controls`）。TUI 退出后继续播放。
//! * **TUI**（[`tui`]，feature `tui`）：浏览/搜索/登录，并把播放指令发给守护进程。
//!
//! 作为库使用时可以只依赖协议与客户端（`default-features = false`）：
//!
//! ```no_run
//! use qqmusic_tui::ipc::IpcClient;
//! use qqmusic_tui::paths::Paths;
//! use qqmusic_tui::protocol::{Command, Event};
//!
//! # async fn demo() -> std::io::Result<()> {
//! let paths = Paths::new()?;
//! let mut client = IpcClient::connect(&paths).await?;
//! client.send(&Command::Toggle).await?;
//! while let Some(event) = client.recv().await? {
//!     if let Event::State(state) = event {
//!         println!("{:?} {:?}", state.status, state.track.map(|t| t.name));
//!         break;
//!     }
//! }
//! # Ok(()) }
//! ```

pub mod config;
#[cfg(feature = "player")]
pub mod daemon;
pub mod ipc;
#[cfg(feature = "media-controls")]
pub mod media;
pub mod paths;
#[cfg(feature = "player")]
pub mod player;
pub mod protocol;
#[cfg(feature = "tui")]
pub mod tui;

use qqmusic_api::{Client, Credential};

use crate::paths::Paths;

/// 按统一的设备/凭证文件构建 API 客户端（TUI 与守护进程共用）。
pub fn build_client(paths: &Paths) -> qqmusic_api::Result<Client> {
    let mut builder = Client::builder().device_path(paths.device());
    if let Some(credential) = load_credential(paths) {
        builder = builder.credential(credential);
    }
    builder.build()
}

/// 读取已保存的登录凭证。
pub fn load_credential(paths: &Paths) -> Option<Credential> {
    let text = std::fs::read_to_string(paths.credential()).ok()?;
    Credential::from_json_str(&text).ok().filter(Credential::is_valid)
}

/// 保存登录凭证（Unix 下权限 0600）。
pub fn save_credential(paths: &Paths, credential: &Credential) -> std::io::Result<()> {
    let path = paths.credential();
    std::fs::write(&path, credential.to_json_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// 删除已保存的登录凭证。
pub fn remove_credential(paths: &Paths) -> std::io::Result<()> {
    match std::fs::remove_file(paths.credential()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}
