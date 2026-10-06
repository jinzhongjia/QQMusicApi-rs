//! Client, builder and request executor.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use futures::future::join_all;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use tokio::sync::Semaphore;

use crate::algorithms::zzc_sign;
use crate::bypass::BypassConfig;
use crate::credential::Credential;
use crate::device::{Device, DeviceStore, Qimei, SessionRecord};
use crate::error::{Error, Result};
use crate::json::FromJson;
use crate::qimei::QimeiProvider;
use crate::ratelimit::{RateLimit, TokenBucket};
use crate::request::{Batch, CgiRequest, CgiSpec, HttpOutput, HttpRequest, HttpSpec};
use crate::response::{parse_cgi_item, unwrap_cgi_envelope};
use crate::session::AndroidSessionManager;
use crate::transport::{Body, Method, Request, Response, StreamingResponse, Transport};
use crate::utils::{bool_to_int, now_millis};
use crate::versioning::{CommContext, Platform, VersionPolicy};

/// Default `musicu.fcg` endpoint.
pub const MUSICU_URL: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
/// Default signed `musics.fcg` endpoint.
pub const MUSICS_URL: &str = "https://u.y.qq.com/cgi-bin/musics.fcg";
/// Default maximum number of in-flight HTTP requests.
pub const DEFAULT_MAX_CONCURRENCY: usize = 20;
/// Default number of module calls merged into one CGI request.
pub const DEFAULT_BATCH_SIZE: usize = 20;

/// CGI endpoints.
///
/// Can point to a reverse proxy (e.g. one located in mainland China) to
/// avoid region based restrictions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    /// Unsigned endpoint.
    pub musicu: String,
    /// Signed endpoint.
    pub musics: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self { musicu: MUSICU_URL.to_string(), musics: MUSICS_URL.to_string() }
    }
}

/// Source of the device identity.
#[derive(Debug, Clone, Default)]
pub enum DeviceSource {
    /// Random device kept in memory.
    #[default]
    Ephemeral,
    /// Device persisted in a JSON file (recommended).
    File(PathBuf),
    /// Caller supplied device.
    Fixed {
        /// Device.
        device: Box<Device>,
        /// Optional cache file for QIMEI / session.
        cache_path: Option<PathBuf>,
    },
}

/// QIMEI handling.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum QimeiMode {
    /// Request from Tencent and cache (default).
    #[default]
    Auto,
    /// Use a fixed value.
    Fixed(Qimei),
    /// Never send a QIMEI.
    Disabled,
}

/// Builder for [`Client`].
pub struct ClientBuilder {
    credential: Credential,
    platform: Platform,
    device: DeviceSource,
    policy: VersionPolicy,
    transport: Option<Arc<dyn Transport>>,
    #[cfg(feature = "reqwest-transport")]
    transport_config: crate::transport::TransportConfig,
    max_concurrency: usize,
    batch_size: usize,
    rate_limit: Option<RateLimit>,
    qimei: QimeiMode,
    android_session: bool,
    headers: Vec<(String, String)>,
    endpoints: Endpoints,
    bypass: BypassConfig,
    mqtt_connector: Option<Arc<dyn crate::mqtt::MqttConnector>>,
}

impl std::fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientBuilder")
            .field("platform", &self.platform)
            .field("device", &self.device)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            credential: Credential::default(),
            platform: Platform::Android,
            device: DeviceSource::Ephemeral,
            policy: VersionPolicy::default(),
            transport: None,
            #[cfg(feature = "reqwest-transport")]
            transport_config: crate::transport::TransportConfig::default(),
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            batch_size: DEFAULT_BATCH_SIZE,
            rate_limit: Some(RateLimit::default()),
            qimei: QimeiMode::Auto,
            android_session: true,
            headers: Vec::new(),
            endpoints: Endpoints::default(),
            bypass: BypassConfig::default(),
            mqtt_connector: None,
        }
    }
}

impl ClientBuilder {
    /// Default login credential.
    #[must_use]
    pub fn credential(mut self, credential: Credential) -> Self {
        self.credential = credential;
        self
    }

    /// Default platform.
    #[must_use]
    pub fn platform(mut self, platform: Platform) -> Self {
        self.platform = platform;
        self
    }

