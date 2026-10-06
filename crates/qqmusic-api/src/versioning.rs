//! Platform version profiles and `comm` parameter construction.
//!
//! Every CGI request carries a `comm` block describing the client (`ct`,
//! `cv`, device ids …). QQ Music applies region/copyright and risk-control
//! decisions based on these values, so all of them are customisable:
//!
//! * replace a whole [`VersionProfile`] (e.g. different `ct`/`cv`);
//! * override the `User-Agent` per platform;
//! * add, replace or remove individual `comm` keys with
//!   [`VersionProfile::comm_overrides`] (an empty value removes the key).

use std::fmt;
use std::str::FromStr;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::credential::Credential;
use crate::device::{Device, Qimei, SessionRecord};
use crate::error::Error;
use crate::utils::{hash33, now_secs, xml_escape};

/// Client platform emulated by a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Android app (default, richest feature set).
    #[default]
    Android,
    /// Windows desktop client.
    Desktop,
    /// Web player (`y.qq.com`).
    Web,
}

impl Platform {
    /// Lowercase name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::Desktop => "desktop",
            Self::Web => "web",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Platform {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        match s.to_ascii_lowercase().as_str() {
            "android" => Ok(Self::Android),
            "desktop" => Ok(Self::Desktop),
            "web" => Ok(Self::Web),
            other => Err(Error::invalid_argument(format!("unknown platform: {other}"))),
        }
    }
}

/// Default desktop/web browser user agent.
pub const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

/// Version parameters of one platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionProfile {
    /// Client type (`ct`).
    pub ct: i64,
    /// Client version (`cv`).
    pub cv: i64,
    /// Optional `v` parameter.
    #[serde(default)]
    pub v: Option<i64>,
    /// Optional `platform` parameter.
    #[serde(default)]
    pub platform: Option<String>,
    /// Version used in the Android user agent (defaults to `cv`).
    #[serde(default)]
    pub ua_version: Option<i64>,
    /// App version reported to the QIMEI service.
    #[serde(default = "default_qimei_app_version")]
    pub qimei_app_version: String,
    /// SDK version reported to the QIMEI service.
    #[serde(default = "default_qimei_sdk_version")]
    pub qimei_sdk_version: String,
    /// Full `User-Agent` override.
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Extra `comm` entries applied after the built-in ones.
    ///
    /// An empty value removes the key from `comm`.
    #[serde(default)]
    pub comm_overrides: IndexMap<String, String>,
}

fn default_qimei_app_version() -> String {
    "20.9.0.8".to_string()
}

fn default_qimei_sdk_version() -> String {
    "1.2.13.6".to_string()
}

impl VersionProfile {
    /// Profile with the given `ct`/`cv` and defaults for everything else.
    pub fn new(ct: i64, cv: i64) -> Self {
        Self {
            ct,
            cv,
            v: None,
            platform: None,
            ua_version: None,
            qimei_app_version: default_qimei_app_version(),
            qimei_sdk_version: default_qimei_sdk_version(),
            user_agent: None,
            comm_overrides: IndexMap::new(),
        }
    }

    /// Default Android profile.
    pub fn android() -> Self {
        Self {
            v: Some(20_090_008),
            ua_version: Some(20_090_008),
            ..Self::new(11, 20_090_008)
        }
    }

    /// Default desktop profile.
    pub fn desktop() -> Self {
        Self::new(19, 2201)
    }

    /// Default web profile.
    pub fn web() -> Self {
        Self {
            platform: Some("yqq.json".to_string()),
            ..Self::new(24, 4_747_474)
        }
    }

    /// Set `ct`.
    #[must_use]
    pub fn with_ct(mut self, ct: i64) -> Self {
        self.ct = ct;
        self
    }

    /// Set `cv`.
    #[must_use]
    pub fn with_cv(mut self, cv: i64) -> Self {
        self.cv = cv;
        self
    }

    /// Override the `User-Agent`.
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }

    /// Add / replace (`value != ""`) or remove (`value == ""`) a `comm` key.
    #[must_use]
    pub fn with_comm(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.comm_overrides.insert(key.into(), value.into());
        self
    }
}

/// Context needed to build the `comm` block.
#[derive(Debug, Clone, Copy)]
pub struct CommContext<'a> {
    /// Credential of the request.
    pub credential: &'a Credential,
    /// Device identity.
    pub device: &'a Device,
    /// QIMEI (Android only).
    pub qimei: Option<&'a Qimei>,
    /// Device GUID (defaults to `device.open_udid`).
    pub guid: &'a str,
    /// Android session.
    pub session: Option<&'a SessionRecord>,
}

/// Version profiles of all platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionPolicy {
    /// Android profile.
    pub android: VersionProfile,
    /// Desktop profile.
    pub desktop: VersionProfile,
    /// Web profile.
    pub web: VersionProfile,
}

impl Default for VersionPolicy {
    fn default() -> Self {
        Self {
            android: VersionProfile::android(),
            desktop: VersionProfile::desktop(),
            web: VersionProfile::web(),
        }
    }
}

struct CommBuilder(IndexMap<String, String>);

