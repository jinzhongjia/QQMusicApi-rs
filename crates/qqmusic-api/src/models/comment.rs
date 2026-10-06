//! Comment models.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::FromJson;
use crate::pagination::PageItems;

/// Commented resource type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub enum CommentBizType {
    /// Song.
    #[default]
    Song,
    /// Album.
    Album,
    /// Playlist.
    Playlist,
    /// MV.
    Mv,
    /// Special audio.
    SpecialAudio,
}

impl CommentBizType {
    /// Numeric value.
    pub fn code(self) -> i64 {
        match self {
            Self::Song => 1,
            Self::Album => 2,
            Self::Playlist => 3,
            Self::Mv => 4,
            Self::SpecialAudio => 15,
        }
    }
}

/// Icon text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct IconTextInfo {
    /// Text.
    pub txt: String,
    /// Unique id.
    pub unique_id: String,
    /// Type.
    pub r#type: i64,
    /// Comment id.
    pub cmid: String,
    /// Dynamic.
    pub is_dynamic: bool,
}

/// `get_comment_count` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CommentCountResponse {
    /// Biz type.
    #[json(path = "$.response.biz_type")]
    pub biz_type: i64,
    /// Biz id.
    #[json(path = "$.response.biz_id")]
    pub biz_id: String,
    /// Biz sub type.
    #[json(path = "$.response.biz_sub_type")]
    pub biz_sub_type: i64,
    /// Count.
    #[json(path = "$.response.count")]
    pub count: i64,
    /// Count version.
    #[json(path = "$.response.count_ver")]
    pub count_ver: String,
    /// Count display.
    #[json(path = "$.response.count_view")]
    pub count_view: String,
    /// Related id.
    #[json(path = "$.response.related_id")]
    pub related_id: String,
    /// Tip.
    #[json(path = "$.response.tip")]
    pub tip: String,
    /// Icons.
    #[json(path = "$.response.icon_list[*]")]
    pub icon_list: Vec<IconTextInfo>,
    /// Tab type.
    #[json(path = "$.cmTabType")]
    pub cm_tab_type: i64,
}

/// Comment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CommentItem {
    /// Comment id.
    #[json(alias = "CmId")]
    pub cmid: String,
    /// Sequence number (pagination cursor).
    #[json(alias = "SeqNo")]
    pub seq_no: String,
    /// Nickname.
    #[json(alias = "Nick")]
    pub nick: String,
    /// Avatar.
    #[json(alias = "Avatar")]
    pub avatar: String,
    /// Encrypted uin.
    #[json(alias = "EncryptUin")]
    pub encrypt_uin: String,
    /// Content.
    #[json(alias = "Content")]
    pub content: String,
    /// Publish time.
    #[json(alias = "PubTime")]
    pub pub_time: i64,
    /// Likes.
    #[json(alias = "PraiseNum")]
    pub praise_num: i64,
    /// Replies.
    #[json(alias = "ReplyCnt")]
    pub reply_cnt: i64,
    /// Liked by me.
    #[json(alias = "IsPraised")]
    pub is_praised: i64,
    /// Written by me.
    #[json(alias = "IsSelf")]
    pub is_self: i64,
    /// State.
    #[json(alias = "State")]
    pub state: i64,
    /// Hot score.
    #[json(alias = "HotScore")]
    pub hot_score: String,
    /// Recommendation score.
    #[json(alias = "RecScore")]
    pub rec_score: String,
    /// Song id.
    #[json(alias = "SongId")]
    pub song_id: i64,
    /// Song name.
    #[json(alias = "SongName")]
    pub song_name: String,
    /// Singer names.
    #[json(alias = "SingerNames")]
    pub singer_names: String,
    /// Timestamp elements.
    #[json(alias = "SongTsElems")]
    pub song_ts_elems: Vec<Value>,
    /// Hash tags.
    #[json(alias = "HashTagList")]
    pub hash_tag_list: Vec<Value>,
    /// Tails.
    #[json(alias = "LittleTails")]
    pub little_tails: Vec<Value>,
    /// Icons.
    #[json(alias = "IconList")]
    pub icon_list: Vec<Value>,
    /// VIP UI.
    #[json(alias = "VipUI")]
    pub vip_ui: Map<String, Value>,
    /// Sub comments.
    #[json(alias = "SubComments")]
    pub sub_comments: Vec<Value>,
}

/// Comment list response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CommentListResponse {
    /// Comments.
    #[json(path = "$.CommentList.Comments[*]")]
    pub comments: Vec<CommentItem>,
    /// Comment ids.
    #[json(path = "$.CommentList.CommentIds[*]")]
    pub comment_ids: Vec<String>,
    /// Has more.
    #[json(path = "$.CommentList.HasMore")]
    pub has_more: i64,
    /// Next offset.
    #[json(path = "$.CommentList.NextOffset")]
    pub next_offset: i64,
    /// Total.
    #[json(path = "$.CommentList.Total")]
    pub total: i64,
    /// Total comments.
    #[json(alias = "TotalCmNum")]
    pub total_cm_num: i64,
    /// Tip.
    #[json(alias = "CommentTip")]
    pub comment_tip: String,
    /// H5 page.
    #[json(alias = "CommentH5Page")]
    pub comment_h5_page: String,
    /// Has moment comments.
    #[json(alias = "HasTsCm")]
    pub has_ts_cm: i64,
    /// Share count.
    #[json(alias = "ShareCnt")]
    pub share_cnt: i64,
    /// Message.
    #[json(alias = "Msg")]
    pub msg: String,
    /// Sub code.
    #[json(alias = "SubCode")]
    pub sub_code: i64,
}

