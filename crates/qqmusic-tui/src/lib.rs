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
#[cfg(any(feature = "player", feature = "tui"))]
mod recommendation;
#[cfg(all(test, any(feature = "player", feature = "tui")))]
mod testing;
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

/// 读取已保存的登录凭证；读不到或无效时返回 `None`。
pub fn load_credential(paths: &Paths) -> Option<Credential> {
    read_credential(paths).ok().flatten()
}

/// 读取已保存的登录凭证，区分「没有凭证」（`Ok(None)`）与「读取失败」（`Err`）。
pub fn read_credential(paths: &Paths) -> std::io::Result<Option<Credential>> {
    let text = match std::fs::read_to_string(paths.credential()) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let credential = Credential::from_json_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("凭证文件已损坏: {e}")))?;
    Ok(Some(credential).filter(Credential::is_valid))
}

/// 保存登录凭证（Unix 下权限 0600）。先写临时文件再改名，失败时不破坏原有凭证。
pub fn save_credential(paths: &Paths, credential: &Credential) -> std::io::Result<()> {
    let path = paths.credential();
    let tmp = path.with_extension("json.tmp");
    let write = || {
        std::fs::write(&tmp, credential.to_json_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, &path)
    };
    write().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// 删除已保存的登录凭证。
pub fn remove_credential(paths: &Paths) -> std::io::Result<()> {
    match std::fs::remove_file(paths.credential()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// 去掉文本中 URL 的查询串（音频链接的查询串里带有 vkey 等临时凭据），用于日志与错误提示。
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("http://").or_else(|| rest.find("https://")) {
        let (before, url) = rest.split_at(start);
        out.push_str(before);
        let end =
            url.find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | '>' | '）')).unwrap_or(url.len());
        match url[..end].split_once('?') {
            Some((base, _)) => {
                out.push_str(base);
                out.push_str("?…");
            }
            None => out.push_str(&url[..end]),
        }
        rest = &url[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_strips_url_queries_only() {
        let text = "下载失败: error sending request for url (https://cdn.example/a/b.flac?vkey=SECRET&uin=1) 超时";
        assert_eq!(redact(text), "下载失败: error sending request for url (https://cdn.example/a/b.flac?…) 超时");
        assert_eq!(redact("http://a/b?x=1 和 https://c/d"), "http://a/b?… 和 https://c/d");
        assert_eq!(redact("没有链接？没有"), "没有链接？没有");
    }

    #[test]
    fn credential_roundtrip_and_failures_are_distinguished() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::with_root(dir.path()).unwrap();
        assert!(read_credential(&paths).unwrap().is_none());
        save_credential(&paths, &Credential::new(7, "key")).unwrap();
        assert_eq!(read_credential(&paths).unwrap().unwrap().musicid, 7);
        assert!(!paths.credential().with_extension("json.tmp").exists());
        std::fs::write(paths.credential(), "{ not json").unwrap();
        assert!(read_credential(&paths).is_err());
        assert!(load_credential(&paths).is_none());
        remove_credential(&paths).unwrap();
        remove_credential(&paths).unwrap();
        assert!(read_credential(&paths).unwrap().is_none());
    }
}