impl CommBuilder {
    fn put(&mut self, key: &str, value: impl ToString) -> &mut Self {
        self.0.insert(key.to_string(), value.to_string());
        self
    }

    fn put_opt<T: ToString>(&mut self, key: &str, value: Option<T>) -> &mut Self {
        if let Some(value) = value {
            self.put(key, value);
        }
        self
    }
}

impl VersionPolicy {
    /// Profile of a platform.
    pub fn profile(&self, platform: Platform) -> &VersionProfile {
        match platform {
            Platform::Android => &self.android,
            Platform::Desktop => &self.desktop,
            Platform::Web => &self.web,
        }
    }

    /// Mutable profile of a platform.
    pub fn profile_mut(&mut self, platform: Platform) -> &mut VersionProfile {
        match platform {
            Platform::Android => &mut self.android,
            Platform::Desktop => &mut self.desktop,
            Platform::Web => &mut self.web,
        }
    }

    /// `g_tk` derived from the `musickey`.
    pub fn g_tk(credential: &Credential) -> i64 {
        if credential.musickey.is_empty() {
            5381
        } else {
            hash33(&credential.musickey, 5381)
        }
    }

    /// Build the `comm` block for a platform.
    pub fn build_comm(&self, platform: Platform, ctx: CommContext<'_>) -> IndexMap<String, String> {
        let profile = self.profile(platform);
        let credential = ctx.credential;
        let mut comm = CommBuilder(IndexMap::new());
        comm.put("ct", profile.ct).put("cv", profile.cv).put_opt("v", profile.v);
        let musicid = (credential.musicid != 0).then_some(credential.musicid);
        match platform {
            Platform::Android => {
                let uid_part = musicid.map_or_else(|| ctx.guid.to_string(), |id| id.to_string());
                comm.put_opt("platform", profile.platform.as_ref())
                    .put("tmeAppID", "qqmusic")
                    .put("chid", "10003505")
                    .put_opt("qq", musicid)
                    .put_opt(
                        "authst",
                        (!credential.musickey.is_empty()).then_some(&credential.musickey),
                    )
                    .put_opt(
                        "tmeLoginType",
                        (credential.login_type != 0).then_some(credential.login_type),
                    )
                    .put("QIMEI36", ctx.qimei.map(|q| q.q36.as_str()).unwrap_or_default())
                    .put("traceid", format!("10002_{uid_part}_{}", now_secs()))
                    .put("OpenUDID", ctx.guid)
                    .put("OpenUDID2", &ctx.device.open_udid2)
                    .put("udid", ctx.guid)
                    .put("aid", &ctx.device.android_id)
                    .put_opt("uid", ctx.session.map(|s| &s.uid))
                    .put_opt("sid", ctx.session.map(|s| &s.sid))
                    .put("os_ver", &ctx.device.version.release)
                    .put("phonetype", xml_escape(&ctx.device.model));
            }
            Platform::Desktop => {
                comm.put_opt("platform", profile.platform.as_ref())
                    .put("chid", "0")
                    .put_opt("uin", musicid)
                    .put("g_tk", Self::g_tk(credential))
                    .put("guid", ctx.guid.to_uppercase());
            }
            Platform::Web => {
                let g_tk = Self::g_tk(credential);
                comm.put_opt("platform", profile.platform.as_ref())
                    .put("chid", "0")
                    .put("uin", credential.musicid)
                    .put("g_tk", g_tk)
                    .put("g_tk_new_20200303", g_tk)
                    .put("format", "json")
                    .put("inCharset", "utf-8")
                    .put("outCharset", "utf-8")
                    .put("notice", 0)
                    .put("need_new_code", 1);
            }
        }
        let mut comm = comm.0;
        apply_comm_overrides(&mut comm, &profile.comm_overrides);
        comm
    }

    /// `User-Agent` of a platform.
    pub fn user_agent(&self, platform: Platform, device: &Device) -> String {
        let profile = self.profile(platform);
        if let Some(ua) = &profile.user_agent {
            return ua.clone();
        }
        match platform {
            Platform::Android => {
                let version = profile.ua_version.unwrap_or(profile.cv);
                format!("QQMusic {version}(android {})", device.version.release)
            }
            Platform::Desktop | Platform::Web => BROWSER_USER_AGENT.to_string(),
        }
    }
}

