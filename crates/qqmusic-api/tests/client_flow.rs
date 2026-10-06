//! End-to-end tests through the public API with a mock transport.

use qqmusic_api::bypass::{CtStrategy, HIGH_QUALITY_CT};
use qqmusic_api::client::QimeiMode;
use qqmusic_api::device::{Device, DeviceProfile, Qimei};
use qqmusic_api::error::ApiErrorKind;
use qqmusic_api::modules::search::SearchType;
use qqmusic_api::modules::song::Quality;
use qqmusic_api::transport::mock::MockTransport;
use qqmusic_api::transport::{Request, Response};
use qqmusic_api::{BypassConfig, Client, Credential, Error, Platform};
use serde_json::{Value, json};

fn device() -> Device {
    Device::generate(Some(DeviceProfile::Vivo), Some(7))
}

fn client_with(mock: &MockTransport, bypass: BypassConfig) -> Client {
    Client::builder()
        .transport(mock.clone())
        .device(device())
        .qimei(QimeiMode::Fixed(Qimei { q16: "q16".into(), q36: "q36".into() }))
        .android_session(false)
        .rate_limit(None)
        .credential(Credential::new(123_456, "Q_H_L_secret"))
        .bypass(bypass)
        .build()
        .unwrap()
}

fn envelope(data: Value) -> Response {
    Response::json(&json!({"code": 0, "req_0": {"code": 0, "data": data}}))
}

fn body(request: &Request) -> Value {
    request.json_body().expect("json body")
}

fn vkey_reply(purls: &[&str]) -> Response {
    let data: Vec<Value> = purls.iter().map(|p| json!({"songmid": "mid", "purl": p, "filename": "f"})).collect();
    envelope(json!({"midurlinfo": data, "sip": ["https://cdn.example/"]}))
}

#[tokio::test]
async fn playable_url_uses_bypass_comm_and_quality_ladder() {
    let mock = MockTransport::new();
    let client = client_with(&mock, BypassConfig::default());
    // Lossless tiers (FLAC, OGG 640) unavailable, MP3 320 available.
    mock.push_response(vkey_reply(&["", "", "M800mid.mp3?vkey=1", "O800mid.ogg?vkey=2", "M500mid.mp3?vkey=3"]));

    let url = client.song().playable_url("mid", None, Quality::Lossless).await.unwrap().expect("url");
    assert_eq!(url.quality, Quality::High);
    assert_eq!(url.url, "https://isure.stream.qqmusic.qq.com/M800mid.mp3?vkey=1");

    let sent = body(&mock.last_request().unwrap());
    let comm = &sent["comm"];
    let guid = device().open_udid;
    let expected_ct = CtStrategy::default().ct_for(&guid);
    assert!(HIGH_QUALITY_CT.contains(&expected_ct));
    assert_eq!(comm["ct"].as_i64().or_else(|| comm["ct"].as_str()?.parse().ok()), Some(expected_ct));
    assert_eq!(comm["cv"].as_i64().or_else(|| comm["cv"].as_str()?.parse().ok()), Some(0));
    assert_eq!(comm["qq"], "123456");
    assert_eq!(comm["authst"], "Q_H_L_secret");
    let param = &sent["req_0"]["param"];
    assert_eq!(param["guid"], guid.as_str(), "guid must match the device identity");
    assert_eq!(param["filename"].as_array().unwrap().len(), 5);
    assert_eq!(param["filename"][2], "M800midmid.mp3");

    // Nothing playable.
    mock.push_response(vkey_reply(&["", "", "", "", ""]));
    assert!(client.song().playable_url("mid", None, Quality::Lossless).await.unwrap().is_none());
}

