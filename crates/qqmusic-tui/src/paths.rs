//! 数据目录与 IPC 端点。

use std::io;
use std::path::{Path, PathBuf};

const APP: &str = "qqmusic-tui";

/// 应用数据目录（凭证、设备信息、配置、日志、IPC socket）。
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// 默认位置：环境变量 `QQMUSIC_TUI_HOME`，否则系统数据目录下的 `qqmusic-tui`。
    pub fn new() -> io::Result<Self> {
        let root = match std::env::var_os("QQMUSIC_TUI_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => dirs::data_dir()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "无法确定系统数据目录"))?
                .join(APP),
        };
        Self::with_root(root)
    }

    /// 指定根目录（不存在则创建）。
    pub fn with_root(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// 根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 登录凭证。
    pub fn credential(&self) -> PathBuf {
        self.root.join("credential.json")
    }

    /// 模拟设备信息。
    pub fn device(&self) -> PathBuf {
        self.root.join("device.json")
    }

    /// 播放器配置（音质、音量、播放模式）。
    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// 守护进程日志。
    pub fn daemon_log(&self) -> PathBuf {
        self.root.join("daemon.log")
    }

    /// 登录二维码图片保存目录。
    pub fn qrcode_dir(&self) -> PathBuf {
        self.root.join("qrcode")
    }

    /// IPC 端点：Unix 为数据目录下的 socket 文件，Windows 为按用户区分的命名管道。
    pub fn socket(&self) -> String {
        if cfg!(windows) {
            let user = std::env::var("USERNAME").unwrap_or_default();
            format!("{APP}-{user}.sock")
        } else {
            self.root.join("player.sock").to_string_lossy().into_owned()
        }
    }
}
