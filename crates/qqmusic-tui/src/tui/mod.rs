//! 终端界面。
//!
//! TUI 只负责浏览与发送指令，播放由守护进程完成；连接不上时会自动以分离进程
//! 启动守护进程，断线后自动重连。

mod app;
mod data;
pub mod lyric;
mod qr;
mod ui;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{Event as TermEvent, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use self::app::{App, Msg};
use crate::paths::Paths;
use crate::protocol::{Command, Status};

/// 如何启动守护进程：`program args...`。
#[derive(Debug, Clone)]
pub struct Launcher {
    /// 可执行文件。
    pub program: PathBuf,
    /// 参数。
    pub args: Vec<String>,
}

impl Launcher {
    /// 当前可执行文件 + `daemon` 子命令。
    pub fn current_exe() -> io::Result<Self> {
        Ok(Self { program: std::env::current_exe()?, args: vec!["daemon".into()] })
    }
}

/// 运行 TUI，直到用户退出。
pub async fn run(paths: Paths, launcher: Launcher) -> io::Result<()> {
    let client = crate::build_client(&paths).map_err(io::Error::other)?;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let ipc = tokio::spawn(ipc_task(paths.clone(), launcher, cmd_rx, tx.clone()));

    let mut terminal = ratatui::init();
    let mut app = App::new(paths, client, tx, cmd_tx.clone());
    app.refresh_credential_if_needed();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));

    let result = loop {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &app)) {
            break Err(e);
        }
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(TermEvent::Key(key))) if key.kind != KeyEventKind::Release => app.handle_key(key),
                Some(Ok(_)) => {}
                Some(Err(e)) => break Err(e),
                None => break Ok(()),
            },
            Some(msg) = rx.recv() => app.handle_msg(msg),
            _ = tick.tick() => {}
        }
        if app.quit.is_some() {
            break Ok(());
        }
    };
    ratatui::restore();

    // 没有在播放时一并关闭守护进程，避免残留空闲进程。
    let stop = app.quit == Some(true) || matches!(app.state.status, Status::Stopped);
    if stop {
        let _ = cmd_tx.send(Command::Shutdown);
    }
    drop(app);
    drop(cmd_tx);
    let _ = tokio::time::timeout(Duration::from_secs(1), ipc).await;
    result
}

/// 维护与守护进程的连接：转发指令与事件，断线自动重连。
async fn ipc_task(
    paths: Paths,
    launcher: Launcher,
    mut commands: UnboundedReceiver<Command>,
    tx: UnboundedSender<Msg>,
) {
    let args: Vec<&str> = launcher.args.iter().map(String::as_str).collect();
    loop {
        let client = match crate::ipc::connect_or_spawn(&paths, &launcher.program, &args).await {
            Ok(client) => client,
            Err(e) => {
                let _ = tx.send(Msg::Toast(format!("无法连接播放进程: {e}（日志: {}）", paths.daemon_log().display())));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let (mut reader, mut writer) = client.into_split();
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => {
                        if writer.send(&command).await.is_err() {
                            break;
                        }
                    }
                    None => return,
                },
                event = reader.recv() => match event {
                    Ok(Some(event)) => {
                        let _ = tx.send(Msg::Player(event));
                    }
                    _ => break,
                },
            }
        }
        let _ = tx.send(Msg::Disconnected);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
