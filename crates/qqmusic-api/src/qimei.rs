//! QIMEI device fingerprint (Tencent beacon id) acquisition.
//!
//! Android requests carry a `QIMEI36` value registered with Tencent's
//! fingerprint service. A missing or random QIMEI is a strong risk-control
//! signal, so it is fetched once per device and cached for a day.

use std::sync::Arc;
use std::time::Duration;

use aes::Aes128;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use cbc::cipher::{BlockModeEncrypt, KeyIvInit, block_padding::Pkcs7};
use rand::{Rng, RngExt};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::algorithms::rsa::RsaPublicKey;
use crate::device::{Device, DeviceStore, Qimei};
use crate::error::{Error, Result};
use crate::transport::{Body, Method, Request, Transport};
use crate::utils::{calc_md5, lock_sync, now_secs, utc_datetime};
use crate::versioning::VersionProfile;

/// QIMEI service endpoint.
pub const QIMEI_URL: &str = "https://api.tencentmusic.com/tme/trpc/proxy";

/// Base64 DER (SubjectPublicKeyInfo) of the QIMEI RSA public key.
const PUBLIC_KEY_DER_B64: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDEIxgwoutfwoJxcGQeedgP7FG9qaIuS0qzfR8gWkrkTZKM2iWHn2ajQpBRZjMSoSf6+KJGvar2ORhBfpDXyVtZCKpqLQ+FLkpncClKVIrBwv6PHyUvuCb0rIarmgDnzkfQAqVufEtR64iazGDKatvJ9y6B9NMbHddGSAUmRTCrHQIDAQAB";
const SECRET: &str = "ZdJqM15EeO2zWc08";
const APP_KEY: &str = "0AND0HD6FE4HY80F";
const EXTRA: &str = r#"{"appKey":"0AND0HD6FE4HY80F"}"#;
const CHANNEL_ID: &str = "10003505";
const PACKAGE_ID: &str = "com.tencent.qqmusic";
const HEX_CHARS: &[u8] = b"0123456789abcdef";
const DEVICE_TOKEN_KEY: &[u8; 16] = b"lvcwmSYVr2Axv1gn";
const DEVICE_TOKEN_IV: &[u8; 16] = b"Zs0ntDqG2jyhKN0c";
const QIMEI_SIGN_KEY: &str = "qimei_qq_androidpzAuCmaFAaFaHrdakPjLIEqKrGnSOOvH";

/// Cache lifetime of a QIMEI.
pub const QIMEI_TTL_SECS: i64 = 86_400;
/// Back-off after a failed QIMEI request.
pub const QIMEI_FAILURE_BACKOFF: Duration = Duration::from_secs(300);

/// AES-128-CBC with PKCS#7 padding (`iv` defaults to the key).
pub fn aes_encrypt(key: &[u8; 16], content: &[u8], iv: Option<&[u8; 16]>) -> Vec<u8> {
    let iv = iv.unwrap_or(key);
    cbc::Encryptor::<Aes128>::new(key.into(), iv.into()).encrypt_padded_vec::<Pkcs7>(content)
}

/// RSA PKCS#1 v1.5 encryption with the QIMEI public key.
pub fn rsa_encrypt(content: &[u8]) -> Result<Vec<u8>> {
    static KEY: std::sync::OnceLock<std::result::Result<RsaPublicKey, String>> = std::sync::OnceLock::new();
    let key = KEY.get_or_init(|| {
        let der = STANDARD.decode(PUBLIC_KEY_DER_B64).map_err(|e| e.to_string())?;
        RsaPublicKey::from_spki_der(&der).map_err(|e| e.to_string())
    });
    let key = key.as_ref().map_err(|e| Error::invalid_argument(format!("invalid QIMEI public key: {e}")))?;
    key.encrypt_pkcs1v15(content).map_err(|e| Error::invalid_argument(format!("RSA encryption failed: {e}")))
}

/// Device token `oz` (encrypted Android id).
pub fn calc_device_oz(android_id: &str) -> String {
    STANDARD.encode(aes_encrypt(DEVICE_TOKEN_KEY, android_id.as_bytes(), Some(DEVICE_TOKEN_IV)))
}

