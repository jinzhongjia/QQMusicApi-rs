//! Private message models.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::FromJson;
use crate::pagination::PageItems;

/// Wrap a raw object without a `data` key into `{"data": obj}`.
fn wrap_data(value: &mut Value) {
    if let Value::Object(map) = value
        && !map.contains_key("data")
    {
        let inner = std::mem::take(map);
        map.insert("data".into(), Value::Object(inner));
    }
}

/// Message participant.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMessageUser {
    /// Avatar URL.
    pub avatar: String,
    /// Encrypted uin.
    pub encrypt_uin: String,
    /// Uin.
    pub uin: String,
    /// Identity badge.
    pub identity_pic: String,
    /// Nickname.
    pub nick: String,
    /// Identity.
    #[json(default = -1)]
    pub identity: i64,
    /// Type.
    #[json(default = -1)]
    pub r#type: i64,
    /// Whether followed.
    pub is_concern: i64,
}

impl Default for PrivateMessageUser {
    fn default() -> Self {
        Self {
            avatar: String::new(),
            encrypt_uin: String::new(),
            uin: String::new(),
            identity_pic: String::new(),
            nick: String::new(),
            identity: -1,
            r#type: -1,
            is_concern: 0,
        }
    }
}

/// Message payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMessageMetaData {
    /// Title.
    pub title: String,
    /// Content.
    pub content: String,
    /// Picture.
    pub pic: String,
    /// Business id.
    pub biz_id: String,
    /// Business type.
    pub biz_type: i64,
    /// URL.
    pub url: String,
    /// Width.
    pub width: i64,
    /// Height.
    pub height: i64,
    /// Duration.
    #[json(alias = "Duration")]
    pub duration: i64,
    /// Size.
    #[json(alias = "Size")]
    pub size: i64,
}

/// One message.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMessageInfo {
    /// Message id.
    pub id: String,
    /// Payload.
    pub meta_data: Option<PrivateMessageMetaData>,
    /// Client key.
    pub client_key: String,
    /// Sender.
    pub from_user: Option<PrivateMessageUser>,
    /// Timestamp.
    pub time: i64,
    /// State.
    pub state: i64,
    /// Result.
    pub result: i64,
    /// Tips.
    pub tips: String,
    /// Sequence.
    pub sequence: i64,
    /// Show type.
    pub show_type: i64,
    /// Message type.
    pub msg_type: i64,
    /// Confirm flag.
    pub confirm: i64,
    /// Sort time.
    pub sort_time: i64,
    /// Complaint tip.
    #[json(alias = "complainTip")]
    pub complain_tip: String,
    /// Complaint URL.
    #[json(alias = "complainUrl")]
    pub complain_url: String,
}

/// Session tail tag (raw object).
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default, preprocess = "wrap_data")]
pub struct PrivateMessageTailTag {
    /// Raw tag data.
    pub data: Map<String, Value>,
}

/// Conversation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMessageSession {
    /// Session id.
    pub session_id: String,
    /// Peer.
    pub user: Option<PrivateMessageUser>,
    /// Latest message.
    pub new_msg: Option<PrivateMessageInfo>,
    /// Unread count.
    pub new_msg_cnt: i64,
    /// Sort time.
    pub sort_time: i64,
    /// URL.
    pub url: String,
    /// Creation time.
    pub create_time: i64,
    /// Source (`from`).
    #[json(alias = "from")]
    pub from_: i64,
    /// Star virtual uin (upstream key is misspelled).
    #[json(alias = "SmStarVirtaulUin")]
    pub sm_star_virtual_uin: String,
    /// Auth.
    #[json(alias = "Auth")]
    pub auth: String,
    /// Extra data.
    #[json(alias = "Ext")]
    pub ext: BTreeMap<String, String>,
    /// Tail tags.
    #[json(alias = "TailTags")]
    pub tail_tags: Vec<PrivateMessageTailTag>,
}