    /// Persist the device identity in a JSON file.
    #[must_use]
    pub fn device_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.device = DeviceSource::File(path.into());
        self
    }

    /// Use a caller supplied device.
    #[must_use]
    pub fn device(mut self, device: Device) -> Self {
        self.device = DeviceSource::Fixed { device: Box::new(device), cache_path: None };
        self
    }

    /// Device source.
    #[must_use]
    pub fn device_source(mut self, source: DeviceSource) -> Self {
        self.device = source;
        self
    }

    /// Version profiles (`ct`, `cv`, UA, comm overrides).
    #[must_use]
    pub fn version_policy(mut self, policy: VersionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Modify the version policy in place.
    #[must_use]
    pub fn configure_version_policy(mut self, f: impl FnOnce(&mut VersionPolicy)) -> Self {
        f(&mut self.policy);
        self
    }

    /// Custom transport.
    #[must_use]
    pub fn transport(mut self, transport: impl Transport) -> Self {
        self.transport = Some(Arc::new(transport));
        self
    }

    /// Custom shared transport.
    #[must_use]
    pub fn transport_arc(mut self, transport: Arc<dyn Transport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Configuration of the default reqwest transport.
    #[cfg(feature = "reqwest-transport")]
    #[must_use]
    pub fn transport_config(mut self, config: crate::transport::TransportConfig) -> Self {
        self.transport_config = config;
        self
    }

    /// Proxy for the default reqwest transport.
    #[cfg(feature = "reqwest-transport")]
    #[must_use]
    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.transport_config.proxy = Some(proxy.into());
        self
    }

    /// Maximum in-flight HTTP requests.
    #[must_use]
    pub fn max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    /// Default number of module calls per CGI request.
    #[must_use]
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    /// Rate limit (`None` disables it).
    #[must_use]
    pub fn rate_limit(mut self, limit: Option<RateLimit>) -> Self {
        self.rate_limit = limit;
        self
    }

    /// QIMEI handling.
    #[must_use]
    pub fn qimei(mut self, mode: QimeiMode) -> Self {
        self.qimei = mode;
        self
    }

    /// Enable/disable the Android `uid`/`sid` session.
    #[must_use]
    pub fn android_session(mut self, enabled: bool) -> Self {
        self.android_session = enabled;
        self
    }

    /// Header added to every request unless the request sets it.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// CGI endpoints.
    #[must_use]
    pub fn endpoints(mut self, endpoints: Endpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    /// Playback URL bypass configuration.
    #[must_use]
    pub fn bypass(mut self, bypass: BypassConfig) -> Self {
        self.bypass = bypass;
        self
    }

    /// MQTT connector used by the QQ Music App QR login (defaults to plain
    /// `wss://` when the `mobile-login` feature is enabled).
    #[must_use]
    pub fn mqtt_connector(mut self, connector: Arc<dyn crate::mqtt::MqttConnector>) -> Self {
        self.mqtt_connector = Some(connector);
        self
    }

    /// Build the client.
    pub fn build(self) -> Result<Client> {
        let transport: Arc<dyn Transport> = match self.transport {
            Some(transport) => transport,
            #[cfg(feature = "reqwest-transport")]
            None => Arc::new(crate::transport::ReqwestTransport::new(&self.transport_config)?),
            #[cfg(not(feature = "reqwest-transport"))]
            None => {
                return Err(Error::invalid_argument(
                    "no transport configured (enable feature `reqwest-transport` or call `transport()`)",
                ));
            }
        };
        let devices = Arc::new(match self.device {
            DeviceSource::Ephemeral => DeviceStore::ephemeral(),
            DeviceSource::File(path) => DeviceStore::file(path),
            DeviceSource::Fixed { device, cache_path } => DeviceStore::fixed(*device, cache_path),
        });
        let qimei = match &self.qimei {
            QimeiMode::Auto => {
                Some(QimeiProvider::new(Arc::clone(&devices), self.policy.android.clone(), Arc::clone(&transport)))
            }
            QimeiMode::Fixed(_) | QimeiMode::Disabled => None,
        };
        let inner = Inner {
            credential: RwLock::new(self.credential),
            platform: RwLock::new(self.platform),
            policy: RwLock::new(Arc::new(self.policy)),
            bypass: RwLock::new(Arc::new(self.bypass)),
            devices,
            qimei_mode: self.qimei,
            qimei,
            session: self.android_session.then(AndroidSessionManager::new),
            transport,
            limiter: self.rate_limit.map(TokenBucket::new),
            semaphore: Semaphore::new(self.max_concurrency),
            headers: self.headers,
            endpoints: self.endpoints,
            batch_size: self.batch_size,
            mqtt_connector: self.mqtt_connector.or_else(default_mqtt_connector),
        };
        Ok(Client { inner: Arc::new(inner) })
    }
}

pub(crate) struct Inner {
    credential: RwLock<Credential>,
    platform: RwLock<Platform>,
    policy: RwLock<Arc<VersionPolicy>>,
    bypass: RwLock<Arc<BypassConfig>>,
    pub(crate) devices: Arc<DeviceStore>,
    qimei_mode: QimeiMode,
    qimei: Option<QimeiProvider>,
    session: Option<AndroidSessionManager>,
    pub(crate) transport: Arc<dyn Transport>,
    limiter: Option<TokenBucket>,
    semaphore: Semaphore,
    headers: Vec<(String, String)>,
    pub(crate) endpoints: Endpoints,
    batch_size: usize,
    pub(crate) mqtt_connector: Option<Arc<dyn crate::mqtt::MqttConnector>>,
}

#[cfg(feature = "mobile-login")]
fn default_mqtt_connector() -> Option<Arc<dyn crate::mqtt::MqttConnector>> {
    Some(Arc::new(crate::mqtt::ws::WsConnector))
}

#[cfg(not(feature = "mobile-login"))]
fn default_mqtt_connector() -> Option<Arc<dyn crate::mqtt::MqttConnector>> {
    None
}

fn read<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

fn write<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(std::sync::PoisonError::into_inner) = value;
}

