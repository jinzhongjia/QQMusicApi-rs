//! Lenient JSON → model conversion.
//!
//! QQ Music responses are notoriously inconsistent: the same entity shows up
//! with different key spellings (`singerMid`, `singer_mid`, `mid` …), numbers
//! are sometimes encoded as strings and optional containers are `null`.
//! [`FromJson`] mirrors the lax validation mode of the upstream pydantic
//! models so that every response model can be parsed from a
//! [`serde_json::Value`] with alias priority, JSONPath extraction and
//! defaults (see the `qqmusic_api_derive::FromJson` derive macro).

mod path;

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

pub use path::select as select_path;
pub use qqmusic_api_derive::FromJson;
pub use serde_json::{Map, Value};

/// A location inside a JSON document used for error reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// Object key.
    Field(String),
    /// Array index.
    Index(usize),
}

/// Error kinds produced by [`FromJson`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonErrorKind {
    /// A required field was missing or `null`.
    Missing(String),
    /// The value had an unexpected JSON type.
    InvalidType {
        /// Expected type description.
        expected: &'static str,
        /// Short description of the received value.
        found: String,
    },
    /// Custom conversion error.
    Custom(String),
}

/// Conversion error with the JSON path where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    path: Vec<PathSegment>,
    kind: JsonErrorKind,
}

impl JsonError {
    /// Missing required field.
    pub fn missing(field: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            kind: JsonErrorKind::Missing(field.into()),
        }
    }

    /// Invalid JSON type.
    pub fn invalid_type(expected: &'static str, found: &Value) -> Self {
        Self {
            path: Vec::new(),
            kind: JsonErrorKind::InvalidType {
                expected,
                found: describe(found),
            },
        }
    }

    /// Custom error message.
    pub fn custom(message: impl fmt::Display) -> Self {
        Self {
            path: Vec::new(),
            kind: JsonErrorKind::Custom(message.to_string()),
        }
    }

    /// Prefix the error path with an object key.
    #[must_use]
    pub fn with_field(mut self, field: &str) -> Self {
        self.path.insert(0, PathSegment::Field(field.to_string()));
        self
    }

    /// Prefix the error path with an array index.
    #[must_use]
    pub fn with_index(mut self, index: usize) -> Self {
        self.path.insert(0, PathSegment::Index(index));
        self
    }

    /// Error kind.
    pub fn kind(&self) -> &JsonErrorKind {
        &self.kind
    }

    /// Error location.
    pub fn path(&self) -> &[PathSegment] {
        &self.path
    }

    /// Dotted representation of [`JsonError::path`].
    pub fn path_string(&self) -> String {
        let mut out = String::from("$");
        for segment in &self.path {
            match segment {
                PathSegment::Field(name) => {
                    out.push('.');
                    out.push_str(name);
                }
                PathSegment::Index(index) => {
                    out.push('[');
                    out.push_str(&index.to_string());
                    out.push(']');
                }
            }
        }
        out
    }
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let location = self.path_string();
        match &self.kind {
            JsonErrorKind::Missing(field) => {
                write!(f, "{location}: missing required field `{field}`")
            }
            JsonErrorKind::InvalidType { expected, found } => {
                write!(f, "{location}: expected {expected}, found {found}")
            }
            JsonErrorKind::Custom(message) => write!(f, "{location}: {message}"),
        }
    }
}

impl std::error::Error for JsonError {}

fn describe(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(b) => format!("boolean `{b}`"),
        Value::Number(n) => format!("number `{n}`"),
        Value::String(s) => {
            let mut preview: String = s.chars().take(32).collect();
            if s.chars().count() > 32 {
                preview.push('…');
            }
            format!("string {preview:?}")
        }
        Value::Array(a) => format!("array of length {}", a.len()),
        Value::Object(_) => "object".to_string(),
    }
}

/// Conversion from a JSON value with lenient (pydantic "lax mode") rules.
pub trait FromJson: Sized {
    /// Convert a JSON value.
    fn from_json(value: &Value) -> Result<Self, JsonError>;