/// Merge overrides into `comm`: empty values remove keys.
pub fn apply_comm_overrides<'a, I>(comm: &mut IndexMap<String, String>, overrides: I)
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    for (key, value) in overrides {
        if value.is_empty() {
            comm.shift_remove(key);
        } else {
            comm.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceProfile;

    fn device() -> Device {
        let mut d = Device::generate(Some(DeviceProfile::Vivo), Some(1));
        d.model = r#"V"2408A"#.into();
        d
    }

    fn keys(comm: &IndexMap<String, String>) -> Vec<&str> {
        comm.keys().map(String::as_str).collect()
    }

    #[test]
    fn android_comm_anonymous() {
        let policy = VersionPolicy::default();
        let device = device();
        let cred = Credential::default();
        let comm = policy.build_comm(
            Platform::Android,
            CommContext { credential: &cred, device: &device, qimei: None, guid: "g", session: None },
        );
        assert_eq!(
            keys(&comm),
            vec![
                "ct", "cv", "v", "tmeAppID", "chid", "QIMEI36", "traceid", "OpenUDID", "OpenUDID2",
                "udid", "aid", "os_ver", "phonetype"
            ]
        );
        assert_eq!(comm["ct"], "11");
        assert_eq!(comm["cv"], "20090008");
        assert_eq!(comm["QIMEI36"], "");
        assert!(comm["traceid"].starts_with("10002_g_"));
        assert_eq!(comm["phonetype"], "V&quot;2408A");
        assert_eq!(comm["os_ver"], "15");
    }

    #[test]
    fn android_comm_logged_in_with_session() {
        let policy = VersionPolicy::default();
        let device = device();
        let cred = Credential::new(123, "W_X_key");
        let qimei = Qimei { q16: "a".into(), q36: "b".into() };
        let session = SessionRecord { uid: "u".into(), sid: "s".into(), saved_at: 0 };
        let comm = policy.build_comm(
            Platform::Android,
            CommContext {
                credential: &cred,
                device: &device,
                qimei: Some(&qimei),
                guid: "g",
                session: Some(&session),
            },
        );
        assert_eq!(comm["qq"], "123");
        assert_eq!(comm["authst"], "W_X_key");
        assert_eq!(comm["tmeLoginType"], "1");
        assert_eq!(comm["QIMEI36"], "b");
        assert_eq!(comm["uid"], "u");
        assert_eq!(comm["sid"], "s");
        assert!(comm["traceid"].starts_with("10002_123_"));
        let pos = |k: &str| comm.get_index_of(k).unwrap();
        assert!(pos("aid") < pos("uid") && pos("sid") < pos("os_ver"));
    }

    #[test]
    fn desktop_and_web_comm() {
        let policy = VersionPolicy::default();
        let device = device();
        let anon = Credential::default();
        let ctx = CommContext { credential: &anon, device: &device, qimei: None, guid: "abc", session: None };
        let desktop = policy.build_comm(Platform::Desktop, ctx);
        assert_eq!(keys(&desktop), vec!["ct", "cv", "chid", "g_tk", "guid"]);
        assert_eq!(desktop["guid"], "ABC");
        assert_eq!(desktop["g_tk"], "5381");

        let web = policy.build_comm(Platform::Web, ctx);
        assert_eq!(
            keys(&web),
            vec![
                "ct", "cv", "platform", "chid", "uin", "g_tk", "g_tk_new_20200303", "format",
                "inCharset", "outCharset", "notice", "need_new_code"
            ]
        );
        assert_eq!(web["uin"], "0");
        assert_eq!(web["platform"], "yqq.json");

        let cred = Credential::new(7, "abc");
        let ctx = CommContext { credential: &cred, ..ctx };
        let desktop = policy.build_comm(Platform::Desktop, ctx);
        assert_eq!(desktop["uin"], "7");
        assert_eq!(desktop["g_tk"], "193485963");
    }

    #[test]
    fn comm_overrides_add_replace_remove() {
        let policy = VersionPolicy {
            android: VersionProfile::android()
                .with_ct(5)
                .with_comm("chid", "999")
                .with_comm("phonetype", "")
                .with_comm("extra", "1"),
            ..VersionPolicy::default()
        };
        let device = device();
        let cred = Credential::default();
        let comm = policy.build_comm(
            Platform::Android,
            CommContext { credential: &cred, device: &device, qimei: None, guid: "g", session: None },
        );
        assert_eq!(comm["ct"], "5");
        assert_eq!(comm["chid"], "999");
        assert!(!comm.contains_key("phonetype"));
        assert_eq!(comm.last().unwrap(), (&"extra".to_string(), &"1".to_string()));
    }

    #[test]
    fn user_agents() {
        let mut policy = VersionPolicy::default();
        let device = device();
        assert_eq!(policy.user_agent(Platform::Android, &device), "QQMusic 20090008(android 15)");
        assert_eq!(policy.user_agent(Platform::Web, &device), BROWSER_USER_AGENT);
        policy.profile_mut(Platform::Web).user_agent = Some("UA".into());
        assert_eq!(policy.user_agent(Platform::Web, &device), "UA");
        policy.android.ua_version = None;
        assert_eq!(policy.user_agent(Platform::Android, &device), "QQMusic 20090008(android 15)");
    }

    #[test]
    fn platform_parsing_and_serde() {
        assert_eq!("WEB".parse::<Platform>().unwrap(), Platform::Web);
        assert!("ios".parse::<Platform>().is_err());
        assert_eq!(serde_json::to_string(&Platform::Desktop).unwrap(), "\"desktop\"");
        let policy: VersionPolicy = serde_json::from_str(
            r#"{"android": {"ct": 11, "cv": 1}, "desktop": {"ct": 19, "cv": 2201}, "web": {"ct": 24, "cv": 3}}"#,
        )
        .unwrap();
        assert_eq!(policy.android.qimei_app_version, "20.9.0.8");
        assert!(policy.web.platform.is_none());
    }
}
