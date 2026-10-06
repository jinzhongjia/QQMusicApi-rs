//! Android `uid`/`sid` session (`music.getSession.session`).
//!
//! The Android client obtains a session once per day and reports it in
//! `comm`. Requests without it look like a freshly installed client.

use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::client::Inner;
use crate::credential::Credential;
use crate::device::SessionRecord;
use crate::error::{Error, Result};
use crate::response::{parse_cgi_item, unwrap_cgi_envelope};
use crate::transport::{Body, Method, Request};
use crate::utils::{china_day, now_secs};
use crate::versioning::{CommContext, Platform};

/// Whether a session was obtained today (China Standard Time).
pub fn saved_today(session: &SessionRecord, now: i64) -> bool {
    china_day(session.saved_at) == china_day(now)
}

/// Parse the `session` object of a GetSession response.
pub fn parse_session(data: &Value, now: i64) -> Result<SessionRecord> {
    let session = data
        .get("session")
        .filter(|s| s.is_object())
        .ok_or_else(|| Error::api_data("Android Session 响应格式异常, 缺少会话字段"))?;
    let uid = match session.get("uid") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::Number(n)) if n.is_i64() || n.is_u64() => n.to_string(),
        _ => return Err(Error::api_data("Android Session 响应缺少有效的 uid")),
    };
    let sid = match session.get("sid") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        _ => return Err(Error::api_data("Android Session 响应缺少有效的 sid")),
    };
    Ok(SessionRecord {
        uid,
        sid,
        saved_at: now,
    })
}

/// Daily Android session cache.
#[derive(Debug, Default)]
pub struct AndroidSessionManager {
    state: Mutex<Option<SessionRecord>>,
}

impl AndroidSessionManager {
    /// Empty manager.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return today's session, refreshing it when necessary.
    pub(crate) async fn ensure(&self, inner: &Inner) -> Result<SessionRecord> {
        let mut state = self.state.lock().await;
        let now = now_secs();
        if state.is_none() {
            *state = inner.devices.cache().session().await.filter(|s| !s.uid.is_empty() && !s.sid.is_empty());
        }
        if let Some(session) = state.as_ref()
            && saved_today(session, now)
        {
            return Ok(session.clone());
        }
        let stale = state.clone();
        let session = refresh(inner, stale.as_ref()).await?;
        inner.devices.cache().set_session(&session).await;
        *state = Some(session.clone());
        Ok(session)
    }
}

async fn refresh(inner: &Inner, stale: Option<&SessionRecord>) -> Result<SessionRecord> {
    let device = inner.devices.get().await?;
    let qimei = inner.cached_qimei().await;
    let policy = inner.policy();
    let anonymous = Credential::default();
    let comm = policy.build_comm(
        Platform::Android,
        CommContext {
            credential: &anonymous,
            device: &device,
            qimei: qimei.as_ref(),
            guid: &device.open_udid,
            session: None,
        },
    );
    let caller = if stale.is_some() { 1 } else { 2 };
    let payload = json!({
        "comm": comm,
        "req_0": {
            "module": "music.getSession.session",
            "method": "GetSession",
            "param": {
                "uid": stale.map(|s| s.uid.as_str()).unwrap_or_default(),
                "vkey": 0,
                "caller": caller,
            }
        }
    });
    let mut request = Request::new(Method::Post, inner.endpoints.musicu.clone());
    request.set_header("User-Agent", policy.user_agent(Platform::Android, &device));
    request.body = Body::json(&payload);
    let response = inner.send(request).await?;
    let item = unwrap_cgi_envelope(&response, 1)?
        .pop()
        .flatten()
        .ok_or_else(|| Error::api_data("Android Session 响应格式异常, 缺少 req_0"))?;
    let data = parse_cgi_item(item, None, false)?;
    parse_session(&data, now_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_variants() {
        let s = parse_session(&json!({"session": {"uid": 12, "sid": "x"}}), 5).unwrap();
        assert_eq!(s, SessionRecord { uid: "12".into(), sid: "x".into(), saved_at: 5 });
        assert!(parse_session(&json!({}), 0).is_err());
        assert!(parse_session(&json!({"session": {"uid": "", "sid": "x"}}), 0).is_err());
        assert!(parse_session(&json!({"session": {"uid": "u", "sid": 1}}), 0).is_err());
        assert!(parse_session(&json!({"session": {"uid": true, "sid": "s"}}), 0).is_err());
    }

    #[test]
    fn same_day_in_china() {
        let base = 1_791_302_399; // 23:59:59 CST
        let session = SessionRecord { uid: "u".into(), sid: "s".into(), saved_at: base };
        assert!(saved_today(&session, base - 60));
        assert!(!saved_today(&session, base + 1));
    }
}