/// `get_sessions` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateSessionListResponse {
    /// `1` when more pages exist.
    pub has_more: i64,
    /// Message.
    pub msg: String,
    /// Total unread count.
    pub new_msg_cnt: i64,
    /// Sessions.
    pub sessions: Vec<PrivateMessageSession>,
    /// Sub code.
    pub subcode: i64,
    /// Setting guide.
    pub setting_guide: i64,
    /// State.
    pub state: i64,
    /// Extra.
    pub extra: BTreeMap<String, String>,
}

impl PageItems for PrivateSessionListResponse {
    type Item = PrivateMessageSession;
    fn into_items(self) -> Vec<Self::Item> {
        self.sessions
    }
}

/// "Pat" text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMessagePatText {
    /// Nickname.
    #[json(alias = "Nick")]
    pub nick: String,
    /// Text.
    #[json(alias = "PatTxt")]
    pub pat_text: String,
}

/// `get_messages` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMessageListResponse {
    /// `1` when more pages exist.
    pub has_more: i64,
    /// Messages.
    pub messages: Vec<PrivateMessageInfo>,
    /// Message.
    pub msg: String,
    /// Session.
    pub session: Option<PrivateMessageSession>,
    /// Sub code.
    pub subcode: i64,
    /// Last sequence.
    pub end_msg_seq: i64,
    /// Attachments.
    #[json(alias = "Attach")]
    pub attach: Map<String, Value>,
    /// Pat interval.
    #[json(alias = "PatInterval")]
    pub pat_interval: i64,
    /// Pat texts.
    #[json(alias = "PatMap")]
    pub pat_map: BTreeMap<String, PrivateMessagePatText>,
    /// Encrypted star.
    #[json(alias = "EncryptStar")]
    pub encrypt_star: String,
    /// Location tips.
    #[json(alias = "LocationTips")]
    pub location_tips: String,
    /// Unread count.
    #[json(alias = "NewMsgCnt")]
    pub new_msg_cnt: i64,
}

impl PageItems for PrivateMessageListResponse {
    type Item = PrivateMessageInfo;
    fn into_items(self) -> Vec<Self::Item> {
        self.messages
    }
}

/// `send_message` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateSendMessageResponse {
    /// Sent messages.
    pub messages: Vec<PrivateMessageInfo>,
    /// Session.
    pub session: Option<PrivateMessageSession>,
    /// Tips.
    pub tips: String,
    /// Identity verification URL.
    pub identify_url: String,
    /// Message.
    pub msg: String,
    /// Reason.
    pub reason: i64,
    /// Last sequence.
    pub end_msg_seq: i64,
    /// Update time.
    pub update_time: i64,
}

/// Generic write operation response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateOperationResponse {
    /// Message.
    pub msg: String,
    /// Sub code.
    pub subcode: i64,
    /// Tips.
    pub tips: String,
}

/// `get_config` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateConfigResponse {
    /// Numeric value.
    pub config_value: i64,
    /// String value.
    pub config_value_str: String,
    /// Message.
    pub msg: String,
}

/// Musician card (raw object).
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default, preprocess = "wrap_data")]
pub struct PrivateMusicianCardResponse {
    /// Raw card data.
    pub data: Map<String, Value>,
}

/// Chat entry.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateEntryItem {
    /// Entry type.
    #[json(alias = "EntryType", default = -1)]
    pub entry_type: i64,
    /// Icon.
    #[json(alias = "Icon")]
    pub icon: String,
    /// Title.
    #[json(alias = "Title")]
    pub title: String,
    /// Scheme.
    #[json(alias = "SkipScheme")]
    pub skip_scheme: String,
    /// Corner tag.
    #[json(alias = "RightTopTag")]
    pub right_top_tag: String,
    /// Extra.
    #[json(alias = "Ext")]
    pub ext: BTreeMap<String, String>,
}