/// Device token `oo` (encrypted model).
pub fn calc_device_oo(model: &str) -> String {
    STANDARD.encode(aes_encrypt(DEVICE_TOKEN_KEY, model.as_bytes(), Some(DEVICE_TOKEN_IV)))
}

/// Stable private LAN IP derived from the Android id.
pub fn private_ip_for_device(android_id: &str) -> String {
    let digest = hex::decode(calc_md5([android_id])).unwrap_or_else(|_| vec![0, 0]);
    format!("192.168.{}.{}", digest[0], digest[1] % 253 + 2)
}

fn random_hex(rng: &mut impl Rng, alphabet: &[u8], len: usize) -> String {
    (0..len).map(|_| char::from(alphabet[rng.random_range(0..alphabet.len())])).collect()
}

/// Random beacon id source string.
pub fn random_beacon_id() -> String {
    let mut rng = rand::rng();
    let (year, month, ..) = utc_datetime(now_secs());
    let time_month = format!("{year:04}-{month:02}-01");
    let rand1: u32 = rng.random_range(100_000..=999_999);
    let rand2: u32 = rng.random_range(100_000_000..=999_999_999);
    let mut beacon = String::new();
    for i in 1..=40 {
        match i {
            1 | 2 | 13 | 14 | 17 | 18 | 21 | 22 | 25 | 26 | 29 | 30 | 33 | 34 | 37 | 38 => {
                beacon.push_str(&format!("k{i}:{time_month}{rand1}.{rand2}"));
            }
            3 => beacon.push_str("k3:0000000000000000"),
            4 => beacon.push_str(&format!("k4:{}", random_hex(&mut rng, &HEX_CHARS[1..], 16))),
            _ => beacon.push_str(&format!("k{i}:{}", rng.random_range(0..=9999))),
        }
        beacon.push(';');
    }
    beacon
}

/// Device payload sent (encrypted) to the QIMEI service.
pub fn device_payload(device: &Device, version: &str, sdk_version: &str) -> Value {
    let fixed_rand: i64 = rand::rng().random_range(0..=14_400);
    let (y, mo, d, h, mi, s) = utc_datetime(now_secs() - fixed_rand);
    let harmony = if device.vendor_os_name.to_lowercase().starts_with("harmonyos") { "1" } else { "0" };
    let manufacturer = if device.manufacturer.is_empty() { &device.brand } else { &device.manufacturer };
    let reserved = json!({
        "harmony": harmony,
        "clone": "0",
        "containe": "",
        "oz": calc_device_oz(&device.android_id),
        "oo": calc_device_oo(&device.model),
        "kelong": "0",
        "ip": private_ip_for_device(&device.android_id),
        "uptimes": format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}"),
        "multiUser": "0",
        "bod": device.board,
        "brd": device.brand,
        "dv": device.device,
        "firstLevel": device.first_api_level.to_string(),
        "manufact": manufacturer,
        "name": device.product,
        "host": device.host,
        "kernel": device.proc_version,
        "pre": "0",
        "av": version,
        "ch": "",
    });
    json!({
        "androidId": device.android_id,
        "platformId": 1,
        "appKey": APP_KEY,
        "appVersion": version,
        "beaconIdSrc": random_beacon_id(),
        "brand": device.brand,
        "channelId": CHANNEL_ID,
        "cid": "",
        "imei": device.imei,
        "imsi": "",
        "mac": "",
        "model": device.model,
        "networkType": "wifi",
        "oaid": "",
        "osVersion": format!("Android {},level {}", device.version.release, device.version.sdk),
        "qimei": "",
        "qimei36": "",
        "sdkVersion": sdk_version,
        "targetSdkVersion": "30",
        "audit": "",
        "userId": "{}",
        "packageId": PACKAGE_ID,
        "deviceType": "Phone",
        "sdkName": "",
        "reserved": reserved.to_string(),
    })
}

