//! Private message (direct message) APIs.

use serde_json::{Map, Value, json};

use crate::credential::Credential;
use crate::models::private_message::{
    PrivateChatEntriesResponse, PrivateConfigResponse, PrivateMediaMessageDetailsResponse, PrivateMessageListResponse,
    PrivateMusicianCardResponse, PrivateOperationResponse, PrivateSafetyHintResponse, PrivateSendMessageResponse,
    PrivateSessionListResponse,
};
use crate::pagination::{FnStrategy, Paged};
use crate::request::CgiRequest;
use crate::versioning::Platform;

const READ_MODULE: &str = "music.privateMsg.PrivateMsgRead";
const WRITE_MODULE: &str = "music.privateMsg.PrivateMsgWrite";

/// Options of [`PrivateMessageApi::get_sessions`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListOptions {
    /// Last session id of the previous page.
    pub last_id: String,
    /// Order.
    pub order: i64,
    /// Page size.
    pub size: i64,
    /// Sort time of the last session of the previous page.
    pub last_time: i64,
    /// Source (`from`).
    pub from: i64,
    /// Fans filter (ignored when `encrypt_from_uin` is set).
    pub fans_flag: Option<i64>,
    /// Only sessions with this user.
    pub encrypt_from_uin: Option<String>,
}

impl Default for SessionListOptions {
    fn default() -> Self {
        Self {
            last_id: String::new(),
            order: 1,
            size: 20,
            last_time: 0,
            from: 0,
            fans_flag: Some(1),
            encrypt_from_uin: None,
        }
    }
}

/// Options of [`PrivateMessageApi::get_messages`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageListOptions {
    /// Session id.
    pub session_id: String,
    /// Peer user id.
    pub user_id: String,
    /// Last message id of the previous page.
    pub last_id: String,
    /// WNS id.
    pub wns_id: String,
    /// Order.
    pub order: i64,
    /// Page size.
    pub size: i64,
    /// Flag.
    pub flag: i64,
    /// Location id.
    pub location_id: Option<String>,
    /// Update time.
    pub update_time: Option<i64>,
}

impl Default for MessageListOptions {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            user_id: String::new(),
            last_id: String::new(),
            wns_id: String::new(),
            order: 1,
            size: 50,
            flag: 0,
            location_id: None,
            update_time: None,
        }
    }
}

/// Options of [`PrivateMessageApi::send_message`].
#[derive(Debug, Clone, PartialEq)]
pub struct SendMessageOptions {
    /// Session id.
    pub session_id: String,
    /// Last message id.
    pub last_id: String,
    /// Last message sequence.
    pub last_msg_seq: i64,
    /// Message payload (`{"content": ..}` for text).
    pub meta_data: Option<Value>,
    /// Entrance.
    pub entrance: i64,
    /// Client generated key.
    pub client_key: String,
    /// Source flag.
    pub source_flag: Option<i64>,
    /// Message id.
    pub msg_id: Option<String>,
    /// User input.
    pub user_input: Option<String>,
    /// Super message flag.
    pub super_msg_flag: Option<i64>,
    /// Use `StarSendSuperMsg` instead of `SendMessageAsync`.
    pub star_send: bool,
}

impl Default for SendMessageOptions {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            last_id: String::new(),
            last_msg_seq: 0,
            meta_data: None,
            entrance: 0,
            client_key: String::new(),
            source_flag: None,
            msg_id: None,
            user_input: None,
            super_msg_flag: Some(0),
            star_send: false,
        }
    }
}