    /// Implicit value used when a field *without* a default is missing.
    ///
    /// Only `Option<T>` returns `Some` so that optional fields become `None`.
    fn from_missing() -> Option<Self> {
        None
    }

    /// Value used for fields marked `#[json(default)]`.
    ///
    /// Returns `None` when the type has no sensible default (for derived
    /// models that contain required fields).
    fn json_default() -> Option<Self> {
        None
    }
}

/// Parse any [`FromJson`] type from a JSON value.
pub fn from_value<T: FromJson>(value: &Value) -> Result<T, JsonError> {
    T::from_json(value)
}

/// Parse any [`FromJson`] type from a JSON string.
pub fn from_str<T: FromJson>(input: &str) -> Result<T, JsonError> {
    let value: Value = serde_json::from_str(input).map_err(JsonError::custom)?;
    T::from_json(&value)
}

macro_rules! impl_int {
    ($($ty:ty),*) => {$(
        impl FromJson for $ty {
            fn from_json(value: &Value) -> Result<Self, JsonError> {
                let parsed: Option<i128> = match value {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Some(i128::from(i))
                        } else if let Some(u) = n.as_u64() {
                            Some(i128::from(u))
                        } else {
                            n.as_f64().and_then(float_to_int)
                        }
                    }
                    Value::Bool(b) => Some(i128::from(*b)),
                    Value::String(s) => parse_int_str(s),
                    _ => None,
                };
                parsed
                    .and_then(|v| <$ty>::try_from(v).ok())
                    .ok_or_else(|| JsonError::invalid_type(stringify!($ty), value))
            }

            fn json_default() -> Option<Self> {
                Some(0)
            }
        }
    )*};
}

#[allow(clippy::cast_possible_truncation)]
fn float_to_int(f: f64) -> Option<i128> {
    (f.is_finite() && f.fract() == 0.0).then_some(f as i128)
}

fn parse_int_str(s: &str) -> Option<i128> {
    let trimmed = s.trim();
    if let Ok(v) = trimmed.parse::<i128>() {
        return Some(v);
    }
    trimmed.parse::<f64>().ok().and_then(float_to_int)
}

impl_int!(i8, i16, i32, i64, i128, u8, u16, u32, u64, u128, isize, usize);

macro_rules! impl_float {
    ($($ty:ty),*) => {$(
        impl FromJson for $ty {
            #[allow(clippy::cast_possible_truncation, clippy::cast_lossless)]
            fn from_json(value: &Value) -> Result<Self, JsonError> {
                let parsed = match value {
                    Value::Number(n) => n.as_f64(),
                    Value::Bool(b) => Some(f64::from(u8::from(*b))),
                    Value::String(s) => s.trim().parse::<f64>().ok(),
                    _ => None,
                };
                parsed
                    .map(|v| v as $ty)
                    .ok_or_else(|| JsonError::invalid_type(stringify!($ty), value))
            }

            fn json_default() -> Option<Self> {
                Some(0.0)
            }
        }
    )*};
}

impl_float!(f32, f64);

impl FromJson for bool {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        match value {
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => match n.as_f64() {
                Some(v) => Ok(v != 0.0),
                None => Err(JsonError::invalid_type("bool", value)),
            },
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "1" | "true" | "t" | "yes" | "y" | "on" => Ok(true),
                "0" | "false" | "f" | "no" | "n" | "off" | "" => Ok(false),
                _ => Err(JsonError::invalid_type("bool", value)),
            },
            _ => Err(JsonError::invalid_type("bool", value)),
        }
    }

    fn json_default() -> Option<Self> {
        Some(false)
    }
}

impl FromJson for String {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        match value {
            Value::String(s) => Ok(s.clone()),
            Value::Number(n) => Ok(n.to_string()),
            Value::Bool(b) => Ok(b.to_string()),
            _ => Err(JsonError::invalid_type("string", value)),
        }
    }

    fn json_default() -> Option<Self> {
        Some(String::new())
    }
}

