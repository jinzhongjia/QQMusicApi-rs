//! 无网络、无用户数据的测试夹具。

use qqmusic_api::Client;
use qqmusic_api::client::QimeiMode;
use qqmusic_api::device::{Device, DeviceProfile, Qimei};
use qqmusic_api::transport::Response;
use qqmusic_api::transport::mock::MockTransport;
use serde_json::{Value, json};

use crate::protocol::Track;

pub(crate) fn client(mock: &MockTransport) -> Client {
    Client::builder()
        .transport(mock.clone())
        .device(Device::generate(Some(DeviceProfile::Vivo), Some(7)))
        .qimei(QimeiMode::Fixed(Qimei { q16: "q16".into(), q36: "q36".into() }))
        .android_session(false)
        .rate_limit(None)
        .build()
        .unwrap()
}

pub(crate) fn reply(data: Value) -> Response {
    Response::json(&json!({"code": 0, "req_0": {"code": 0, "data": data}}))
}

pub(crate) fn track(mid: &str) -> Track {
    Track {
        id: 1,
        mid: mid.into(),
        media_mid: mid.into(),
        song_type: 0,
        name: mid.into(),
        singers: String::new(),
        album: String::new(),
        cover: String::new(),
        duration: 180,
        vip: false,
    }
}
