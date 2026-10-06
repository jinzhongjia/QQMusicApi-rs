//! Default transport backed by `reqwest` + rustls.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;

use super::{Method, Request, Response, StreamingResponse, Transport, TransportError};
use crate::error::Error;

/// Configuration of [`ReqwestTransport`].
///
/// Several knobs exist specifically to help with QQ Music risk control:
/// routing through a proxy located in an allowed region, pinning DNS
/// results, binding a local address, or forcing HTTP/1.1 (some networks
/// stall on multiplexed HTTP/2 connections).
#[derive(Debug, Clone)]
pub struct TransportConfig {
    /// Total request timeout.
    pub timeout: Duration,
    /// Connect timeout.
    pub connect_timeout: Duration,
    /// Proxy URL (`http://`, `https://` or `socks5://` with feature `socks`).
    pub proxy: Option<String>,
    /// Only use HTTP/1.1.
    pub http1_only: bool,
    /// Number of retries for connection failures.
    pub connect_retries: u32,
    /// Base backoff between connection retries (doubled each attempt).
    pub retry_backoff: Duration,
    /// Accept invalid TLS certificates (debugging only).
    pub accept_invalid_certs: bool,
    /// Headers added to every request (request headers take precedence).
    pub default_headers: Vec<(String, String)>,
    /// Static DNS overrides (`host` → socket address).
    pub resolve: Vec<(String, SocketAddr)>,
    /// Local address to bind outgoing connections to.
    pub local_address: Option<IpAddr>,
    /// Maximum redirects for requests that follow redirects.
    pub max_redirects: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            connect_timeout: Duration::from_secs(10),
            proxy: None,
            http1_only: false,
            connect_retries: 2,
            retry_backoff: Duration::from_millis(200),
            accept_invalid_certs: false,
            default_headers: Vec::new(),
            resolve: Vec::new(),
            local_address: None,
            max_redirects: 10,
        }
    }
}

/// [`Transport`] implementation using `reqwest`.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    follow: reqwest::Client,
    no_follow: reqwest::Client,
    connect_retries: u32,
    retry_backoff: Duration,
}

fn build_client(config: &TransportConfig, follow: bool) -> Result<reqwest::Client, Error> {
    let mut headers = reqwest::header::HeaderMap::new();
    for (name, value) in &config.default_headers {
        let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| Error::invalid_argument(format!("invalid header name {name}: {e}")))?;
        let value = reqwest::header::HeaderValue::from_str(value)
            .map_err(|e| Error::invalid_argument(format!("invalid header value: {e}")))?;
        headers.append(name, value);
    }
    let mut builder = reqwest::Client::builder()
        .timeout(config.timeout)
        .connect_timeout(config.connect_timeout)
        .default_headers(headers)
        .redirect(if follow {
            reqwest::redirect::Policy::limited(config.max_redirects)
        } else {
            reqwest::redirect::Policy::none()
        })
        .no_proxy();
    if config.http1_only {
        builder = builder.http1_only();
    }
    if config.accept_invalid_certs {
        builder = builder.tls_danger_accept_invalid_certs(true);
    }
    if let Some(proxy) = &config.proxy {
        let proxy =
            reqwest::Proxy::all(proxy).map_err(|e| Error::invalid_argument(format!("invalid proxy {proxy}: {e}")))?;
        builder = builder.proxy(proxy);
    }
    for (host, addr) in &config.resolve {
        builder = builder.resolve(host, *addr);
    }
    if let Some(addr) = config.local_address {
        builder = builder.local_address(addr);
    }
    builder.build().map_err(|e| Error::invalid_argument(format!("failed to build HTTP client: {e}")))
}

fn map_error(err: reqwest::Error) -> TransportError {
    let message = err.to_string();
    let mapped = if err.is_timeout() {
        TransportError::timeout(message)
    } else if err.is_connect() {
        TransportError::connect(message)
    } else {
        TransportError::new(message)
    };
    mapped.with_source(err)
}

fn convert_method(method: Method) -> reqwest::Method {
    match method {
        Method::Get => reqwest::Method::GET,
        Method::Post => reqwest::Method::POST,
        Method::Put => reqwest::Method::PUT,
        Method::Delete => reqwest::Method::DELETE,
        Method::Head => reqwest::Method::HEAD,
    }
}

