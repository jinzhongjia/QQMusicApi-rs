# qqmusic-tui

[go-musicfox](https://github.com/go-musicfox/go-musicfox) 风格的 QQ 音乐终端播放器，基于 [`qqmusic-api`](../qqmusic-api)、ratatui 与 crossterm。

- **界面与播放分离**：TUI 只负责浏览和发指令，播放由后台守护进程完成，两者通过系统本地 IPC 通信（Unix domain socket / Windows 命名管道）。按 `q` 退出界面后音乐继续播放，再次打开会自动连上。
- **系统媒体控制**：macOS Now Playing / 媒体键、Windows SMTC、Linux MPRIS（纯 Rust 的 zbus，无需 libdbus）。
- **音质选择**：无损 / 高品质 / 标准，取不到时自动降级；切换后当前歌曲从原位置按新音质重新加载。
- 猜你喜欢、雷达推荐、我喜欢的音乐、我的歌单、推荐歌单、排行榜、新歌、搜索（单曲 / 歌单 / 专辑）、歌词（含翻译）、喜欢 / 取消喜欢、QQ / 微信 / QQ 音乐 App 扫码登录、四种播放模式。

## 使用

```sh
cargo install --path crates/qqmusic-tui  # 安装后的命令名为 qqm
qqm              # 打开界面（自动在后台启动播放进程）
qqm ctl toggle   # 控制后台播放：toggle | play | pause | next | prev | stop | status | quit
qqm daemon       # 前台运行播放进程（调试用）
```

数据（凭证、设备信息、配置、日志）默认存放在系统数据目录下的 `qqm`（Linux 为 `~/.local/share/qqm`），可用环境变量 `QQM_HOME` 指定。其中 `config.json` 保存音质、音量、播放模式，以及切歌 / 暂停时的渐入渐出时长 `fade_ms`（默认 400，设为 0 关闭）。Linux 需要 ALSA 开发库（`libasound2-dev` / `alsa-lib-devel`）。

按 `?` 查看全部快捷键。按键沿用 go-musicfox：`hjkl` 移动（双列时 `h` `l` 左右切换、到边缘翻页），`n`/`Enter` 进入，`b`/`Esc` 返回，`Space` 播放/暂停，`[` `]` 切歌，`,` `.` 快退/快进，`-` `=` 音量，`m` 播放模式，`v` 音质，`/` 搜索，`f` 喜欢，`q` 退出界面（音乐继续），`Q` 退出并停止播放。

## 作为库使用

| feature | 内容 |
| --- | --- |
| （无） | 协议 `protocol`、IPC 客户端 `ipc`、配置与路径 |
| `player` | 播放引擎 `player` 与守护进程 `daemon` |
| `media-controls` | 系统媒体控制 `media`（隐含 `player`） |
| `tui` | 终端界面 `tui` |

默认开启 `tui` + `media-controls`。只想控制一个正在运行的播放进程：

```toml
qqmusic-tui = { version = "0.0.2", default-features = false }
```

```rust,no_run
use qqmusic_tui::ipc::IpcClient;
use qqmusic_tui::paths::Paths;
use qqmusic_tui::protocol::Command;

# async fn demo() -> std::io::Result<()> {
let mut client = IpcClient::connect(&Paths::new()?).await?;
client.send(&Command::Next).await?;
# Ok(()) }
```

也可以用 `player::spawn` 把播放引擎直接嵌入自己的程序。IPC 协议是按行分隔的 JSON（见 `protocol::Command` / `protocol::Event`），任何语言都能接入。