impl PageItems for CommentListResponse {
    type Item = CommentItem;
    fn into_items(self) -> Vec<CommentItem> {
        self.comments
    }
}

/// Moment (timestamped) comment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MomentCommentItem {
    /// Comment id.
    #[json(alias = "CmId")]
    pub cmid: String,
    /// Sequence number.
    #[json(alias = "SeqNo")]
    pub seq_no: String,
    /// Content.
    #[json(alias = "Content")]
    pub content: String,
    /// Encrypted uin.
    #[json(alias = "EncryptUin")]
    pub encrypt_uin: String,
    /// Publish time.
    #[json(alias = "PubTime")]
    pub pub_time: i64,
    /// Likes.
    #[json(alias = "PraiseNum")]
    pub praise_num: i64,
    /// Replies.
    #[json(alias = "ReplyCnt")]
    pub reply_cnt: i64,
    /// State.
    #[json(alias = "State")]
    pub state: i64,
    /// Written by me.
    #[json(alias = "IsSelf")]
    pub is_self: i64,
    /// Location.
    #[json(alias = "Location")]
    pub location: String,
    /// Phone type.
    #[json(alias = "PhoneType")]
    pub phone_type: String,
    /// Picture.
    #[json(alias = "Pic")]
    pub pic: String,
    /// Picture size.
    #[json(alias = "PicSize")]
    pub pic_size: String,
    /// Timestamp elements.
    #[json(alias = "SongTsElems")]
    pub song_ts_elems: Vec<Value>,
    /// Hash tags.
    #[json(alias = "HashTagList")]
    pub hash_tag_list: Vec<Value>,
    /// Tails.
    #[json(alias = "LittleTails")]
    pub little_tails: Vec<Value>,
}

/// `get_moment_comments` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MomentCommentResponse {
    /// Comments.
    #[json(path = "$.CmList[*]")]
    pub comments: Vec<MomentCommentItem>,
    /// Has more.
    #[json(alias = "HasMore")]
    pub has_more: i64,
    /// Next cursor.
    #[json(alias = "NextPos")]
    pub next_pos: String,
    /// Hint.
    #[json(alias = "Hint")]
    pub hint: String,
    /// Previous list loaded.
    #[json(alias = "PrevListLoaded")]
    pub prev_list_loaded: i64,
    /// Extra info by comment id.
    #[json(alias = "MapCmExt")]
    pub map_cm_ext: BTreeMap<String, Map<String, Value>>,
}

impl PageItems for MomentCommentResponse {
    type Item = MomentCommentItem;
    fn into_items(self) -> Vec<MomentCommentItem> {
        self.comments
    }
}

/// `add_comment` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AddCommentResponse {
    /// Sub code.
    #[json(alias = "SubCode")]
    pub subcode: i64,
    /// Message.
    #[json(alias = "Msg")]
    pub msg: String,
    /// New comment id.
    #[json(alias = "AddedCmId")]
    pub id: String,
    /// Parent comment id.
    #[json(alias = "ParentCmId")]
    pub parent: String,
    /// Floor.
    #[json(path = "$.Floor.Num")]
    pub floor: i64,
    /// Verification URL.
    #[json(alias = "VerifyUrl")]
    pub verify_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn comment_models() {
        let count: CommentCountResponse =
            from_value(&json!({"response": {"biz_id": "1", "count": 99, "icon_list": [{"txt": "t"}]}, "cmTabType": 2}))
                .unwrap();
        assert_eq!((count.count, count.cm_tab_type, count.icon_list[0].txt.as_str()), (99, 2, "t"));
        let list: CommentListResponse = from_value(&json!({
            "CommentList": {"Comments": [{"CmId": "c", "SeqNo": "s", "Content": "hi", "VipUI": {"a": 1}}], "HasMore": 1, "Total": 5, "CommentIds": ["c"]},
            "TotalCmNum": 5
        }))
        .unwrap();
        assert_eq!(list.comments[0].seq_no, "s");
        assert_eq!(list.comment_ids, vec!["c"]);
        assert_eq!((list.has_more, list.total, list.total_cm_num), (1, 5, 5));
        let moment: MomentCommentResponse =
            from_value(&json!({"CmList": [{"CmId": "m"}], "HasMore": 1, "NextPos": "p", "MapCmExt": {"m": {"x": 1}}}))
                .unwrap();
        assert_eq!(moment.next_pos, "p");
        assert_eq!(moment.map_cm_ext["m"]["x"], 1);
        let added: AddCommentResponse = from_value(&json!({"AddedCmId": "n", "Floor": {"Num": 3}})).unwrap();
        assert_eq!((added.id.as_str(), added.floor), ("n", 3));
        assert_eq!(CommentBizType::SpecialAudio.code(), 15);
    }
}
