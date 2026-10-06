//! HTTP transport abstraction.
//!
//! The client never talks to the network directly; it goes through a
//! [`Transport`]. The default implementation is
//! `ReqwestTransport` (feature
//! `reqwest-transport`), and [`mock::MockTransport`] is provided for tests.
//! Custom transports can be plugged in to control TLS fingerprints, routing
//! or proxies for risk-control bypass.

pub mod mock;
#[cfg(feature = "reqwest-transport")]
mod reqwest_impl;

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use indexmap::IndexMap;
use serde_json::Value;

pub use crate::error::TransportError;
#[cfg(feature = "reqwest-transport")]
pub use reqwest_impl::{ReqwestTransport, TransportConfig};

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Method {
    /// GET
    Get,
    /// POST
    Post,
    /// PUT
    Put,
    /// DELETE
    Delete,
    /// HEAD
    Head,
}

impl Method {
    /// Uppercase method name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Request body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Body {
    /// No body.
    #[default]
    Empty,
    /// JSON body (already serialized; sent as `application/json`).
    Json(Vec<u8>),
    /// URL encoded form.
    Form(Vec<(String, String)>),
    /// Raw bytes with a content type.
    Raw {
        /// `Content-Type` header.
        content_type: String,
        /// Payload.
        data: Vec<u8>,
    },
}

impl Body {
    /// JSON body from a value (compact serialization).
    pub fn json(value: &Value) -> Self {
        Self::Json(serde_json::to_vec(value).unwrap_or_default())
    }

    /// Body bytes and content type.
    pub fn encode(&self) -> Option<(String, Vec<u8>)> {
        match self {
            Self::Empty => None,
            Self::Json(data) => Some(("application/json".to_string(), data.clone())),
            Self::Form(pairs) => {
                let encoded = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish();
                Some(("application/x-www-form-urlencoded".to_string(), encoded.into_bytes()))
            }
            Self::Raw { content_type, data } => Some((content_type.clone(), data.clone())),
        }
    }

    /// Content type of the body.
    pub fn content_type(&self) -> Option<&str> {
        match self {
            Self::Empty => None,
            Self::Json(_) => Some("application/json"),
            Self::Form(_) => Some("application/x-www-form-urlencoded"),
            Self::Raw { content_type, .. } => Some(content_type),
        }
    }
}

/// Fully prepared HTTP request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// Method.
    pub method: Method,
    /// URL without the query parameters below.
    pub url: String,
    /// Query parameters appended to `url`.
    pub query: Vec<(String, String)>,
    /// Headers (including `Cookie`).
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Body,
    /// Per-request timeout.
    pub timeout: Option<Duration>,
    /// Whether redirects should be followed.
    pub follow_redirects: bool,
}

