# QQMusicApi-rs

QQ 音乐 API 的 Rust 异步客户端库，是 Python 项目 [QQMusicApi](https://github.com/L-1124/QQMusicApi) 的 Rust 实现。它还集成了 [decky-music](https://github.com/jinzhongjia/decky-music) 的播放链接地区限制 / 风控绕过逻辑，让付费用户在受限地区也能拿到应得的播放链接和音质。

- 覆盖上游全部接口：搜索、歌曲、歌词、专辑、歌手、歌单、排行榜、MV、评论、推荐、用户、听歌等级、私信、文件上传，以及 QQ / 微信 / App 扫码和手机号登录
- 请求协议与上游一致：zzc 签名、QIMEI 设备指纹、Android 会话、QRC 歌词解密等
- 风控相关参数均可自定义：`ct`、`cv`、`comm` 中的任意字段、User-Agent、设备身份、代理等
- 默认构建为纯 Rust，不需要 C 编译器

使用示例见 [`crates/qqmusic-api/examples`](crates/qqmusic-api/examples)，API 文档可用 `cargo doc --open` 查看。

## 许可证

[GPL-3.0](LICENSE)，与上游 QQMusicApi 保持一致。

本项目仅供学习交流使用，请勿用于商业或任何侵犯版权的用途。
