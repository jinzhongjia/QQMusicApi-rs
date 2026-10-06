//! CGI envelope / HTTP response decoding.

use indexmap::IndexMap;
use serde_json::Value;

use crate::error::{ApiError, ApiErrorKind, Error, Result};
use crate::transport::Response;

/// Error codes for which a CGI sub-response is *not* treated as an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AllowErrorCodes {
    /// Accept every code.
    All,
    /// Accept the listed codes.
    Codes(Vec<i64>),
}

impl AllowErrorCodes {
    /// Whether `code` is accepted.
    pub fn contains(&self, code: i64) -> bool {
        match self {
            Self::All => true,
            Self::Codes(codes) => codes.contains(&code),
        }
    }
}

impl<const N: usize> From<[i64; N]> for AllowErrorCodes {
    fn from(codes: [i64; N]) -> Self {
        Self::Codes(codes.to_vec())
    }
}

impl From<Vec<i64>> for AllowErrorCodes {
    fn from(codes: Vec<i64>) -> Self {
        Self::Codes(codes)
    }
}

/// Snapshot of a raw HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPayload {
    /// Status code.
    pub status: u16,
    /// Final URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// Cookies set by the response.
    pub cookies: IndexMap<String, String>,
    /// Raw body.
    pub content: Vec<u8>,
}

impl RawPayload {
    /// Build from a transport response.
    pub fn from_response(response: Response) -> Self {
        let cookies = response.cookies();
        Self {
            status: response.status,
            url: response.url,
            headers: response.headers,
            cookies,
            content: response.body,
        }
    }

    /// Body as text (lossy UTF-8).
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.content).into_owned()
    }

    /// Body parsed as JSON.
    pub fn json(&self) -> Result<Value> {
        serde_json::from_slice(&self.content)
            .map_err(|e| Error::api_data(format!("响应内容非有效 JSON 格式: {e}")))
    }

    /// First header value (case insensitive).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Fail on error responses.
///
/// Like `requests`' `raise_for_status`, only `4xx`/`5xx` are errors so that
/// redirect responses (`allow_redirects=False`) can be inspected.
pub fn ensure_http_success(response: &Response) -> Result<()> {
    if response.status < 400 {
        Ok(())
    } else {
        Err(Error::Http {
            status: response.status,
            message: format!("HTTP 请求状态码异常: {}", response.status),
        })
    }
}

fn code_of(value: Option<&Value>) -> Option<Option<i64>> {
    match value {
        None => Some(Some(0)),
        Some(Value::Number(n)) => Some(n.as_i64()),
        Some(_) => None,
    }
}

fn type_name(value: Option<&Value>) -> &'static str {
    match value {
        None | Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "bool",
        Some(Value::Number(_)) => "float",
        Some(Value::String(_)) => "str",
        Some(Value::Array(_)) => "list",
        Some(Value::Object(_)) => "dict",
    }
}

/// Unwrap a `musicu.fcg` envelope into its `req_<i>` items.
pub fn unwrap_cgi_envelope(response: &Response, expected_count: usize) -> Result<Vec<Option<Value>>> {
    if response.status != 200 {
        return Err(Error::Http {
            status: response.status,
            message: format!("HTTP 请求状态码异常: {}", response.status),
        });
    }
    if response.body.is_empty() {
        return Err(Error::api_data("响应无内容"));
    }
    let payload: Value =
        serde_json::from_slice(&response.body).map_err(|_| Error::api_data("响应内容非有效 JSON 格式"))?;
    let Value::Object(mut map) = payload else {
        return Err(Error::api_data("响应内容非 JSON 对象"));
    };
    let code = match code_of(map.get("code")) {
        Some(Some(code)) => code,
        _ => {
            let name = type_name(map.get("code"));
            return Err(Error::api_data_with(
                format!("CGI 外层 code 类型异常: {name}"),
                Value::Object(map),
            ));
        }
    };
    if code != 0 {
        return Err(ApiError::new(ApiErrorKind::Global, code, Value::String(response.text()))
            .with_message("Module 请求失败")
            .into());
    }
    Ok((0..expected_count)
        .map(|i| match map.shift_remove(&format!("req_{i}")) {
            Some(item @ Value::Object(_)) => Some(item),
            _ => None,
        })
        .collect())
}