impl Inner {
    pub(crate) fn policy(&self) -> Arc<VersionPolicy> {
        read(&self.policy)
    }

    /// QIMEI for `comm` (never fails).
    pub(crate) async fn cached_qimei(&self) -> Option<Qimei> {
        match &self.qimei_mode {
            QimeiMode::Fixed(qimei) => Some(qimei.clone()),
            QimeiMode::Disabled => None,
            QimeiMode::Auto => match &self.qimei {
                Some(provider) => provider.get_lenient().await,
                None => None,
            },
        }
    }

    async fn android_session(&self) -> Option<SessionRecord> {
        let manager = self.session.as_ref()?;
        match manager.ensure(self).await {
            Ok(session) => Some(session),
            Err(err) => {
                tracing::warn!(error = %err, "failed to obtain Android session, continuing without it");
                None
            }
        }
    }

    fn apply_default_headers(&self, request: &mut Request) {
        for (name, value) in &self.headers {
            if request.header(name).is_none() {
                request.headers.push((name.clone(), value.clone()));
            }
        }
    }

    /// Send a request through the concurrency and rate limiters.
    pub(crate) async fn send(&self, mut request: Request) -> Result<Response> {
        self.apply_default_headers(&mut request);
        let _permit = self.semaphore.acquire().await.map_err(|_| Error::Closed)?;
        if let Some(limiter) = &self.limiter {
            limiter.acquire().await;
        }
        tracing::debug!(method = %request.method, url = %request.url, "sending request");
        Ok(self.transport.send(request).await?)
    }

