//! Request descriptors.
//!
//! API methods return lazy request objects that can be customised before
//! being awaited:
//!
//! ```no_run
//! # async fn demo(client: qqmusic_api::Client) -> qqmusic_api::Result<()> {
//! use qqmusic_api::Platform;
//! use serde_json::{Value, json};
//! let detail: Value = client
//!     .cgi("music.pf_song_detail_svr", "get_song_detail_yqq", json!({"song_mid": "0039MnYb0qxYhV"}))
//!     .platform(Platform::Desktop) // emulate another client
//!     .comm("ct", "19")             // override a single comm field
//!     .param("song_type", 0)        // add / replace a parameter
//!     .await?;
//! # Ok(()) }
//! ```

use std::future::{Future, IntoFuture};
use std::marker::PhantomData;
use std::pin::Pin;
use std::time::Duration;

use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::client::Client;
use crate::credential::Credential;
use crate::error::{Error, Result};
use crate::json::FromJson;
use crate::response::{AllowErrorCodes, RawPayload, ensure_http_success};
use crate::transport::{Body, Method, Response};
use crate::versioning::Platform;

/// Boxed future returned by request descriptors.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Description of a single `musicu.fcg` module call.
#[derive(Debug, Clone, PartialEq)]
pub struct CgiSpec {
    /// Module name, e.g. `music.trackInfo.UniformRuleCtrl`.
    pub module: String,
    /// Method name, e.g. `CgiGetTrackInfo`.
    pub method: String,
    /// Parameters (JSON object).
    pub param: Value,
    /// Request level `comm` changes.
    ///
    /// When merging, `None` or `Some("")` removes a key; with
    /// [`CgiSpec::override_comm`] only `Some` entries are sent.
    pub comm: IndexMap<String, Option<String>>,
    /// Use `comm` verbatim instead of merging it into the platform comm.
    pub override_comm: bool,
    /// Keep booleans in `param` (otherwise converted to `0`/`1`).
    pub preserve_bool: bool,
    /// Credential override.
    pub credential: Option<Credential>,
    /// Platform override.
    pub platform: Option<Platform>,
    /// Fail early when no valid credential is available.
    pub require_login: bool,
    /// Send through `musics.fcg` with a `zzc` signature.
    pub sign: bool,
    /// Codes that must not be treated as errors.
    pub allow_error_codes: Option<AllowErrorCodes>,
    /// With allowed codes: parse `data` (true) or return the raw item (false).
    pub parse_on_allow: bool,
}

impl CgiSpec {
    /// New spec with default options.
    pub fn new(module: impl Into<String>, method: impl Into<String>, param: Value) -> Self {
        let param = if param.is_null() {
            Value::Object(Map::new())
        } else {
            param
        };
        Self {
            module: module.into(),
            method: method.into(),
            param,
            comm: IndexMap::new(),
            override_comm: false,
            preserve_bool: false,
            credential: None,
            platform: None,
            require_login: false,
            sign: false,
            allow_error_codes: None,
            parse_on_allow: false,
        }
    }
}

