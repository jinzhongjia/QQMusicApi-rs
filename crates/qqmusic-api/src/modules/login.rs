//! Login APIs: QQ / WeChat / QQ Music App QR codes, SMS login, credential
//! refresh and logout.

use std::time::Duration;

use base64::Engine as _;
use futures::stream::{self, BoxStream, StreamExt};
use serde_json::{Value, json};
use tokio::time::{Instant, sleep, sleep_until, timeout_at};

use crate::FromJson;
use crate::credential::Credential;
use crate::error::{ApiError, ApiErrorKind, Error, Result};
use crate::models::login::{
    PhoneAuthCodeResult, PhoneLoginEvent, PhoneNumber, QrCode, QrCodeLoginEvent, QrLoginResult, QrLoginType, uuid4,
};
use crate::mqtt::{ConnectOptions, MqttError, MqttSession, Properties, WsTarget};
use crate::request::{CgiRequest, HttpSpec};
use crate::response::RawPayload;
use crate::transport::{Body, Method};
use crate::utils::{hash33, now_millis, now_secs};
use crate::versioning::Platform;

/// Codes returned as data (instead of errors) by login endpoints so they can
/// be mapped to login specific errors.
pub const LOGIN_ERROR_CODES: [i64; 12] =
    [1000, 104_401, 104_400, 20_261, 20_271, 20_272, 20_274, 20_277, 20_278, 20_279, 20_450, 104_604];

const QQ_APPID: &str = "716027609";
const QQ_3RD_AID: &str = "100497308";
const WX_APPID: &str = "wx48db31d50e334801";
const PTLOGIN_REFERER: &str = "https://xui.ptlogin2.qq.com/";
const MOBILE_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";

/// Map a login response (`data` on success, `{code, data}` for an allowed
/// error code) to the data or a typed login error.
pub fn validate_login_result(resp: Value) -> Result<Value> {
    let code = resp.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code == 0 {
        return Ok(resp);
    }
    let data = resp.get("data").cloned().unwrap_or_else(|| json!({}));
    let err = ApiError::login(code, data);
    let message = match code {
        20_261 => Some("登录参数错误"),
        20_271 => Some("验证码错误"),
        20_272 => Some("账号绑定异常"),
        20_274 => Some("账号绑定缺失"),
        20_450 => Some("账号已被封禁"),
        _ => None,
    };
    Err(match message {
        Some(message) => err.with_message(message),
        None => err,
    }
    .into())
}

fn login_error(message: &str, data: Value) -> Error {
    ApiError::login(-1, data).with_message(message).into()
}

/// The refresh response omits some fields (e.g. `unionid` for QQ logins);
/// carry them over from the credential being refreshed.
fn keep_missing(mut new: Credential, old: &Credential) -> Credential {
    for (field, previous) in [
        (&mut new.openid, &old.openid),
        (&mut new.refresh_token, &old.refresh_token),
        (&mut new.access_token, &old.access_token),
        (&mut new.unionid, &old.unionid),
        (&mut new.str_musicid, &old.str_musicid),
        (&mut new.refresh_key, &old.refresh_key),
        (&mut new.encrypt_uin, &old.encrypt_uin),
    ] {
        if field.is_empty() {
            field.clone_from(previous);
        }
    }
    for (field, previous) in [
        (&mut new.expired_at, old.expired_at),
        (&mut new.musicid, old.musicid),
        (&mut new.bind_account_type, old.bind_account_type),
        (&mut new.login_type, old.login_type),
    ] {
        if *field == 0 {
            *field = previous;
        }
    }
    new
}

fn credential_from(value: Value) -> Result<Credential> {
    Ok(Credential::from_json(&validate_login_result(value)?)?)
}

/// Contents of every single-quoted string (backslash escapes kept verbatim),
/// like the regex `'((?:\\.|[^'])*)'`.
fn quoted_args(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\'' {
            continue;
        }
        let mut current = String::new();
        let mut closed = false;
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    current.push(c);
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                }
                '\'' => {
                    closed = true;
                    break;
                }
                _ => current.push(c),
            }
        }
        if closed {
            out.push(current);
        }
    }
    out
}

/// First `key(.+?)end` match whose key is preceded by one of `prefixes`
/// (any position when `prefixes` is empty).
fn capture<'a>(text: &'a str, key: &str, end: &str, prefixes: &[char]) -> Option<&'a str> {
    text.match_indices(key).find_map(|(idx, _)| {
        if !prefixes.is_empty() && !text[..idx].chars().next_back().is_some_and(|c| prefixes.contains(&c)) {
            return None;
        }
        let start = idx + key.len();
        let first = text[start..].chars().next()?;
        let search_from = start + first.len_utf8();
        let rel = text[search_from..].find(end)?;
        let value = &text[start..search_from + rel];
        (!value.contains('\n')).then_some(value)
    })
}

