//! Login credential.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::json::{self, FromJson};
use crate::utils::now_secs;

/// Login credential (`musicid` + `musickey` and OAuth metadata).
///
/// Serializes with snake_case field names; deserialization additionally
/// accepts the camelCase aliases used by the QQ Music login APIs and is
/// lenient about number/string types.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, FromJson)]
#[json(default, preprocess = "infer_login_type")]
pub struct Credential {
    /// OpenID.
    pub openid: String,
    /// OAuth refresh token.
    pub refresh_token: String,
    /// OAuth access token.
    pub access_token: String,
    /// Access token expiry.
    pub expired_at: i64,
    /// QQ Music user id (`uin`).
    pub musicid: i64,
    /// Login key (`qm_keyst`).
    pub musickey: String,
    /// Union id.
    pub unionid: String,
    /// String form of `musicid`.
    pub str_musicid: String,
    /// Refresh key.
    pub refresh_key: String,
    /// `musickey` creation timestamp.
    #[json(alias = "musickeyCreateTime")]
    pub musickey_create_time: i64,
    /// `musickey` lifetime in seconds.
    #[json(alias = "keyExpiresIn")]
    pub key_expires_in: i64,
    /// First login flag.
    pub first_login: i64,
    /// Bound account type.
    #[json(alias = "bindAccountType")]
    pub bind_account_type: i64,
    /// Refresh interval hint.
    #[json(alias = "needRefreshKeyIn")]
    pub need_refresh_key_in: i64,
    /// Encrypted uin.
    #[json(alias = "encryptUin")]
    pub encrypt_uin: String,
    /// Login type: `1` WeChat, `2` QQ.
    #[json(alias = "loginType")]
    pub login_type: i64,
}

fn infer_login_type(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    if map.contains_key("loginType") || map.contains_key("login_type") {
        return;
    }
    let inferred = match map.get("musickey") {
        Some(Value::String(key)) if !key.is_empty() => {
            if key.starts_with("W_X") {
                1
            } else {
                2
            }
        }
        Some(Value::Null | Value::Bool(false)) | None => return,
        Some(Value::String(_)) => return,
        Some(_) => 2,
    };
    map.insert("loginType".into(), Value::from(inferred));
}

impl Credential {
    /// Credential from `musicid` and `musickey` (cookie `uin` / `qm_keyst`).
    pub fn new(musicid: i64, musickey: impl Into<String>) -> Self {
        let musickey = musickey.into();
        let login_type = if musickey.is_empty() {
            0
        } else if musickey.starts_with("W_X") {
            1
        } else {
            2
        };
        Self {
            musicid,
            str_musicid: musicid.to_string(),
            musickey,
            login_type,
            ..Self::default()
        }
    }

    /// Parse a credential from a JSON value (login responses or saved files).
    pub fn from_value(value: &Value) -> Result<Self, json::JsonError> {
        Self::from_json(value)
    }

    /// Parse a credential from a JSON string.
    pub fn from_json_str(input: &str) -> Result<Self, json::JsonError> {
        json::from_str(input)
    }

    /// Serialize as a JSON string.
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Whether `musickey` has expired according to its own metadata.
    pub fn is_expired(&self) -> bool {
        now_secs() >= self.musickey_create_time + self.key_expires_in
    }

    /// Whether this credential contains a usable login state.
    pub fn is_valid(&self) -> bool {
        self.musicid > 0 && !self.musickey.is_empty()
    }

    /// `uin` value used in cookies.
    pub fn uin(&self) -> String {
        if self.str_musicid.is_empty() {
            self.musicid.to_string()
        } else {
            self.str_musicid.clone()
        }
    }
}

impl<'de> Deserialize<'de> for Credential {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_json(&value).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_login_response_aliases() {
        let cred = Credential::from_value(&json!({
            "musicid": "123456",
            "musickey": "Q_H_L_xxx",
            "musickeyCreateTime": 1_700_000_000,
            "keyExpiresIn": "259200",
            "encryptUin": "enc",
            "refresh_key": "rk",
            "unknown": 1
        }))
        .unwrap();
        assert_eq!(cred.musicid, 123_456);
        assert_eq!(cred.key_expires_in, 259_200);
        assert_eq!(cred.encrypt_uin, "enc");
        assert_eq!(cred.login_type, 2);
    }

    #[test]
    fn login_type_inference() {
        let wx = Credential::from_value(&json!({"musickey": "W_X_abc"})).unwrap();
        assert_eq!(wx.login_type, 1);
        let explicit = Credential::from_value(&json!({"musickey": "W_X_abc", "loginType": 2})).unwrap();
        assert_eq!(explicit.login_type, 2);
        let snake = Credential::from_value(&json!({"musickey": "W_X_abc", "login_type": 2})).unwrap();
        assert_eq!(snake.login_type, 2);
        let none = Credential::from_value(&json!({"musickey": ""})).unwrap();
        assert_eq!(none.login_type, 0);
        assert_eq!(Credential::new(1, "W_X_k").login_type, 1);
        assert_eq!(Credential::new(1, "Q_H").login_type, 2);
        assert_eq!(Credential::new(1, "").login_type, 0);
    }

    #[test]
    fn validity_and_expiry() {
        assert!(!Credential::default().is_valid());
        let cred = Credential::new(10, "key");
        assert!(cred.is_valid());
        assert!(cred.is_expired());
        let fresh = Credential {
            musickey_create_time: now_secs(),
            key_expires_in: 3600,
            ..cred.clone()
        };
        assert!(!fresh.is_expired());
        assert_eq!(cred.uin(), "10");
        let custom = Credential { str_musicid: "o10".into(), ..cred };
        assert_eq!(custom.uin(), "o10");
    }

    #[test]
    fn serde_roundtrip() {
        let cred = Credential {
            musickey_create_time: 5,
            ..Credential::new(42, "W_X_key")
        };
        let text = cred.to_json_string();
        assert!(text.contains("\"musickey_create_time\":5"));
        let back: Credential = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cred);
        assert_eq!(Credential::from_json_str(&text).unwrap(), cred);
        assert!(serde_json::from_str::<Credential>("[1]").is_err());
    }
}
