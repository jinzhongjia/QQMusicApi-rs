//! Login models.

use std::path::{Path, PathBuf};

use crate::credential::Credential;

/// QR login channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QrLoginType {
    /// QQ (`ptlogin2`).
    Qq,
    /// WeChat (`open.weixin.qq.com`).
    Wx,
    /// QQ Music App (MQTT push).
    Mobile,
}

impl QrLoginType {
    /// Lowercase name (`qq`, `wx`, `mobile`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Qq => "qq",
            Self::Wx => "wx",
            Self::Mobile => "mobile",
        }
    }
}

/// QR code image plus the identifier needed to poll its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrCode {
    /// Image bytes.
    pub data: Vec<u8>,
    /// Channel.
    pub qr_type: QrLoginType,
    /// MIME type of `data`.
    pub mimetype: String,
    /// `qrsig` (QQ), `uuid` (WeChat) or `qrcodeID` (App).
    pub identifier: String,
}

impl QrCode {
    /// File extension derived from the MIME type.
    pub fn extension(&self) -> &'static str {
        match self.mimetype.as_str() {
            "image/jpeg" | "image/jpg" => ".jpg",
            "image/gif" => ".gif",
            "image/webp" => ".webp",
            _ => ".png",
        }
    }

    /// Save the image into `dir` (created if needed) as
    /// `{type}-{uuid}{ext}`; returns `None` for empty images.
    pub fn save(&self, dir: impl AsRef<Path>) -> std::io::Result<Option<PathBuf>> {
        if self.data.is_empty() {
            return Ok(None);
        }
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}-{}{}", self.qr_type.as_str(), uuid4(), self.extension()));
        std::fs::write(&path, &self.data)?;
        Ok(Some(path))
    }
}

/// Random RFC 4122 version 4 UUID string.
pub(crate) fn uuid4() -> String {
    let mut bytes: [u8; 16] = rand::random();
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    let hex = hex::encode(bytes);
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// QR login state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QrCodeLoginEvent {
    /// Logged in.
    Done,
    /// Waiting for a scan.
    Scan,
    /// Scanned, waiting for confirmation.
    Conf,
    /// QR code expired.
    Timeout,
    /// Login refused on the phone.
    Refuse,
}

impl QrCodeLoginEvent {
    /// Map a QQ (`ptuiCB`) or WeChat (`wx_errcode`) status code.
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 | 405 => Self::Done,
            66 | 408 => Self::Scan,
            67 | 404 => Self::Conf,
            65 | 402 => Self::Timeout,
            68 | 403 => Self::Refuse,
            _ => return None,
        })
    }

    /// Whether polling stops after this event.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Refuse | Self::Timeout)
    }
}

/// One QR polling result.
#[derive(Debug, Clone, PartialEq)]
pub struct QrLoginResult {
    /// Event.
    pub event: QrCodeLoginEvent,
    /// Credential (only for [`QrCodeLoginEvent::Done`]).
    pub credential: Option<Credential>,
}

impl QrLoginResult {
    /// Result without credential.
    pub fn event(event: QrCodeLoginEvent) -> Self {
        Self { event, credential: None }
    }

    /// Successful login.
    pub fn done(credential: Credential) -> Self {
        Self {
            event: QrCodeLoginEvent::Done,
            credential: Some(credential),
        }
    }

    /// Whether the login succeeded.
    pub fn is_done(&self) -> bool {
        self.event == QrCodeLoginEvent::Done
    }
}

/// Result of sending a phone verification code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhoneLoginEvent {
    /// Code sent (`0`).
    Send,
    /// Captcha required (`20276`).
    Captcha,
    /// Too frequent (`100001`).
    Frequency,
}

impl PhoneLoginEvent {
    /// Server code.
    pub fn code(self) -> i64 {
        match self {
            Self::Send => 0,
            Self::Captcha => 20_276,
            Self::Frequency => 100_001,
        }
    }
}

/// `send_authcode` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneAuthCodeResult {
    /// Event.
    pub event: PhoneLoginEvent,
    /// Captcha URL for [`PhoneLoginEvent::Captcha`].
    pub info: Option<String>,
}

/// Phone number used by the SMS login.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PhoneNumber {
    /// Plain number (`phoneNo`).
    Plain(String),
    /// Encrypted number (`encryptedPhoneNo`).
    Encrypted(String),
}

impl PhoneNumber {
    pub(crate) fn insert(&self, param: &mut serde_json::Value) {
        match self {
            Self::Plain(no) => param["phoneNo"] = no.clone().into(),
            Self::Encrypted(no) => param["encryptedPhoneNo"] = no.clone().into(),
        }
    }
}

impl From<u64> for PhoneNumber {
    fn from(value: u64) -> Self {
        Self::Plain(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_and_types() {
        assert_eq!(QrCodeLoginEvent::from_code(405), Some(QrCodeLoginEvent::Done));
        assert_eq!(QrCodeLoginEvent::from_code(66), Some(QrCodeLoginEvent::Scan));
        assert_eq!(QrCodeLoginEvent::from_code(404), Some(QrCodeLoginEvent::Conf));
        assert_eq!(QrCodeLoginEvent::from_code(65), Some(QrCodeLoginEvent::Timeout));
        assert_eq!(QrCodeLoginEvent::from_code(403), Some(QrCodeLoginEvent::Refuse));
        assert_eq!(QrCodeLoginEvent::from_code(1), None);
        assert!(QrCodeLoginEvent::Refuse.is_terminal());
        assert!(!QrCodeLoginEvent::Conf.is_terminal());
        assert_eq!(QrLoginType::Mobile.as_str(), "mobile");
        assert_eq!(PhoneLoginEvent::Captcha.code(), 20_276);
        let mut param = serde_json::json!({});
        PhoneNumber::from(138u64).insert(&mut param);
        PhoneNumber::Encrypted("e".into()).insert(&mut param);
        assert_eq!(param, serde_json::json!({"phoneNo": "138", "encryptedPhoneNo": "e"}));
        assert!(QrLoginResult::done(Credential::default()).is_done());
    }

    #[test]
    fn uuid_format() {
        let id = uuid4();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(uuid4(), id);
    }

    #[test]
    fn save_qrcode() {
        let dir = tempfile::tempdir().unwrap();
        let qr = QrCode {
            data: vec![1, 2, 3],
            qr_type: QrLoginType::Wx,
            mimetype: "image/jpeg".into(),
            identifier: "u".into(),
        };
        let path = qr.save(dir.path().join("sub")).unwrap().unwrap();
        assert!(path.file_name().unwrap().to_str().unwrap().starts_with("wx-"));
        assert_eq!(path.extension().unwrap(), "jpg");
        assert_eq!(std::fs::read(path).unwrap(), vec![1, 2, 3]);
        let empty = QrCode { data: vec![], ..qr };
        assert!(empty.save(dir.path()).unwrap().is_none());
    }
}