/// Parse `ptuiCB('code', ..)` from a `ptqrlogin` response.
fn parse_ptui_cb(text: &str) -> Option<Vec<String>> {
    let start = text.find("ptuiCB(")? + "ptuiCB(".len();
    let end = text[start..].find(')')? + start;
    Some(quoted_args(&text[start..end]))
}

/// Parse `window.wx_errcode=..;window.wx_code='..'`.
fn parse_wx_status(text: &str) -> Option<(&str, &str)> {
    const KEY: &str = "window.wx_errcode=";
    text.match_indices(KEY).find_map(|(idx, _)| {
        let rest = &text[idx + KEY.len()..];
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let tail = rest[digits..].strip_prefix(";window.wx_code='")?;
        let end = tail.find('\'')?;
        Some((&rest[..digits], &tail[..end]))
    })
}

/// Poll intervals of a [`QrCodeLoginSession`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollInterval {
    /// Default interval.
    pub default: Duration,
    /// Interval after the code was scanned (default: half of `default`).
    pub scanned: Option<Duration>,
    /// Base back-off after network errors (default: twice `default`).
    pub error: Option<Duration>,
}

impl PollInterval {
    /// Interval while waiting for confirmation.
    pub fn scanned_interval(&self) -> Duration {
        self.scanned.unwrap_or(self.default / 2)
    }

    /// Maximum back-off after network errors.
    pub fn error_interval(&self) -> Duration {
        self.error.unwrap_or(self.default * 2)
    }
}

impl Default for PollInterval {
    fn default() -> Self {
        Duration::from_millis(1500).into()
    }
}

impl From<Duration> for PollInterval {
    fn from(default: Duration) -> Self {
        Self { default, scanned: None, error: None }
    }
}

api_module! {
    /// Login APIs.
    LoginApi
}

impl LoginApi {
    fn login_cgi(&self, module: &str, method: &str, param: Value) -> CgiRequest<Value> {
        self.cgi(module, method, param).allow_error_codes(LOGIN_ERROR_CODES, false)
    }

    /// Whether the credential (default: the client's) is expired.
    pub async fn check_expired(&self, credential: Option<Credential>) -> Result<bool> {
        let target = credential.unwrap_or_else(|| self.client.credential());
        if self.client.platform() == Platform::Web {
            let musicid = target.musicid.to_string();
            let spec = HttpSpec::new(Method::Get, "https://c6.y.qq.com/rsc/fcgi-bin/fcg_get_profile_homepage.fcg")
                .query([
                    ("g_tk", hash33(&target.musickey, 5381).to_string()),
                    ("format", "json".into()),
                    ("inCharset", "utf-8".into()),
                    ("outCharset", "utf-8".into()),
                    ("notice", "0".into()),
                    ("cid", "205360838".into()),
                    ("needNewCode", "0".into()),
                    ("loginUin", musicid.clone()),
                    ("hostUin", "0".into()),
                    ("userid", musicid),
                    ("reqfrom", "1".into()),
                ]);
            let resp: Value = self.client.http(spec).credential(target).send().await?;
            return Ok(resp.get("code").and_then(Value::as_i64) != Some(0));
        }
        let data: Value = self
            .cgi("music.UserInfo.userInfoServer", "GetLoginUserInfo", json!({}))
            .credential(target)
            .allow_error_codes([1000, 104_401, 104_400], false)
            .send()
            .await?;
        Ok(data.get("code").and_then(Value::as_i64).unwrap_or(0) != 0)
    }

    /// Refresh `musickey`. When `credential` is `None` the client's credential
    /// is refreshed and replaced.
    pub async fn refresh_credential(&self, credential: Option<Credential>) -> Result<Credential> {
        let update_client = credential.is_none();
        let target = credential.unwrap_or_else(|| self.client.credential());
        let str_musicid =
            if target.str_musicid.is_empty() { target.musicid.to_string() } else { target.str_musicid.clone() };
        let param = match target.login_type {
            1 => json!({
                "openid": target.openid,
                "refresh_token": target.refresh_token,
                "str_musicid": str_musicid,
                "musickey": target.musickey,
                "unionid": target.unionid,
                "refresh_key": target.refresh_key,
                "loginMode": 2,
            }),
            2 => json!({
                "openid": target.openid,
                "access_token": target.access_token,
                "refresh_token": target.refresh_token,
                "expired_in": target.expired_at,
                "musicid": target.musicid,
                "musickey": target.musickey,
                "refresh_key": target.refresh_key,
                "loginMode": 2,
            }),
            _ => json!({
                "openid": target.openid,
                "access_token": target.access_token,
                "refresh_token": target.refresh_token,
                "expired_in": target.expired_at,
                "str_musicid": str_musicid,
                "musicid": target.musicid,
                "musickey": target.musickey,
                "unionid": target.unionid,
                "refresh_key": target.refresh_key,
                "loginMode": 2,
            }),
        };
        let data = self
            .login_cgi("music.login.LoginServer", "Login", param)
            .comm("tmeLoginType", target.login_type.to_string())
            .credential(target.clone())
            .send()
            .await?;
        let refreshed = credential_from(data).map_err(|err| match err {
            Error::Api(api) if matches!(api.kind, ApiErrorKind::Login(_)) => Error::from(
                ApiError::new(ApiErrorKind::CredentialRefresh, api.code, api.data).with_message(api.message),
            ),
            other => other,
        })?;
        let refreshed = keep_missing(refreshed, &target);
        if update_client {
            self.client.set_credential(refreshed.clone());
        }
        Ok(refreshed)
    }