impl FromJson for Value {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        Ok(value.clone())
    }

    fn from_missing() -> Option<Self> {
        Some(Value::Null)
    }

    fn json_default() -> Option<Self> {
        Some(Value::Null)
    }
}

impl FromJson for Map<String, Value> {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        match value {
            Value::Object(map) => Ok(map.clone()),
            _ => Err(JsonError::invalid_type("object", value)),
        }
    }

    fn json_default() -> Option<Self> {
        Some(Map::new())
    }
}

impl<T: FromJson> FromJson for Option<T> {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        if value.is_null() {
            Ok(None)
        } else {
            T::from_json(value).map(Some)
        }
    }

    fn from_missing() -> Option<Self> {
        Some(None)
    }

    fn json_default() -> Option<Self> {
        Some(None)
    }
}

impl<T: FromJson> FromJson for Box<T> {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        T::from_json(value).map(Box::new)
    }

    fn from_missing() -> Option<Self> {
        T::from_missing().map(Box::new)
    }

    fn json_default() -> Option<Self> {
        T::json_default().map(Box::new)
    }
}

impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(value: &Value) -> Result<Self, JsonError> {
        match value {
            Value::Array(items) => items
                .iter()
                .enumerate()
                .map(|(index, item)| T::from_json(item).map_err(|e| e.with_index(index)))
                .collect(),
            _ => Err(JsonError::invalid_type("array", value)),
        }
    }

    fn json_default() -> Option<Self> {
        Some(Vec::new())
    }
}

macro_rules! impl_map {
    ($map:ident $(, $bound:path)?) => {
        impl<T: FromJson> FromJson for $map<String, T> {
            fn from_json(value: &Value) -> Result<Self, JsonError> {
                match value {
                    Value::Object(map) => map
                        .iter()
                        .map(|(key, item)| {
                            T::from_json(item)
                                .map(|parsed| (key.clone(), parsed))
                                .map_err(|e| e.with_field(key))
                        })
                        .collect(),
                    _ => Err(JsonError::invalid_type("object", value)),
                }
            }

            fn json_default() -> Option<Self> {
                Some($map::new())
            }
        }
    };
}

use indexmap::IndexMap;
impl_map!(IndexMap);
impl_map!(BTreeMap);
impl_map!(HashMap);

#[doc(hidden)]
pub mod __private {
    use super::{Cow, JsonError, Map, Value, path};

    /// Ensure the value is an object.
    pub fn expect_object<'a>(
        value: &'a Value,
        _model: &'static str,
    ) -> Result<&'a Map<String, Value>, JsonError> {
        match value {
            Value::Object(map) => Ok(map),
            _ => Err(JsonError::invalid_type("object", value)),
        }
    }

    /// Resolve a field: JSONPath first, then keys in priority order.
    ///
    /// `null` values are treated as missing.
    pub fn lookup<'a>(
        root: &'a Value,
        object: &'a Map<String, Value>,
        keys: &[&str],
        json_path: Option<&str>,
    ) -> Option<Cow<'a, Value>> {
        if let Some(expr) = json_path
            && let Some(found) = path::select(root, expr)
            && !found.is_null()
        {
            return Some(found);
        }
        keys.iter()
            .filter_map(|key| object.get(*key))
            .find(|value| !value.is_null())
            .map(Cow::Borrowed)
    }
}

/// Helpers for `#[json(with = ...)]` conversions shared by several models.
pub mod convert {
    use super::{FromJson, JsonError, Value};

    /// `None` or `0` → empty string, everything else stringified.
    pub fn none_or_zero_to_empty_string(value: &Value) -> Result<String, JsonError> {
        match value {
            Value::Number(n) if n.as_f64() == Some(0.0) => Ok(String::new()),
            Value::Bool(false) | Value::Null => Ok(String::new()),
            other => String::from_json(other),
        }
    }