fn collect_headers(map: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    map.iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect()
}

impl ReqwestTransport {
    /// Build a transport from a configuration.
    pub fn new(config: &TransportConfig) -> Result<Self, Error> {
        Ok(Self {
            follow: build_client(config, true)?,
            no_follow: build_client(config, false)?,
            connect_retries: config.connect_retries,
            retry_backoff: config.retry_backoff,
        })
    }

    /// Wrap existing reqwest clients.
    ///
    /// `no_follow` must be configured with `redirect::Policy::none()`.
    pub fn from_clients(follow: reqwest::Client, no_follow: reqwest::Client) -> Self {
        Self { follow, no_follow, connect_retries: 0, retry_backoff: Duration::ZERO }
    }

    fn build(&self, request: &Request) -> reqwest::RequestBuilder {
        let client = if request.follow_redirects { &self.follow } else { &self.no_follow };
        let mut builder = client.request(convert_method(request.method), request.full_url());
        for (name, value) in &request.headers {
            builder = builder.header(name.as_str(), value.as_str());
        }
        if let Some((content_type, data)) = request.body.encode() {
            if request.header("content-type").is_none() && !content_type.is_empty() {
                builder = builder.header("content-type", content_type);
            }
            builder = builder.body(data);
        }
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        builder
    }

    async fn execute(&self, request: &Request) -> Result<reqwest::Response, TransportError> {
        let mut attempt = 0;
        loop {
            match self.build(request).send().await {
                Ok(response) => return Ok(response),
                Err(err) if err.is_connect() && attempt < self.connect_retries => {
                    let delay = self.retry_backoff * 2u32.saturating_pow(attempt);
                    tracing::debug!(attempt, error = %err, "connect failed, retrying");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(err) => return Err(map_error(err)),
            }
        }
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        let response = self.execute(&request).await?;
        let status = response.status().as_u16();
        let url = response.url().to_string();
        let headers = collect_headers(response.headers());
        let body = response.bytes().await.map_err(map_error)?.to_vec();
        Ok(Response { status, url, headers, body })
    }

    async fn send_streaming(&self, request: Request) -> Result<StreamingResponse, TransportError> {
        let response = self.execute(&request).await?;
        Ok(StreamingResponse {
            status: response.status().as_u16(),
            url: response.url().to_string(),
            headers: collect_headers(response.headers()),
            body: response.bytes_stream().map(|chunk| chunk.map(|b| b.to_vec()).map_err(map_error)).boxed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_with_custom_config() {
        let config = TransportConfig {
            proxy: Some("http://127.0.0.1:8080".into()),
            http1_only: true,
            default_headers: vec![("X-Forwarded-For".into(), "1.2.3.4".into())],
            resolve: vec![("u.y.qq.com".into(), "127.0.0.1:443".parse().unwrap())],
            local_address: Some("0.0.0.0".parse().unwrap()),
            ..TransportConfig::default()
        };
        assert!(ReqwestTransport::new(&config).is_ok());
    }

    #[test]
    fn rejects_invalid_config() {
        let bad_header =
            TransportConfig { default_headers: vec![("bad header".into(), "x".into())], ..TransportConfig::default() };
        assert!(ReqwestTransport::new(&bad_header).is_err());
        let bad_proxy = TransportConfig { proxy: Some("::not a url".into()), ..TransportConfig::default() };
        assert!(ReqwestTransport::new(&bad_proxy).is_err());
    }

    #[tokio::test]
    async fn connection_errors_are_mapped() {
        let config = TransportConfig {
            connect_retries: 1,
            retry_backoff: Duration::from_millis(1),
            connect_timeout: Duration::from_millis(500),
            ..TransportConfig::default()
        };
        let transport = ReqwestTransport::new(&config).unwrap();
        // Port 9 (discard) on localhost is almost always closed.
        let err = transport.send(Request::new(Method::Get, "http://127.0.0.1:9/")).await.unwrap_err();
        assert!(err.connect || err.timeout, "{err:?}");
    }
}
