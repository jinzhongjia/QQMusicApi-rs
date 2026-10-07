//! QR code login: `cargo run --example qrcode_login -- qq` (or `wx`, or
//! `mobile` for the QQ Music app).
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
        Some("mobile") => QrLoginType::Mobile,
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
            write_private(".qqmusic/credential.json", &credential.to_json_string())?;
            println!("登录成功, musicid = {}", credential.musicid);
            break;
        }
    }
    Ok(())
}

/// Write a secret file readable only by the owner.
fn write_private(path: &str, contents: &str) -> std::io::Result<()> {
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
