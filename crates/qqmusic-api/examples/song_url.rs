//! Resolve a playable URL with the region / risk-control bypass.
//!
//! ```sh
//! # credential saved by the `qrcode_login` example
//! cargo run --example song_url -- 0039MnYb0qxYhV
//! # pin ct, use another CDN
//! QQMUSIC_CT=24 QQMUSIC_CDN=https://ws.stream.qqmusic.qq.com/ cargo run --example song_url -- 0039MnYb0qxYhV
//! ```

use qqmusic_api::modules::song::Quality;
use qqmusic_api::{BypassConfig, Client, Credential};

const CREDENTIAL_PATH: &str = ".qqmusic/credential.json";

#[tokio::main]
async fn main() -> qqmusic_api::Result<()> {
    let mid = std::env::args().nth(1).unwrap_or_else(|| "0039MnYb0qxYhV".to_string());

    let mut bypass = BypassConfig::default();
    if let Some(ct) = std::env::var("QQMUSIC_CT").ok().and_then(|v| v.parse().ok()) {
        bypass = bypass.with_fixed_ct(ct);
    }
    if let Ok(cdn) = std::env::var("QQMUSIC_CDN") {
        bypass = bypass.with_cdn(cdn);
    }

    let mut builder = Client::builder().device_path(".qqmusic/device.json").bypass(bypass);
    match std::fs::read_to_string(CREDENTIAL_PATH) {
        Ok(text) => {
            let credential = Credential::from_json_str(&text).map_err(|e| qqmusic_api::Error::Model(Box::new(e)))?;
            builder = builder.credential(credential);
        }
        Err(_) => eprintln!("未找到 {CREDENTIAL_PATH}, 以游客身份请求 (仅能获取试听/免费音质)"),
    }
    let client = builder.build()?;

    match client.song().playable_url(&mid, None, Quality::Lossless).await? {
        Some(url) => println!("[{}] {}\n{}", url.quality.as_str(), url.filename, url.url),
        None => println!("没有可用的播放链接 (无版权或需要会员)"),
    }
    Ok(())
}
