//! CDN 测速选择。
//!
//! 默认 CDN 在部分网络下被限速到几十 KB/s，连无损都无法实时播放。加载歌曲时用这首歌的
//! 真实音频路径，对官方调度接口返回的节点（及当前节点）并行测速，选出最快的节点写入客户端
//! bypass 配置；结果缓存 30 分钟。

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::future::join_all;
use qqmusic_api::Client;
use qqmusic_api::transport::{Method, Request, ReqwestTransport, Transport, TransportConfig};
use tokio::sync::Mutex;

/// 单个节点的测速时长。
const PROBE_TIME: Duration = Duration::from_secs(2);
/// 单个节点最多下载的字节数。
const PROBE_BYTES: usize = 1024 * 1024;
/// 测速结果有效期。
const TTL: Duration = Duration::from_secs(30 * 60);

/// 音频下载用的 HTTP 客户端与 CDN 选择（测速结果缓存 30 分钟）。
pub struct CdnSelector {
    transport: Arc<dyn Transport>,
    last: Mutex<Option<Instant>>,
}

impl CdnSelector {
    /// 音频下载强制使用 HTTP/1.1：HTTP/2 下中途取消的流（切歌、测速）不会归还连接级
    /// 流量窗口，之后复用同一连接的下载会一直拿不到数据。创建失败时退回 `client` 的传输层。
    pub fn new(client: &Client) -> Self {
        let config = TransportConfig { http1_only: true, timeout: Duration::from_secs(3600), ..Default::default() };
        let transport: Arc<dyn Transport> = match ReqwestTransport::new(&config) {
            Ok(transport) => Arc::new(transport),
            Err(_) => client.transport().clone(),
        };
        Self { transport, last: Mutex::new(None) }
    }

    /// 下载音频用的传输层。
    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    /// 测速结果过期时重新选择，返回本次应使用的完整 URL（`url` 换成选中的节点）。
    pub async fn resolve(&self, client: &Client, url: &str) -> String {
        let Some(path) = split_path(url) else { return url.to_string() };
        let mut last = self.last.lock().await;
        if last.is_none_or(|at| at.elapsed() > TTL) {
            select_fastest(client, self.transport.as_ref(), path).await;
            *last = Some(Instant::now());
        }
        let cdn = client.bypass().cdn.clone();
        format!("{}/{path}", cdn.trim_end_matches('/'))
    }
}

/// `https://host/path?query` → `path?query`。
fn split_path(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    Some(rest.split_once('/')?.1)
}

async fn select_fastest(client: &Client, transport: &dyn Transport, path: &str) {
    let mut hosts = vec![client.bypass().cdn.clone()];
    if let Ok(dispatch) = client.song().get_cdn_dispatch().await {
        hosts.extend(dispatch.sip);
    }
    let mut candidates: Vec<String> = Vec::new();
    for host in &hosts {
        let host = host.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
        let cdn = format!("https://{host}/");
        if !host.is_empty() && !candidates.contains(&cdn) {
            candidates.push(cdn);
        }
    }
    let speeds = join_all(candidates.iter().map(|cdn| probe(transport, format!("{cdn}{path}")))).await;
    if let Some((cdn, speed)) = candidates.into_iter().zip(speeds).max_by_key(|(_, speed)| *speed)
        && speed > 0
    {
        eprintln!("选用 CDN {cdn}（{} KB/s）", speed / 1024);
        client.set_bypass(client.bypass().as_ref().clone().with_cdn(cdn));
    }
}

/// 下载开头一段，返回字节/秒（失败为 0）。
async fn probe(transport: &dyn Transport, url: String) -> u64 {
    let mut request = Request::new(Method::Get, url);
    request.timeout = Some(PROBE_TIME + Duration::from_secs(2));
    let start = Instant::now();
    let mut bytes = 0usize;
    let _ = tokio::time::timeout(PROBE_TIME, async {
        let Ok(response) = transport.send_streaming(request).await else { return };
        if !(200..300).contains(&response.status) {
            return;
        }
        let mut body = response.body;
        while let Some(Ok(chunk)) = body.next().await {
            bytes += chunk.len();
            if bytes >= PROBE_BYTES {
                break;
            }
        }
    })
    .await;
    (bytes as f64 / start.elapsed().as_secs_f64()) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn split_path() {
        assert_eq!(super::split_path("https://a.b/M500x.mp3?vkey=1"), Some("M500x.mp3?vkey=1"));
        assert_eq!(super::split_path("x"), None);
    }
}