    /// Accept both a single object and an array of objects.
    pub fn one_or_many<T: FromJson>(value: &Value) -> Result<Vec<T>, JsonError> {
        match value {
            Value::Array(_) => Vec::<T>::from_json(value),
            other => Ok(vec![T::from_json(other)?]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ints_accept_numbers_strings_and_bools() {
        assert_eq!(i64::from_json(&json!(42)).unwrap(), 42);
        assert_eq!(i64::from_json(&json!("42")).unwrap(), 42);
        assert_eq!(i64::from_json(&json!(" -7 ")).unwrap(), -7);
        assert_eq!(i64::from_json(&json!(3.0)).unwrap(), 3);
        assert_eq!(i64::from_json(&json!("3.0")).unwrap(), 3);
        assert_eq!(i64::from_json(&json!(true)).unwrap(), 1);
        assert!(i64::from_json(&json!(3.5)).is_err());
        assert!(i64::from_json(&json!("abc")).is_err());
        assert!(u8::from_json(&json!(300)).is_err());
        assert_eq!(u64::from_json(&json!(u64::MAX)).unwrap(), u64::MAX);
    }

    #[test]
    fn floats_and_bools() {
        assert!((f64::from_json(&json!("1.5")).unwrap() - 1.5).abs() < f64::EPSILON);
        assert!((f64::from_json(&json!(2)).unwrap() - 2.0).abs() < f64::EPSILON);
        assert!(bool::from_json(&json!(1)).unwrap());
        assert!(!bool::from_json(&json!(0)).unwrap());
        assert!(bool::from_json(&json!("true")).unwrap());
        assert!(!bool::from_json(&json!("0")).unwrap());
        assert!(bool::from_json(&json!("maybe")).is_err());
        assert!(bool::from_json(&json!([])).is_err());
    }

    #[test]
    fn strings_accept_scalars() {
        assert_eq!(String::from_json(&json!("x")).unwrap(), "x");
        assert_eq!(String::from_json(&json!(12)).unwrap(), "12");
        assert_eq!(String::from_json(&json!(false)).unwrap(), "false");
        assert!(String::from_json(&json!({})).is_err());
    }

    #[test]
    fn containers() {
        let v: Vec<i32> = from_value(&json!([1, "2", 3])).unwrap();
        assert_eq!(v, vec![1, 2, 3]);
        let err = Vec::<i32>::from_json(&json!([1, "x"])).unwrap_err();
        assert_eq!(err.path(), &[PathSegment::Index(1)]);
        let m: IndexMap<String, i32> = from_value(&json!({"b": 1, "a": 2})).unwrap();
        assert_eq!(m.keys().collect::<Vec<_>>(), vec!["b", "a"]);
        let o: Option<i32> = from_value(&json!(null)).unwrap();
        assert_eq!(o, None);
        assert_eq!(Option::<i32>::from_missing(), Some(None));
        assert_eq!(i32::from_missing(), None);
        assert!(Map::<String, Value>::from_json(&json!([])).is_err());
    }

    #[test]
    fn error_display_contains_path() {
        let err = JsonError::invalid_type("i64", &json!("x"))
            .with_index(2)
            .with_field("songs");
        assert_eq!(err.path_string(), "$.songs[2]");
        assert!(err.to_string().contains("expected i64"));
        assert!(JsonError::missing("id").to_string().contains("`id`"));
    }

    #[test]
    fn none_or_zero_conversion() {
        use convert::none_or_zero_to_empty_string as f;
        assert_eq!(f(&json!(0)).unwrap(), "");
        assert_eq!(f(&json!(null)).unwrap(), "");
        assert_eq!(f(&json!("0")).unwrap(), "0");
        assert_eq!(f(&json!("北京")).unwrap(), "北京");
        assert_eq!(f(&json!(12)).unwrap(), "12");
    }

    #[test]
    fn one_or_many_conversion() {
        let single: Vec<i32> = convert::one_or_many(&json!(1)).unwrap();
        assert_eq!(single, vec![1]);
        let many: Vec<i32> = convert::one_or_many(&json!([1, 2])).unwrap();
        assert_eq!(many, vec![1, 2]);
    }
}
