//! Region / risk-control bypass for playback URLs.
//!
//! QQ Music decides which audio qualities (and whether *any* URL) are
//! returned for `music.vkey.GetVkey` mainly from the `comm.ct` value. The
//! library default (`ct=11`, Android) is downgraded by region/copyright
//! rules and CDN risk control even for paying users. Following
//! [decky-music](https://github.com/jinzhongjia/decky-music), playback URL
//! requests are therefore sent with a replacement `comm`:
//!
//! * `ct` – picked from [`HIGH_QUALITY_CT`] (values verified to return every
//!   quality tier). It is derived from the device GUID so one device always
//!   uses the same value while different installs spread across values –
//!   both a constant and a random-per-request `ct` are risk signals;
//! * `cv = 0`;
//! * `qq` / `authst` – the logged-in credential (needed for VIP qualities).
//!
//! Every part is configurable through [`BypassConfig`].

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::credential::Credential;

/// `ct` values that receive every quality tier (FLAC / 320k / 128k).
///
/// `ct=2/6/26` only get 128k and the default `ct=11` may get nothing.
pub const HIGH_QUALITY_CT: [i64; 19] = [5, 7, 8, 9, 10, 12, 13, 14, 15, 16, 17, 20, 21, 22, 24, 25, 28, 29, 30];

/// CDN used when the API returns a bare `purl` path.
pub const DEFAULT_CDN: &str = "https://isure.stream.qqmusic.qq.com/";

/// How the bypass `ct` is chosen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CtStrategy {
    /// Stable value derived from the device GUID (`sha256(guid)[0] % len`).
    DerivedFromGuid(Vec<i64>),
    /// Always use the given value.
    Fixed(i64),
}

impl Default for CtStrategy {
    fn default() -> Self {
        Self::DerivedFromGuid(HIGH_QUALITY_CT.to_vec())
    }
}

impl CtStrategy {
    /// `ct` for a device GUID.
    pub fn ct_for(&self, guid: &str) -> i64 {
        match self {
            Self::Fixed(ct) => *ct,
            Self::DerivedFromGuid(candidates) => {
                let candidates: &[i64] = if candidates.is_empty() { &HIGH_QUALITY_CT } else { candidates };
                if guid.is_empty() {
                    return candidates[0];
                }
                let digest = Sha256::digest(guid.as_bytes());
                candidates[usize::from(digest[0]) % candidates.len()]
            }
        }
    }
}

/// Bypass configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BypassConfig {
    /// Use the bypass `comm` for playback URL requests.
    pub enabled: bool,
    /// `ct` selection.
    pub ct: CtStrategy,
    /// `cv` value.
    pub cv: i64,
    /// Include `qq`/`authst` from the credential.
    pub include_credential: bool,
    /// Extra comm entries (an empty value removes the key).
    pub extra_comm: IndexMap<String, String>,
    /// CDN prefix for bare `purl` values.
    pub cdn: String,
    /// Upgrade `http://` URLs to `https://`.
    pub force_https: bool,
    /// User-Agent for bypass requests (`None` = the web/browser UA of the
    /// version policy, matching a browser TLS fingerprint).
    pub user_agent: Option<String>,
}

impl Default for BypassConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ct: CtStrategy::default(),
            cv: 0,
            include_credential: true,
            extra_comm: IndexMap::new(),
            cdn: DEFAULT_CDN.to_string(),
            force_https: true,
            user_agent: None,
        }
    }
}

impl BypassConfig {
    /// Disabled bypass (requests use the regular platform `comm`).
    pub fn disabled() -> Self {
        Self { enabled: false, ..Self::default() }
    }

    /// Use a fixed `ct`.
    #[must_use]
    pub fn with_fixed_ct(mut self, ct: i64) -> Self {
        self.ct = CtStrategy::Fixed(ct);
        self
    }

    /// Use a custom CDN.
    #[must_use]
    pub fn with_cdn(mut self, cdn: impl Into<String>) -> Self {
        self.cdn = cdn.into();
        self
    }

    /// Add/replace (or with an empty value remove) a comm entry.
    #[must_use]
    pub fn with_comm(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_comm.insert(key.into(), value.into());
        self
    }

