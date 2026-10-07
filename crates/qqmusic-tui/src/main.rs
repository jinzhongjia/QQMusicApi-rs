//! `qqm` 命令行入口。

use std::io;
use std::process::ExitCode;

use qqmusic_tui::daemon;
use qqmusic_tui::ipc::IpcClient;
use qqmusic_tui::paths::Paths;
use qqmusic_tui::protocol::{Command, Event, quality_label};
use qqmusic_tui::tui::{self, Launcher};

const USAGE: &str = "\
用法:
  qqm              打开终端界面（自动启动后台播放进程）
  qqm daemon       前台运行播放进程
  qqm ctl <指令>   控制正在运行的播放进程:
                   toggle | play | pause | next | prev | stop | status | quit

环境变量:
  QQMUSIC_TUI_HOME 数据目录（默认为系统数据目录下的 qqmusic-tui）";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = Paths::new().and_then(|paths| match args.first().map(String::as_str) {
        None | Some("tui") => runtime()?.block_on(tui::run(paths, Launcher::current_exe()?)),
        Some("daemon") => daemon::run(paths),
        Some("ctl") => runtime()?.block_on(ctl(paths, args.get(1).map(String::as_str).unwrap_or("status"))),
        Some("-h" | "--help" | "help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(io::Error::new(io::ErrorKind::InvalidInput, format!("未知命令 {other}\n\n{USAGE}"))),
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build()
}

async fn ctl(paths: Paths, action: &str) -> io::Result<()> {
    let command = match action {
        "toggle" => Some(Command::Toggle),
        "play" => Some(Command::Resume),
        "pause" => Some(Command::Pause),
        "next" => Some(Command::Next),
        "prev" => Some(Command::Prev),
        "stop" => Some(Command::Stop),
        "quit" => Some(Command::Shutdown),
        "status" => None,
        other => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("未知指令 {other}\n\n{USAGE}"))),
    };
    let mut client =
        IpcClient::connect(&paths).await.map_err(|e| io::Error::new(e.kind(), format!("播放进程未运行: {e}")))?;
    if let Some(command) = command {
        return client.send(&command).await;
    }
    while let Some(event) = client.recv().await? {
        if let Event::State(state) = event {
            let track = state.track.map_or_else(|| "-".to_string(), |t| format!("{} - {}", t.name, t.singers));
            let quality = state.playing_quality.unwrap_or(state.quality);
            println!(
                "{:?} {track} [{}/{}s] [{}] [{}] [音量 {}]",
                state.status,
                state.position_ms / 1000,
                state.duration_ms / 1000,
                quality_label(quality),
                state.mode.label(),
                state.volume
            );
            break;
        }
    }
    Ok(())
}