macro_rules! cgi_builder_methods {
    () => {
        /// Use a specific credential for this request.
        pub fn credential(mut self, credential: Credential) -> Self {
            self.spec.credential = Some(credential);
            self
        }

        /// Emulate a specific platform for this request.
        pub fn platform(mut self, platform: Platform) -> Self {
            self.spec.platform = Some(platform);
            self
        }

        /// Set (or with an empty value remove) a `comm` field.
        pub fn comm(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
            self.spec.comm.insert(key.into(), Some(value.into()));
            self
        }

        /// Remove a `comm` field.
        pub fn remove_comm(mut self, key: impl Into<String>) -> Self {
            self.spec.comm.insert(key.into(), None);
            self
        }

        /// Replace the whole `comm` block.
        pub fn override_comm<I, K, V>(mut self, comm: I) -> Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: Into<String>,
            V: Into<String>,
        {
            self.spec.comm = comm.into_iter().map(|(k, v)| (k.into(), Some(v.into()))).collect();
            self.spec.override_comm = true;
            self
        }

        /// Set (or replace) a request parameter.
        pub fn param(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
            if !self.spec.param.is_object() {
                self.spec.param = Value::Object(Map::new());
            }
            if let Some(map) = self.spec.param.as_object_mut() {
                map.insert(key.into(), value.into());
            }
            self
        }

        /// Toggle the `zzc` signature.
        pub fn sign(mut self, sign: bool) -> Self {
            self.spec.sign = sign;
            self
        }

        /// Keep booleans in the parameters.
        pub fn preserve_bool(mut self, preserve: bool) -> Self {
            self.spec.preserve_bool = preserve;
            self
        }

        /// Underlying spec.
        pub fn spec(&self) -> &CgiSpec {
            &self.spec
        }

        /// Mutable access to the underlying spec.
        pub fn spec_mut(&mut self) -> &mut CgiSpec {
            &mut self.spec
        }
    };
}

/// Lazy CGI request returning `T`.
#[must_use = "requests do nothing unless awaited"]
pub struct CgiRequest<T> {
    client: Client,
    spec: CgiSpec,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for CgiRequest<T> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            spec: self.spec.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> std::fmt::Debug for CgiRequest<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CgiRequest").field("spec", &self.spec).finish_non_exhaustive()
    }
}

impl<T: FromJson + Send + 'static> CgiRequest<T> {
    /// Create a request.
    pub fn new(client: &Client, spec: CgiSpec) -> Self {
        Self {
            client: client.clone(),
            spec,
            _marker: PhantomData,
        }
    }

    cgi_builder_methods!();

    /// Return the raw JSON payload instead of the typed model.
    pub fn raw(self) -> CgiRequest<Value> {
        self.into_model()
    }

    /// Decode the response into another model.
    pub fn into_model<U: FromJson + Send + 'static>(self) -> CgiRequest<U> {
        CgiRequest {
            client: self.client,
            spec: self.spec,
            _marker: PhantomData,
        }
    }

    /// Consume the request returning its spec.
    pub fn into_spec(self) -> CgiSpec {
        self.spec
    }

    /// Execute the request.
    pub async fn send(self) -> Result<T> {
        let value = self.client.execute_cgi_one(self.spec).await?;
        Ok(T::from_json(&value)?)
    }
}

impl<T: FromJson + Send + 'static> IntoFuture for CgiRequest<T> {
    type Output = Result<T>;
    type IntoFuture = BoxFuture<Result<T>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.send())
    }
}

/// Conversion of an HTTP response into a request result.
pub trait HttpOutput: Sized {
    /// Convert the response.
    fn from_response(response: Response) -> Result<Self>;
}

impl HttpOutput for RawPayload {
    fn from_response(response: Response) -> Result<Self> {
        ensure_http_success(&response)?;
        Ok(Self::from_response(response))
    }
}

impl<T: FromJson> HttpOutput for T {
    fn from_response(response: Response) -> Result<Self> {
        ensure_http_success(&response)?;
        match response.json_value() {
            Ok(value) => Ok(T::from_json(&value)?),
            Err(_) => Ok(T::from_json(&Value::String(response.text()))?),
        }
    }
}

/// Description of a plain HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpSpec {
    /// Method.
    pub method: Method,
    /// URL.
    pub url: String,
    /// Query parameters.
    pub query: Vec<(String, String)>,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// Cookies (merged over the credential cookies).
    pub cookies: IndexMap<String, String>,
    /// Body.
    pub body: Body,
    /// Credential override.
    pub credential: Option<Credential>,
    /// Timeout override.
    pub timeout: Option<Duration>,
    /// Follow redirects.
    pub follow_redirects: bool,
}

