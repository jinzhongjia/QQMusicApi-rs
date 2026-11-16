//! 终端界面。
//!
//! TUI 只负责浏览与发送指令，播放由守护进程完成；连接不上时会自动以分离进程
//! 启动守护进程，断线后自动重连。

mod app;
mod data;
mod input;
pub mod lyric;
mod qr;
#[cfg(test)]
mod tests;
mod ui;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event as TermEvent, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use self::app::{App, Msg, allowed_offline};
use crate::paths::Paths;
use crate::protocol::{Command, ErrorKind, PROTOCOL_VERSION, Status};

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
    // 粘贴作为一个整体事件送达，不会被逐字符当成快捷键。
    let _ = crossterm::execute!(io::stdout(), EnableBracketedPaste);
    let mut app = App::new(paths, client, tx, cmd_tx.clone());
    app.check_session();
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(250));

    let result = loop {
        if let Err(e) = terminal.draw(|f| ui::draw(f, &app)) {
            break Err(e);
        }
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(TermEvent::Key(key))) if key.kind != KeyEventKind::Release => app.handle_key(key),
                Some(Ok(TermEvent::Paste(text))) => app.handle_paste(&text),
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
    let _ = crossterm::execute!(io::stdout(), DisableBracketedPaste);
    ratatui::restore();

    // 没有在播放时一并关闭守护进程，避免残留空闲进程。
    let stop = app.quit == Some(true) || (app.connected && matches!(app.state.status, Status::Stopped));
    if stop {
        let _ = cmd_tx.send(Command::Shutdown);
    }
    drop(app);
    drop(cmd_tx);
    let _ = tokio::time::timeout(Duration::from_secs(1), ipc).await;
    result
}

/// 重连后处理断线期间积压的指令：设置类只保留每种最新的一条，其余丢弃。
///
/// 界面在断线时已经拒绝播放类操作，这里兜住「连接刚断、界面还没发现」那一小段时间里发出的指令，
/// 保证重连后不会突然执行一串切歌 / 删除。返回要补发的指令与被丢弃的数量。
fn coalesce(backlog: Vec<Command>) -> (Vec<Command>, usize) {
    let mut keep: Vec<Command> = Vec::new();
    let mut dropped = 0;
    for command in backlog {
        if !allowed_offline(&command) {
            dropped += 1;
            continue;
        }
        // 查询 / 握手重连后会重新发，不必补。
        if matches!(command, Command::GetQueue | Command::Hello { .. }) {
            continue;
        }
        keep.retain(|kept| std::mem::discriminant(kept) != std::mem::discriminant(&command));
        keep.push(command);
    }
    (keep, dropped)
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
                let message = format!("无法连接后台播放进程: {e}（日志: {}）", paths.daemon_log().display());
                let _ = tx.send(Msg::Error(ErrorKind::Local, message));
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        let (mut reader, mut writer) = client.into_split();
        let mut backlog = Vec::new();
        while let Ok(command) = commands.try_recv() {
            backlog.push(command);
        }
        let quitting = backlog.contains(&Command::Shutdown);
        let (resend, dropped) = coalesce(backlog);
        if dropped > 0 && !quitting {
            let _ = tx.send(Msg::Dropped(dropped));
        }
        let mut alive = writer.send(&Command::Hello { protocol: PROTOCOL_VERSION }).await.is_ok();
        for command in &resend {
            alive = alive && writer.send(command).await.is_ok();
        }
        while alive {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(command) => alive = writer.send(&command).await.is_ok(),
                    None => return,
                },
                event = reader.recv() => match event {
                    Ok(Some(event)) => {
                        let _ = tx.send(Msg::Player(event));
                    }
                    _ => alive = false,
                },
            }
        }
        let _ = tx.send(Msg::Disconnected);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod ipc_tests {
    use super::*;
    use crate::protocol::PlayMode;

    #[test]
    fn backlog_keeps_only_the_latest_settings_and_drops_playback_commands() {
        let backlog = vec![
            Command::Next,
            Command::SetVolume { volume: 10 },
            Command::Remove { index: 0, entry: None, tag: None },
            Command::SetVolume { volume: 30 },
            Command::SetMode { mode: PlayMode::Shuffle },
            Command::Next,
            Command::GetQueue,
            Command::Toggle,
            Command::ReloadCredential { tag: Some(1) },
            Command::ReloadCredential { tag: Some(2) },
        ];
        let (resend, dropped) = coalesce(backlog);
        assert_eq!(dropped, 4, "切歌、删除、播放 / 暂停都不补发");
        assert_eq!(
            resend,
            [
                Command::SetVolume { volume: 30 },
                Command::SetMode { mode: PlayMode::Shuffle },
                Command::ReloadCredential { tag: Some(2) },
            ]
        );
    }
}