    /// Log out. When `credential` is `None` the client's credential is cleared.
    pub async fn logout(&self, credential: Option<Credential>) -> Result<()> {
        let clear = credential.is_none();
        self.login_cgi("music.login.LoginServer", "Logout", json!({}))
            .require_login(true)
            .credential_opt(credential)
            .send()
            .await?;
        if clear {
            self.client.set_credential(Credential::default());
        }
        Ok(())
    }

    /// Fetch a login QR code.
    pub async fn get_qrcode(&self, login_type: QrLoginType) -> Result<QrCode> {
        match login_type {
            QrLoginType::Qq => self.get_qq_qr().await,
            QrLoginType::Wx => self.get_wx_qr().await,
            QrLoginType::Mobile => self.get_mobile_qr().await,
        }
    }

    /// Poll the state of a QQ / WeChat QR code once.
    pub async fn check_qrcode(&self, qrcode: &QrCode) -> Result<QrLoginResult> {
        match qrcode.qr_type {
            QrLoginType::Wx => self.check_wx_qr(qrcode).await,
            _ => self.check_qq_qr(qrcode).await,
        }
    }

    /// Event stream of a QQ Music App QR code (MQTT push).
    ///
    /// Yields [`QrCodeLoginEvent::Scan`] once subscribed, then the pushed
    /// events until a terminal one. `deadline` turns into a
    /// [`QrCodeLoginEvent::Timeout`] event.
    pub fn mobile_qrcode_events(
        &self,
        qrcode: &QrCode,
        deadline: Option<Instant>,
    ) -> BoxStream<'static, Result<QrLoginResult>> {
        let state = MobilePoll {
            api: self.clone(),
            qrcode_id: qrcode.identifier.clone(),
            deadline,
            session: None,
            started: false,
            finished: false,
        };
        stream::unfold(state, MobilePoll::next).boxed()
    }

    /// Send an SMS verification code.
    pub async fn send_authcode(&self, phone: impl Into<PhoneNumber>, country_code: u32) -> Result<PhoneAuthCodeResult> {
        let mut param = json!({"tmeAppid": "qqmusic", "areaCode": country_code.to_string()});
        phone.into().insert(&mut param);
        let resp: Value = self
            .cgi("music.login.LoginServer", "SendPhoneAuthCode", param)
            .comm("tmeLoginMethod", "3")
            .platform(Platform::Android)
            .allow_error_codes(crate::response::AllowErrorCodes::All, false)
            .send()
            .await?;
        let code = resp.get("code").and_then(Value::as_i64).unwrap_or(0);
        let data = resp.get("data").cloned().unwrap_or_else(|| json!({}));
        let event = match code {
            0 => PhoneLoginEvent::Send,
            20_276 => PhoneLoginEvent::Captcha,
            100_001 => PhoneLoginEvent::Frequency,
            _ => return Err(ApiError::login(code, data).with_message("发送验证码失败").into()),
        };
        let info = (event == PhoneLoginEvent::Captcha)
            .then(|| data.get("securityURL").and_then(Value::as_str).map(str::to_string))
            .flatten();
        Ok(PhoneAuthCodeResult { event, info })
    }

    /// Log in with an SMS code; the client's credential is replaced.
    pub async fn phone_authorize(&self, phone: impl Into<PhoneNumber>, auth_code: &str) -> Result<Credential> {
        let mut param = json!({"code": auth_code, "loginMode": 1});
        phone.into().insert(&mut param);
        let data = self
            .login_cgi("music.login.LoginServer", "Login", param)
            .comm("tmeLoginMethod", "3")
            .comm("tmeLoginType", "0")
            .platform(Platform::Android)
            .send()
            .await?;
        let credential = credential_from(data)?;
        self.client.set_credential(credential.clone());
        Ok(credential)
    }

    /// High level QR login flow.
    pub fn qrcode_session(&self, login_type: QrLoginType) -> QrCodeLoginSession {
        QrCodeLoginSession {
            api: self.clone(),
            login_type,
            interval: PollInterval::default(),
            timeout: Duration::from_secs(180),
            emit_repeat: false,
            qrcode: None,
        }
    }

    /// High level SMS login flow.
    pub fn phone_session(&self, phone: impl Into<PhoneNumber>, country_code: u32) -> PhoneLoginSession {
        PhoneLoginSession { api: self.clone(), phone: phone.into(), country_code, last_result: None }
    }

    async fn get_qq_qr(&self) -> Result<QrCode> {
        let spec = HttpSpec::new(Method::Get, "https://ssl.ptlogin2.qq.com/ptqrshow")
            .query([
                ("appid", QQ_APPID.to_string()),
                ("e", "2".into()),
                ("l", "M".into()),
                ("s", "3".into()),
                ("d", "72".into()),
                ("v", "4".into()),
                ("t", rand::random::<f64>().to_string()),
                ("daid", "383".into()),
                ("pt_3rd_aid", QQ_3RD_AID.into()),
            ])
            .header("Referer", PTLOGIN_REFERER);
        let payload: RawPayload = self.client.http(spec).send().await?;
        let qrsig = payload.cookies.get("qrsig").cloned().ok_or_else(|| Error::api_data("获取 qrsig 失败"))?;
        Ok(QrCode { data: payload.content, qr_type: QrLoginType::Qq, mimetype: "image/png".into(), identifier: qrsig })
    }

    async fn get_wx_qr(&self) -> Result<QrCode> {
        let spec = HttpSpec::new(Method::Get, "https://open.weixin.qq.com/connect/qrconnect").query([
            ("appid", WX_APPID),
            ("redirect_uri", "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/"),
            ("response_type", "code"),
            ("scope", "snsapi_login"),
            ("state", "STATE"),
            ("href", "https://y.qq.com/mediastyle/music_v17/src/css/popup_wechat.css#wechat_redirect"),
        ]);
        let payload: RawPayload = self.client.http(spec).send().await?;
        let text = payload.text();
        if text.is_empty() {
            return Err(Error::api_data("获取二维码失败"));
        }
        let uuid = capture(&text, "uuid=", "\"", &[]).ok_or_else(|| Error::api_data("获取 uuid 失败"))?;
        let spec = HttpSpec::new(Method::Get, format!("https://open.weixin.qq.com/connect/qrcode/{uuid}"))
            .header("Referer", "https://open.weixin.qq.com/connect/qrconnect");
        let image: RawPayload = self.client.http(spec).send().await?;
        Ok(QrCode {
            data: image.content,
            qr_type: QrLoginType::Wx,
            mimetype: "image/jpeg".into(),
            identifier: uuid.to_string(),
        })
    }

    async fn get_mobile_qr(&self) -> Result<QrCode> {
        let platform = self.client.platform();
        let (ct, cv) = {
            let policy = self.client.version_policy();
            let profile = policy.profile(platform);
            (profile.ct, profile.cv)
        };
        let mut request = self
            .cgi::<Value>("music.login.LoginServer", "CreateQRCode", json!({"tmeAppID": "qqmusic", "ct": ct, "cv": cv}))
            .comm("ct", "23")
            .comm("cv", "0");
        if platform == Platform::Web {
            request = request.platform(Platform::Android);
        }
        let data = request.send().await?;
        let text = |key: &str| match data.get(key) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let (qrcode, qrcode_id) = (text("qrcode"), text("qrcodeID"));
        if qrcode.is_empty() || qrcode_id.is_empty() {
            return Err(Error::api_data("获取二维码失败"));
        }
        let encoded = qrcode.rsplit(',').next().unwrap_or_default();
        let image = base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .map_err(|err| Error::api_data(format!("二维码解码失败: {err}")))?;
        Ok(QrCode { data: image, qr_type: QrLoginType::Mobile, mimetype: "image/png".into(), identifier: qrcode_id })
    }

    async fn check_qq_qr(&self, qrcode: &QrCode) -> Result<QrLoginResult> {
        let qrsig = &qrcode.identifier;
        let spec = HttpSpec::new(Method::Get, "https://ssl.ptlogin2.qq.com/ptqrlogin")
            .query([
                ("u1", "https://graph.qq.com/oauth2.0/login_jump".to_string()),
                ("ptqrtoken", hash33(qrsig, 0).to_string()),
                ("ptredirect", "0".into()),
                ("h", "1".into()),
                ("t", "1".into()),
                ("g", "1".into()),
                ("from_ui", "1".into()),
                ("ptlang", "2052".into()),
                ("action", format!("0-0-{}", now_millis())),
                ("js_ver", "20102616".into()),
                ("js_type", "1".into()),
                ("pt_uistyle", "40".into()),
                ("aid", QQ_APPID.into()),
                ("daid", "383".into()),
                ("pt_3rd_aid", QQ_3RD_AID.into()),
                ("has_onekey", "1".into()),
            ])
            .header("Referer", PTLOGIN_REFERER)
            .cookie("qrsig", qrsig.clone());
        let payload: RawPayload = match self.client.http(spec).send().await {
            Ok(payload) => payload,
            Err(Error::Http { .. }) => return Err(Error::api_data("无效 qrsig")),
            Err(err) => return Err(err),
        };
        let text = payload.text();
        let args = parse_ptui_cb(&text).ok_or_else(|| Error::api_data("获取二维码状态失败: 无法解析响应"))?;
        let code = args.first().ok_or_else(|| Error::api_data("获取二维码状态失败: 无法解析状态参数"))?;
        if code.is_empty() || !code.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::api_data("获取二维码状态失败: 无效的状态码"));
        }
        let event = event_from_code(code)?;
        if event != QrCodeLoginEvent::Done {
            return Ok(QrLoginResult::event(event));
        }
        let url = args.get(2).ok_or_else(|| Error::api_data("获取登录凭据失败: 缺少必要参数"))?;
        let sigx = capture(url, "ptsigx=", "&s_url", &['?', '&']);
        let uin = capture(url, "uin=", "&service", &['?', '&']);
        let (Some(sigx), Some(uin)) = (sigx, uin) else {
            return Err(Error::api_data("获取登录凭据失败: 无法解析必要参数"));
        };
        Ok(QrLoginResult::done(self.authorize_qq_qr(uin, sigx).await?))
    }

    async fn check_wx_qr(&self, qrcode: &QrCode) -> Result<QrLoginResult> {
        let spec = HttpSpec::new(Method::Get, "https://lp.open.weixin.qq.com/connect/l/qrconnect")
            .query([("uuid", qrcode.identifier.clone()), ("_", (now_secs() * 1000).to_string())])
            .header("Referer", "https://open.weixin.qq.com/");
        let result = self
            .client
            .http::<RawPayload>(spec)
            .credential(Credential::default())
            .timeout(Duration::from_secs(35))
            .send()
            .await;
        let payload = match result {
            Ok(payload) => payload,
            Err(Error::Network(err)) if err.timeout => return Ok(QrLoginResult::event(QrCodeLoginEvent::Scan)),
            Err(err) => return Err(err),
        };
        let text = payload.text();
        let (errcode, wx_code) =
            parse_wx_status(&text).ok_or_else(|| Error::api_data("获取二维码状态失败: 无法解析响应"))?;
        let event = event_from_code(errcode)?;
        if event != QrCodeLoginEvent::Done {
            return Ok(QrLoginResult::event(event));
        }
        if wx_code.is_empty() {
            return Err(Error::api_data("获取 code 失败: 无效的 code"));
        }
        Ok(QrLoginResult::done(self.authorize_wx_qr(wx_code).await?))
    }

    async fn authorize_qq_qr(&self, uin: &str, sigx: &str) -> Result<Credential> {
        let spec = HttpSpec::new(Method::Get, "https://ssl.ptlogin2.graph.qq.com/check_sig")
            .query([
                ("uin", uin),
                ("pttype", "1"),
                ("service", "ptqrlogin"),
                ("nodirect", "0"),
                ("ptsigx", sigx),
                ("s_url", "https://graph.qq.com/oauth2.0/login_jump"),
                ("ptlang", "2052"),
                ("ptredirect", "100"),
                ("aid", QQ_APPID),
                ("daid", "383"),
                ("j_later", "0"),
                ("low_login_hour", "0"),
                ("regmaster", "0"),
                ("pt_login_type", "3"),
                ("pt_aid", "0"),
                ("pt_aaid", "16"),
                ("pt_light", "0"),
                ("pt_3rd_aid", QQ_3RD_AID),
            ])
            .header("Referer", PTLOGIN_REFERER)
            .no_redirects();
        let payload: RawPayload = self.client.http(spec).send().await?;
        let p_skey = payload.cookies.get("p_skey").cloned().unwrap_or_default();
        if p_skey.is_empty() {
            return Err(Error::api_data("获取 p_skey 失败"));
        }
        let form = [
            ("response_type", "code".to_string()),
            ("client_id", QQ_3RD_AID.into()),
            ("redirect_uri", "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/".into()),
            ("scope", "get_user_info,get_app_friends".into()),
            ("state", "state".into()),
            ("switch", String::new()),
            ("from_ptlogin", "1".into()),
            ("src", "1".into()),
            ("update_auth", "1".into()),
            ("openapi", "1010_1030".into()),
            ("g_tk", hash33(&p_skey, 5381).to_string()),
            ("auth_time", (now_secs() * 1000).to_string()),
            ("ui", uuid4()),
        ];
        let mut spec = HttpSpec::new(Method::Post, "https://graph.qq.com/oauth2.0/authorize")
            .body(Body::Form(form.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))
            .no_redirects();
        for (name, value) in &payload.cookies {
            spec = spec.cookie(name.clone(), value.clone());
        }
        let authorize: RawPayload = self.client.http(spec).send().await?;
        let location = authorize
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("location"))
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        let code = capture(location, "code=", "&", &[]).ok_or_else(|| Error::api_data("获取 code 失败"))?;
        let data = self
            .login_cgi("QQConnectLogin.LoginServer", "QQLogin", json!({"code": code}))
            .comm("tmeLoginType", "2")
            .send()
            .await?;
        credential_from(data)
    }

    async fn authorize_wx_qr(&self, code: &str) -> Result<Credential> {
        let data = self
            .login_cgi("music.login.LoginServer", "Login", json!({"code": code, "strAppid": WX_APPID}))
            .comm("tmeLoginType", "1")
            .send()
            .await?;
        credential_from(data)
    }

    async fn handle_mobile_message(
        &self,
        qrcode_id: &str,
        event_type: Option<&str>,
        payload: Option<Value>,
    ) -> Result<Option<QrLoginResult>> {
        let event = match event_type {
            Some("scanned") => QrCodeLoginEvent::Conf,
            Some("canceled") => QrCodeLoginEvent::Refuse,
            Some("timeout") => QrCodeLoginEvent::Timeout,
            Some("loginFailed") => return Err(login_error("登录失败", payload.unwrap_or(Value::Null))),
            Some("cookies") => {
                let Some(Value::Object(payload)) = payload else {
                    return Err(Error::api_data("无效的 MQTT 消息格式"));
                };
                let cookie = |name: &str| -> Option<String> {
                    match payload.get("cookies")?.get(name)?.get("value")? {
                        Value::String(s) if !s.is_empty() => Some(s.clone()),
                        Value::Number(n) => Some(n.to_string()),
                        _ => None,
                    }
                };
                let (Some(uin), Some(key)) = (cookie("qqmusic_uin"), cookie("qqmusic_key")) else {
                    return Err(Error::api_data("获取登录凭据失败: 缺少必要参数"));
                };
                let musicid: i64 = uin.parse().map_err(|_| Error::api_data(format!("无效的 qqmusic_uin: {uin}")))?;
                let data = self
                    .login_cgi(
                        "music.login.LoginServer",
                        "Login",
                        json!({"musicid": musicid, "qrCodeID": qrcode_id, "token": key}),
                    )
                    .comm("tmeLoginType", "6")
                    .send()
                    .await?;
                return Ok(Some(QrLoginResult::done(credential_from(data)?)));
            }
            _ => return Ok(None),
        };
        Ok(Some(QrLoginResult::event(event)))
    }
}

