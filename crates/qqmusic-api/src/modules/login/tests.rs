use std::sync::Arc;

use futures::StreamExt;
use serde_json::json;

use super::*;
use crate::client::{Client, QimeiMode};
use crate::device::{Device, DeviceProfile, Qimei};
use crate::error::{LoginErrorKind, TransportError};
use crate::mqtt::codec::{encode_connack_for_test, encode_publish_for_test, encode_suback_for_test};
use crate::mqtt::mock::MockConnector;
use crate::testing::*;
use crate::transport::Response;
use crate::transport::mock::MockTransport;

fn login_data(musicid: i64, key: &str) -> Value {
    json!({"musicid": musicid, "musickey": key, "encryptUin": "enc", "refresh_key": "rk"})
}

fn api_kind(err: &Error) -> &ApiErrorKind {
    match err {
        Error::Api(api) => &api.kind,
        other => panic!("unexpected error {other:?}"),
    }
}

fn qq_qr(sig: &str) -> QrCode {
    QrCode {
        data: vec![],
        qr_type: QrLoginType::Qq,
        mimetype: "image/png".into(),
        identifier: sig.into(),
    }
}

fn wx_qr() -> QrCode {
    QrCode {
        qr_type: QrLoginType::Wx,
        mimetype: "image/jpeg".into(),
        ..qq_qr("uuid1")
    }
}

fn ptui(code: &str) -> Response {
    Response::new(200, format!("ptuiCB('{code}','0','','0','msg', '')"))
}

#[test]
fn validate_result_codes() {
    assert_eq!(validate_login_result(json!({"musicid": 1})).unwrap(), json!({"musicid": 1}));
    let cases = [
        (1000, LoginErrorKind::AuthExpired, None),
        (20_261, LoginErrorKind::Generic, Some("登录参数错误")),
        (20_271, LoginErrorKind::Generic, Some("验证码错误")),
        (20_272, LoginErrorKind::Generic, Some("账号绑定异常")),
        (20_274, LoginErrorKind::Generic, Some("账号绑定缺失")),
        (20_277, LoginErrorKind::AccountRestricted, None),
        (20_279, LoginErrorKind::DeviceLimit, None),
        (20_450, LoginErrorKind::AccountRestricted, Some("账号已被封禁")),
        (104_604, LoginErrorKind::RateLimit, None),
        (5, LoginErrorKind::Generic, None),
    ];
    for (code, kind, message) in cases {
        let err = validate_login_result(json!({"code": code, "data": {"x": 1}})).unwrap_err();
        let Error::Api(api) = err else { panic!() };
        assert_eq!(api.kind, ApiErrorKind::Login(kind), "code {code}");
        assert_eq!(api.code, code);
        assert_eq!(api.data, json!({"x": 1}));
        if let Some(message) = message {
            assert_eq!(api.message, message);
        }
    }
}

