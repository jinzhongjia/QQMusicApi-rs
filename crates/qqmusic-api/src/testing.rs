//! Test helpers (only compiled for tests).

use serde_json::{Map, Value, json};

use crate::client::{Client, QimeiMode};
use crate::credential::Credential;
use crate::device::{Device, DeviceProfile, Qimei};
use crate::transport::mock::MockTransport;
use crate::transport::{Request, Response};

/// Client backed by a mock transport (no QIMEI/session/rate limit).
pub(crate) fn mock_client() -> (Client, MockTransport) {
    let mock = MockTransport::new();
    let client = Client::builder()
        .transport(mock.clone())
        .device(Device::generate(Some(DeviceProfile::Vivo), Some(1)))
        .qimei(QimeiMode::Fixed(Qimei {
            q16: "q16".into(),
            q36: "q36".into(),
        }))
        .android_session(false)
        .rate_limit(None)
        .build()
        .expect("client");
    (client, mock)
}

/// Mock client with a logged-in credential.
pub(crate) fn logged_in_client() -> (Client, MockTransport) {
    let (client, mock) = mock_client();
    client.set_credential(Credential::new(10_001, "Q_H_L_key"));
    (client, mock)
}

/// CGI envelope answering every `req_N` with `data`.
pub(crate) fn reply_all(data: Value) -> impl Fn(&Request) -> Result<Response, crate::transport::TransportError> + Send + Sync + 'static {
    move |req: &Request| {
        let body = req.json_body().unwrap_or_default();
        let mut out = Map::new();
        out.insert("code".into(), json!(0));
        if let Some(obj) = body.as_object() {
            for key in obj.keys().filter(|k| k.starts_with("req_")) {
                out.insert(key.clone(), json!({"code": 0, "data": data.clone()}));
            }
        }
        Ok(Response::json(&Value::Object(out)))
    }
}

/// Queue a single successful CGI response.
pub(crate) fn push_cgi(mock: &MockTransport, data: Value) {
    mock.push_response(Response::json(&json!({"code": 0, "req_0": {"code": 0, "data": data}})));
}

/// Queue a single CGI sub-response with an error code.
pub(crate) fn push_cgi_code(mock: &MockTransport, code: i64, data: Value) {
    mock.push_response(Response::json(&json!({"code": 0, "req_0": {"code": code, "data": data}})));
}

/// `req_0` of the last request.
pub(crate) fn last_req0(mock: &MockTransport) -> Value {
    last_body(mock)["req_0"].clone()
}

/// JSON body of the last request.
pub(crate) fn last_body(mock: &MockTransport) -> Value {
    mock.last_request()
        .and_then(|r| r.json_body())
        .expect("last request with JSON body")
}