fn event_from_code(code: &str) -> Result<QrCodeLoginEvent> {
    code.parse::<i64>()
        .ok()
        .and_then(QrCodeLoginEvent::from_code)
        .ok_or_else(|| Error::api_data(format!("无法识别的二维码登录状态码: {code}")))
}

async fn before<F: std::future::Future>(deadline: Option<Instant>, future: F) -> Option<F::Output> {
    match deadline {
        Some(deadline) => timeout_at(deadline, future).await.ok(),
        None => Some(future.await),
    }
}

/// Sleep `delay` unless the deadline comes first; `false` when it did.
async fn sleep_before_deadline(deadline: Instant, delay: Duration) -> bool {
    let now = Instant::now();
    if deadline <= now {
        return false;
    }
    if now + delay > deadline {
        sleep_until(deadline).await;
        return false;
    }
    sleep(delay).await;
    true
}

struct MobilePoll {
    api: LoginApi,
    qrcode_id: String,
    deadline: Option<Instant>,
    session: Option<MqttSession>,
    started: bool,
    finished: bool,
}

impl MobilePoll {
    async fn finish(mut self, item: Result<QrLoginResult>) -> Option<(Result<QrLoginResult>, Self)> {
        self.finished = true;
        if let Some(session) = self.session.take() {
            session.disconnect().await;
        }
        Some((item, self))
    }