/// Build the QIMEI HTTP request.
pub fn build_qimei_request(device: &Device, version: &str, sdk_version: &str) -> Result<Request> {
    let payload = device_payload(device, version, sdk_version);
    let (crypt_key, nonce) = {
        let mut rng = rand::rng();
        (random_hex(&mut rng, HEX_CHARS, 16), random_hex(&mut rng, HEX_CHARS, 16))
    };
    let ts = now_secs();
    let key = STANDARD.encode(rsa_encrypt(crypt_key.as_bytes())?);
    let crypt_key_bytes: [u8; 16] =
        crypt_key.as_bytes().try_into().map_err(|_| Error::invalid_argument("invalid crypt key"))?;
    let params = STANDARD.encode(aes_encrypt(&crypt_key_bytes, payload.to_string().as_bytes(), None));
    let millis = (ts * 1000).to_string();
    let req_sign = calc_md5([key.as_str(), params.as_str(), millis.as_str(), nonce.as_str(), SECRET, EXTRA]);
    let body = json!({
        "app": 0,
        "os": 1,
        "qimeiParams": {
            "key": key,
            "params": params,
            "time": ts.to_string(),
            "nonce": nonce,
            "sign": req_sign,
            "extra": EXTRA,
        }
    });
    let mut request = Request::new(Method::Post, QIMEI_URL);
    request.headers = vec![
        ("Host".into(), "api.tencentmusic.com".into()),
        ("method".into(), "GetQimei".into()),
        ("service".into(), "trpc.tme_datasvr.qimeiproxy.QimeiProxy".into()),
        ("appid".into(), "qimei_qq_android".into()),
        ("sign".into(), calc_md5([QIMEI_SIGN_KEY, ts.to_string().as_str()])),
        ("user-agent".into(), "QQMusic".into()),
        ("timestamp".into(), ts.to_string()),
    ];
    request.body = Body::json(&body);
    Ok(request)
}

/// Parse the QIMEI service response body.
pub fn parse_qimei_response(body: &[u8]) -> Result<Qimei> {
    let outer: Value = serde_json::from_slice(body).map_err(|_| Error::api_data("QIMEI response is not valid JSON"))?;
    let inner = match outer.get("data") {
        Some(Value::String(text)) => serde_json::from_str::<Value>(text)
            .map_err(|_| Error::api_data_with("QIMEI response data is not valid JSON", outer.clone()))?,
        Some(other) => other.clone(),
        None => Value::Null,
    };
    let data = inner.get("data").cloned().unwrap_or(Value::Null);
    match (data.get("q16").and_then(Value::as_str), data.get("q36").and_then(Value::as_str)) {
        (Some(q16), Some(q36)) => Ok(Qimei { q16: q16.to_string(), q36: q36.to_string() }),
        _ => Err(Error::api_data_with("QIMEI response missing required fields", outer)),
    }
}

/// Lazily fetches and caches the QIMEI of the current device.
pub struct QimeiProvider {
    devices: Arc<DeviceStore>,
    profile: VersionProfile,
    transport: Arc<dyn Transport>,
    /// Hot path: checked on every Android request without awaiting.
    state: std::sync::Mutex<State>,
    /// Single-flight guard for the disk / network lookup.
    fetch: Mutex<()>,
}

#[derive(Default)]
struct State {
    cached: Option<(Qimei, i64)>,
    last_failure: Option<Instant>,
}

impl State {
    fn fresh(&self, now: i64) -> Option<Qimei> {
        self.cached.as_ref().filter(|(_, saved_at)| now - saved_at < QIMEI_TTL_SECS).map(|(qimei, _)| qimei.clone())
    }

    /// `Some(cached)` while the failure backoff is active.
    fn backoff(&self) -> Option<Option<Qimei>> {
        self.last_failure
            .filter(|failed| failed.elapsed() < QIMEI_FAILURE_BACKOFF)
            .map(|_| self.cached.as_ref().map(|(qimei, _)| qimei.clone()))
    }
}

impl std::fmt::Debug for QimeiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QimeiProvider").finish_non_exhaustive()
    }
}

impl QimeiProvider {
    /// Create a provider.
    pub fn new(devices: Arc<DeviceStore>, profile: VersionProfile, transport: Arc<dyn Transport>) -> Self {
        Self { devices, profile, transport, state: std::sync::Mutex::default(), fetch: Mutex::new(()) }
    }

