//! Client-side overhead benchmark (no network).
//!
//! A zero-latency transport replays real (anonymised) QQ Music responses, so
//! the numbers measure only what this crate adds per request: comm/payload
//! building, locking, JSON serialisation, envelope unwrapping and model
//! construction.
//!
//! ```text
//! cargo bench -p qqmusic-api --bench overhead
//! ```

use std::collections::HashMap;
use std::hint::black_box;
use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use qqmusic_api::client::QimeiMode;
use qqmusic_api::device::Device;
use qqmusic_api::json::FromJson;
use qqmusic_api::models::search::SearchByTypeResponse;
use qqmusic_api::modules::search::SearchOptions;
use qqmusic_api::transport::{Body, Request, Response, Transport, TransportError};
use qqmusic_api::versioning::Platform;
use qqmusic_api::{Client, Error};
use serde_json::Value;

const SONG_DETAIL: &[u8] = include_bytes!("fixtures/song_detail.json");
const SESSION: &[u8] = include_bytes!("fixtures/session.json");
const QIMEI: &[u8] = include_bytes!("fixtures/qimei.json");
const SEARCH_GZ: &[u8] = include_bytes!("fixtures/search_60_songs.json.gz");

/// Replays canned responses; song detail batches of any size are synthesised
/// from one real `req_0` item and cached.
struct Replay {
    detail_item: Vec<u8>,
    search: Vec<u8>,
    batches: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
}

impl Replay {
    fn new() -> Self {
        let detail: Value = serde_json::from_slice(SONG_DETAIL).unwrap();
        let mut search = Vec::new();
        flate2::read::GzDecoder::new(SEARCH_GZ).read_to_end(&mut search).unwrap();
        Self { detail_item: serde_json::to_vec(&detail["req_0"]).unwrap(), search, batches: Mutex::new(HashMap::new()) }
    }