    async fn open(api: &LoginApi, id: &str) -> std::result::Result<MqttSession, MqttError> {
        let connector = api.client.inner.mqtt_connector.clone().ok_or_else(|| {
            MqttError::Transport("no MQTT connector (enable the `mobile-login` feature or set one)".into())
        })?;
        let target = WsTarget {
            host: "mu.y.qq.com".into(),
            port: 443,
            path: "/ws/handshake".into(),
            headers: vec![
                ("Origin".into(), "https://y.qq.com".into()),
                ("Referer".into(), "https://y.qq.com/".into()),
                ("User-Agent".into(), MOBILE_UA.into()),
            ],
        };
        let mut options = ConnectOptions::new(format!("{}{}", now_millis(), rand::random_range(1000..10_000)));
        options.properties = Properties {
            auth_method: Some("pass".into()),
            ..Properties::with_user([
                ("tmeAppID", "qqmusic"),
                ("business", "management"),
                ("hashTag", id),
                ("clientTag", "management.user"),
                ("userID", id),
            ])
        };
        let mut session = MqttSession::connect(connector.as_ref(), target, &options).await?;
        let topic = format!("management.qrcode_login/{id}");
        session
            .subscribe(&topic, &Properties::with_user([("authorization", "tmelogin"), ("pubsub", "unicast")]))
            .await?;
        Ok(session)
    }