    /// Get the cached QIMEI or request a new one.
    pub async fn get(&self) -> Result<Qimei> {
        if let Some(qimei) = lock_sync(&self.state).fresh(now_secs()) {
            return Ok(qimei);
        }
        let _fetch = self.fetch.lock().await;
        self.fetch_locked().await
    }

    /// Like [`QimeiProvider::get`] but never fails: errors are logged and
    /// further attempts are suppressed for [`QIMEI_FAILURE_BACKOFF`].
    pub async fn get_lenient(&self) -> Option<Qimei> {
        {
            let state = lock_sync(&self.state);
            if let Some(qimei) = state.fresh(now_secs()) {
                return Some(qimei);
            }
            if let Some(cached) = state.backoff() {
                return cached;
            }
        }
        let _fetch = self.fetch.lock().await;
        // A concurrent caller may have failed while we were waiting.
        if let Some(cached) = lock_sync(&self.state).backoff() {
            return cached;
        }
        match self.fetch_locked().await {
            Ok(qimei) => Some(qimei),
            Err(err) => {
                tracing::warn!(error = %err, "failed to obtain QIMEI, continuing without it");
                let mut state = lock_sync(&self.state);
                state.last_failure = Some(Instant::now());
                state.cached.as_ref().map(|(qimei, _)| qimei.clone())
            }
        }
    }