/// Validate a CGI sub-response and extract its payload.
///
/// Returns `data` on success. When the code is allowed by
/// `allow_error_codes`, returns `data` if `parse_on_allow` is set and the
/// whole raw item otherwise.
pub fn parse_cgi_item(
    raw: Value,
    allow_error_codes: Option<&AllowErrorCodes>,
    parse_on_allow: bool,
) -> Result<Value> {
    let code = match code_of(raw.get("code")) {
        Some(Some(code)) => code,
        _ => {
            let name = type_name(raw.get("code"));
            return Err(Error::api_data_with(format!("CGI 子响应 code 类型异常: {name}"), raw));
        }
    };
    let take_data = |raw: Value| match raw {
        Value::Object(mut map) => map
            .shift_remove("data")
            .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        _ => Value::Object(serde_json::Map::new()),
    };
    if allow_error_codes.is_some_and(|allow| allow.contains(code)) {
        return Ok(if parse_on_allow { take_data(raw) } else { raw });
    }
    if code != 0 {
        return Err(ApiError::from_cgi_code(code, take_data(raw)).into());
    }
    Ok(take_data(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resp(value: Value) -> Response {
        Response::json(&value)
    }

    #[test]
    fn envelope_success() {
        let items = unwrap_cgi_envelope(
            &resp(json!({"code": 0, "req_0": {"code": 0, "data": {"a": 1}}, "req_1": "bad"})),
            3,
        )
        .unwrap();
        assert_eq!(items.len(), 3);
        assert!(items[0].is_some());
        assert!(items[1].is_none());
        assert!(items[2].is_none());
        // missing outer code means success
        assert!(unwrap_cgi_envelope(&resp(json!({"req_0": {}})), 1).is_ok());
    }

    #[test]
    fn envelope_errors() {
        let err = unwrap_cgi_envelope(&Response::new(502, "x"), 1).unwrap_err();
        assert!(matches!(err, Error::Http { status: 502, .. }));
        let err = unwrap_cgi_envelope(&Response::new(200, ""), 1).unwrap_err();
        assert_eq!(err.to_string(), "API Data Error: 响应无内容");
        let err = unwrap_cgi_envelope(&Response::new(200, "nope"), 1).unwrap_err();
        assert!(err.to_string().contains("非有效 JSON"));
        let err = unwrap_cgi_envelope(&resp(json!([1])), 1).unwrap_err();
        assert!(err.to_string().contains("非 JSON 对象"));
        let err = unwrap_cgi_envelope(&resp(json!({"code": "0"})), 1).unwrap_err();
        assert!(err.to_string().contains("code 类型异常: str"));
        let err = unwrap_cgi_envelope(&resp(json!({"code": 500})), 1).unwrap_err();
        let api = err.as_api().unwrap();
        assert_eq!(api.kind, ApiErrorKind::Global);
        assert_eq!(api.code, 500);
        assert_eq!(api.message, "Module 请求失败");
    }

    #[test]
    fn item_parsing() {
        assert_eq!(
            parse_cgi_item(json!({"code": 0, "data": {"x": 1}}), None, false).unwrap(),
            json!({"x": 1})
        );
        assert_eq!(parse_cgi_item(json!({}), None, false).unwrap(), json!({}));
        let err = parse_cgi_item(json!({"code": 2001, "data": {"feedbackURL": "u"}}), None, false).unwrap_err();
        assert!(err.is_ratelimited());
        let err = parse_cgi_item(json!({"code": 1000}), None, false).unwrap_err();
        assert!(err.is_credential_expired());
        let err = parse_cgi_item(json!({"code": 1.5}), None, false).unwrap_err();
        assert!(err.to_string().contains("子响应 code 类型异常"));
    }

    #[test]
    fn allowed_codes() {
        let raw = json!({"code": 10007, "data": {"midurlinfo": []}});
        let allow = AllowErrorCodes::from([10007]);
        assert_eq!(parse_cgi_item(raw.clone(), Some(&allow), false).unwrap(), raw);
        assert_eq!(
            parse_cgi_item(raw.clone(), Some(&allow), true).unwrap(),
            json!({"midurlinfo": []})
        );
        assert_eq!(parse_cgi_item(raw.clone(), Some(&AllowErrorCodes::All), false).unwrap(), raw);
        assert!(parse_cgi_item(raw, Some(&AllowErrorCodes::from(vec![1])), false).is_err());
        assert!(AllowErrorCodes::All.contains(5));
    }

    #[test]
    fn raw_payload_helpers() {
        let response = Response::new(200, br#"{"a":1}"#.to_vec()).with_header("Set-Cookie", "k=v; Path=/");
        let payload = RawPayload::from_response(response);
        assert_eq!(payload.cookies["k"], "v");
        assert_eq!(payload.json().unwrap(), json!({"a": 1}));
        assert_eq!(payload.text(), r#"{"a":1}"#);
        assert_eq!(payload.header("SET-COOKIE"), Some("k=v; Path=/"));
        assert!(ensure_http_success(&Response::new(403, "")).is_err());
    }

    #[test]
    fn http_status_semantics() {
        assert!(ensure_http_success(&Response::new(200, "")).is_ok());
        assert!(ensure_http_success(&Response::new(302, "")).is_ok());
        assert!(matches!(
            ensure_http_success(&Response::new(404, "")),
            Err(Error::Http { status: 404, .. })
        ));
        assert!(ensure_http_success(&Response::new(503, "")).is_err());
    }
}