    async fn next(mut self) -> Option<(Result<QrLoginResult>, Self)> {
        if self.finished {
            return None;
        }
        let timeout_event = || Ok(QrLoginResult::event(QrCodeLoginEvent::Timeout));
        if !self.started {
            self.started = true;
            if self.deadline.is_some_and(|d| d <= Instant::now()) {
                return self.finish(timeout_event()).await;
            }
            return match before(self.deadline, Self::open(&self.api, &self.qrcode_id)).await {
                None | Some(Err(MqttError::Timeout(_))) => self.finish(timeout_event()).await,
                Some(Err(err)) => self.finish(Err(err.into())).await,
                Some(Ok(session)) => {
                    self.session = Some(session);
                    Some((Ok(QrLoginResult::event(QrCodeLoginEvent::Scan)), self))
                }
            };
        }
        loop {
            let session = self.session.as_mut()?;
            let message = match before(self.deadline, session.next_message()).await {
                None => return self.finish(timeout_event()).await,
                Some(Err(err)) => return self.finish(Err(err.into())).await,
                Some(Ok(None)) => {
                    self.finished = true;
                    return None;
                }
                Some(Ok(Some(message))) => message,
            };
            let event_type = message.properties.get("type").cloned();
            let payload = message.json();
            let handled =
                before(self.deadline, self.api.handle_mobile_message(&self.qrcode_id, event_type.as_deref(), payload))
                    .await;
            match handled {
                None => return self.finish(timeout_event()).await,
                Some(Err(err)) => return self.finish(Err(err)).await,
                Some(Ok(None)) => {}
                Some(Ok(Some(item))) if item.event.is_terminal() => return self.finish(Ok(item)).await,
                Some(Ok(Some(item))) => return Some((Ok(item), self)),
            }
        }
    }
}

