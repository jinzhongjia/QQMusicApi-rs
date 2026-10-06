//! Error types.

use std::fmt;

use serde_json::Value;

use crate::json::JsonError;

/// Convenient result alias.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Login specific error categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginErrorKind {
    /// Generic login failure.
    Generic,
    /// Login parameters invalid or expired (`1000`, `104400`, `104401`).
    AuthExpired,
    /// Too many devices (`20279`).
    DeviceLimit,
    /// Account restricted (`20277`, `20278`, `20450`).
    AccountRestricted,
    /// Rate limited (`104604`).
    RateLimit,
}

impl LoginErrorKind {
    /// Map a login API error code.
    pub fn from_code(code: i64) -> Self {
        match code {
            1000 | 104_400 | 104_401 => Self::AuthExpired,
            20_279 => Self::DeviceLimit,
            20_277 | 20_278 | 20_450 => Self::AccountRestricted,
            104_604 => Self::RateLimit,
            _ => Self::Generic,
        }
    }

    fn default_message(self) -> &'static str {
        match self {
            Self::Generic => "登录失败",
            Self::AuthExpired => "登录鉴权参数无效或已过期",
            Self::DeviceLimit => "登录设备超限",
            Self::AccountRestricted => "账号受限",
            Self::RateLimit => "操作过于频繁",
        }
    }
}

/// Category of an API level error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiErrorKind {
    /// The gateway rejected the whole batch (outer `code != 0`).
    Global,
    /// Generic CGI error for a single request.
    Cgi,
    /// Login credential expired (`1000`, `104400`, `104401`).
    CredentialExpired,
    /// Credential refresh failed.
    CredentialRefresh,
    /// Risk control triggered (`2001`); login or verification required.
    Ratelimited {
        /// Verification URL returned by the server, if any.
        feedback_url: Option<String>,
    },
    /// The endpoint requires a `zzc` signature (`2000`).
    SignatureRequired,
    /// Login flow error.
    Login(LoginErrorKind),
}

/// Error returned by the QQ Music API itself.
#[derive(Debug, Clone, PartialEq)]
pub struct ApiError {
    /// Error category.
    pub kind: ApiErrorKind,
    /// Business error code.
    pub code: i64,
    /// Human readable message.
    pub message: String,
    /// Raw `data` payload attached to the error.
    pub data: Value,
}

impl ApiError {
    /// Build an error with the default message of its kind.
    pub fn new(kind: ApiErrorKind, code: i64, data: Value) -> Self {
        let message = match &kind {
            ApiErrorKind::Global => format!("请求被网关拒绝 (code={code})"),
            ApiErrorKind::Cgi => format!("CGI 请求错误 (code={code})"),
            ApiErrorKind::CredentialExpired => "登录凭证已过期, 请重新登录".to_string(),
            ApiErrorKind::CredentialRefresh => "登录凭证刷新失败".to_string(),
            ApiErrorKind::Ratelimited { .. } => "触发风控, 需登录或者安全验证".to_string(),
            ApiErrorKind::SignatureRequired => "请求需要签名".to_string(),
            ApiErrorKind::Login(kind) => kind.default_message().to_string(),
        };
        Self { kind, code, message, data }
    }

    /// Override the message.
    #[must_use]
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Map a CGI sub-response code to a typed error.
    pub fn from_cgi_code(code: i64, data: Value) -> Self {
        let kind = match code {
            2000 => ApiErrorKind::SignatureRequired,
            2001 => ApiErrorKind::Ratelimited {
                feedback_url: data.get("feedbackURL").and_then(Value::as_str).map(str::to_string),
            },
            1000 | 104_400 | 104_401 => ApiErrorKind::CredentialExpired,
            _ => ApiErrorKind::Cgi,
        };
        Self::new(kind, code, data)
    }

    /// Build a login error from a login API code.
    pub fn login(code: i64, data: Value) -> Self {
        Self::new(ApiErrorKind::Login(LoginErrorKind::from_code(code)), code, data)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

/// Transport level failure.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct TransportError {
    /// Description of the failure.
    pub message: String,
    /// Whether the failure was a timeout.
    pub timeout: bool,
    /// Whether the failure happened while connecting (safe to retry).
    pub connect: bool,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl TransportError {
    /// Generic transport error.
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), timeout: false, connect: false, source: None }
    }

    /// Timeout error.
    pub fn timeout(message: impl Into<String>) -> Self {
        Self { timeout: true, ..Self::new(message) }
    }

    /// Connection error.
    pub fn connect(message: impl Into<String>) -> Self {
        Self { connect: true, ..Self::new(message) }
    }

    /// Attach the underlying error.
    #[must_use]
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }
}

