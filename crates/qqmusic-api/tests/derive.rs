//! Behavioural tests for the `FromJson` derive macro.

use qqmusic_api::json::{FromJson, JsonErrorKind, from_value};
use serde_json::json;

#[derive(Debug, PartialEq, FromJson)]
struct Singer {
    #[json(alias("id", "singerID", "singerId"), default)]
    id: i64,
    #[json(alias("mid", "singerMid"), default)]
    mid: String,
    #[json(alias("name", "singerName"), default)]
    name: String,
    #[json(alias("title", "singerName", "name"), default)]
    title: String,
}

#[test]
fn alias_priority_and_shared_keys() {
    let singer: Singer = from_value(&json!({"singerID": 5, "singerMid": "m", "singerName": "周杰伦"})).unwrap();
    assert_eq!(singer.id, 5);
    assert_eq!(singer.mid, "m");
    assert_eq!(singer.name, "周杰伦");
    // the same key may feed several fields
    assert_eq!(singer.title, "周杰伦");

    let singer: Singer = from_value(&json!({"id": "7", "singerID": 5, "title": "t", "name": "n"})).unwrap();
    assert_eq!(singer.id, 7, "first alias wins");
    assert_eq!(singer.title, "t");
}

#[test]
fn defaults_and_nulls() {
    let singer: Singer = from_value(&json!({"id": null})).unwrap();
    assert_eq!(singer, Singer { id: 0, mid: String::new(), name: String::new(), title: String::new() });
}

#[derive(Debug, FromJson)]
struct Required {
    id: i64,
    #[json(alias = "songName")]
    name: String,
    maybe: Option<String>,
    #[json(default = -1)]
    neg: i64,
    #[json(default = "0")]
    jump_tab: String,
}

#[test]
fn required_fields() {
    let parsed: Required = from_value(&json!({"id": 1, "songName": "x"})).unwrap();
    assert_eq!(parsed.id, 1);
    assert_eq!(parsed.name, "x");
    assert_eq!(parsed.maybe, None);
    assert_eq!(parsed.neg, -1);
    assert_eq!(parsed.jump_tab, "0");

    // field name accepted as well (populate_by_name)
    let parsed: Required = from_value(&json!({"id": 1, "name": "y"})).unwrap();
    assert_eq!(parsed.name, "y");

    let err = from_value::<Required>(&json!({"songName": "x"})).unwrap_err();
    assert_eq!(err.kind(), &JsonErrorKind::Missing("id".into()));

    let err = from_value::<Required>(&json!({"id": "abc", "name": "x"})).unwrap_err();
    assert_eq!(err.path_string(), "$.id");

    assert!(from_value::<Required>(&json!([1])).is_err());
}

#[derive(Debug, FromJson)]
struct Song {
    id: i64,
}

#[derive(Debug, FromJson)]
#[json(default)]
struct WithPaths {
    #[json(path = "$.meta.sum")]
    total: i64,
    #[json(path = "$.songList[*].songInfo")]
    songs: Vec<Song>,
    #[json(path = "$.List[*]")]
    single_as_list: Vec<Song>,
    #[json(path = "$")]
    raw: serde_json::Value,
    #[json(required, path = "$.result.tid")]
    tid: i64,
}

#[test]
fn jsonpath_fields() {
    let data = json!({
        "meta": {"sum": "12"},
        "songList": [{"songInfo": {"id": 1}}, {"songInfo": {"id": 2}}],
        "List": {"id": 3},
        "result": {"tid": 9}
    });
    let parsed: WithPaths = from_value(&data).unwrap();
    assert_eq!(parsed.total, 12);
    assert_eq!(parsed.songs.iter().map(|s| s.id).collect::<Vec<_>>(), vec![1, 2]);
    assert_eq!(parsed.single_as_list.len(), 1);
    assert_eq!(parsed.raw, data);
    assert_eq!(parsed.tid, 9);

    // jsonpath miss falls back to the field name (round trip of serialized output)
    let parsed: WithPaths = from_value(&json!({"total": 3, "songs": [{"id": 4}], "tid": 1})).unwrap();
    assert_eq!(parsed.total, 3);
    assert_eq!(parsed.songs[0].id, 4);

    let err = from_value::<WithPaths>(&json!({})).unwrap_err();
    assert_eq!(err.kind(), &JsonErrorKind::Missing("tid".into()));

    let err = from_value::<WithPaths>(&json!({"tid": 1, "songList": [{"songInfo": {"id": "x"}}]})).unwrap_err();
    assert_eq!(err.path_string(), "$.songs[0].id");
}

#[derive(Debug, FromJson)]
struct Bucket<T> {
    #[json(default)]
    total_num: i64,
    #[json(default)]
    items: Vec<T>,
}

#[derive(Debug, FromJson)]
struct Outer {
    #[json(default)]
    bucket: Bucket<Song>,
    #[json(flatten)]
    inline: Bucket<Song>,
}

#[test]
fn generics_nested_defaults_and_flatten() {
    let parsed: Outer = from_value(&json!({"total_num": 2, "items": [{"id": 1}]})).unwrap();
    assert_eq!(parsed.bucket.total_num, 0);
    assert!(parsed.bucket.items.is_empty());
    assert_eq!(parsed.inline.total_num, 2);
    assert_eq!(parsed.inline.items[0].id, 1);
}

fn upper(value: &serde_json::Value) -> Result<String, qqmusic_api::json::JsonError> {
    String::from_json(value).map(|s| s.to_uppercase())
}

fn add_marker(value: &mut serde_json::Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("marker".into(), json!("pre"));
    }
}

#[derive(Debug, FromJson)]
#[json(preprocess = "add_marker")]
struct Custom {
    #[json(with = "upper")]
    name: String,
    marker: String,
    #[json(skip)]
    skipped: i32,
}

#[test]
fn with_preprocess_and_skip() {
    let parsed: Custom = from_value(&json!({"name": "abc", "skipped": 5})).unwrap();
    assert_eq!(parsed.name, "ABC");
    assert_eq!(parsed.marker, "pre");
    assert_eq!(parsed.skipped, 0);
}

#[test]
fn json_default_respects_required() {
    assert!(Song::json_default().is_none());
    let bucket = Bucket::<Song>::json_default().unwrap();
    assert_eq!(bucket.total_num, 0);
}
