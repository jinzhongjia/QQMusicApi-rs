# QQMusicApi-rs

[QQMusicApi](https://github.com/L-1124/QQMusicApi) 的 Rust 异步实现，并集成了 [decky-music](https://github.com/jinzhongjia/decky-music) 的播放链接**地区限制 / 风控绕过**逻辑，让付费用户在受限地区也能获取应得的音质。

- 覆盖上游全部 15 个 API 模块（搜索、歌曲、歌词、专辑、歌手、歌单、排行榜、MV、评论、推荐、用户、听歌等级、私信、文件上传、登录）
- 与上游一致的请求协议：Android / 桌面 / Web 三套 `comm`、zzc 签名、QIMEI 设备指纹、Android 会话（`uid`/`sid`）、QRC 歌词解密、MQTT 扫码登录
- 所有风控相关参数均可自定义：`ct`、`cv`、`comm` 任意字段、User-Agent、设备身份、CDN、HTTP 传输层（TLS 指纹 / 代理）
- 惰性请求对象：可在 `await` 前逐请求覆盖平台、`comm`、参数、凭证；同平台请求自动合并为一次批量调用
- 统一分页：`Paged<T>` 支持取首页、按页流式迭代、按条目收集
- 完善的测试：200+ 单元测试、离线端到端集成测试、可选的真实接口在线测试

## 安装

```toml
[dependencies]
qqmusic-api = { git = "https://github.com/jinzhongjia/QQMusicApi-rs" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

最低支持 Rust 版本（MSRV）：**1.89**。

| Feature | 默认 | 说明 |
| --- | --- | --- |
| `reqwest-transport` | 是 | 基于 reqwest + rustls 的默认 HTTP 传输层 |
| `mobile-login` | 是 | QQ 音乐 App 扫码登录（MQTT over WebSocket） |
| `socks` | 否 | reqwest 传输层的 SOCKS 代理支持 |
| `tls-graviola` | 是 | rustls 加密后端 [graviola](https://github.com/ctz/graviola)：纯 Rust + 形式化验证汇编，**无需 C 编译器** |
| `tls-aws-lc` | 否 | rustls 加密后端 aws-lc-rs（需要 cmake 与 C 编译器） |
| `tls-ring` | 否 | rustls 加密后端 ring（需要 C 编译器） |
| `native-roots` | 否 | 在内置的 webpki-roots 之外额外信任系统证书库 |

关闭 `reqwest-transport` 时需通过 `ClientBuilder::transport` 提供自定义的 `Transport` 实现。

### 纯 Rust 构建

默认特性下整个依赖树不编译任何 C 代码（不依赖 aws-lc-sys / ring / openssl / cmake），
可以直接交叉编译到 `x86_64/aarch64-unknown-linux-musl` 等目标，CI 中会在 `CC=false` 下验证这一点。

- 根证书来自内置的 `webpki-roots`，不读取系统证书库（可用 `native-roots` 开启）。
- graviola 仅支持 x86_64（需 AES-NI、PCLMULQDQ、BMI1/2、ADX、AVX2，约 2014 年后的 CPU）与 aarch64；
  运行时会检测 CPU，不满足时不会 panic，而是回退到应用通过
  `rustls::crypto::CryptoProvider::install_default()` 安装的进程级 provider。
- 同时启用多个后端时优先级为 aws-lc-rs > ring > graviola；需要支持更老的 CPU 或其他架构时可启用 `tls-ring` / `tls-aws-lc`。

## 快速开始

```rust,no_run
use qqmusic_api::Client;
use qqmusic_api::modules::search::SearchType;

#[tokio::main]
async fn main() -> qqmusic_api::Result<()> {
    // 持久化模拟设备，让服务端始终看到同一台稳定的客户端
    let client = Client::builder().device_path("device.json").build()?;

    let page = client.search().search_by_type("周杰伦", SearchType::Song).await?;
    for item in &page.song {
        println!("{} {}", item.song.mid, item.song.name);
    }

    // 分页：最多收集 60 条
    let songs = client
        .search()
        .search_by_type("周杰伦", SearchType::Song)
        .collect_items(Some(60))
        .await?;
    println!("{}", songs.len());
    Ok(())
}
```

更多示例见 [`crates/qqmusic-api/examples`](crates/qqmusic-api/examples)：

| 示例 | 命令 |
| --- | --- |
| 搜索 | `cargo run --example search -- 周杰伦` |
| 扫码登录（保存凭证） | `cargo run --example qrcode_login -- qq` |
| 绕过风控获取播放链接 | `cargo run --example song_url -- 0039MnYb0qxYhV` |

## 地区限制 / 风控绕过

QQ 音乐主要依据请求 `comm` 中的 `ct`（客户端类型）决定 `music.vkey.GetVkey` 返回哪些音质，甚至是否返回链接。库默认的 Android 身份（`ct=11`）在部分地区会被版权规则降级，甚至即使是付费用户也拿不到任何链接。参照 decky-music 的实践，播放链接请求会改用一套专门的 `comm`：

| 字段 | 默认值 | 说明 |
| --- | --- | --- |
| `ct` | 由设备 GUID 派生 | 从已验证可拿到全部音质的取值集合 `HIGH_QUALITY_CT` 中按 `sha256(guid)[0] % len` 选出：同一设备始终稳定，不同安装自然分散。固定值与每次随机都是风控信号 |
| `cv` | `0` | |
| `qq` / `authst` | 当前凭证 | 获取会员音质所必需 |
| 请求参数 `guid` | 设备 `open_udid` | 与库其他请求保持一致，避免同一客户端报出两个 GUID |

`song().playable_url()` 会在**一次请求**中按音质阶梯（FLAC → OGG 640 → MP3 320 → OGG 320 → MP3 128）请求全部档位，返回可用的最高音质；无版权或需要会员时返回 `None`。

```rust,no_run
use qqmusic_api::modules::song::Quality;
use qqmusic_api::{BypassConfig, Client, Credential};

async fn demo() -> qqmusic_api::Result<()> {
let client = Client::builder()
    .credential(Credential::new(123456, "Q_H_L_..."))
    .bypass(
        BypassConfig::default()
            .with_fixed_ct(24)                    // 固定 ct（默认按 GUID 派生）
            .with_comm("tmeLoginType", "2")       // 追加 / 覆盖任意 comm 字段，空值表示删除
            .with_cdn("https://ws.stream.qqmusic.qq.com/"),
    )
    .build()?;

if let Some(url) = client.song().playable_url("0039MnYb0qxYhV", None, Quality::Lossless).await? {
    println!("{:?} {}", url.quality, url.url);
}

// 低层接口同样默认走 bypass，可逐请求关闭
let urls = client
    .song()
    .get_song_urls(["0039MnYb0qxYhV"], qqmusic_api::modules::song::SongFileType::MP3_320, None)?
    .bypass(false)
    .await?;
    let _ = urls;
    Ok(())
}
```

`BypassConfig` 的全部字段（均可序列化，便于放入配置文件）：

| 字段 | 说明 |
| --- | --- |
| `enabled` | 是否启用（`BypassConfig::disabled()` 关闭） |
| `ct` | `CtStrategy::DerivedFromGuid(候选列表)` 或 `CtStrategy::Fixed(值)` |
| `cv` | `cv` 取值 |
| `include_credential` | 是否附带 `qq` / `authst` |
| `extra_comm` | 额外 `comm` 字段（空值删除该字段） |
| `cdn` | 接口只返回 `purl` 相对路径时使用的 CDN 前缀 |
| `force_https` | 将 `http://` 链接升级为 `https://` |
| `user_agent` | bypass 请求使用的 User-Agent（默认使用 Web 端浏览器 UA） |

运行时可通过 `client.set_bypass(...)` 热更新。

## 自定义请求参数

除 bypass 外，所有影响风控判定的参数都可以覆盖：

```rust,no_run
use qqmusic_api::{Client, Device, DeviceProfile, Platform, VersionProfile};
use serde_json::{Value, json};

async fn demo() -> qqmusic_api::Result<()> {
let client = Client::builder()
    // 设备身份：文件持久化 / 固定设备 / 指定机型
    .device(Device::generate(Some(DeviceProfile::Vivo), None))
    // 各平台的 ct / cv / UA / comm 覆盖
    .configure_version_policy(|policy| {
        policy.android = VersionProfile::android().with_cv(20_090_008).with_comm("chid", "10003505");
        policy.web = VersionProfile::web().with_user_agent("Mozilla/5.0 ...");
    })
    .platform(Platform::Android)        // 默认平台
    .header("X-Forwarded-For", "1.2.3.4")
    .proxy("http://127.0.0.1:7890")
    .build()?;

// 单个请求级别的覆盖
let detail: Value = client
    .cgi("music.pf_song_detail_svr", "get_song_detail_yqq", json!({"song_mid": "0039MnYb0qxYhV"}))
    .platform(Platform::Desktop)
    .comm("ct", "19")
    .param("song_type", 0)
    .await?;
    let _ = detail;
    Ok(())
}
```

如需控制 TLS 指纹（例如模拟浏览器 JA3/JA4）、自定义路由或连接池，实现 `qqmusic_api::transport::Transport` trait 并通过 `ClientBuilder::transport` 注入即可。

## 登录

```rust,no_run
use qqmusic_api::Client;
use qqmusic_api::models::login::QrLoginType;

async fn demo() -> qqmusic_api::Result<()> {
let client = Client::new()?;
let mut session = client.login().qrcode_session(QrLoginType::Qq);
let qrcode = session.get_qrcode().await?;
qrcode.save(".")?;                         // 保存二维码图片
let credential = session.wait().await?;    // 等待扫码确认
client.set_credential(credential.clone()); // 后续请求使用该凭证
std::fs::write("credential.json", credential.to_json_string())?;
    Ok(())
}
```

支持 QQ / 微信 / QQ 音乐 App（MQTT 推送）扫码、手机验证码登录、凭证过期检测与刷新（`check_expired` / `refresh_credential`）。

## 模块一览

| 模块 | 访问方式 | 内容 |
| --- | --- | --- |
| search | `client.search()` | 热搜、联想、综合搜索、分类搜索 |
| song | `client.song()` | 歌曲详情、播放链接（含 bypass 音质阶梯）、相似歌曲、曲谱等 |
| lyric | `client.lyric()` | 歌词（QRC 自动解密）、翻译、罗马音、AI 词典 |
| album | `client.album()` | 专辑详情、曲目、新碟、收藏 |
| singer | `client.singer()` | 歌手列表、主页、歌曲 / 专辑 / MV |
| songlist | `client.songlist()` | 歌单详情、创建、编辑、增删歌曲 |
| top | `client.top()` | 排行榜分类与详情 |
| mv | `client.mv()` | MV 详情、播放链接、列表 |
| comment | `client.comment()` | 评论读取、发表、时刻评论 |
| recommend | `client.recommend()` | 首页推荐、每日推荐、猜你喜欢 |
| user | `client.user()` | 用户主页、收藏、关注、粉丝 |
| sound_power | `client.sound_power()` | 听歌等级 |
| private_message | `client.private_message()` | 私信会话、消息、设置 |
| helper | `client.helper()` | 文件上传（InitUpload / 腾讯云 COS 直传 / FinishUpload） |
| login | `client.login()` | 扫码 / 手机号登录、凭证刷新、登出 |

## 测试

```sh
cargo test --workspace                                     # 单元测试 + 离线集成测试 + doctest
cargo test -p qqmusic-api --test live -- --ignored         # 真实接口在线测试（需网络）
QQMUSIC_CREDENTIAL=credential.json cargo test -p qqmusic-api --test live -- --ignored
```

- 算法（zzc 签名、QQ 音乐自定义 TripleDES、QRC、hash33）均使用上游 Python 实现生成的测试向量
- COS 上传签名使用腾讯云官方 `cos-python-sdk-v5` 生成的向量验证
- 未登录状态下的匿名搜索可能被服务端按 IP 限流（`2001 触发风控`），上游 Python 实现表现相同；在线测试会将其视为可容忍的结果

## 与上游的差异

- 方法名保持一致，Rust 风格调整：`checking_mobile_qrcode` → `mobile_qrcode_events`，可选参数较多的接口改为 `*Options` 结构体
- 文件上传使用单次 `PUT Object`（上限 5 GiB），而非上游借助 COS SDK 在 5 MiB 以上切换分块上传
- 新增 `BypassConfig` 及 `song().playable_url()`（来自 decky-music）

## 许可证

[GPL-3.0](LICENSE)，与上游 QQMusicApi 保持一致。

本项目仅供学习交流使用，请勿用于商业或任何侵犯版权的用途。