impl Request {
    /// New request without query, headers or body.
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        Self {
            method,
            url: url.into(),
            query: Vec::new(),
            headers: Vec::new(),
            body: Body::Empty,
            timeout: None,
            follow_redirects: true,
        }
    }

    /// URL including the encoded query string.
    pub fn full_url(&self) -> String {
        if self.query.is_empty() {
            return self.url.clone();
        }
        let query = url::form_urlencoded::Serializer::new(String::new()).extend_pairs(&self.query).finish();
        let separator = if self.url.contains('?') { '&' } else { '?' };
        format!("{}{separator}{query}", self.url)
    }

    /// First header value (case insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Insert or replace a header (case insensitive).
    pub fn set_header(&mut self, name: impl Into<String>, value: impl Into<String>) {
        let name = name.into();
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(&name));
        self.headers.push((name, value.into()));
    }

    /// Value of a query parameter.
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// Parse the JSON body (if any).
    pub fn json_body(&self) -> Option<Value> {
        match &self.body {
            Body::Json(data) => serde_json::from_slice(data).ok(),
            _ => None,
        }
    }

    /// Cookies sent with the request.
    pub fn cookies(&self) -> IndexMap<String, String> {
        self.header("cookie")
            .map(|raw| {
                raw.split(';')
                    .filter_map(|pair| {
                        let (k, v) = pair.split_once('=')?;
                        Some((k.trim().to_string(), v.trim().to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Buffered HTTP response.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Response {
    /// Status code.
    pub status: u16,
    /// Final URL.
    pub url: String,
    /// Response headers (repeated headers are kept).
    pub headers: Vec<(String, String)>,
    /// Body.
    pub body: Vec<u8>,
}

impl Response {
    /// Response with a status and body.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self { status, url: String::new(), headers: Vec::new(), body: body.into() }
    }

    /// `200 OK` JSON response.
    pub fn json(value: &Value) -> Self {
        let mut response = Self::new(200, serde_json::to_vec(value).unwrap_or_default());
        response.headers.push(("content-type".into(), "application/json".into()));
        response
    }

    /// Add a header (builder style).
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// First header value (case insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Cookies set by the response (`Set-Cookie`).
    pub fn cookies(&self) -> IndexMap<String, String> {
        parse_set_cookies(
            self.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie")).map(|(_, v)| v.as_str()),
        )
    }

    /// Whether the status is 2xx.
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Body decoded as UTF-8 (lossy).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// Body parsed as JSON.
    pub fn json_value(&self) -> serde_json::Result<Value> {
        serde_json::from_slice(&self.body)
    }
}

/// Parse `Set-Cookie` header values into name → value pairs.
pub fn parse_set_cookies<'a>(values: impl IntoIterator<Item = &'a str>) -> IndexMap<String, String> {
    values
        .into_iter()
        .filter_map(|raw| {
            let first = raw.split(';').next()?;
            let (name, value) = first.split_once('=')?;
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), value.trim().to_string()))
        })
        .collect()
}

/// Streaming HTTP response.
pub struct StreamingResponse {
    /// Status code.
    pub status: u16,
    /// Final URL.
    pub url: String,
    /// Response headers.
    pub headers: Vec<(String, String)>,
    /// Body chunks.
    pub body: BoxStream<'static, Result<Vec<u8>, TransportError>>,
}

impl fmt::Debug for StreamingResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamingResponse")
            .field("status", &self.status)
            .field("url", &self.url)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

impl StreamingResponse {
    /// First header value (case insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Collect the remaining body.
    pub async fn collect(mut self) -> Result<Vec<u8>, TransportError> {
        let mut out = Vec::new();
        while let Some(chunk) = self.body.next().await {
            out.extend_from_slice(&chunk?);
        }
        Ok(out)
    }
}

impl From<Response> for StreamingResponse {
    fn from(response: Response) -> Self {
        Self {
            status: response.status,
            url: response.url,
            headers: response.headers,
            body: stream::once(async move { Ok(response.body) }).boxed(),
        }
    }
}

/// Pluggable HTTP transport.
#[async_trait]
pub trait Transport: Send + Sync + 'static {
    /// Send a request and buffer the response.
    async fn send(&self, request: Request) -> Result<Response, TransportError>;

    /// Send a request and stream the response body.
    ///
    /// The default implementation buffers via [`Transport::send`].
    async fn send_streaming(&self, request: Request) -> Result<StreamingResponse, TransportError> {
        self.send(request).await.map(StreamingResponse::from)
    }
}

#[async_trait]
impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        (**self).send(request).await
    }

    async fn send_streaming(&self, request: Request) -> Result<StreamingResponse, TransportError> {
        (**self).send_streaming(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn full_url_encodes_query() {
        let mut req = Request::new(Method::Get, "https://a.b/c");
        assert_eq!(req.full_url(), "https://a.b/c");
        req.query = vec![("q".into(), "周 杰伦".into()), ("n".into(), "1&2".into())];
        assert_eq!(req.full_url(), "https://a.b/c?q=%E5%91%A8+%E6%9D%B0%E4%BC%A6&n=1%262");
        let mut req2 = Request::new(Method::Get, "https://a.b/c?x=1");
        req2.query = vec![("y".into(), "2".into())];
        assert_eq!(req2.full_url(), "https://a.b/c?x=1&y=2");
        assert_eq!(req2.query_param("y"), Some("2"));
    }

    #[test]
    fn headers_and_cookies() {
        let mut req = Request::new(Method::Post, "u");
        req.set_header("User-Agent", "a");
        req.set_header("user-agent", "b");
        req.set_header("Cookie", "uin=1; qm_keyst=k");
        assert_eq!(req.header("USER-AGENT"), Some("b"));
        assert_eq!(req.headers.len(), 2);
        let cookies = req.cookies();
        assert_eq!(cookies["uin"], "1");
        assert_eq!(cookies["qm_keyst"], "k");
    }

    #[test]
    fn body_encoding() {
        assert_eq!(Body::Empty.encode(), None);
        let (ct, data) = Body::json(&json!({"a": 1})).encode().unwrap();
        assert_eq!(ct, "application/json");
        assert_eq!(data, br#"{"a":1}"#);
        let (ct, data) = Body::Form(vec![("a".into(), "b c".into())]).encode().unwrap();
        assert_eq!(ct, "application/x-www-form-urlencoded");
        assert_eq!(data, b"a=b+c");
        let raw = Body::Raw { content_type: "text/plain".into(), data: b"x".to_vec() };
        assert_eq!(raw.content_type(), Some("text/plain"));
    }

    #[test]
    fn response_helpers() {
        let resp = Response::json(&json!({"ok": true}))
            .with_header("Set-Cookie", "qrsig=abc; Path=/; Domain=qq.com")
            .with_header("set-cookie", "pt_login_sig=xyz")
            .with_header("Set-Cookie", "broken");
        assert!(resp.is_success());
        assert_eq!(resp.json_value().unwrap(), json!({"ok": true}));
        let cookies = resp.cookies();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies["qrsig"], "abc");
        assert_eq!(cookies["pt_login_sig"], "xyz");
        assert_eq!(resp.header("content-type"), Some("application/json"));
        assert!(!Response::new(404, "").is_success());
    }

    #[tokio::test]
    async fn streaming_from_buffered() {
        let streaming = StreamingResponse::from(Response::new(200, b"abc".to_vec()));
        assert_eq!(streaming.status, 200);
        assert_eq!(streaming.collect().await.unwrap(), b"abc");
    }
}