impl Default for PrivateEntryItem {
    fn default() -> Self {
        Self {
            entry_type: -1,
            icon: String::new(),
            title: String::new(),
            skip_scheme: String::new(),
            right_top_tag: String::new(),
            ext: BTreeMap::new(),
        }
    }
}

/// `get_chat_entries` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateChatEntriesResponse {
    /// Return code.
    #[json(alias = "RetCode")]
    pub ret_code: i64,
    /// Return message.
    #[json(alias = "RetMsg")]
    pub ret_msg: String,
    /// Entries keyed by scene (numeric string keys).
    #[json(alias = "Entries")]
    pub entries: BTreeMap<String, Vec<PrivateEntryItem>>,
    /// Whether "dazi" is available.
    #[json(alias = "CanBeDazi")]
    pub can_be_dazi: Option<bool>,
    /// Dazi data.
    #[json(alias = "DzData")]
    pub dz_data: Map<String, Value>,
}

/// `get_media_message_details` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateMediaMessageDetailsResponse {
    /// Messages keyed by id.
    #[json(alias = "MsgIDs")]
    pub msg_ids: BTreeMap<String, PrivateMessageInfo>,
}

/// `get_safety_hint` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct PrivateSafetyHintResponse {
    /// Hint.
    pub hint: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_sessions_and_messages() {
        let sessions = PrivateSessionListResponse::from_json(&json!({
            "has_more": 1,
            "sessions": [{
                "session_id": "s1",
                "user": {"nick": "n", "uin": 123},
                "new_msg": {"id": "m1", "meta_data": {"content": "hi", "Duration": 3}},
                "from": 2,
                "SmStarVirtaulUin": "v",
                "Ext": {"k": 1},
                "TailTags": [{"text": "t"}, {"data": {"x": 1}}]
            }]
        }))
        .unwrap();
        let session = &sessions.sessions[0];
        let user = session.user.as_ref().unwrap();
        assert_eq!((user.uin.as_str(), user.identity, user.r#type), ("123", -1, -1));
        let msg = session.new_msg.as_ref().unwrap();
        assert_eq!(msg.meta_data.as_ref().unwrap().duration, 3);
        assert_eq!((session.from_, session.sm_star_virtual_uin.as_str()), (2, "v"));
        assert_eq!(session.ext["k"], "1");
        assert_eq!(session.tail_tags[0].data["text"], "t");
        assert_eq!(session.tail_tags[1].data["x"], 1);
        assert_eq!(sessions.into_items().len(), 1);

        let messages = PrivateMessageListResponse::from_json(&json!({
            "messages": [{"id": "a", "complainTip": "c"}],
            "Attach": null,
            "PatMap": {"u": {"Nick": "n", "PatTxt": "p"}},
            "NewMsgCnt": 4
        }))
        .unwrap();
        assert!(messages.attach.is_empty());
        assert_eq!(messages.pat_map["u"].pat_text, "p");
        assert_eq!(messages.messages[0].complain_tip, "c");
        assert_eq!(messages.new_msg_cnt, 4);
    }

    #[test]
    fn parses_misc_responses() {
        let card = PrivateMusicianCardResponse::from_json(&json!({"name": "x"})).unwrap();
        assert_eq!(card.data["name"], "x");
        let entries = PrivateChatEntriesResponse::from_json(&json!({
            "RetCode": 0,
            "Entries": {"1": [{"Title": "t"}]},
            "CanBeDazi": true
        }))
        .unwrap();
        assert_eq!(entries.entries["1"][0].entry_type, -1);
        assert_eq!(entries.can_be_dazi, Some(true));
        let details = PrivateMediaMessageDetailsResponse::from_json(&json!({"MsgIDs": {"m": {"id": "m"}}})).unwrap();
        assert_eq!(details.msg_ids["m"].id, "m");
        assert_eq!(PrivateMessageUser::from_json(&json!({})).unwrap(), PrivateMessageUser::default());
    }
}