/// Insert values that are neither `None` nor an empty string.
fn insert_present(params: &mut Map<String, Value>, entries: impl IntoIterator<Item = (&'static str, Option<Value>)>) {
    for (key, value) in entries {
        match value {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.is_empty() => {}
            Some(value) => {
                params.insert(key.into(), value);
            }
        }
    }
}

api_module! {
    /// Private message APIs (Android only, login required).
    PrivateMessageApi
}

impl PrivateMessageApi {
    fn private<T: crate::FromJson + Send + 'static>(
        &self,
        module: &str,
        method: &str,
        param: Value,
        credential: Option<Credential>,
    ) -> CgiRequest<T> {
        self.cgi(module, method, param).require_login(true).platform(Platform::Android).credential_opt(credential)
    }

    /// Conversation list (paginated by `last_id` + `last_time`).
    pub fn get_sessions(
        &self,
        options: SessionListOptions,
        credential: Option<Credential>,
    ) -> Paged<PrivateSessionListResponse> {
        let mut param = json!({
            "last_id": options.last_id,
            "order": options.order,
            "size": options.size,
            "last_time": options.last_time,
            "from": options.from,
        });
        if let Some(uin) = options.encrypt_from_uin.filter(|u| !u.is_empty()) {
            param["EncryptFromUin"] = json!(uin);
        } else if let Some(flag) = options.fans_flag {
            param["FansFlag"] = json!(flag);
        }
        Paged::new(
            self.private(READ_MODULE, "GetSessionList", param, credential),
            FnStrategy(|params: &Value, r: &PrivateSessionListResponse| {
                if r.has_more != 1 {
                    return None;
                }
                let last = r.sessions.last()?;
                let mut next = params.clone();
                next["last_id"] = json!(last.session_id);
                next["last_time"] = json!(last.sort_time);
                Some(next)
            }),
        )
    }

    /// Delete a conversation.
    pub fn delete_session(
        &self,
        session_id: &str,
        super_msg_flag: i64,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateOperationResponse> {
        self.private(
            WRITE_MODULE,
            "DeleteSession",
            json!({"session_id": session_id, "super_msg_flag": super_msg_flag}),
            credential,
        )
    }

    /// Messages of a conversation (paginated by `last_id`).
    pub fn get_messages(
        &self,
        options: MessageListOptions,
        credential: Option<Credential>,
    ) -> Paged<PrivateMessageListResponse> {
        let mut param = Map::new();
        param.insert("order".into(), json!(options.order));
        param.insert("size".into(), json!(options.size));
        param.insert("flag".into(), json!(options.flag));
        insert_present(
            &mut param,
            [
                ("session_id", Some(json!(options.session_id))),
                ("last_id", Some(json!(options.last_id))),
                ("wns_id", Some(json!(options.wns_id))),
                ("user_id", Some(json!(options.user_id))),
                ("location_id", options.location_id.map(Value::from)),
                ("update_time", options.update_time.map(Value::from)),
            ],
        );
        Paged::new(
            self.private(READ_MODULE, "GetMessage", Value::Object(param), credential),
            FnStrategy(|params: &Value, r: &PrivateMessageListResponse| {
                if r.has_more != 1 {
                    return None;
                }
                let mut next = params.clone();
                next["last_id"] = json!(r.messages.last()?.id);
                Some(next)
            }),
        )
    }

    /// Send a message.
    pub fn send_message(
        &self,
        user_id: &str,
        msg_type: i64,
        options: SendMessageOptions,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateSendMessageResponse> {
        let mut param = Map::new();
        param.insert("last_msg_seq".into(), json!(options.last_msg_seq));
        param.insert("user_id".into(), json!(user_id));
        param.insert("entrance".into(), json!(options.entrance));
        param.insert("client_key".into(), json!(options.client_key));
        param.insert("msg_type".into(), json!(msg_type));
        insert_present(
            &mut param,
            [
                ("session_id", Some(json!(options.session_id))),
                ("last_id", Some(json!(options.last_id))),
                ("meta_data", options.meta_data),
                ("source_flag", options.source_flag.map(Value::from)),
                ("msg_id", options.msg_id.map(Value::from)),
                ("user_input", options.user_input.map(Value::from)),
                ("super_msg_flag", options.super_msg_flag.map(Value::from)),
            ],
        );
        let method = if options.star_send { "StarSendSuperMsg" } else { "SendMessageAsync" };
        self.private(WRITE_MODULE, method, Value::Object(param), credential)
    }

    /// Delete a message.
    pub fn delete_message(
        &self,
        session_id: &str,
        msg_id: &str,
        super_msg_flag: i64,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateOperationResponse> {
        self.private(
            WRITE_MODULE,
            "DeleteMessage",
            json!({"session_id": session_id, "msg_id": msg_id, "super_msg_flag": super_msg_flag}),
            credential,
        )
    }

    /// Clear all messages of a conversation.
    pub fn clear_session(
        &self,
        session_id: &str,
        super_msg_flag: i64,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateOperationResponse> {
        self.private(
            WRITE_MODULE,
            "ClearSession",
            json!({"session_id": session_id, "super_msg_flag": super_msg_flag}),
            credential,
        )
    }

    /// Set a private message setting.
    ///
    /// Some settings are integers (`config_value`, e.g. types 1 and 2) and
    /// some strings (`config_value_str`, e.g. type 5); a numeric value is
    /// sent in both fields so either kind is updated.
    pub fn set_config(
        &self,
        config_type: i64,
        config_value: &str,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateOperationResponse> {
        let mut param = json!({"config_type": config_type, "config_value_str": config_value});
        if let Ok(number) = config_value.trim().parse::<i64>() {
            param["config_value"] = json!(number);
        }
        self.private(WRITE_MODULE, "SetConfig", param, credential)
    }

    /// Read a private message setting.
    pub fn get_config(
        &self,
        config_type: i64,
        config_value: &str,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateConfigResponse> {
        self.private(
            READ_MODULE,
            "GetConfig",
            json!({"config_type": config_type, "config_value_str": config_value}),
            credential,
        )
    }

    /// Musician message card.
    pub fn get_musician_message_card(
        &self,
        enc_uin: &str,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateMusicianCardResponse> {
        self.private("music.privateMsg.MusicianMsgCardSvr", "GetMusicianCard", json!({"EncUin": enc_uin}), credential)
    }

    /// Report an action on a card message.
    pub fn report_card_message_action(
        &self,
        target_user_id: &str,
        msg_type: i64,
        confirm: i64,
        msg_id: &str,
        ext: Option<Map<String, Value>>,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateOperationResponse> {
        let mut param = json!({
            "target_user_id": target_user_id,
            "msg_type": msg_type,
            "confirm": confirm,
            "msg_id": msg_id,
        });
        if let Some(ext) = ext.filter(|e| !e.is_empty()) {
            param["ext"] = Value::Object(ext);
        }
        self.private(WRITE_MODULE, "ActCardMsgCallBack", param, credential)
    }

    /// Chat entries for the given scenes.
    pub fn get_chat_entries(
        &self,
        scenes: Vec<i64>,
        from_user_type: Option<i64>,
        user_id: Option<&str>,
        ext: Option<Map<String, Value>>,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateChatEntriesResponse> {
        let mut param = json!({"Scence": scenes});
        if let Some(t) = from_user_type {
            param["FromUserType"] = json!(t);
        }
        if let Some(id) = user_id {
            param["UserID"] = json!(id);
        }
        if let Some(ext) = ext {
            param["Ext"] = Value::Object(ext);
        }
        self.private(READ_MODULE, "GetEntries", param, credential)
    }

    /// Details of media messages.
    pub fn get_media_message_details(
        &self,
        session_id: &str,
        msg_ids: Vec<String>,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateMediaMessageDetailsResponse> {
        // Unsigned requests are rejected with 2000.
        self.private(READ_MODULE, "GetMsgDetails", json!({"SessionID": session_id, "MsgIDs": msg_ids}), credential)
            .sign(true)
    }

    /// Mark every message as read.
    ///
    /// With `cmd_flag` 1 this clears the unread counters of **all** sessions;
    /// `encrypt_uin` does not narrow it to one peer.
    pub fn mark_all_messages_read(
        &self,
        cmd_flag: i64,
        encrypt_uin: &str,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateOperationResponse> {
        self.private(
            WRITE_MODULE,
            "SetAllMsgMardRead",
            json!({"CmdFlag": cmd_flag, "EncryptUin": encrypt_uin}),
            credential,
        )
    }

    /// Safety hint shown before chatting with a user.
    pub fn get_safety_hint(
        &self,
        enc_uin: &str,
        close: i64,
        credential: Option<Credential>,
    ) -> CgiRequest<PrivateSafetyHintResponse> {
        self.private(READ_MODULE, "GetSafetyHint", json!({"encUin": enc_uin, "close": close}), credential)
    }

    /// Friendship floating badge (raw data).
    pub fn get_friendship_badge(&self, target_enc_uin: &str, credential: Option<Credential>) -> CgiRequest<Value> {
        // Unsigned requests are rejected with 500031.
        self.private(
            "music.dazi.DzEntrySrv",
            "GetFriendFloatingIcon",
            json!({"TargetEncuin": target_enc_uin}),
            credential,
        )
        .sign(true)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::Error;
    use crate::testing::*;

    #[tokio::test]
    async fn requires_login_and_android() {
        let (client, mock) = mock_client();
        let err = client.private_message().get_config(1, "", None).await.unwrap_err();
        assert!(matches!(err, Error::CredentialInvalid(_)));
        assert_eq!(mock.request_count(), 0);

        let (client, mock) = logged_in_client();
        client.set_platform(Platform::Web);
        push_cgi(&mock, json!({"hint": "h"}));
        let hint = client.private_message().get_safety_hint("e", 0, None).await.unwrap();
        assert_eq!(hint.hint, "h");
        let body = last_body(&mock);
        assert_eq!(body["comm"]["ct"], "11", "forced Android comm");
        assert_eq!(body["req_0"]["param"], json!({"encUin": "e", "close": 0}));
    }

    #[tokio::test]
    async fn session_pagination() {
        let (client, mock) = logged_in_client();
        push_cgi(
            &mock,
            json!({"has_more": 1, "sessions": [{"session_id": "a", "sort_time": 5}, {"session_id": "b", "sort_time": 9}]}),
        );
        push_cgi(&mock, json!({"has_more": 0, "sessions": [{"session_id": "c"}]}));
        let sessions = client
            .private_message()
            .get_sessions(SessionListOptions::default(), None)
            .collect_items(None)
            .await
            .unwrap();
        assert_eq!(sessions.len(), 3);
        let requests = mock.requests();
        let first = requests[0].json_body().unwrap()["req_0"]["param"].clone();
        assert_eq!(first, json!({"last_id": "", "order": 1, "size": 20, "last_time": 0, "from": 0, "FansFlag": 1}));
        let second = requests[1].json_body().unwrap()["req_0"]["param"].clone();
        assert_eq!((second["last_id"].as_str(), second["last_time"].as_i64()), (Some("b"), Some(9)));

        let options = SessionListOptions { encrypt_from_uin: Some("enc".into()), ..SessionListOptions::default() };
        push_cgi(&mock, json!({"has_more": 1, "sessions": []}));
        let page = client.private_message().get_sessions(options, None).collect_items(None).await.unwrap();
        assert!(page.is_empty());
        let param = last_req0(&mock)["param"].clone();
        assert_eq!(param["EncryptFromUin"], "enc");
        assert!(param.get("FansFlag").is_none());
    }

    #[tokio::test]
    async fn message_pagination_and_send() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"has_more": 1, "messages": [{"id": "m1"}, {"id": "m2"}]}));
        push_cgi(&mock, json!({"has_more": 1, "messages": []}));
        let options =
            MessageListOptions { session_id: "s".into(), update_time: Some(0), ..MessageListOptions::default() };
        let messages = client.private_message().get_messages(options, None).collect_items(None).await.unwrap();
        assert_eq!(messages.len(), 2);
        let requests = mock.requests();
        let first = requests[0].json_body().unwrap()["req_0"]["param"].clone();
        assert_eq!(first, json!({"order": 1, "size": 50, "flag": 0, "session_id": "s", "update_time": 0}));
        assert_eq!(requests[1].json_body().unwrap()["req_0"]["param"]["last_id"], "m2");

        push_cgi(&mock, json!({"messages": [{"id": "x"}], "tips": "ok"}));
        let options = SendMessageOptions { meta_data: Some(json!({"content": "hi"})), ..SendMessageOptions::default() };
        let sent = client.private_message().send_message("u1", 0, options, None).await.unwrap();
        assert_eq!(sent.messages[0].id, "x");
        let req = last_req0(&mock);
        assert_eq!(req["method"], "SendMessageAsync");
        assert_eq!(
            req["param"],
            json!({"last_msg_seq": 0, "user_id": "u1", "entrance": 0, "client_key": "", "msg_type": 0, "meta_data": {"content": "hi"}, "super_msg_flag": 0})
        );
        push_cgi(&mock, json!({}));
        let options = SendMessageOptions { star_send: true, super_msg_flag: None, ..SendMessageOptions::default() };
        client.private_message().send_message("u1", 1, options, None).await.unwrap();
        let req = last_req0(&mock);
        assert_eq!(req["method"], "StarSendSuperMsg");
        assert!(req["param"].get("super_msg_flag").is_none());
    }

    #[tokio::test]
    async fn write_and_misc_requests() {
        let (client, mock) = logged_in_client();
        mock.route_url("musicu", reply_all(json!({})));
        mock.route_url("musics", reply_all(json!({})));
        let api = client.private_message();
        let mut ext = Map::new();
        ext.insert("k".into(), json!("v"));
        let cases: Vec<(&str, &str, Value)> = vec![
            ("DeleteSession", WRITE_MODULE, json!({"session_id": "s", "super_msg_flag": 0})),
            ("DeleteMessage", WRITE_MODULE, json!({"session_id": "s", "msg_id": "m", "super_msg_flag": 1})),
            ("ClearSession", WRITE_MODULE, json!({"session_id": "s", "super_msg_flag": 0})),
            ("SetConfig", WRITE_MODULE, json!({"config_type": 2, "config_value_str": "1", "config_value": 1})),
            ("GetConfig", READ_MODULE, json!({"config_type": 2, "config_value_str": ""})),
            ("GetMusicianCard", "music.privateMsg.MusicianMsgCardSvr", json!({"EncUin": "e"})),
            (
                "ActCardMsgCallBack",
                WRITE_MODULE,
                json!({"target_user_id": "t", "msg_type": 1, "confirm": 1, "msg_id": "m", "ext": {"k": "v"}}),
            ),
            ("GetEntries", READ_MODULE, json!({"Scence": [1, 2], "UserID": "u"})),
            ("GetMsgDetails", READ_MODULE, json!({"SessionID": "s", "MsgIDs": ["a"]})),
            ("SetAllMsgMardRead", WRITE_MODULE, json!({"CmdFlag": 1, "EncryptUin": "e"})),
            ("GetFriendFloatingIcon", "music.dazi.DzEntrySrv", json!({"TargetEncuin": "t"})),
        ];
        api.delete_session("s", 0, None).await.unwrap();
        let mut seen = vec![last_req0(&mock)];
        api.delete_message("s", "m", 1, None).await.unwrap();
        seen.push(last_req0(&mock));
        api.clear_session("s", 0, None).await.unwrap();
        seen.push(last_req0(&mock));
        api.set_config(2, "1", None).await.unwrap();
        seen.push(last_req0(&mock));
        api.get_config(2, "", None).await.unwrap();
        seen.push(last_req0(&mock));
        api.get_musician_message_card("e", None).await.unwrap();
        seen.push(last_req0(&mock));
        api.report_card_message_action("t", 1, 1, "m", Some(ext), None).await.unwrap();
        seen.push(last_req0(&mock));
        api.get_chat_entries(vec![1, 2], None, Some("u"), None, None).await.unwrap();
        seen.push(last_req0(&mock));
        api.get_media_message_details("s", vec!["a".into()], None).await.unwrap();
        assert!(mock.last_request().unwrap().query_param("sign").is_some());
        seen.push(last_req0(&mock));
        api.mark_all_messages_read(1, "e", None).await.unwrap();
        seen.push(last_req0(&mock));
        api.get_friendship_badge("t", None).await.unwrap();
        assert!(mock.last_request().unwrap().query_param("sign").is_some());
        seen.push(last_req0(&mock));
        api.set_config(5, "abc", None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"config_type": 5, "config_value_str": "abc"}));
        for ((method, module, param), req) in cases.into_iter().zip(seen) {
            assert_eq!(req["method"], method);
            assert_eq!(req["module"], module, "{method}");
            assert_eq!(req["param"], param, "{method}");
        }
    }
}