    /// Disk cache, then network. Caller holds `self.fetch`.
    async fn fetch_locked(&self) -> Result<Qimei> {
        let now = now_secs();
        if let Some(qimei) = lock_sync(&self.state).fresh(now) {
            return Ok(qimei);
        }
        if let Some((qimei, saved_at)) = self.devices.cache().qimei().await
            && now - saved_at < QIMEI_TTL_SECS
            && !qimei.q16.is_empty()
            && !qimei.q36.is_empty()
        {
            lock_sync(&self.state).cached = Some((qimei.clone(), saved_at));
            return Ok(qimei);
        }
        let device = self.devices.get().await?;
        let request = build_qimei_request(&device, &self.profile.qimei_app_version, &self.profile.qimei_sdk_version)?;
        let response = self.transport.send(request).await?;
        if response.status != 200 {
            return Err(Error::Http {
                status: response.status,
                message: format!("HTTP 请求状态码异常: {}", response.status),
            });
        }
        let qimei = parse_qimei_response(&response.body)?;
        self.devices.cache().set_qimei(&qimei, now).await;
        let mut state = lock_sync(&self.state);
        state.cached = Some((qimei.clone(), now));
        state.last_failure = None;
        Ok(qimei)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceProfile;
    use crate::transport::Response;
    use crate::transport::mock::MockTransport;

    #[test]
    fn matches_python_reference_values() {
        assert_eq!(calc_device_oz("abcdef0123456789"), "2rzkeEXSBv8jdLVxCFhK47RzsGZF1RUfVH0MA7xRm/I=");
        assert_eq!(calc_device_oo("V2408A"), "AB3Bkaa2vuN47x/MquvfUw==");
        assert_eq!(private_ip_for_device("abcdef0123456789"), "192.168.52.40");
        assert_eq!(
            hex::encode(aes_encrypt(b"0123456789abcdef", b"hello world", None)),
            "c1e9b4529aac9793010f4677f6358efe"
        );
    }

    #[test]
    fn rsa_key_is_valid() {
        let encrypted = rsa_encrypt(b"0123456789abcdef").unwrap();
        assert_eq!(encrypted.len(), 128);
    }

    #[test]
    fn beacon_id_layout() {
        let beacon = random_beacon_id();
        let parts: Vec<&str> = beacon.trim_end_matches(';').split(';').collect();
        assert_eq!(parts.len(), 40);
        assert_eq!(parts[2], "k3:0000000000000000");
        assert!(parts[3].starts_with("k4:") && parts[3].len() == 19);
        assert!(!parts[3][3..].contains('0'));
        assert!(parts[0].starts_with("k1:") && parts[0].contains("-01"));
    }

    #[test]
    fn request_shape() {
        let device = Device::generate(Some(DeviceProfile::Vivo), Some(3));
        let request = build_qimei_request(&device, "20.9.0.8", "1.2.13.6").unwrap();
        assert_eq!(request.url, QIMEI_URL);
        assert_eq!(request.header("method"), Some("GetQimei"));
        let body = request.json_body().unwrap();
        assert_eq!(body["qimeiParams"]["extra"], EXTRA);
        let params = &body["qimeiParams"];
        let expected_sign = calc_md5([
            params["key"].as_str().unwrap(),
            params["params"].as_str().unwrap(),
            &format!("{}000", params["time"].as_str().unwrap()),
            params["nonce"].as_str().unwrap(),
            SECRET,
            EXTRA,
        ]);
        assert_eq!(params["sign"], expected_sign.as_str());
        let payload = device_payload(&device, "v", "s");
        assert_eq!(payload["osVersion"], "Android 15,level 35");
        let reserved: Value = serde_json::from_str(payload["reserved"].as_str().unwrap()).unwrap();
        assert_eq!(reserved["harmony"], "0");
        assert_eq!(reserved["firstLevel"], "35");
    }

    #[test]
    fn response_parsing() {
        let body = json!({"data": json!({"data": {"q16": "a", "q36": "b"}}).to_string()});
        let qimei = parse_qimei_response(body.to_string().as_bytes()).unwrap();
        assert_eq!(qimei, Qimei { q16: "a".into(), q36: "b".into() });
        assert!(parse_qimei_response(b"{}").is_err());
        assert!(parse_qimei_response(b"nope").is_err());
        assert!(parse_qimei_response(br#"{"data": "{bad"}"#).is_err());
    }

    fn provider(mock: &MockTransport) -> QimeiProvider {
        let device = Device::generate(Some(DeviceProfile::Oppo), Some(1));
        QimeiProvider::new(
            Arc::new(DeviceStore::fixed(device, None)),
            VersionProfile::android(),
            Arc::new(mock.clone()),
        )
    }

    #[tokio::test]
    async fn provider_caches_results() {
        let mock = MockTransport::new();
        mock.route_url("tencentmusic", |_| {
            let inner = json!({"data": {"q16": "x16", "q36": "x36"}}).to_string();
            Ok(Response::json(&json!({"data": inner})))
        });
        let provider = provider(&mock);
        assert_eq!(provider.get().await.unwrap().q36, "x36");
        assert_eq!(provider.get().await.unwrap().q16, "x16");
        assert_eq!(mock.request_count(), 1);
    }

    fn slow_provider(mock: &MockTransport) -> QimeiProvider {
        let device = Device::generate(Some(DeviceProfile::Oppo), Some(1));
        QimeiProvider::new(
            Arc::new(DeviceStore::fixed(device, None)),
            VersionProfile::android(),
            Arc::new(crate::testing::SlowTransport(mock.clone())),
        )
    }

    #[tokio::test]
    async fn concurrent_get_is_single_flight() {
        let mock = MockTransport::new();
        mock.route_url("tencentmusic", |_| {
            let inner = json!({"data": {"q16": "x16", "q36": "x36"}}).to_string();
            Ok(Response::json(&json!({"data": inner})))
        });
        let provider = slow_provider(&mock);
        let results = futures::future::join_all((0..16).map(|_| provider.get())).await;
        assert!(results.iter().all(|r| r.as_ref().is_ok_and(|q| q.q36 == "x36")));
        assert_eq!(mock.request_count(), 1);
    }

    #[tokio::test]
    async fn concurrent_lenient_failures_share_backoff() {
        let mock = MockTransport::with_handler(|_| Ok(Response::new(500, "")));
        let provider = slow_provider(&mock);
        let results = futures::future::join_all((0..16).map(|_| provider.get_lenient())).await;
        assert!(results.iter().all(Option::is_none));
        assert_eq!(mock.request_count(), 1, "waiters must honour the backoff set by the failed fetch");
    }

    #[tokio::test]
    async fn lenient_failure_backs_off() {
        let mock = MockTransport::with_handler(|_| Ok(Response::new(500, "")));
        let provider = provider(&mock);
        assert!(provider.get().await.is_err());
        assert!(provider.get_lenient().await.is_none());
        assert!(provider.get_lenient().await.is_none());
        assert_eq!(mock.request_count(), 2);
    }
}
