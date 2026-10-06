//! Lyric models.

use serde::Serialize;
use serde_json::Value;

use crate::FromJson;
use crate::algorithms::qrc_decrypt;

/// Decrypt the given string fields in place (silently keeping undecryptable
/// values, like upstream).
pub(crate) fn decrypt_fields(value: &mut Value, fields: &[&str]) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    for field in fields {
        if let Some(Value::String(text)) = map.get_mut(*field)
            && !text.is_empty()
            && let Ok(plain) = qrc_decrypt(text)
        {
            *text = plain;
        }
    }
}

fn decrypt_lyric_response(value: &mut Value) {
    decrypt_fields(value, &["lyric", "trans", "roma", "singingAnnotationsLyric"]);
}

fn decrypt_lyric_item(value: &mut Value) {
    decrypt_fields(value, &["lyric"]);
}

/// `get_lyric` response (lyrics are decrypted automatically).
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default, preprocess = "decrypt_lyric_response")]
pub struct GetLyricResponse {
    /// Song id.
    #[json(alias = "songID")]
    pub songid: i64,
    /// Lyric (LRC or QRC XML).
    pub lyric: String,
    /// Translation.
    pub trans: String,
    /// Romanization.
    pub roma: String,
    /// Singing annotations lyric.
    #[json(alias = "singingAnnotationsLyric")]
    pub singing_annotations_lyric: String,
    /// LRC timestamp.
    pub lrc_t: i64,
    /// QRC timestamp.
    pub qrc_t: i64,
    /// Translation timestamp.
    pub trans_t: i64,
    /// Romanization timestamp.
    pub roma_t: i64,
    /// Singing annotations timestamp.
    #[json(alias = "singingAnnotationsTs")]
    pub singing_annotations_ts: i64,
    /// Has contributor.
    #[json(alias = "hasContributor")]
    pub has_contributor: bool,
    /// Has translation contributor.
    #[json(alias = "hasTransContributor")]
    pub has_trans_contributor: bool,
    /// Has multiple translations.
    #[json(alias = "hasMultiTrans")]
    pub has_multi_trans: bool,
}

/// `get_singing_annotations_info` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSingingAnnotationsInfoResponse {
    /// Has singing annotations.
    #[json(alias = "hasSingingAnnotationsLyric")]
    pub has_singing_annotations_lyric: bool,
}

/// Translated lyric in a given style.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default, preprocess = "decrypt_lyric_item")]
pub struct MultiStyleLyricItem {
    /// Style.
    pub style: i64,
    /// Style name.
    #[json(alias = "styleName")]
    pub style_name: String,
    /// Lyric.
    pub lyric: String,
    /// Timestamp.
    pub timestamp: i64,
}

/// `get_multi_style_trans_lyric` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct BatchGetMultiStyleTransLyricResponse {
    /// Lyrics.
    pub lyrics: Vec<MultiStyleLyricItem>,
}

/// `is_ai_dict_exists` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct IsAiDictExistsResponse {
    /// Exists.
    pub exists: bool,
}

/// AI dictionary entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AiDictItem {
    /// Phrase.
    pub phrase: String,
    /// Explanation.
    pub explain: String,
    /// Lyric text.
    pub lyric_text: String,
    /// Translated lyric text.
    pub trans_lyric_text: String,
    /// Lyric timestamp.
    pub lyric_timestamp: String,
}

/// `get_ai_dict` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetAiDictResponse {
    /// Entries.
    #[json(alias = "dictList")]
    pub dict_list: Vec<AiDictItem>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::qrc_encrypt;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn lyrics_are_decrypted() {
        let encrypted = qrc_encrypt("[00:00.00]晴天");
        let res: GetLyricResponse = from_value(&json!({
            "songID": 97773,
            "lyric": encrypted,
            "trans": "not-hex",
            "roma": "",
            "hasMultiTrans": 1
        }))
        .unwrap();
        assert_eq!(res.songid, 97773);
        assert_eq!(res.lyric, "[00:00.00]晴天");
        assert_eq!(res.trans, "not-hex");
        assert!(res.has_multi_trans);
        let item: MultiStyleLyricItem = from_value(&json!({"style": 1, "styleName": "s", "lyric": qrc_encrypt("x")})).unwrap();
        assert_eq!(item.lyric, "x");
        let dict: GetAiDictResponse = from_value(&json!({"dictList": [{"phrase": "p"}]})).unwrap();
        assert_eq!(dict.dict_list[0].phrase, "p");
        let mut not_object = json!([1]);
        decrypt_fields(&mut not_object, &["lyric"]);
        assert_eq!(not_object, json!([1]));
    }
}
