//! QR code login: `cargo run --example qrcode_login -- qq` (or `wx`).
//!
//! The QR image is written to `.qqmusic/` and the credential to
//! `.qqmusic/credential.json` (used by the `song_url` example).

use std::time::Duration;

use futures::StreamExt;
use qqmusic_api::Client;
use qqmusic_api::models::login::{QrCodeLoginEvent, QrLoginType};

#[tokio::main]
async fn main() -> qqmusic_api::Result<()> {
    let login_type = match std::env::args().nth(1).as_deref() {
        Some("wx") => QrLoginType::Wx,
        _ => QrLoginType::Qq,
    };
    std::fs::create_dir_all(".qqmusic")?;
    let client = Client::builder().device_path(".qqmusic/device.json").build()?;

    let mut session = client.login().qrcode_session(login_type).timeout(Duration::from_secs(180));
    let qrcode = session.get_qrcode().await?;
    if let Some(path) = qrcode.save(".qqmusic")? {
        println!("请使用 {} 扫描二维码: {}", login_type.as_str(), path.display());
    }

    let mut events = session.events().await?;
    while let Some(result) = events.next().await {
        let result = result?;
        println!("状态: {:?}", result.event);
        if result.event == QrCodeLoginEvent::Done
            && let Some(credential) = result.credential
        {
            std::fs::write(".qqmusic/credential.json", credential.to_json_string())?;
            println!("登录成功, musicid = {}", credential.musicid);
            break;
        }
    }
    Ok(())
}
