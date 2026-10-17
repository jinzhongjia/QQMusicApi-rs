# Windows 媒体卡片的应用身份

SMTC 的歌曲信息与应用身份是两条独立路径。souvlaki 0.8.3 在 Windows 上不读取 `PlatformConfig.display_name`。只设置窗口标题、进程 AppUserModelID 或窗口重启属性，也没有让本项目的测试会话获得可解析的应用信息。

这里使用 Windows 的外部位置身份包（Sparse Package）：`qqm.exe` 仍是普通 Win32 程序，通过嵌入的 `qqm.manifest` 与 `AppxManifest.xml` 关联。注册后，命令行启动的进程也能获得包身份，无需通过开始菜单启动。媒体卡片的应用名称为「QQ 音乐 TUI」。

## 注册与卸载

需要 Windows 10 2004 或更高版本和 MSVC 构建。发布 zip 内附带此目录。先退出 TUI 并关闭后台播放：

```powershell
qqm ctl quit
# 在解压后的发布目录中执行；使用系统自带 Windows PowerShell 5.1。
powershell.exe -NoProfile -File .\windows\Register-Identity.ps1
qqm
```

默认执行开发清单注册，是否允许取决于系统的开发部署策略。脚本不会开启开发者模式、调整执行策略或导入证书。如果系统拒绝开发注册，应使用受信任签名的身份包：

```powershell
powershell.exe -NoProfile -File .\windows\Register-Identity.ps1 -PackagePath .\qqm-identity.msix
```

升级或移动程序目录后，关闭旧进程并重新执行注册。脚本只替换当前用户的 `Jinzhongjia.QQMusicTui` 身份；不会删除登录凭证或配置。卸载身份：

```powershell
qqm ctl quit
powershell.exe -NoProfile -File .\windows\Unregister-Identity.ps1
```

不执行注册时，exe 仍可便携运行，应用名称可能继续显示为「未知应用」。注册脚本会将两张图标复制到 exe 旁的 `qqm-assets` 目录；卸载身份后该目录保留，可随程序目录一起移除。

## 从源码构建

`build.rs` 只给 MSVC 目标的 `qqm` 二进制嵌入清单，不给本库的使用者指定身份，也不增加构建依赖。GNU 构建不自动嵌入该清单。

```powershell
cargo build -p qqmusic-tui --release --locked
powershell.exe -NoProfile -File .\crates\qqmusic-tui\windows\Register-Identity.ps1 -ExecutablePath .\target\release\qqm.exe
```

通过 `cargo install` 安装时，同样需要保留本目录并用 `-ExecutablePath` 指定安装后的 `qqm.exe`。请勿在应用运行期间移动注册清单或更新程序。

## 正式分发

当前 zip 附带的是开发注册清单，不是已签名安装包。正式提供身份包时，用 Windows SDK 的 MakeAppx 打包本目录，再用受信任证书签名：

```powershell
makeappx.exe pack /d .\crates\qqmusic-tui\windows /nv /p .\qqm-identity.msix
# 用发布者的受信任代码签名证书执行 signtool sign /fd SHA256 ...
```

证书 Subject 必须匹配两个清单中的 `CN=jinzhongjia`。如果更改发布者或包名称，必须同步更改两个清单并重新构建 exe。正式版应维护身份包版本；开发清单的版本独立于 Rust crate 版本。安装程序或 Scoop 集成应在安装/升级时调用注册脚本，在卸载时移除身份。

Windows 的 `-AllowUnsigned` 不适合绕过这个要求：本次实验中，带可执行激活的未签名身份包被拒绝，错误为 `0x80073D2B`。

## 验证方法

检查后台播放进程，而不只检查 TUI：

1. 使用 `GetCurrentApplicationUserModelId` 确认进程获得 `<PackageFamilyName>!QQMusicTui`。
2. 使用 `GlobalSystemMediaTransportControlsSessionManager` 找到对应会话，检查 `SourceAppUserModelId`。
3. 通过 `AppInfo.GetFromAppUserModelId(id).DisplayInfo` 读取应用名称及图标。图标路径必须相对于注册的 exe 目录；仅把 PNG 放在清单旁会导致图标查询失败。
4. 验证暂停/继续事件，确认退出 TUI 后仍能控制后台播放。
5. 卸载身份后再次启动，确认便携运行仍正常。

在 Windows 11 build 26100 / Windows PowerShell 5.1 上，使用本项目原有 `media.rs` 和模拟播放状态进行了隔离验证：通过与 daemon 相同的进程标志启动后台子进程，父进程退出后，会话仍可解析出「QQ 音乐 TUI」，图标读取返回 1658 字节，暂停和继续事件均进入原有回调。移动安装目录并重新注册后，身份仍可解析；卸载后进程仍可便携启动。测试身份和注册表项已清理。

实际 Windows 卡片的视觉显示仍需人工检查，开发注册成功不代表其他机器允许同样的部署方式。受信任签名包的分发流程尚未实测。

参考：[Microsoft 外部位置身份包文档](https://learn.microsoft.com/en-us/windows/apps/desktop/modernize/grant-identity-to-nonpackaged-apps)、[souvlaki #67](https://github.com/Sinono3/souvlaki/issues/67)。