struct WebPoll {
    api: LoginApi,
    qrcode: QrCode,
    deadline: Instant,
    interval: PollInterval,
    retries: u32,
    pending_sleep: Option<Duration>,
    finished: bool,
}

impl WebPoll {
    fn finish(mut self, item: Result<QrLoginResult>) -> Option<(Result<QrLoginResult>, Self)> {
        self.finished = true;
        Some((item, self))
    }

    async fn next(mut self) -> Option<(Result<QrLoginResult>, Self)> {
        const MIN_SAFE_INTERVAL: Duration = Duration::from_secs(1);
        let timeout_event = || Ok(QrLoginResult::event(QrCodeLoginEvent::Timeout));
        if self.finished {
            return None;
        }
        if let Some(delay) = self.pending_sleep.take()
            && !sleep_before_deadline(self.deadline, delay).await
        {
            return self.finish(timeout_event());
        }
        loop {
            let loop_start = Instant::now();
            if self.deadline <= loop_start {
                return self.finish(timeout_event());
            }
            match timeout_at(self.deadline, self.api.check_qrcode(&self.qrcode)).await {
                Err(_) => return self.finish(timeout_event()),
                Ok(Err(Error::Network(_))) => {
                    let factor = 2u32.saturating_pow(self.retries);
                    let backoff = self.interval.error_interval().min(self.interval.default.saturating_mul(factor));
                    if !sleep_before_deadline(self.deadline, backoff).await {
                        return self.finish(timeout_event());
                    }
                    self.retries += 1;
                }
                Ok(Err(err)) => return self.finish(Err(err)),
                Ok(Ok(item)) => {
                    self.retries = 0;
                    if item.event.is_terminal() {
                        return self.finish(Ok(item));
                    }
                    let sleep_time = match item.event {
                        QrCodeLoginEvent::Conf => self.interval.scanned_interval(),
                        QrCodeLoginEvent::Scan if self.qrcode.qr_type == QrLoginType::Wx => Duration::from_millis(500),
                        _ => self.interval.default,
                    };
                    let min_safe = MIN_SAFE_INTERVAL.saturating_sub(loop_start.elapsed());
                    self.pending_sleep = Some(sleep_time.max(min_safe));
                    return Some((Ok(item), self));
                }
            }
        }
    }
}