#[tokio::test]
async fn bypass_is_fully_configurable() {
    let mock = MockTransport::new();
    let bypass = BypassConfig::default().with_fixed_ct(24).with_cdn("https://my.cdn/").with_comm("tmeLoginType", "2");
    let client = client_with(&mock, bypass);
    mock.push_response(vkey_reply(&["a.flac"]));
    let url = client.song().playable_url("mid", Some("media"), Quality::Standard).await.unwrap().unwrap();
    assert_eq!(url.url, "https://my.cdn/a.flac");
    let sent = body(&mock.last_request().unwrap());
    assert_eq!(sent["comm"]["ct"].to_string().trim_matches('"'), "24");
    assert_eq!(sent["comm"]["tmeLoginType"], "2");
    assert!(sent["req_0"]["param"]["filename"][0].as_str().unwrap().contains("mediamedia"));

    // Disabled bypass: ordinary comm of the client platform.
    client.set_bypass(BypassConfig::disabled());
    mock.push_response(vkey_reply(&["a.mp3"]));
    client.song().playable_url("mid", None, Quality::Standard).await.unwrap();
    let sent = body(&mock.last_request().unwrap());
    assert_eq!(sent["comm"]["ct"].to_string().trim_matches('"'), "11", "Android default ct");
}

#[tokio::test]
async fn batch_requests_share_one_http_call() {
    let mock = MockTransport::new();
    let client = client_with(&mock, BypassConfig::default());
    mock.push_response(Response::json(&json!({
        "code": 0,
        "req_0": {"code": 0, "data": {"hotkey": [{"query": "a"}]}},
        "req_1": {"code": 1000, "data": {}},
    })));
    let mut batch = client.batch();
    let hot = batch.add(client.search().get_hotkey());
    let complete = batch.add(client.search().complete("jay"));
    let mut results = batch.send().await;
    assert_eq!(mock.request_count(), 1);
    assert!(results.take(hot).is_ok());
    match results.take(complete) {
        Err(Error::Api(err)) => assert_eq!(err.kind, ApiErrorKind::CredentialExpired),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn search_pagination_and_platform_override() {
    let mock = MockTransport::new();
    let client = client_with(&mock, BypassConfig::default());
    let page = |n: usize| {
        let songs: Vec<Value> = (0..n).map(|i| json!({"id": i, "mid": format!("m{i}"), "name": "s"})).collect();
        envelope(json!({"body": {"item_song": songs}, "meta": {"sum": 25, "nextpage": 2}}))
    };
    mock.push_response(page(10));
    mock.push_response(page(10));
    let songs = client.search().search_by_type("jay", SearchType::Song).collect_items(Some(15)).await.unwrap();
    assert_eq!(songs.len(), 15);
    assert_eq!(mock.request_count(), 2);
    let second = body(&mock.requests()[1]);
    assert_eq!(second["req_0"]["param"]["page_num"], 2);

    // `search_by_type` is pinned to Android like upstream; other requests
    // follow the client platform.
    client.set_platform(Platform::Web);
    mock.push_response(page(1));
    client.search().search_by_type("jay", SearchType::Song).await.unwrap();
    let sent = body(&mock.last_request().unwrap());
    assert_eq!(sent["comm"]["ct"].to_string().trim_matches('"'), "11", "pinned Android ct");
    mock.push_response(envelope(json!({})));
    client.search().get_hotkey().await.unwrap();
    let sent = body(&mock.last_request().unwrap());
    assert_eq!(sent["comm"]["ct"].to_string().trim_matches('"'), "24", "web ct");
}

#[tokio::test]
async fn device_is_persisted_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.json");
    let build = || {
        Client::builder()
            .transport(MockTransport::new())
            .device_path(&path)
            .qimei(QimeiMode::Disabled)
            .android_session(false)
            .rate_limit(None)
            .build()
            .unwrap()
    };
    let first = build().device_store().get().await.unwrap();
    assert!(path.exists());
    let second = build().device_store().get().await.unwrap();
    assert_eq!(first.open_udid, second.open_udid);
    assert_eq!(first.imei, second.imei);
}

#[tokio::test]
async fn login_required_requests_fail_fast() {
    let mock = MockTransport::new();
    let client = Client::builder()
        .transport(mock.clone())
        .device(device())
        .qimei(QimeiMode::Disabled)
        .android_session(false)
        .rate_limit(None)
        .build()
        .unwrap();
    let err = client.sound_power().get_detail(None).await.unwrap_err();
    assert!(matches!(err, Error::CredentialInvalid(_)), "{err:?}");
    assert_eq!(mock.request_count(), 0);
}
