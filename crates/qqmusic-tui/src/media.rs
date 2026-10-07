//! 系统媒体控制：macOS Now Playing、Windows SMTC、Linux MPRIS（souvlaki）。
//!
//! macOS 要求 `MPRemoteCommandCenter` / `MPNowPlayingInfoCenter` 在主线程上使用，
//! 且主线程需要运行 NSApplication 事件循环（见 [`crate::daemon::run`]），否则控制中心里
//! 上一首 / 下一首等按钮会显示为不可用；因此所有 souvlaki 调用都派发到主线程执行。

use std::time::Duration;

use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection,
};

use crate::player::PlayerHandle;
use crate::protocol::{Command, Status};

const SEEK_STEP: Duration = Duration::from_secs(10);

/// 在后台线程中接入系统媒体控制，失败时只打印日志。
pub fn spawn(player: PlayerHandle) {
    let result = std::thread::Builder::new().name("media-controls".into()).spawn(move || {
        if let Err(e) = run(player) {
            eprintln!("系统媒体控制不可用: {e}");
        }
    });
    if let Err(e) = result {
        eprintln!("无法启动媒体控制线程: {e}");
    }
}

/// 在主线程同步执行。
#[cfg(target_os = "macos")]
fn on_main<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    dispatch::Queue::main().exec_sync(f)
}

/// 其他平台无主线程要求，直接执行。
#[cfg(not(target_os = "macos"))]
fn on_main<T>(f: impl FnOnce() -> T) -> T {
    f()
}

fn run(player: PlayerHandle) -> Result<(), String> {
    #[cfg(windows)]
    let hwnd = windows::hidden_window().ok_or("无法创建 SMTC 所需的隐藏窗口")?;

    let handler = player.clone();
    let mut controls = on_main(move || {
        #[cfg(windows)]
        let hwnd = Some(hwnd);
        #[cfg(not(windows))]
        let hwnd = None;
        let config = PlatformConfig { display_name: "QQ 音乐", dbus_name: "qqmusic_tui", hwnd };
        let mut controls = MediaControls::new(config).map_err(|e| format!("{e:?}"))?;
        controls
            .attach(move |event| {
                if let Some(command) = to_command(event) {
                    handler.send(command);
                }
            })
            .map_err(|e| format!("{e:?}"))?;
        Ok::<_, String>(controls)
    })?;

    let mut state_rx = player.watch_state();
    let mut shown: Option<(Option<usize>, String)> = None;
    let mut last: Option<(Status, u64, std::time::Instant)> = None;
    loop {
        #[cfg(windows)]
        windows::pump_messages();
        std::thread::sleep(Duration::from_millis(200));
        match state_rx.has_changed() {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => return Ok(()),
        }
        let state = state_rx.borrow_and_update().clone();

        let key = (state.index, state.track.as_ref().map(|t| t.mid.clone()).unwrap_or_default());
        if shown.as_ref() != Some(&key) {
            shown = Some(key);
            let track = state.track.as_ref();
            let metadata = MediaMetadata {
                title: track.map(|t| t.name.as_str()),
                artist: track.map(|t| t.singers.as_str()),
                album: track.map(|t| t.album.as_str()),
                cover_url: track.map(|t| t.cover.as_str()).filter(|c| !c.is_empty()),
                duration: (state.duration_ms > 0).then(|| Duration::from_millis(state.duration_ms)),
            };
            on_main(|| {
                let _ = controls.set_metadata(metadata);
            });
            last = None;
        }

        // 状态变化或进度跳变（seek）时才更新，系统会自行推算播放进度。
        let now = std::time::Instant::now();
        let jumped = last.is_none_or(|(status, position, at)| {
            let expected =
                if status == Status::Playing { position + now.duration_since(at).as_millis() as u64 } else { position };
            status != state.status || expected.abs_diff(state.position_ms) > 1500
        });
        if jumped {
            last = Some((state.status, state.position_ms, now));
            let progress = Some(MediaPosition(Duration::from_millis(state.position_ms)));
            let playback = match state.status {
                Status::Playing | Status::Loading => MediaPlayback::Playing { progress },
                Status::Paused => MediaPlayback::Paused { progress },
                Status::Stopped => MediaPlayback::Stopped,
            };
            on_main(|| {
                let _ = controls.set_playback(playback);
                #[cfg(target_os = "linux")]
                let _ = controls.set_volume(f64::from(state.volume) / 100.0);
            });
        }
    }
}

fn to_command(event: MediaControlEvent) -> Option<Command> {
    let seek = |direction: SeekDirection, step: Duration| {
        let ms = step.as_millis() as i64;
        Command::SeekBy { offset_ms: if matches!(direction, SeekDirection::Forward) { ms } else { -ms } }
    };
    Some(match event {
        MediaControlEvent::Play => Command::Resume,
        MediaControlEvent::Pause => Command::Pause,
        MediaControlEvent::Toggle => Command::Toggle,
        MediaControlEvent::Next => Command::Next,
        MediaControlEvent::Previous => Command::Prev,
        MediaControlEvent::Stop => Command::Stop,
        MediaControlEvent::Seek(direction) => seek(direction, SEEK_STEP),
        MediaControlEvent::SeekBy(direction, step) => seek(direction, step),
        MediaControlEvent::SetPosition(MediaPosition(position)) => {
            Command::Seek { position_ms: position.as_millis() as u64 }
        }
        MediaControlEvent::SetVolume(volume) => Command::SetVolume { volume: (volume * 100.0).clamp(0.0, 100.0) as u8 },
        MediaControlEvent::Quit => Command::Shutdown,
        MediaControlEvent::OpenUri(_) | MediaControlEvent::Raise => return None,
    })
}

/// SMTC 需要一个窗口句柄；守护进程没有窗口，创建一个永不显示的隐藏窗口。
#[cfg(windows)]
#[allow(unsafe_code)]
mod windows {
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, RegisterClassW,
        TranslateMessage, WNDCLASSW, WS_OVERLAPPEDWINDOW,
    };

    pub fn hidden_window() -> Option<*mut c_void> {
        let class: Vec<u16> = "qqmusic_tui_smtc\0".encode_utf16().collect();
        // SAFETY: 参数均为有效的以 NUL 结尾的 UTF-16 字符串和空指针；窗口从不显示。
        unsafe {
            let instance = GetModuleHandleW(null());
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(DefWindowProcW);
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            (!hwnd.is_null()).then_some(hwnd)
        }
    }

    /// 处理隐藏窗口的消息，避免消息队列堆积。
    pub fn pump_messages() {
        // SAFETY: MSG 为纯数据结构，PeekMessageW 只写入它。
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