/// QR login flow: fetch the code, then poll / listen until a terminal event.
#[derive(Debug, Clone)]
pub struct QrCodeLoginSession {
    api: LoginApi,
    login_type: QrLoginType,
    interval: PollInterval,
    timeout: Duration,
    emit_repeat: bool,
    qrcode: Option<QrCode>,
}

impl QrCodeLoginSession {
    /// Poll interval (QQ / WeChat).
    #[must_use]
    pub fn interval(mut self, interval: impl Into<PollInterval>) -> Self {
        self.interval = interval.into();
        self
    }

    /// Overall timeout (default 180 s).
    #[must_use]
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Emit repeated identical events (default: only changes).
    #[must_use]
    pub fn emit_repeat(mut self, emit: bool) -> Self {
        self.emit_repeat = emit;
        self
    }

    /// Reuse an already fetched QR code.
    #[must_use]
    pub fn with_qrcode(mut self, qrcode: QrCode) -> Self {
        self.qrcode = Some(qrcode);
        self
    }

    /// Fetch (once) and return the QR code.
    pub async fn get_qrcode(&mut self) -> Result<QrCode> {
        if let Some(qrcode) = &self.qrcode {
            return Ok(qrcode.clone());
        }
        let qrcode = self.api.get_qrcode(self.login_type).await?;
        self.qrcode = Some(qrcode.clone());
        Ok(qrcode)
    }

    /// Event stream (fetches the QR code first if needed).
    pub async fn events(&mut self) -> Result<BoxStream<'static, Result<QrLoginResult>>> {
        if self.timeout.is_zero() {
            return Err(Error::invalid_argument("timeout 必须大于 0"));
        }
        let qrcode = self.get_qrcode().await?;
        let deadline = Instant::now() + self.timeout;
        let events = if qrcode.qr_type == QrLoginType::Mobile {
            self.api.mobile_qrcode_events(&qrcode, Some(deadline))
        } else {
            let state = WebPoll {
                api: self.api.clone(),
                qrcode,
                deadline,
                interval: self.interval,
                retries: 0,
                pending_sleep: None,
                finished: false,
            };
            stream::unfold(state, WebPoll::next).boxed()
        };
        if self.emit_repeat {
            return Ok(events);
        }
        Ok(events
            .scan(None, |last: &mut Option<QrCodeLoginEvent>, item| {
                let out = match &item {
                    Ok(result) if *last == Some(result.event) => None,
                    Ok(result) => {
                        *last = Some(result.event);
                        Some(item)
                    }
                    Err(_) => Some(item),
                };
                futures::future::ready(Some(out))
            })
            .filter_map(futures::future::ready)
            .boxed())
    }

    /// Wait for the login to finish and return the credential.
    pub async fn wait(&mut self) -> Result<Credential> {
        let mut events = self.events().await?;
        while let Some(item) = events.next().await {
            let item = item?;
            match item.event {
                QrCodeLoginEvent::Done => {
                    return item.credential.ok_or_else(|| login_error("登录结果缺少凭证", Value::Null));
                }
                QrCodeLoginEvent::Refuse => return Err(login_error("用户拒绝了登录请求", Value::Null)),
                QrCodeLoginEvent::Timeout => return Err(login_error("登录二维码已超时", Value::Null)),
                _ => {}
            }
        }
        Err(login_error("登录流程异常结束", Value::Null))
    }
}

/// SMS login flow.
#[derive(Debug, Clone)]
pub struct PhoneLoginSession {
    api: LoginApi,
    phone: PhoneNumber,
    country_code: u32,
    /// Result of the last [`PhoneLoginSession::send_authcode`].
    pub last_result: Option<PhoneAuthCodeResult>,
}

impl PhoneLoginSession {
    /// Send the verification code.
    pub async fn send_authcode(&mut self) -> Result<PhoneAuthCodeResult> {
        let result = self.api.send_authcode(self.phone.clone(), self.country_code).await?;
        self.last_result = Some(result.clone());
        Ok(result)
    }

    /// Log in with the received code.
    pub async fn authorize(&self, auth_code: &str) -> Result<Credential> {
        self.api.phone_authorize(self.phone.clone(), auth_code).await
    }
}

#[cfg(test)]
mod tests;