impl HttpSpec {
    /// New spec.
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: IndexMap::new(),
            body: Body::Empty,
            credential: None,
            timeout: None,
            follow_redirects: true,
        }
    }

    /// Append query parameters.
    #[must_use]
    pub fn query<I, K, V>(mut self, pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.query
            .extend(pairs.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Add a header.
    #[must_use]
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Add a cookie.
    #[must_use]
    pub fn cookie(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.cookies.insert(name.into(), value.into());
        self
    }

    /// Set the body.
    #[must_use]
    pub fn body(mut self, body: Body) -> Self {
        self.body = body;
        self
    }

    /// Do not follow redirects.
    #[must_use]
    pub fn no_redirects(mut self) -> Self {
        self.follow_redirects = false;
        self
    }
}

/// Lazy HTTP request returning `T`.
#[must_use = "requests do nothing unless awaited"]
pub struct HttpRequest<T> {
    client: Client,
    spec: HttpSpec,
    _marker: PhantomData<fn() -> T>,
}

impl<T> std::fmt::Debug for HttpRequest<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRequest").field("spec", &self.spec).finish_non_exhaustive()
    }
}

impl<T: HttpOutput + Send + 'static> HttpRequest<T> {
    /// Create a request.
    pub fn new(client: &Client, spec: HttpSpec) -> Self {
        Self {
            client: client.clone(),
            spec,
            _marker: PhantomData,
        }
    }

    /// Use a specific credential (cookies) for this request.
    pub fn credential(mut self, credential: Credential) -> Self {
        self.spec.credential = Some(credential);
        self
    }

    /// Add a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.spec.headers.push((name.into(), value.into()));
        self
    }

    /// Set a timeout.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.spec.timeout = Some(timeout);
        self
    }

    /// Underlying spec.
    pub fn spec(&self) -> &HttpSpec {
        &self.spec
    }

    /// Execute the request.
    pub async fn send(self) -> Result<T> {
        let response = self.client.execute_http(self.spec).await?;
        T::from_response(response)
    }
}

impl<T: HttpOutput + Send + 'static> IntoFuture for HttpRequest<T> {
    type Output = Result<T>;
    type IntoFuture = BoxFuture<Result<T>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.send())
    }
}

/// Typed handle into a [`BatchResults`].
#[derive(Debug)]
pub struct Handle<T> {
    index: usize,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

/// Several CGI requests merged into as few HTTP round trips as possible.
///
/// Requests sharing platform, credential and `comm` are sent in one
/// `musicu.fcg` call (up to [`Batch::batch_size`] per call).
#[derive(Debug)]
pub struct Batch {
    client: Client,
    specs: Vec<CgiSpec>,
    batch_size: Option<usize>,
}

impl Batch {
    pub(crate) fn new(client: &Client) -> Self {
        Self {
            client: client.clone(),
            specs: Vec::new(),
            batch_size: None,
        }
    }

    /// Add a request.
    pub fn add<T: FromJson + Send + 'static>(&mut self, request: CgiRequest<T>) -> Handle<T> {
        self.specs.push(request.into_spec());
        Handle {
            index: self.specs.len() - 1,
            _marker: PhantomData,
        }
    }

    /// Maximum number of module calls per HTTP request.
    #[must_use]
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = Some(size.max(1));
        self
    }

    /// Number of queued requests.
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Whether the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Execute all requests.
    pub async fn send(self) -> BatchResults {
        let items = self.client.execute_cgi(self.specs, self.batch_size).await;
        BatchResults {
            items: items.into_iter().map(Some).collect(),
        }
    }
}

/// Results of a [`Batch`].
#[derive(Debug)]
pub struct BatchResults {
    items: Vec<Option<Result<Value>>>,
}

impl BatchResults {
    /// Take the typed result of a request (each handle can be taken once).
    pub fn take<T: FromJson>(&mut self, handle: Handle<T>) -> Result<T> {
        match self.items.get_mut(handle.index).and_then(Option::take) {
            Some(Ok(value)) => Ok(T::from_json(&value)?),
            Some(Err(err)) => Err(err),
            None => Err(Error::invalid_argument("batch result already taken or unknown handle")),
        }
    }

    /// Number of results.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are no results.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}