#[test]
fn text_parsers() {
    assert_eq!(quoted_args(r"'a','b\'c', 'd'"), vec!["a", r"b\'c", "d"]);
    assert_eq!(quoted_args("'open"), Vec::<String>::new());
    let url = "https://x/check_sig?pttype=1&uin=123&service=ptqrlogin&ptsigx=SIG&s_url=y";
    assert_eq!(capture(url, "uin=", "&service", &['?', '&']), Some("123"));
    assert_eq!(capture(url, "ptsigx=", "&s_url", &['?', '&']), Some("SIG"));
    assert_eq!(capture("?ptuin=9&uin=1&service", "uin=", "&service", &['?', '&']), Some("1"));
    assert_eq!(capture("a?uin=&service", "uin=", "&service", &['?', '&']), None);
    assert_eq!(capture(r#"src="/connect/qrcode/x?uuid=AB12" "#, "uuid=", "\"", &[]), Some("AB12"));
    assert_eq!(capture("https://y.qq.com/?login_type=1&code=XY&state=s", "code=", "&", &[]), Some("XY"));
    let args = parse_ptui_cb("ptuiCB('66','0','','0','二维码未失效', '')").unwrap();
    assert_eq!(args[0], "66");
    assert!(parse_ptui_cb("nothing").is_none());
    assert_eq!(parse_wx_status("window.wx_errcode=405;window.wx_code='C1';"), Some(("405", "C1")));
    assert_eq!(parse_wx_status("window.wx_errcode=408;window.wx_code='';"), Some(("408", "")));
    assert!(parse_wx_status("window.wx_errcode=;window.wx_code='';").is_none());
    assert!(event_from_code("999").is_err());
    let interval = PollInterval::default();
    assert_eq!(interval.scanned_interval(), Duration::from_millis(750));
    assert_eq!(interval.error_interval(), Duration::from_secs(3));
}

#[tokio::test]
async fn check_expired_cgi_and_web() {
    let (client, mock) = logged_in_client();
    push_cgi(&mock, json!({"nick": "n"}));
    assert!(!client.login().check_expired(None).await.unwrap());
    let req = last_req0(&mock);
    assert_eq!(req["module"], "music.UserInfo.userInfoServer");
    push_cgi_code(&mock, 1000, json!({}));
    assert!(client.login().check_expired(None).await.unwrap());

    client.set_platform(Platform::Web);
    mock.push_json(json!({"code": 0}));
    let cred = Credential::new(7, "key");
    assert!(!client.login().check_expired(Some(cred.clone())).await.unwrap());
    let request = mock.last_request().unwrap();
    assert!(request.url.contains("fcg_get_profile_homepage.fcg"));
    assert_eq!(request.query_param("loginUin"), Some("7"));
    assert_eq!(request.query_param("g_tk"), Some(hash33("key", 5381).to_string().as_str()));
    assert_eq!(request.cookies()["qm_keyst"], "key");
    mock.push_json(json!({"code": 1000}));
    assert!(client.login().check_expired(Some(cred)).await.unwrap());
}

#[tokio::test]
async fn refresh_and_logout() {
    let (client, mock) = mock_client();
    let mut cred = Credential::new(10, "W_X_old");
    cred.login_type = 1;
    cred.openid = "oid".into();
    client.set_credential(cred);
    push_cgi(&mock, login_data(10, "W_X_new"));
    let refreshed = client.login().refresh_credential(None).await.unwrap();
    assert_eq!(refreshed.musickey, "W_X_new");
    assert_eq!(client.credential().musickey, "W_X_new");
    let body = last_body(&mock);
    assert_eq!(body["comm"]["tmeLoginType"], "1");
    assert_eq!(
        body["req_0"]["param"],
        json!({"openid": "oid", "refresh_token": "", "str_musicid": "10", "musickey": "W_X_old", "unionid": "", "refresh_key": "", "loginMode": 2})
    );

    // Explicit QQ credential: param shape for type 2, client untouched.
    let mut qq = Credential::new(20, "Q_H_L_k");
    qq.login_type = 2;
    push_cgi(&mock, login_data(20, "Q_H_L_n"));
    client.login().refresh_credential(Some(qq.clone())).await.unwrap();
    assert_eq!(client.credential().musicid, 10);
    let param = last_req0(&mock)["param"].clone();
    assert_eq!(param["musicid"], 20);
    assert!(param.get("unionid").is_none());
    assert!(param.get("expired_in").is_some());

    push_cgi_code(&mock, 20_271, json!({}));
    let err = client.login().refresh_credential(Some(qq.clone())).await.unwrap_err();
    assert_eq!(api_kind(&err), &ApiErrorKind::CredentialRefresh);
    let Error::Api(api) = err else { panic!() };
    assert_eq!((api.code, api.message.as_str()), (20_271, "验证码错误"));

    // Other login types send every field.
    qq.login_type = 0;
    push_cgi(&mock, login_data(20, "k"));
    client.login().refresh_credential(Some(qq)).await.unwrap();
    let param = last_req0(&mock)["param"].clone();
    assert!(param.get("unionid").is_some() && param.get("musicid").is_some());

    push_cgi_code(&mock, 1000, json!({}));
    client.login().logout(None).await.unwrap();
    assert_eq!(last_req0(&mock)["method"], "Logout");
    assert!(!client.credential().is_valid());
}

#[tokio::test]
async fn qq_qrcode_flow() {
    let (client, mock) = mock_client();
    mock.push_response(Response::new(200, vec![0x89, b'P']).with_header("Set-Cookie", "qrsig=SIG; Path=/"));
    let qr = client.login().get_qrcode(QrLoginType::Qq).await.unwrap();
    assert_eq!((qr.identifier.as_str(), qr.data.as_slice()), ("SIG", [0x89, b'P'].as_slice()));
    let request = mock.last_request().unwrap();
    assert_eq!(request.query_param("appid"), Some(QQ_APPID));
    assert_eq!(request.header("referer"), Some(PTLOGIN_REFERER));
    mock.push_response(Response::new(200, ""));
    assert!(client.login().get_qrcode(QrLoginType::Qq).await.is_err());

    for (code, event) in [("66", QrCodeLoginEvent::Scan), ("67", QrCodeLoginEvent::Conf), ("65", QrCodeLoginEvent::Timeout), ("68", QrCodeLoginEvent::Refuse)] {
        mock.push_response(ptui(code));
        assert_eq!(client.login().check_qrcode(&qr).await.unwrap().event, event);
    }
    let request = mock.last_request().unwrap();
    assert_eq!(request.query_param("ptqrtoken"), Some(hash33("SIG", 0).to_string().as_str()));
    assert_eq!(request.cookies()["qrsig"], "SIG");

    mock.push_response(Response::new(400, ""));
    assert!(matches!(client.login().check_qrcode(&qr).await, Err(Error::ApiData { ref message, .. }) if message == "无效 qrsig"));
    mock.push_response(Response::new(200, "garbage"));
    assert!(client.login().check_qrcode(&qr).await.is_err());
    mock.push_response(ptui("x"));
    assert!(client.login().check_qrcode(&qr).await.is_err());

    // Successful scan -> check_sig -> authorize -> QQLogin.
    mock.push_response(Response::new(
        200,
        "ptuiCB('0','0','https://ssl.ptlogin2.graph.qq.com/check_sig?pttype=1&uin=123&service=ptqrlogin&nodirect=0&ptsigx=SIGX&s_url=https%3A%2F%2Fgraph.qq.com','0','登录成功！', 'nick')",
    ));
    mock.push_response(
        Response::new(302, "")
            .with_header("Set-Cookie", "p_skey=PSKEY; Path=/")
            .with_header("Set-Cookie", "p_uin=o123; Path=/"),
    );
    mock.push_response(Response::new(302, "").with_header(
        "Location",
        "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/&code=AUTHCODE&state=state",
    ));
    push_cgi(&mock, login_data(123, "Q_H_L_x"));
    let result = client.login().check_qrcode(&qr).await.unwrap();
    assert!(result.is_done());
    assert_eq!(result.credential.unwrap().musicid, 123);
    let requests = mock.requests();
    let n = requests.len();
    let check_sig = &requests[n - 3];
    assert_eq!(check_sig.query_param("ptsigx"), Some("SIGX"));
    assert_eq!(check_sig.query_param("uin"), Some("123"));
    assert!(!check_sig.follow_redirects);
    let authorize = &requests[n - 2];
    assert_eq!(authorize.method, Method::Post);
    assert_eq!(authorize.cookies()["p_skey"], "PSKEY");
    let Body::Form(form) = &authorize.body else { panic!("form body") };
    let g_tk = form.iter().find(|(k, _)| k == "g_tk").unwrap();
    assert_eq!(g_tk.1, hash33("PSKEY", 5381).to_string());
    let body = last_body(&mock);
    assert_eq!(body["comm"]["tmeLoginType"], "2");
    assert_eq!(body["req_0"]["param"], json!({"code": "AUTHCODE"}));
    // Client credential is not replaced by QR login.
    assert!(!client.credential().is_valid());
}

#[tokio::test]
async fn qq_authorize_failures() {
    let (client, mock) = mock_client();
    let done = "ptuiCB('0','0','https://x/check_sig?uin=1&service=a&ptsigx=S&s_url=b','0','ok', '')";
    mock.push_response(Response::new(200, done));
    mock.push_response(Response::new(302, ""));
    let err = client.login().check_qrcode(&qq_qr("s")).await.unwrap_err();
    assert!(matches!(err, Error::ApiData { ref message, .. } if message == "获取 p_skey 失败"));

    mock.push_response(Response::new(200, done));
    mock.push_response(Response::new(302, "").with_header("Set-Cookie", "p_skey=k"));
    mock.push_response(Response::new(302, "").with_header("Location", "https://y.qq.com/"));
    let err = client.login().check_qrcode(&qq_qr("s")).await.unwrap_err();
    assert!(matches!(err, Error::ApiData { ref message, .. } if message == "获取 code 失败"));

    mock.push_response(Response::new(200, "ptuiCB('0','0')"));
    assert!(client.login().check_qrcode(&qq_qr("s")).await.is_err());
    mock.push_response(Response::new(200, "ptuiCB('0','0','https://x/?a=1','0')"));
    assert!(client.login().check_qrcode(&qq_qr("s")).await.is_err());

    mock.push_response(Response::new(200, done));
    mock.push_response(Response::new(302, "").with_header("Set-Cookie", "p_skey=k"));
    mock.push_response(Response::new(302, "").with_header("Location", "https://y.qq.com/?code=C&x=1"));
    push_cgi_code(&mock, 20_279, json!({}));
    let err = client.login().check_qrcode(&qq_qr("s")).await.unwrap_err();
    assert_eq!(api_kind(&err), &ApiErrorKind::Login(LoginErrorKind::DeviceLimit));
}

#[tokio::test]
async fn wx_qrcode_flow() {
    let (client, mock) = logged_in_client();
    mock.push_response(Response::new(200, r#"<img class="qrcode" src="/connect/qrcode/x?uuid=WXUUID"/>"#));
    mock.push_response(Response::new(200, vec![0xFF, 0xD8]));
    let qr = client.login().get_qrcode(QrLoginType::Wx).await.unwrap();
    assert_eq!((qr.identifier.as_str(), qr.mimetype.as_str()), ("WXUUID", "image/jpeg"));
    assert!(mock.last_request().unwrap().url.ends_with("/connect/qrcode/WXUUID"));
    mock.push_response(Response::new(200, ""));
    assert!(client.login().get_qrcode(QrLoginType::Wx).await.is_err());
    mock.push_response(Response::new(200, "no uuid"));
    assert!(client.login().get_qrcode(QrLoginType::Wx).await.is_err());

    mock.push_response(Response::new(200, "window.wx_errcode=408;window.wx_code='';"));
    assert_eq!(client.login().check_qrcode(&qr).await.unwrap().event, QrCodeLoginEvent::Scan);
    let request = mock.last_request().unwrap();
    assert_eq!(request.timeout, Some(Duration::from_secs(35)));
    assert!(request.header("cookie").is_none(), "anonymous long poll");
    mock.push_error(TransportError::timeout("slow"));
    assert_eq!(client.login().check_qrcode(&qr).await.unwrap().event, QrCodeLoginEvent::Scan);
    mock.push_error(TransportError::connect("down"));
    assert!(matches!(client.login().check_qrcode(&qr).await, Err(Error::Network(_))));
    mock.push_response(Response::new(200, "window.wx_errcode=405;window.wx_code='';"));
    assert!(client.login().check_qrcode(&qr).await.is_err());
    mock.push_response(Response::new(200, "?"));
    assert!(client.login().check_qrcode(&qr).await.is_err());

    mock.push_response(Response::new(200, "window.wx_errcode=405;window.wx_code='WXCODE';"));
    push_cgi(&mock, login_data(9, "W_X_k"));
    let result = client.login().check_qrcode(&qr).await.unwrap();
    assert_eq!(result.credential.unwrap().login_type, 1);
    let body = last_body(&mock);
    assert_eq!(body["comm"]["tmeLoginType"], "1");
    assert_eq!(body["req_0"]["param"], json!({"code": "WXCODE", "strAppid": WX_APPID}));
}

fn mqtt_client(connector: &MockConnector) -> (Client, MockTransport) {
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
        .mqtt_connector(Arc::new(connector.clone()))
        .build()
        .unwrap();
    (client, mock)
}

fn mqtt_ready() -> Vec<u8> {
    let mut chunk = encode_connack_for_test(0, &Properties::default());
    chunk.extend(encode_suback_for_test(1, &[0]));
    chunk
}

#[tokio::test]
async fn mobile_qrcode_flow() {
    let connector = MockConnector::default();
    let (client, mock) = mqtt_client(&connector);
    push_cgi(&mock, json!({"qrcode": "data:image/png;base64,iVBORw==", "qrcodeID": "QRID"}));
    let qr = client.login().get_qrcode(QrLoginType::Mobile).await.unwrap();
    assert_eq!(qr.identifier, "QRID");
    assert_eq!(qr.data, vec![0x89, b'P', b'N', b'G']);
    let body = last_body(&mock);
    assert_eq!((body["comm"]["ct"].as_str(), body["comm"]["cv"].as_str()), (Some("23"), Some("0")));
    assert_eq!(body["req_0"]["param"]["tmeAppID"], "qqmusic");
    push_cgi(&mock, json!({"qrcode": "", "qrcodeID": "x"}));
    assert!(client.login().get_qrcode(QrLoginType::Mobile).await.is_err());

    let cookies = json!({"cookies": {"qqmusic_uin": {"value": "555"}, "qqmusic_key": {"value": "Q_H_L_m"}}});
    connector.push_script(vec![
        mqtt_ready(),
        encode_publish_for_test("t", b"{}", 0, None, &[("type", "scanned")]),
        encode_publish_for_test("t", b"{}", 0, None, &[("type", "other")]),
        encode_publish_for_test("t", cookies.to_string().as_bytes(), 0, None, &[("type", "cookies")]),
    ]);
    push_cgi(&mock, login_data(555, "Q_H_L_m"));
    let events: Vec<_> = client.login().mobile_qrcode_events(&qr, None).collect().await;
    let kinds: Vec<_> = events.iter().map(|e| e.as_ref().unwrap().event).collect();
    assert_eq!(kinds, vec![QrCodeLoginEvent::Scan, QrCodeLoginEvent::Conf, QrCodeLoginEvent::Done]);
    let done = events.last().unwrap().as_ref().unwrap();
    assert_eq!(done.credential.as_ref().unwrap().musicid, 555);
    let body = last_body(&mock);
    assert_eq!(body["comm"]["tmeLoginType"], "6");
    assert_eq!(body["req_0"]["param"], json!({"musicid": 555, "qrCodeID": "QRID", "token": "Q_H_L_m"}));

    let target = connector.targets.lock().unwrap()[0].clone();
    assert_eq!(target.url(), "wss://mu.y.qq.com/ws/handshake");
    assert!(target.headers.contains(&("Origin".into(), "https://y.qq.com".into())));
    let sent = connector.sent();
    let connect = String::from_utf8_lossy(&sent[0]);
    assert!(connect.contains("pass") && connect.contains("hashTag") && connect.contains("QRID"));
    let subscribe = String::from_utf8_lossy(&sent[1]);
    assert!(subscribe.contains("management.qrcode_login/QRID") && subscribe.contains("tmelogin"));
    assert_eq!(sent.last().unwrap(), &vec![0xE0, 0x00], "disconnect after DONE");
}

#[tokio::test]
async fn mobile_qrcode_failures() {
    let connector = MockConnector::default();
    let (client, _mock) = mqtt_client(&connector);
    let qr = QrCode {
        qr_type: QrLoginType::Mobile,
        ..qq_qr("ID")
    };
    connector.push_script(vec![mqtt_ready(), encode_publish_for_test("t", b"{}", 0, None, &[("type", "canceled")])]);
    let events: Vec<_> = client.login().mobile_qrcode_events(&qr, None).collect().await;
    assert_eq!(events.last().unwrap().as_ref().unwrap().event, QrCodeLoginEvent::Refuse);

    connector.push_script(vec![mqtt_ready(), encode_publish_for_test("t", b"x", 0, None, &[("type", "loginFailed")])]);
    let events: Vec<_> = client.login().mobile_qrcode_events(&qr, None).collect().await;
    assert_eq!(events.len(), 2);
    assert!(events[1].is_err());

    connector.push_script(vec![mqtt_ready(), encode_publish_for_test("t", b"[1]", 0, None, &[("type", "cookies")])]);
    let events: Vec<_> = client.login().mobile_qrcode_events(&qr, None).collect().await;
    assert!(matches!(events[1], Err(Error::ApiData { .. })));

    connector.push_script(vec![encode_connack_for_test(0x87, &Properties::default())]);
    let events: Vec<_> = client.login().mobile_qrcode_events(&qr, None).collect().await;
    assert!(matches!(events.as_slice(), [Err(Error::Network(_))]));

    // Past deadline -> immediate TIMEOUT.
    let events: Vec<_> = client.login().mobile_qrcode_events(&qr, Some(Instant::now())).collect().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].as_ref().unwrap().event, QrCodeLoginEvent::Timeout);
}

#[tokio::test(start_paused = true)]
async fn mobile_deadline_while_listening() {
    let connector = MockConnector::default();
    let (client, _mock) = mqtt_client(&connector);
    let qr = QrCode {
        qr_type: QrLoginType::Mobile,
        ..qq_qr("ID")
    };
    connector.push_script(vec![mqtt_ready()]);
    let mut session = client.login().qrcode_session(QrLoginType::Mobile).with_qrcode(qr).timeout(Duration::from_secs(10));
    let events: Vec<_> = session.events().await.unwrap().collect().await;
    let kinds: Vec<_> = events.iter().map(|e| e.as_ref().unwrap().event).collect();
    assert_eq!(kinds, vec![QrCodeLoginEvent::Scan, QrCodeLoginEvent::Timeout]);
}

#[tokio::test]
async fn phone_login() {
    let (client, mock) = mock_client();
    push_cgi(&mock, json!({}));
    let result = client.login().send_authcode(13_800_000_000u64, 86).await.unwrap();
    assert_eq!(result, PhoneAuthCodeResult { event: PhoneLoginEvent::Send, info: None });
    let body = last_body(&mock);
    assert_eq!(body["comm"]["tmeLoginMethod"], "3");
    assert_eq!(body["req_0"]["param"], json!({"tmeAppid": "qqmusic", "areaCode": "86", "phoneNo": "13800000000"}));

    push_cgi_code(&mock, 20_276, json!({"securityURL": "https://captcha"}));
    let result = client.login().send_authcode(PhoneNumber::Encrypted("enc".into()), 852).await.unwrap();
    assert_eq!(result.event, PhoneLoginEvent::Captcha);
    assert_eq!(result.info.as_deref(), Some("https://captcha"));
    assert_eq!(last_req0(&mock)["param"]["encryptedPhoneNo"], "enc");
    push_cgi_code(&mock, 100_001, json!({}));
    assert_eq!(client.login().send_authcode(1u64, 86).await.unwrap().event, PhoneLoginEvent::Frequency);
    push_cgi_code(&mock, 7, json!({}));
    let err = client.login().send_authcode(1u64, 86).await.unwrap_err();
    assert_eq!(api_kind(&err), &ApiErrorKind::Login(LoginErrorKind::Generic));

    let mut session = client.login().phone_session(13_800_000_000u64, 86);
    push_cgi(&mock, json!({}));
    session.send_authcode().await.unwrap();
    assert_eq!(session.last_result.as_ref().unwrap().event, PhoneLoginEvent::Send);
    push_cgi(&mock, login_data(77, "Q_H_L_p"));
    let cred = session.authorize("1234").await.unwrap();
    assert_eq!(cred.musicid, 77);
    assert_eq!(client.credential().musicid, 77);
    let body = last_body(&mock);
    assert_eq!(body["comm"]["tmeLoginType"], "0");
    assert_eq!(body["req_0"]["param"], json!({"code": "1234", "loginMode": 1, "phoneNo": "13800000000"}));
    push_cgi_code(&mock, 20_271, json!({}));
    assert!(session.authorize("0000").await.is_err());
}

#[tokio::test(start_paused = true)]
async fn qr_session_polls_dedupes_and_backs_off() {
    let (client, mock) = mock_client();
    for code in ["66", "66"] {
        mock.push_response(ptui(code));
    }
    mock.push_error(TransportError::connect("blip"));
    mock.push_response(ptui("67"));
    mock.push_response(ptui("68"));
    let mut session = client
        .login()
        .qrcode_session(QrLoginType::Qq)
        .with_qrcode(qq_qr("S"))
        .interval(Duration::from_secs(2));
    let start = Instant::now();
    let events: Vec<_> = session.events().await.unwrap().collect().await;
    let kinds: Vec<_> = events.iter().map(|e| e.as_ref().unwrap().event).collect();
    assert_eq!(kinds, vec![QrCodeLoginEvent::Scan, QrCodeLoginEvent::Conf, QrCodeLoginEvent::Refuse]);
    // 2s + 2s sleeps, 2s back-off, 1s scanned interval.
    assert_eq!(start.elapsed(), Duration::from_secs(7));
    assert_eq!(mock.request_count(), 5);

    // emit_repeat keeps duplicates; wait() maps REFUSE to an error.
    mock.push_response(ptui("66"));
    mock.push_response(ptui("66"));
    mock.push_response(ptui("68"));
    let mut session = client.login().qrcode_session(QrLoginType::Qq).with_qrcode(qq_qr("S")).emit_repeat(true);
    assert_eq!(session.events().await.unwrap().collect::<Vec<_>>().await.len(), 3);
    mock.push_response(ptui("68"));
    let err = session.wait().await.unwrap_err();
    assert!(matches!(err, Error::Api(ref api) if api.message == "用户拒绝了登录请求"));
    mock.push_response(ptui("65"));
    assert!(session.wait().await.is_err());
}

#[tokio::test(start_paused = true)]
async fn qr_session_timeout_and_done() {
    let (client, mock) = mock_client();
    mock.route_url("ptqrlogin", |_| Ok(ptui("66")));
    let mut session = client
        .login()
        .qrcode_session(QrLoginType::Qq)
        .with_qrcode(qq_qr("S"))
        .timeout(Duration::from_secs(5));
    let events: Vec<_> = session.events().await.unwrap().collect().await;
    let kinds: Vec<_> = events.iter().map(|e| e.as_ref().unwrap().event).collect();
    assert_eq!(kinds, vec![QrCodeLoginEvent::Scan, QrCodeLoginEvent::Timeout]);
    let err = session.wait().await.unwrap_err();
    assert!(matches!(err, Error::Api(ref api) if api.message == "登录二维码已超时"));

    let mut zero = client.login().qrcode_session(QrLoginType::Qq).with_qrcode(qq_qr("S")).timeout(Duration::ZERO);
    assert!(matches!(zero.events().await, Err(Error::InvalidArgument(_))));

    // WeChat DONE through wait().
    let (client, mock) = mock_client();
    mock.push_response(Response::new(200, "window.wx_errcode=405;window.wx_code='C';"));
    push_cgi(&mock, login_data(3, "W_X_z"));
    let mut session = client.login().qrcode_session(QrLoginType::Wx).with_qrcode(wx_qr());
    assert_eq!(session.wait().await.unwrap().musicid, 3);
    // get_qrcode reuses the cached code.
    assert_eq!(session.get_qrcode().await.unwrap().identifier, "uuid1");
}
