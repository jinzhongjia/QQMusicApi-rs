//! 后台播放守护进程。
//!
//! 守护进程持有播放引擎并通过本地 socket 服务任意数量的客户端；
//! TUI 退出后它继续播放，可通过系统媒体控制或再次打开 TUI 操作。

use std::io;
use std::time::Duration;

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::tokio::prelude::*;
use tokio::sync::broadcast::{self, error::RecvError};

use crate::ipc::{self, IpcClient, MessageWriter};
use crate::paths::Paths;
use crate::player::{self, PlayerHandle};
use crate::protocol::{Command, Event};

/// 阻塞运行守护进程，直到收到 [`Command::Shutdown`] 或终止信号。
///
/// macOS 上系统媒体控制需要主线程运行 NSApplication 事件循环，因此主线程运行
/// NSApplication（Accessory 策略：不显示 Dock 图标，但可以作为「正在播放」应用；Prohibited 会被系统忽略），异步运行时放在后台线程，结束时直接退出进程。
pub fn run(paths: Paths) -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    #[cfg(all(target_os = "macos", feature = "media-controls"))]
    if let Some(mtm) = objc2::MainThreadMarker::new() {
        std::thread::Builder::new().name("daemon".into()).spawn(move || {
            let code = match runtime.block_on(serve(paths)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("守护进程退出: {e}");
                    1
                }
            };
            std::process::exit(code);
        })?;
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Accessory);
        app.run();
        std::process::exit(0);
    }

    runtime.block_on(serve(paths))
}

/// 在当前运行时中提供服务（不处理 macOS 主线程 run loop）。
pub async fn serve(paths: Paths) -> io::Result<()> {
    if IpcClient::connect(&paths).await.is_ok() {
        eprintln!("已有播放守护进程在运行");
        return Ok(());
    }
    let listener = ipc::listen(&paths)?;
    let client = crate::build_client(&paths).map_err(io::Error::other)?;
    let player = player::spawn(client, paths.clone()).map_err(io::Error::other)?;
    #[cfg(feature = "media-controls")]
    crate::media::spawn(player.clone());
    eprintln!("播放守护进程已启动: {}", paths.socket());

    let signals = shutdown_signal(player.clone());
    tokio::pin!(signals);
    loop {
        tokio::select! {
            conn = listener.accept() => match conn {
                Ok(conn) => {
                    tokio::spawn(handle_connection(conn, player.clone()));
                }
                Err(e) => eprintln!("接受连接失败: {e}"),
            },
            () = player.closed() => break,
            () = &mut signals => {}
        }
    }
    // 给客户端一点时间收到 Shutdown 事件。
    tokio::time::sleep(Duration::from_millis(100)).await;
    eprintln!("播放守护进程已退出");
    Ok(())
}

/// 读写互不影响：客户端发完指令立即断开（如 `ctl`）时，写快照失败也不能丢掉已到达的指令。
async fn handle_connection(conn: Stream, player: PlayerHandle) {
    let (mut reader, writer) = ipc::split(conn);
    let events = player.subscribe();
    let push = tokio::spawn(push_events(writer, events, player.clone()));
    while let Ok(Some(command)) = reader.recv::<Command>().await {
        if !player.send(command) {
            break;
        }
    }
    push.abort();
}

async fn push_events(mut writer: MessageWriter, mut events: broadcast::Receiver<Event>, player: PlayerHandle) {
    let snapshot = |player: &PlayerHandle| {
        [Event::State(Box::new(player.state())), Event::Queue { tracks: player.queue().to_vec() }]
    };
    for event in &snapshot(&player) {
        if writer.send(event).await.is_err() {
            return;
        }
    }
    loop {
        match events.recv().await {
            Ok(event) => {
                if writer.send(&event).await.is_err() || event == Event::Shutdown {
                    return;
                }
            }
            // 客户端太慢错过了事件，补发完整快照。
            Err(RecvError::Lagged(_)) => {
                for event in &snapshot(&player) {
                    if writer.send(event).await.is_err() {
                        return;
                    }
                }
            }
            Err(RecvError::Closed) => return,
        }
    }
}

/// 收到 SIGINT / SIGTERM 时关闭播放器；忽略 SIGHUP，避免关闭终端时被杀。
async fn shutdown_signal(player: PlayerHandle) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
            return std::future::pending().await;
        };
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = term.recv() => break,
                _ = hup.recv() => continue,
            }
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    player.send(Command::Shutdown);
    std::future::pending().await
}