/// Library error.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request requires a valid login credential.
    #[error("{0}")]
    CredentialInvalid(String),
    /// Network failure.
    #[error("network error: {0}")]
    Network(#[from] TransportError),
    /// Unexpected HTTP status.
    #[error("HTTP {status}: {message}")]
    Http {
        /// Status code (`0` when unknown).
        status: u16,
        /// Description.
        message: String,
    },
    /// Malformed response payload.
    #[error("API Data Error: {message}")]
    ApiData {
        /// Description.
        message: String,
        /// Offending payload.
        data: Option<Box<Value>>,
    },
    /// Error reported by the QQ Music API.
    #[error(transparent)]
    Api(Box<ApiError>),
    /// The response could not be converted into the requested model.
    #[error("response model error: {0}")]
    Model(Box<JsonError>),
    /// Invalid argument supplied by the caller.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// Lyric decryption error.
    #[error(transparent)]
    Qrc(#[from] crate::algorithms::QrcError),
    /// Local IO failure (device persistence).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// The client has been closed.
    #[error("client is closed")]
    Closed,
}

impl From<ApiError> for Error {
    fn from(err: ApiError) -> Self {
        Self::Api(Box::new(err))
    }
}

impl From<JsonError> for Error {
    fn from(err: JsonError) -> Self {
        Self::Model(Box::new(err))
    }
}

impl Error {
    /// Malformed response helper.
    pub fn api_data(message: impl Into<String>) -> Self {
        Self::ApiData { message: message.into(), data: None }
    }

    /// Malformed response helper with payload.
    pub fn api_data_with(message: impl Into<String>, data: Value) -> Self {
        Self::ApiData { message: message.into(), data: Some(Box::new(data)) }
    }

    /// Invalid argument helper.
    pub fn invalid_argument(message: impl Into<String>) -> Self {
        Self::InvalidArgument(message.into())
    }

    /// API error code, if this is an API error.
    pub fn code(&self) -> Option<i64> {
        match self {
            Self::Api(err) => Some(err.code),
            _ => None,
        }
    }

    /// The underlying API error, if any.
    pub fn as_api(&self) -> Option<&ApiError> {
        match self {
            Self::Api(err) => Some(err),
            _ => None,
        }
    }

    /// Whether the credential expired (re-login or refresh required).
    pub fn is_credential_expired(&self) -> bool {
        matches!(
            self.as_api().map(|e| &e.kind),
            Some(ApiErrorKind::CredentialExpired | ApiErrorKind::Login(LoginErrorKind::AuthExpired))
        )
    }

    /// Whether risk control was triggered.
    pub fn is_ratelimited(&self) -> bool {
        matches!(self.as_api().map(|e| &e.kind), Some(ApiErrorKind::Ratelimited { .. }))
    }

    /// Whether this is a network timeout.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Network(err) if err.timeout)
    }

    /// Best-effort copy (error sources are flattened into messages).
    ///
    /// Used when one failure (e.g. a failed batch request) has to be
    /// reported for several requests.
    pub fn duplicate(&self) -> Self {
        match self {
            Self::CredentialInvalid(msg) => Self::CredentialInvalid(msg.clone()),
            Self::Network(err) => Self::Network(TransportError {
                message: err.message.clone(),
                timeout: err.timeout,
                connect: err.connect,
                source: None,
            }),
            Self::Http { status, message } => Self::Http { status: *status, message: message.clone() },
            Self::ApiData { message, data } => Self::ApiData { message: message.clone(), data: data.clone() },
            Self::Api(err) => Self::Api(err.clone()),
            Self::Model(err) => Self::Model(err.clone()),
            Self::InvalidArgument(msg) => Self::InvalidArgument(msg.clone()),
            Self::Qrc(err) => Self::api_data(err.to_string()),
            Self::Io(err) => Self::Io(std::io::Error::new(err.kind(), err.to_string())),
            Self::Closed => Self::Closed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cgi_code_mapping() {
        assert_eq!(ApiError::from_cgi_code(2000, json!({})).kind, ApiErrorKind::SignatureRequired);
        let rl = ApiError::from_cgi_code(2001, json!({"feedbackURL": "https://x"}));
        assert_eq!(rl.kind, ApiErrorKind::Ratelimited { feedback_url: Some("https://x".into()) });
        assert_eq!(rl.to_string(), "触发风控, 需登录或者安全验证");
        for code in [1000, 104_400, 104_401] {
            assert_eq!(ApiError::from_cgi_code(code, Value::Null).kind, ApiErrorKind::CredentialExpired);
        }
        let generic = ApiError::from_cgi_code(500, Value::Null);
        assert_eq!(generic.kind, ApiErrorKind::Cgi);
        assert_eq!(generic.to_string(), "CGI 请求错误 (code=500)");
    }

    #[test]
    fn login_code_mapping() {
        assert_eq!(LoginErrorKind::from_code(20_279), LoginErrorKind::DeviceLimit);
        assert_eq!(LoginErrorKind::from_code(20_450), LoginErrorKind::AccountRestricted);
        assert_eq!(LoginErrorKind::from_code(104_604), LoginErrorKind::RateLimit);
        assert_eq!(LoginErrorKind::from_code(1), LoginErrorKind::Generic);
        assert_eq!(ApiError::login(20_279, Value::Null).message, "登录设备超限");
    }

    #[test]
    fn error_helpers() {
        let err: Error = ApiError::from_cgi_code(1000, Value::Null).into();
        assert!(err.is_credential_expired());
        assert_eq!(err.code(), Some(1000));
        assert!(!err.is_ratelimited());
        let err: Error = TransportError::timeout("slow").into();
        assert!(err.is_timeout());
        assert_eq!(err.code(), None);
        assert_eq!(Error::api_data("x").to_string(), "API Data Error: x");
        let global = ApiError::new(ApiErrorKind::Global, 500, Value::Null);
        assert_eq!(global.to_string(), "请求被网关拒绝 (code=500)");
    }
}
