//! 基于系统本地 socket（Unix domain socket / Windows 命名管道）的 IPC。
//!
//! 协议为按行分隔的 JSON：客户端发送 [`Command`]，守护进程推送 [`Event`]。

use std::io;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use interprocess::local_socket::tokio::prelude::*;
use interprocess::local_socket::tokio::{Listener, RecvHalf, SendHalf, Stream};
use interprocess::local_socket::{GenericFilePath, GenericNamespaced, ListenerOptions, Name};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};

use crate::paths::Paths;
use crate::protocol::{Command, Event};

fn socket_name(paths: &Paths) -> io::Result<Name<'static>> {
    let name = paths.socket();
    if cfg!(windows) { name.to_ns_name::<GenericNamespaced>() } else { name.to_fs_name::<GenericFilePath>() }
}

/// 创建监听端（守护进程使用）；会覆盖残留的 socket 文件。
pub fn listen(paths: &Paths) -> io::Result<Listener> {
    ListenerOptions::new().name(socket_name(paths)?).try_overwrite(true).create_tokio()
}

/// 按行读取 JSON 消息。
#[derive(Debug)]
pub struct MessageReader {
    lines: Lines<BufReader<RecvHalf>>,
}

impl MessageReader {
    /// 读取下一条消息；连接关闭时返回 `None`，无法解析的行会被跳过。
    pub async fn recv<T: DeserializeOwned>(&mut self) -> io::Result<Option<T>> {
        while let Some(line) = self.lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str(&line) {
                Ok(message) => return Ok(Some(message)),
                Err(e) => eprintln!("忽略无法解析的 IPC 消息: {e}: {line}"),
            }
        }
        Ok(None)
    }
}

/// 按行写入 JSON 消息。
#[derive(Debug)]
pub struct MessageWriter {
    half: SendHalf,
}

impl MessageWriter {
    /// 发送一条消息。
    pub async fn send<T: Serialize>(&mut self, message: &T) -> io::Result<()> {
        let mut line = serde_json::to_vec(message).map_err(io::Error::other)?;
        line.push(b'\n');
        self.half.write_all(&line).await
    }
}

/// 拆分一个连接为读写两端。
pub fn split(stream: Stream) -> (MessageReader, MessageWriter) {
    let (recv, send) = stream.split();
    (MessageReader { lines: BufReader::new(recv).lines() }, MessageWriter { half: send })
}

/// 连接守护进程的客户端。
#[derive(Debug)]
pub struct IpcClient {
    reader: MessageReader,
    writer: MessageWriter,
}

impl IpcClient {
    /// 连接正在运行的守护进程。
    pub async fn connect(paths: &Paths) -> io::Result<Self> {
        let stream = Stream::connect(socket_name(paths)?).await?;
        let (reader, writer) = split(stream);
        Ok(Self { reader, writer })
    }

    /// 发送指令。
    pub async fn send(&mut self, command: &Command) -> io::Result<()> {
        self.writer.send(command).await
    }

    /// 接收事件；连接关闭时返回 `None`。
    pub async fn recv(&mut self) -> io::Result<Option<Event>> {
        self.reader.recv().await
    }

    /// 拆分为读写两端，便于在不同任务中使用。
    pub fn into_split(self) -> (MessageReader, MessageWriter) {
        (self.reader, self.writer)
    }
}

/// 以分离进程方式启动守护进程：`program args...`，标准错误写入数据目录下的日志。
///
/// 子进程通过 `QQM_HOME` 使用同一数据目录，并脱离当前终端的进程组（Unix）/
/// 控制台（Windows），TUI 退出后继续运行。
pub fn spawn_detached(paths: &Paths, program: &Path, args: &[&str]) -> io::Result<()> {
    let log = std::fs::OpenOptions::new().create(true).append(true).open(paths.daemon_log())?;
    let mut command = std::process::Command::new(program);
    command.args(args).env("QQM_HOME", paths.root()).stdin(Stdio::null()).stdout(Stdio::null()).stderr(log);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn().map(drop)
}

/// 连接守护进程，未运行时用 `program args...` 启动并等待其就绪。
pub async fn connect_or_spawn(paths: &Paths, program: &Path, args: &[&str]) -> io::Result<IpcClient> {
    if let Ok(client) = IpcClient::connect(paths).await {
        return Ok(client);
    }
    spawn_detached(paths, program, args)?;
    let mut last = None;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        match IpcClient::connect(paths).await {
            Ok(client) => return Ok(client),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("守护进程启动超时")))
}