    async fn send_streaming(&self, mut request: Request) -> Result<StreamingResponse> {
        self.apply_default_headers(&mut request);
        let _permit = self.semaphore.acquire().await.map_err(|_| Error::Closed)?;
        if let Some(limiter) = &self.limiter {
            limiter.acquire().await;
        }
        Ok(self.transport.send_streaming(request).await?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GroupKey {
    platform: Platform,
    credential: Credential,
    comm: Option<String>,
    override_comm: bool,
    sign: bool,
    bypass: bool,
}

fn canonical_comm(comm: &IndexMap<String, Option<String>>) -> Option<String> {
    if comm.is_empty() {
        return None;
    }
    let sorted: BTreeMap<&String, &Option<String>> = comm.iter().collect();
    serde_json::to_string(&sorted).ok()
}

/// QQ Music API client.
///
/// Cheap to clone; clones share connections, device identity and caches.
#[derive(Clone)]
pub struct Client {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("platform", &self.platform())
            .field("musicid", &self.credential().musicid)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Builder.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Client with default settings.
    pub fn new() -> Result<Self> {
        Self::builder().build()
    }

    /// Current default credential.
    pub fn credential(&self) -> Credential {
        read(&self.inner.credential)
    }

    /// Replace the default credential.
    pub fn set_credential(&self, credential: Credential) {
        write(&self.inner.credential, credential);
    }

    /// Current default platform.
    pub fn platform(&self) -> Platform {
        read(&self.inner.platform)
    }

    /// Replace the default platform.
    pub fn set_platform(&self, platform: Platform) {
        write(&self.inner.platform, platform);
    }

    /// Current version policy.
    pub fn version_policy(&self) -> Arc<VersionPolicy> {
        self.inner.policy()
    }

    /// Replace the version policy.
    pub fn set_version_policy(&self, policy: VersionPolicy) {
        write(&self.inner.policy, Arc::new(policy));
    }

    /// Current bypass configuration.
    pub fn bypass(&self) -> Arc<BypassConfig> {
        read(&self.inner.bypass)
    }

    /// Replace the bypass configuration.
    pub fn set_bypass(&self, bypass: BypassConfig) {
        write(&self.inner.bypass, Arc::new(bypass));
    }

    /// Device identity.
    pub async fn device(&self) -> Result<Arc<Device>> {
        self.inner.devices.get().await
    }

    /// Device store.
    pub fn device_store(&self) -> &Arc<DeviceStore> {
        &self.inner.devices
    }

    /// QIMEI used in `comm` (fetched on demand in [`QimeiMode::Auto`]).
    pub async fn qimei(&self) -> Option<Qimei> {
        self.inner.cached_qimei().await
    }

    /// Underlying transport.
    pub fn transport(&self) -> &Arc<dyn Transport> {
        &self.inner.transport
    }

    /// CGI endpoints.
    pub fn endpoints(&self) -> &Endpoints {
        &self.inner.endpoints
    }

    /// Generic CGI request for any module/method.
    pub fn cgi<T: FromJson + Send + 'static>(
        &self,
        module: impl Into<String>,
        method: impl Into<String>,
        param: Value,
    ) -> CgiRequest<T> {
        CgiRequest::new(self, CgiSpec::new(module, method, param))
    }

    /// Generic HTTP request.
    pub fn http<T: HttpOutput + Send + 'static>(&self, spec: HttpSpec) -> HttpRequest<T> {
        HttpRequest::new(self, spec)
    }

    /// New batch of CGI requests.
    pub fn batch(&self) -> Batch {
        Batch::new(self)
    }

    /// Execute many requests of the same type with batching.
    pub async fn gather<T, I>(&self, requests: I) -> Vec<Result<T>>
    where
        T: FromJson + Send + 'static,
        I: IntoIterator<Item = CgiRequest<T>>,
    {
        let specs = requests.into_iter().map(CgiRequest::into_spec).collect();
        self.execute_cgi(specs, None).await.into_iter().map(|r| r.and_then(|value| Ok(T::from_json(&value)?))).collect()
    }

    /// Open a streaming HTTP request (e.g. to download audio).
    pub async fn stream(&self, spec: HttpSpec) -> Result<StreamingResponse> {
        let request = self.prepare_http(spec).await?;
        let response = self.inner.send_streaming(request).await?;
        if !(200..300).contains(&response.status) {
            return Err(Error::Http {
                status: response.status,
                message: format!("HTTP 请求状态码异常: {}", response.status),
            });
        }
        Ok(response)
    }

    pub(crate) async fn execute_cgi_one(&self, spec: CgiSpec) -> Result<Value> {
        self.execute_cgi(vec![spec], Some(1)).await.pop().unwrap_or_else(|| Err(Error::api_data("缺少请求结果")))
    }

    /// Execute CGI specs, merging compatible ones into shared requests.
    pub(crate) async fn execute_cgi(&self, specs: Vec<CgiSpec>, batch_size: Option<usize>) -> Vec<Result<Value>> {
        let batch_size = batch_size.unwrap_or(self.inner.batch_size).max(1);
        let mut results: Vec<Option<Result<Value>>> = specs.iter().map(|_| None).collect();
        let default_credential = self.credential();
        let default_platform = self.platform();

        let mut groups: IndexMap<GroupKey, Vec<(usize, CgiSpec)>> = IndexMap::new();
        for (index, spec) in specs.into_iter().enumerate() {
            let credential = spec.credential.clone().unwrap_or_else(|| default_credential.clone());
            if spec.require_login && !credential.is_valid() {
                results[index] = Some(Err(Error::CredentialInvalid("请求需要登录, 未提供有效的登录凭证".into())));
                continue;
            }
            let key = GroupKey {
                platform: spec.platform.unwrap_or(default_platform),
                credential,
                comm: canonical_comm(&spec.comm),
                override_comm: spec.override_comm,
                sign: spec.sign,
                bypass: spec.bypass,
            };
            groups.entry(key).or_default().push((index, spec));
        }

        let mut batches = Vec::new();
        for (key, mut items) in groups {
            while !items.is_empty() {
                let rest = items.split_off(batch_size.min(items.len()));
                batches.push((key.clone(), items));
                items = rest;
            }
        }

        let outcomes = join_all(batches.into_iter().map(|(key, items)| async move {
            let outcome = self.run_batch(&key, &items).await;
            (items, outcome)
        }))
        .await;

        for (items, outcome) in outcomes {
            match outcome {
                Ok(values) => {
                    for ((index, _), value) in items.into_iter().zip(values) {
                        results[index] = Some(value);
                    }
                }
                Err(err) => {
                    for (index, _) in items {
                        results[index] = Some(Err(err.duplicate()));
                    }
                }
            }
        }
        results.into_iter().map(|r| r.unwrap_or_else(|| Err(Error::api_data("缺少请求结果")))).collect()
    }

    async fn run_batch(&self, key: &GroupKey, items: &[(usize, CgiSpec)]) -> Result<Vec<Result<Value>>> {
        let request = self.prepare_cgi(key, items).await?;
        let response = self.inner.send(request).await?;
        let raw_items = unwrap_cgi_envelope(&response, items.len())?;
        Ok(items
            .iter()
            .zip(raw_items)
            .enumerate()
            .map(|(position, ((_, spec), raw))| match raw {
                None => Err(Error::api_data(format!("CGI 响应格式异常, 缺少或畸形子响应 req_{position}"))),
                Some(raw) => parse_cgi_item(raw, spec.allow_error_codes.as_ref(), spec.parse_on_allow),
            })
            .collect())
    }

    /// Build the HTTP request for a batch.
    async fn prepare_cgi(&self, key: &GroupKey, items: &[(usize, CgiSpec)]) -> Result<Request> {
        let base = &items.first().ok_or_else(|| Error::invalid_argument("CGI 批次不能为空"))?.1;
        let device = self.inner.devices.get().await?;
        let policy = self.inner.policy();
        let bypass = self.bypass();
        let use_bypass = key.bypass && bypass.enabled;
        let comm: IndexMap<String, String> = if use_bypass {
            let mut comm = bypass.comm(&device.open_udid, &key.credential);
            let overrides: IndexMap<String, String> =
                base.comm.iter().map(|(k, v)| (k.clone(), v.clone().unwrap_or_default())).collect();
            crate::versioning::apply_comm_overrides(&mut comm, &overrides);
            comm
        } else if base.override_comm {
            base.comm.iter().filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone()))).collect()
        } else {
            let (session, qimei) = if key.platform == Platform::Android {
                (self.inner.android_session().await, self.inner.cached_qimei().await)
            } else {
                (None, None)
            };
            let mut comm = policy.build_comm(
                key.platform,
                CommContext {
                    credential: &key.credential,
                    device: &device,
                    qimei: qimei.as_ref(),
                    guid: &device.open_udid,
                    session: session.as_ref(),
                },
            );
            for (k, v) in &base.comm {
                match v {
                    Some(v) if !v.is_empty() => {
                        comm.insert(k.clone(), v.clone());
                    }
                    _ => {
                        comm.shift_remove(k);
                    }
                }
            }
            comm
        };

        let mut payload = Map::new();
        payload.insert("comm".into(), Value::Object(comm.into_iter().map(|(k, v)| (k, Value::String(v))).collect()));
        for (idx, (_, spec)) in items.iter().enumerate() {
            let mut param = if spec.preserve_bool { spec.param.clone() } else { bool_to_int(&spec.param) };
            if use_bypass && let Some(guid) = param.get_mut("guid") {
                *guid = Value::String(device.open_udid.clone());
            }
            let mut req = Map::new();
            req.insert("module".into(), Value::String(spec.module.clone()));
            req.insert("method".into(), Value::String(spec.method.clone()));
            req.insert("param".into(), param);
            payload.insert(format!("req_{idx}"), Value::Object(req));
        }
        let body = serde_json::to_vec(&Value::Object(payload)).map_err(|e| Error::api_data(e.to_string()))?;

        let mut request = if key.sign {
            let mut request = Request::new(Method::Post, self.inner.endpoints.musics.clone());
            request.query = vec![("_".into(), now_millis().to_string()), ("sign".into(), zzc_sign(&body))];
            request
        } else {
            Request::new(Method::Post, self.inner.endpoints.musicu.clone())
        };
        let user_agent = if use_bypass {
            bypass.user_agent.clone().unwrap_or_else(|| policy.user_agent(Platform::Web, &device))
        } else {
            policy.user_agent(key.platform, &device)
        };
        request.set_header("User-Agent", user_agent);
        request.body = Body::Json(body);
        Ok(request)
    }

    async fn prepare_http(&self, spec: HttpSpec) -> Result<Request> {
        let credential = spec.credential.clone().unwrap_or_else(|| self.credential());
        let mut cookies: IndexMap<String, String> = IndexMap::new();
        if credential.musicid != 0 {
            let uin = credential.uin();
            cookies.insert("uin".into(), uin.clone());
            cookies.insert("qqmusic_uin".into(), uin);
        }
        if !credential.musickey.is_empty() {
            cookies.insert("qm_keyst".into(), credential.musickey.clone());
            cookies.insert("qqmusic_key".into(), credential.musickey.clone());
        }
        cookies.extend(spec.cookies);

        let mut request = Request::new(spec.method, spec.url);
        request.query = spec.query;
        request.headers = spec.headers;
        request.body = spec.body;
        request.timeout = spec.timeout;
        request.follow_redirects = spec.follow_redirects;
        if !cookies.is_empty() {
            let mut value = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ");
            if let Some(existing) = request.header("cookie") {
                value = format!("{existing}; {value}");
            }
            request.set_header("Cookie", value);
        }
        if request.header("user-agent").is_none() {
            let device = self.inner.devices.get().await?;
            let ua = self.inner.policy().user_agent(Platform::Web, &device);
            request.headers.push(("User-Agent".into(), ua));
        }
        Ok(request)
    }

    pub(crate) async fn execute_http(&self, spec: HttpSpec) -> Result<Response> {
        let request = self.prepare_http(spec).await?;
        self.inner.send(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::DeviceProfile;
    use crate::transport::mock::MockTransport;
    use serde_json::json;

    fn client(mock: &MockTransport) -> Client {
        Client::builder()
            .transport(mock.clone())
            .device(Device::generate(Some(DeviceProfile::Vivo), Some(1)))
            .qimei(QimeiMode::Fixed(Qimei { q16: "q16".into(), q36: "q36".into() }))
            .android_session(false)
            .rate_limit(None)
            .build()
            .unwrap()
    }

    fn echo_ok(req: &Request) -> Result<Response, crate::transport::TransportError> {
        let body = req.json_body().unwrap();
        let mut out = Map::new();
        out.insert("code".into(), json!(0));
        for (key, value) in body.as_object().unwrap() {
            if key.starts_with("req_") {
                out.insert(key.clone(), json!({"code": 0, "data": {"echo": value}}));
            }
        }
        Ok(Response::json(&Value::Object(out)))
    }

    #[tokio::test]
    async fn single_request_payload() {
        let mock = MockTransport::with_handler(echo_ok);
        let client = client(&mock);
        let value: Value = client.cgi("mod", "meth", json!({"flag": true, "n": 1})).send().await.unwrap();
        assert_eq!(value["echo"]["module"], "mod");
        assert_eq!(value["echo"]["param"]["flag"], 1);
        let request = mock.last_request().unwrap();
        assert_eq!(request.url, MUSICU_URL);
        assert_eq!(request.header("user-agent"), Some("QQMusic 20090008(android 15)"));
        let body = request.json_body().unwrap();
        assert_eq!(body["comm"]["ct"], "11");
        assert_eq!(body["comm"]["QIMEI36"], "q36");
        assert!(body["comm"].get("uid").is_none());
    }

    #[tokio::test]
    async fn preserve_bool_comm_and_platform() {
        let mock = MockTransport::with_handler(echo_ok);
        let client = client(&mock);
        let _: Value = client
            .cgi("m", "x", json!({"flag": true}))
            .preserve_bool(true)
            .platform(Platform::Web)
            .comm("ct", "99")
            .remove_comm("notice")
            .comm("format", "")
            .send()
            .await
            .unwrap();
        let request = mock.last_request().unwrap();
        let body = request.json_body().unwrap();
        assert_eq!(body["req_0"]["param"]["flag"], true);
        assert_eq!(body["comm"]["ct"], "99");
        assert!(body["comm"].get("notice").is_none());
        assert!(body["comm"].get("format").is_none());
        assert_eq!(body["comm"]["platform"], "yqq.json");
        assert!(request.header("user-agent").unwrap().starts_with("Mozilla"));
    }

    #[tokio::test]
    async fn override_comm_replaces_everything() {
        let mock = MockTransport::with_handler(echo_ok);
        let client = client(&mock);
        let _: Value = client.cgi("m", "x", json!({})).override_comm([("ct", "5"), ("cv", "0")]).send().await.unwrap();
        let body = mock.last_request().unwrap().json_body().unwrap();
        assert_eq!(body["comm"], json!({"ct": "5", "cv": "0"}));
    }

    #[tokio::test]
    async fn signed_requests_use_musics() {
        let mock = MockTransport::with_handler(echo_ok);
        let client = client(&mock);
        let _: Value = client.cgi("m", "x", json!({})).sign(true).send().await.unwrap();
        let request = mock.last_request().unwrap();
        assert_eq!(request.url, MUSICS_URL);
        let Body::Json(bytes) = &request.body else { panic!("json body") };
        assert_eq!(request.query_param("sign").unwrap(), zzc_sign(bytes));
        assert!(request.query_param("_").is_some());
    }

    #[tokio::test]
    async fn gather_groups_and_chunks() {
        let mock = MockTransport::with_handler(echo_ok);
        let client = client(&mock);
        let requests = (0..5).map(|i| client.cgi::<Value>("m", "x", json!({"i": i})));
        let mut other = vec![client.cgi::<Value>("m", "y", json!({})).platform(Platform::Desktop)];
        other.extend(requests);
        let specs = other.into_iter().map(CgiRequest::into_spec).collect();
        let results = client.execute_cgi(specs, Some(2)).await;
        assert_eq!(results.len(), 6);
        // desktop group: 1 request; android group: 5 requests in chunks of 2 -> 3
        assert_eq!(mock.request_count(), 4);
        for (i, result) in results.iter().enumerate().skip(1) {
            assert_eq!(result.as_ref().unwrap()["echo"]["param"]["i"], i - 1);
        }
        let gathered = client.gather((0..3).map(|i| client.cgi::<Value>("m", "x", json!({"i": i})))).await;
        assert_eq!(gathered.len(), 3);
        assert_eq!(gathered[2].as_ref().unwrap()["echo"]["param"]["i"], 2);
    }

    #[tokio::test]
    async fn batch_handles_and_partial_errors() {
        let mock = MockTransport::with_handler(|_| {
            Ok(Response::json(&json!({
                "code": 0,
                "req_0": {"code": 0, "data": {"a": 1}},
                "req_1": {"code": 2001, "data": {"feedbackURL": "https://verify"}},
            })))
        });
        let client = client(&mock);
        let mut batch = client.batch();
        let ok = batch.add(client.cgi::<Value>("m", "a", json!({})));
        let bad = batch.add(client.cgi::<Value>("m", "b", json!({})));
        let missing = batch.add(client.cgi::<Value>("m", "c", json!({})));
        assert_eq!(batch.len(), 3);
        let mut results = batch.send().await;
        assert_eq!(results.take(ok).unwrap(), json!({"a": 1}));
        let err = results.take(bad).unwrap_err();
        assert!(err.is_ratelimited());
        assert!(results.take(missing).unwrap_err().to_string().contains("req_2"));
        assert!(results.take(ok).is_err(), "handles can only be taken once");
        assert_eq!(mock.request_count(), 1);
    }

    #[tokio::test]
    async fn require_login_and_transport_errors() {
        let mock = MockTransport::new();
        mock.push_error(crate::transport::TransportError::timeout("slow"));
        let client = client(&mock);
        let mut spec = CgiSpec::new("m", "x", json!({}));
        spec.require_login = true;
        let err = client.execute_cgi_one(spec.clone()).await.unwrap_err();
        assert!(matches!(err, Error::CredentialInvalid(_)));
        assert_eq!(mock.request_count(), 0);

        spec.require_login = false;
        let results = client.execute_cgi(vec![spec.clone(), spec], None).await;
        assert!(results.iter().all(|r| r.as_ref().unwrap_err().is_timeout()));
        assert_eq!(mock.request_count(), 1);
    }

    #[tokio::test]
    async fn credential_switching_and_cookies() {
        let mock = MockTransport::new();
        mock.route_url("example", |_| Ok(Response::json(&json!({"ok": 1}))));
        mock.route_url("musicu", echo_ok);
        let client = client(&mock);
        client.set_credential(Credential::new(7, "W_X_key"));
        let _: Value = client.cgi("m", "x", json!({})).send().await.unwrap();
        let body = mock.last_request().unwrap().json_body().unwrap();
        assert_eq!(body["comm"]["qq"], "7");
        assert_eq!(body["comm"]["tmeLoginType"], "1");

        let value: Value =
            client.http(HttpSpec::new(Method::Get, "https://example.com/a").cookie("extra", "1")).send().await.unwrap();
        assert_eq!(value, json!({"ok": 1}));
        let request = mock.last_request().unwrap();
        let cookies = request.cookies();
        assert_eq!(cookies["uin"], "7");
        assert_eq!(cookies["qm_keyst"], "W_X_key");
        assert_eq!(cookies["extra"], "1");
        assert!(request.header("user-agent").unwrap().starts_with("Mozilla"));
    }

    #[tokio::test]
    async fn http_outputs_and_errors() {
        let mock = MockTransport::new();
        mock.push_response(Response::new(200, "plain text"));
        mock.push_response(Response::new(500, "boom"));
        mock.push_response(Response::new(200, "raw").with_header("Set-Cookie", "a=b"));
        let client = client(&mock);
        let text: String = client.http(HttpSpec::new(Method::Get, "u")).send().await.unwrap();
        assert_eq!(text, "plain text");
        let err = client.http::<Value>(HttpSpec::new(Method::Get, "u")).send().await.unwrap_err();
        assert!(matches!(err, Error::Http { status: 500, .. }));
        let raw: crate::response::RawPayload =
            client.http(HttpSpec::new(Method::Get, "u").header("User-Agent", "custom")).send().await.unwrap();
        assert_eq!(raw.cookies["a"], "b");
        assert_eq!(mock.last_request().unwrap().header("user-agent"), Some("custom"));
    }

    #[tokio::test]
    async fn default_headers_and_endpoints() {
        let mock = MockTransport::with_handler(echo_ok);
        let client = Client::builder()
            .transport(mock.clone())
            .header("X-Forwarded-For", "1.2.3.4")
            .endpoints(Endpoints {
                musicu: "https://proxy.example/musicu".into(),
                musics: "https://proxy.example/musics".into(),
            })
            .qimei(QimeiMode::Disabled)
            .android_session(false)
            .build()
            .unwrap();
        let _: Value = client.cgi("m", "x", json!({})).send().await.unwrap();
        let request = mock.last_request().unwrap();
        assert_eq!(request.url, "https://proxy.example/musicu");
        assert_eq!(request.header("x-forwarded-for"), Some("1.2.3.4"));
        assert_eq!(request.json_body().unwrap()["comm"]["QIMEI36"], "");
    }

    #[tokio::test]
    async fn android_session_is_fetched_once_and_used() {
        let mock = MockTransport::new();
        mock.route(
            |req| req.json_body().is_some_and(|b| b["req_0"]["module"] == "music.getSession.session"),
            |_| {
                Ok(Response::json(&json!({
                    "code": 0,
                    "req_0": {"code": 0, "data": {"session": {"uid": "u1", "sid": "s1"}}}
                })))
            },
        );
        mock.route_url("musicu", echo_ok);
        let client =
            Client::builder().transport(mock.clone()).qimei(QimeiMode::Disabled).rate_limit(None).build().unwrap();
        for _ in 0..2 {
            let _: Value = client.cgi("m", "x", json!({})).send().await.unwrap();
        }
        assert_eq!(mock.request_count(), 3);
        let body = mock.last_request().unwrap().json_body().unwrap();
        assert_eq!(body["comm"]["uid"], "u1");
        assert_eq!(body["comm"]["sid"], "s1");
        let session_req = &mock.requests()[0];
        assert_eq!(session_req.json_body().unwrap()["req_0"]["param"]["caller"], 2);
    }

    #[tokio::test]
    async fn concurrent_requests_share_one_session_fetch() {
        let mock = MockTransport::new();
        mock.route(
            |req| req.json_body().is_some_and(|b| b["req_0"]["module"] == "music.getSession.session"),
            |_| {
                Ok(Response::json(&json!({
                    "code": 0,
                    "req_0": {"code": 0, "data": {"session": {"uid": "u1", "sid": "s1"}}}
                })))
            },
        );
        mock.route_url("musicu", echo_ok);
        let client = Client::builder()
            .transport(crate::testing::SlowTransport(mock.clone()))
            .qimei(QimeiMode::Disabled)
            .rate_limit(None)
            .build()
            .unwrap();
        let calls = (0..8).map(|i| client.cgi::<Value>("m", format!("x{i}"), json!({})).send());
        let results = futures::future::join_all(calls).await;
        assert!(results.iter().all(Result::is_ok));
        let session_fetches = mock
            .requests()
            .iter()
            .filter(|r| r.json_body().is_some_and(|b| b["req_0"]["module"] == "music.getSession.session"))
            .count();
        assert_eq!(session_fetches, 1);
        assert!(mock.requests().iter().skip(1).all(|r| r.json_body().unwrap()["comm"]["sid"] == "s1"));
    }

    #[tokio::test]
    async fn android_session_failure_is_not_fatal() {
        let mock = MockTransport::new();
        mock.route(
            |req| req.json_body().is_some_and(|b| b["req_0"]["module"] == "music.getSession.session"),
            |_| Ok(Response::new(500, "")),
        );
        mock.route_url("musicu", echo_ok);
        let client = Client::builder().transport(mock.clone()).qimei(QimeiMode::Disabled).build().unwrap();
        let value: Value = client.cgi("m", "x", json!({})).send().await.unwrap();
        assert_eq!(value["echo"]["module"], "m");
    }

    #[tokio::test]
    async fn settings_accessors() {
        let mock = MockTransport::new();
        let client = client(&mock);
        client.set_platform(Platform::Desktop);
        assert_eq!(client.platform(), Platform::Desktop);
        let mut policy = (*client.version_policy()).clone();
        policy.desktop.ct = 1;
        client.set_version_policy(policy);
        assert_eq!(client.version_policy().desktop.ct, 1);
        client.set_bypass(BypassConfig::disabled());
        assert!(!client.bypass().enabled);
        assert_eq!(client.qimei().await.unwrap().q36, "q36");
        assert_eq!(client.device().await.unwrap().model, "V2408A");
        assert!(format!("{client:?}").contains("Desktop"));
    }

    #[tokio::test]
    async fn streaming_requests() {
        let mock = MockTransport::new();
        mock.push_response(Response::new(200, b"audio".to_vec()));
        mock.push_response(Response::new(403, ""));
        let client = client(&mock);
        let stream = client.stream(HttpSpec::new(Method::Get, "https://cdn/x")).await.unwrap();
        assert_eq!(stream.collect().await.unwrap(), b"audio");
        assert!(client.stream(HttpSpec::new(Method::Get, "https://cdn/x")).await.is_err());
    }
}
