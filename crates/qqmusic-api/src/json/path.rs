//! Minimal JSONPath evaluator covering the subset used by the response models.
//!
//! Supported syntax: `$`, `.key`, `[*]` and `[<index>]` (negative indexes
//! count from the end). The semantics follow `jsonpath-ng` as used by the
//! upstream project:
//!
//! * `[*]` applied to a non-array truthy value wraps it into a single element
//!   list (QQ Music returns an object instead of a one-element array in some
//!   cases);
//! * falsy values (`null`, `0`, `""`, `false`, `[]`, `{}`) yield no match for
//!   `[*]`;
//! * the result is a list whenever the expression contains `[*]` or yields
//!   more than one match.
//!
//! One deliberate leniency: when a `[*]` step is applied to an *existing*
//! array that turns out to be empty, the result is an empty array rather than
//! "no match" so that list fields resolve to `[]` instead of falling back.

use std::borrow::Cow;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment<'a> {
    Key(&'a str),
    Wildcard,
    Index(i64),
}

fn parse(expr: &str) -> Option<Vec<Segment<'_>>> {
    let mut rest = expr.strip_prefix('$')?;
    let mut segments = Vec::new();
    while !rest.is_empty() {
        if let Some(stripped) = rest.strip_prefix('.') {
            let end = stripped.find(['.', '[']).unwrap_or(stripped.len());
            if end == 0 {
                return None;
            }
            segments.push(Segment::Key(&stripped[..end]));
            rest = &stripped[end..];
        } else {
            let stripped = rest.strip_prefix('[')?;
            let end = stripped.find(']')?;
            let inner = stripped[..end].trim();
            if inner == "*" {
                segments.push(Segment::Wildcard);
            } else {
                segments.push(Segment::Index(inner.parse().ok()?));
            }
            rest = &stripped[end + 1..];
        }
    }
    Some(segments)
}

fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(b) => !b,
        Value::Number(n) => n.as_f64() == Some(0.0),
        Value::String(s) => s.is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
    }
}

/// Evaluate a JSONPath expression.
///
/// Returns `None` when nothing matched or the expression is invalid.
pub fn select<'a>(root: &'a Value, expr: &str) -> Option<Cow<'a, Value>> {
    let segments = parse(expr)?;
    let mut matches: Vec<&'a Value> = vec![root];
    let mut has_wildcard = false;
    let mut wildcard_hit_array = false;

    for segment in &segments {
        let mut next = Vec::with_capacity(matches.len());
        for value in matches {
            match segment {
                Segment::Key(key) => {
                    if let Some(child) = value.as_object().and_then(|o| o.get(*key)) {
                        next.push(child);
                    }
                }
                Segment::Wildcard => {
                    has_wildcard = true;
                    match value {
                        Value::Array(items) => {
                            wildcard_hit_array = true;
                            next.extend(items.iter());
                        }
                        other if is_falsy(other) => {}
                        other => next.push(other),
                    }
                }
                Segment::Index(index) => {
                    if let Value::Array(items) = value {
                        let len = i64::try_from(items.len()).unwrap_or(i64::MAX);
                        let idx = if *index < 0 { len + index } else { *index };
                        if let Some(item) = usize::try_from(idx).ok().and_then(|i| items.get(i)) {
                            next.push(item);
                        }
                    }
                }
            }
        }
        matches = next;
    }

    if matches.is_empty() {
        if has_wildcard && wildcard_hit_array {
            return Some(Cow::Owned(Value::Array(Vec::new())));
        }
        return None;
    }
    if has_wildcard || matches.len() > 1 {
        Some(Cow::Owned(Value::Array(matches.into_iter().cloned().collect())))
    } else {
        Some(Cow::Borrowed(matches[0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn root_and_keys() {
        let data = json!({"a": {"b": 1}});
        assert_eq!(select(&data, "$").unwrap().as_ref(), &data);
        assert_eq!(select(&data, "$.a.b").unwrap().as_ref(), &json!(1));
        assert!(select(&data, "$.a.c").is_none());
        assert!(select(&data, "$.x.y").is_none());
    }

    #[test]
    fn wildcard_over_arrays() {
        let data = json!({"songList": [{"songInfo": {"id": 1}}, {"songInfo": {"id": 2}}]});
        let result = select(&data, "$.songList[*].songInfo").unwrap();
        assert_eq!(result.as_ref(), &json!([{"id": 1}, {"id": 2}]));
    }

    #[test]
    fn wildcard_wraps_single_object() {
        let data = json!({"List": {"id": 1}});
        assert_eq!(select(&data, "$.List[*]").unwrap().as_ref(), &json!([{"id": 1}]));
    }

    #[test]
    fn wildcard_on_empty_or_null() {
        let empty = json!({"List": []});
        assert_eq!(select(&empty, "$.List[*]").unwrap().as_ref(), &json!([]));
        let null = json!({"List": null});
        assert!(select(&null, "$.List[*]").is_none());
        let missing = json!({});
        assert!(select(&missing, "$.List[*]").is_none());
    }

    #[test]
    fn nested_wildcards() {
        let data = json!({"vecPlaylistNew": [
            {"playlists": [{"id": 1}, {"id": 2}]},
            {"playlists": [{"id": 3}]}
        ]});
        let result = select(&data, "$.vecPlaylistNew[*].playlists[*]").unwrap();
        assert_eq!(result.as_ref(), &json!([{"id": 1}, {"id": 2}, {"id": 3}]));
    }

    #[test]
    fn index_access() {
        let data = json!({"a": [10, 20, 30]});
        assert_eq!(select(&data, "$.a[0]").unwrap().as_ref(), &json!(10));
        assert_eq!(select(&data, "$.a[-1]").unwrap().as_ref(), &json!(30));
        assert!(select(&data, "$.a[5]").is_none());
    }

    #[test]
    fn invalid_expressions() {
        let data = json!({"a": 1});
        assert!(select(&data, "a").is_none());
        assert!(select(&data, "$..a").is_none());
        assert!(select(&data, "$.a[").is_none());
        assert!(select(&data, "$.a[x]").is_none());
    }
}