    /// The replacement `comm` for a playback URL request.
    pub fn comm(&self, guid: &str, credential: &Credential) -> IndexMap<String, String> {
        let mut comm = IndexMap::new();
        comm.insert("ct".to_string(), self.ct.ct_for(guid).to_string());
        comm.insert("cv".to_string(), self.cv.to_string());
        if self.include_credential && !credential.musickey.is_empty() {
            comm.insert("qq".to_string(), credential.musicid.to_string());
            comm.insert("authst".to_string(), credential.musickey.clone());
        }
        crate::versioning::apply_comm_overrides(&mut comm, &self.extra_comm);
        comm
    }

    /// Turn a `purl` into a full URL.
    pub fn full_url(&self, purl: &str) -> String {
        if let Some(rest) = purl.strip_prefix("http://") {
            if self.force_https {
                return format!("https://{rest}");
            }
            return purl.to_string();
        }
        if purl.starts_with("https://") {
            return purl.to_string();
        }
        let cdn = if self.cdn.ends_with('/') { self.cdn.clone() } else { format!("{}/", self.cdn) };
        format!("{cdn}{}", purl.trim_start_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_is_stable_and_from_candidates() {
        let strategy = CtStrategy::default();
        let a = strategy.ct_for("ffffffffffffffffffffffffffffffff");
        assert_eq!(a, strategy.ct_for("ffffffffffffffffffffffffffffffff"));
        assert!(HIGH_QUALITY_CT.contains(&a));
        assert_eq!(strategy.ct_for(""), 5);
        assert_eq!(CtStrategy::Fixed(17).ct_for("x"), 17);
        assert_eq!(CtStrategy::DerivedFromGuid(vec![]).ct_for(""), 5);
        assert_eq!(CtStrategy::DerivedFromGuid(vec![9]).ct_for("abc"), 9);
    }

    #[test]
    fn ct_matches_python_reference() {
        // python: HIGH_QUALITY_CT[hashlib.sha256(g.encode()).digest()[0] % 19]
        let strategy = CtStrategy::default();
        assert_eq!(strategy.ct_for("abc"), PY_CT_ABC);
        assert_eq!(strategy.ct_for("0123456789abcdef0123456789abcdef"), PY_CT_HEX);
    }

    const PY_CT_ABC: i64 = 25;
    const PY_CT_HEX: i64 = 12;

    #[test]
    fn comm_contents() {
        let config = BypassConfig::default().with_fixed_ct(13);
        let anon = config.comm("g", &Credential::default());
        assert_eq!(anon.keys().collect::<Vec<_>>(), ["ct", "cv"]);
        assert_eq!(anon["ct"], "13");
        assert_eq!(anon["cv"], "0");
        let cred = Credential::new(42, "Q_H_L_key");
        let comm = config.with_comm("cv", "").with_comm("chid", "1").comm("g", &cred);
        assert_eq!(comm["qq"], "42");
        assert_eq!(comm["authst"], "Q_H_L_key");
        assert!(!comm.contains_key("cv"));
        assert_eq!(comm["chid"], "1");
        let no_cred = BypassConfig { include_credential: false, ..BypassConfig::default() };
        assert!(!no_cred.comm("g", &cred).contains_key("qq"));
    }

    #[test]
    fn full_urls() {
        let config = BypassConfig::default();
        assert_eq!(config.full_url("M500x.mp3?vkey=1"), "https://isure.stream.qqmusic.qq.com/M500x.mp3?vkey=1");
        assert_eq!(config.full_url("http://a/b"), "https://a/b");
        assert_eq!(config.full_url("https://a/b"), "https://a/b");
        let custom = BypassConfig { force_https: false, ..config.with_cdn("http://cdn.example") };
        assert_eq!(custom.full_url("/x"), "http://cdn.example/x");
        assert_eq!(custom.full_url("http://a/b"), "http://a/b");
        assert!(!BypassConfig::disabled().enabled);
    }

    #[test]
    fn serde_roundtrip() {
        let config = BypassConfig::default().with_fixed_ct(7);
        let text = serde_json::to_string(&config).unwrap();
        assert!(text.contains(r#""ct":{"fixed":7}"#));
        assert_eq!(serde_json::from_str::<BypassConfig>(&text).unwrap(), config);
        let partial: BypassConfig = serde_json::from_str(r#"{"cv": 1}"#).unwrap();
        assert_eq!(partial.cv, 1);
        assert!(partial.enabled);
    }
}