    fn detail_batch(&self, n: usize) -> Arc<Vec<u8>> {
        let mut cache = self.batches.lock().unwrap();
        Arc::clone(cache.entry(n).or_insert_with(|| {
            let mut body = br#"{"code":0"#.to_vec();
            for i in 0..n {
                body.extend_from_slice(format!(r#","req_{i}":"#).as_bytes());
                body.extend_from_slice(&self.detail_item);
            }
            body.push(b'}');
            Arc::new(body)
        }))
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[async_trait]
impl Transport for Replay {
    async fn send(&self, request: Request) -> Result<Response, TransportError> {
        if request.url.contains("tencentmusic.com") {
            return Ok(Response::new(200, QIMEI));
        }
        let Body::Json(body) = &request.body else {
            return Ok(Response::new(404, Vec::new()));
        };
        let reply = if contains(body, b"music.getSession.session") {
            SESSION.to_vec()
        } else if contains(body, b"DoSearchForQQMusicMobile") {
            self.search.clone()
        } else {
            let n = body.windows(5).filter(|w| w == b"\"req_").count();
            self.detail_batch(n).as_ref().clone()
        };
        Ok(Response::new(200, reply))
    }
}

fn client(platform: Platform, replay: &Arc<Replay>) -> Client {
    Client::builder()
        .platform(platform)
        .device(Device::random())
        .transport_arc(replay.clone())
        .rate_limit(None)
        .max_concurrency(1024)
        .build()
        .unwrap()
}

struct Report {
    name: &'static str,
    ops: u64,
    elapsed: Duration,
    unit: &'static str,
}

impl Report {
    fn print(&self) {
        let per_op = self.elapsed.as_secs_f64() * 1e6 / self.ops as f64;
        let rate = self.ops as f64 / self.elapsed.as_secs_f64();
        println!("{:<44} {:>9} {:<9} {:>10.2} µs/op {:>12.0} op/s", self.name, self.ops, self.unit, per_op, rate);
    }
}

async fn bench<F, Fut>(name: &'static str, unit: &'static str, ops: u64, mut f: F) -> Report
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = u64>,
{
    let ops = ((ops as f64 * scale()) as u64).max(1);
    // Warm-up (device, QIMEI, session caches; allocator): ~5% of the ops,
    // counted in operations (one call may perform many).
    let mut warm = 0;
    while warm < (ops / 20).max(1) {
        warm += black_box(f().await);
    }
    // Best of three runs (big.LITTLE scheduling makes single runs noisy).
    let mut best: Option<Report> = None;
    for _ in 0..3 {
        let start = Instant::now();
        let mut done = 0;
        while done < ops {
            done += f().await;
        }
        let report = Report { name, ops: done, elapsed: start.elapsed(), unit };
        let per_op = |r: &Report| r.elapsed.as_secs_f64() / r.ops as f64;
        if best.as_ref().is_none_or(|b| per_op(&report) < per_op(b)) {
            best = Some(report);
        }
    }
    let report = best.expect("three runs");
    report.print();
    report
}

/// `BENCH_SCALE=0.1` runs a tenth of the iterations.
fn scale() -> f64 {
    std::env::var("BENCH_SCALE").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0)
}

fn main() {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    runtime.block_on(run());
}

async fn run() {
    let replay = Arc::new(Replay::new());
    let web = client(Platform::Web, &replay);
    let android = client(Platform::Android, &replay);
    let android_no_qimei = Client::builder()
        .platform(Platform::Android)
        .device(Device::random())
        .transport_arc(replay.clone())
        .rate_limit(None)
        .qimei(QimeiMode::Disabled)
        .android_session(false)
        .build()
        .unwrap();
    let search_body = replay.search.clone();
    let search_value: Value = serde_json::from_slice(&search_body).unwrap();
    let search_data = search_value["req_0"]["data"].clone();

    println!("{:<44} {:>9} {:<9} {:>16} {:>15}", "benchmark", "ops", "unit", "latency", "throughput");
    let mut reports = Vec::new();

    reports.push(
        bench("json: serde_json::from_slice (234 KB)", "parse", 500, || {
            let value: Value = serde_json::from_slice(black_box(&search_body)).unwrap();
            black_box(value);
            async { 1 }
        })
        .await,
    );
    reports.push(
        bench("model: SearchByTypeResponse::from_json", "parse", 500, || {
            black_box(SearchByTypeResponse::from_json(black_box(&search_data)).unwrap());
            async { 1 }
        })
        .await,
    );
    reports.push(
        bench("search_by_type 60 songs (android, e2e)", "request", 500, || {
            let android = android.clone();
            async move {
                let options = SearchOptions { num: 60, ..Default::default() };
                let page = android.search().search_by_type_with("周杰伦", options).await.unwrap();
                assert_eq!(page.song.len(), 60);
                1
            }
        })
        .await,
    );
    for (name, client) in [
        ("song detail (web, sequential)", &web),
        ("song detail (android, sequential)", &android),
        ("song detail (android, no qimei/session)", &android_no_qimei),
    ] {
        reports.push(
            bench(name, "request", 20_000, || {
                let client = client.clone();
                async move {
                    black_box(client.song().get_detail("0039MnYb0qxYhV").await.unwrap());
                    1
                }
            })
            .await,
        );
    }
    reports.push(
        bench("song detail (android, 64 concurrent tasks)", "request", 128_000, || {
            let android = android.clone();
            async move {
                let tasks: Vec<_> = (0..64)
                    .map(|_| {
                        let android = android.clone();
                        tokio::spawn(async move {
                            for _ in 0..100 {
                                black_box(android.song().get_detail("0039MnYb0qxYhV").await.unwrap());
                            }
                        })
                    })
                    .collect();
                for task in tasks {
                    task.await.unwrap();
                }
                6_400
            }
        })
        .await,
    );
    reports.push(
        bench("gather 1000 song details (batch 20)", "request", 50_000, || {
            let android = android.clone();
            async move {
                let requests = (0..1000).map(|_| android.song().get_detail("0039MnYb0qxYhV"));
                let results = android.gather(requests).await;
                assert!(results.iter().all(Result::is_ok));
                1000
            }
        })
        .await,
    );
    let err = android.cgi::<Value>("m", "x", Value::Null).require_login(true).await.unwrap_err();
    assert!(matches!(err, Error::CredentialInvalid(_)));
    drop(reports);
}
