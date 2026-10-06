//! MV models.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::base::{Mv, Singer};
use crate::FromJson;
use crate::pagination::PageItems;

/// MV detail.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MvDetail {
    /// MV.
    #[json(flatten)]
    #[serde(flatten)]
    pub mv: Mv,
    /// Cover.
    pub cover_pic: String,
    /// Duration.
    pub duration: i64,
    /// Singers.
    pub singers: Vec<Value>,
    /// Video switch.
    pub video_switch: i64,
    /// Message.
    pub msg: String,
    /// Description.
    pub desc: String,
    /// Play count.
    pub playcnt: i64,
    /// Publish date.
    pub pubdate: i64,
    /// Favourite flag.
    pub isfav: i64,
    /// gmid.
    pub gmid: String,
    /// Uploader avatar.
    pub uploader_headurl: String,
    /// Uploader nickname.
    pub uploader_nick: String,
    /// Uploader encrypted uin.
    pub uploader_encuin: String,
    /// Uploader uin.
    pub uploader_uin: String,
    /// Following the uploader.
    pub uploader_hasfollow: i64,
    /// Uploader followers.
    pub uploader_follower_num: i64,
    /// Related song ids.
    pub related_songs: Vec<i64>,
}

/// `get_detail` response (by vid).
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetMvDetailResponse {
    /// Details by vid.
    #[json(path = "$")]
    pub data: BTreeMap<String, MvDetail>,
}

/// One MV stream.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MvUrlItem {
    /// URLs.
    pub url: Vec<String>,
    /// Free-flow URLs.
    pub freeflow_url: Vec<String>,
    /// Common URLs.
    pub comm_url: Vec<String>,
    /// File name.
    pub cn: String,
    /// vkey.
    pub vkey: String,
    /// Expiration.
    pub expire: i64,
    /// Code.
    pub code: i64,
    /// File type (resolution).
    pub filetype: i64,
    /// HLS playlist.
    pub m3u8: String,
    /// New file type.
    #[json(alias = "newFileType")]
    pub new_file_type: i64,
    /// Format.
    pub format: i64,
    /// File size.
    #[json(alias = "fileSize")]
    pub file_size: i64,
}

/// Streams of one MV.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MvUrlSet {
    /// MP4 streams.
    pub mp4: Vec<MvUrlItem>,
    /// HLS streams.
    pub hls: Vec<MvUrlItem>,
    /// SVP flag.
    pub svp_flag: i64,
    /// Duration.
    pub duration: i64,
}

/// `get_mv_urls` response (by vid).
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetMvUrlsResponse {
    /// Streams by vid.
    #[json(path = "$")]
    pub data: BTreeMap<String, MvUrlSet>,
}

/// MV list item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MvListItem {
    /// MV.
    #[json(flatten)]
    #[serde(flatten)]
    pub mv: Mv,
    /// Singers.
    pub singers: Vec<Singer>,
    /// Subtitle.
    pub subtitle: String,
    /// Play count.
    pub playcnt: i64,
    /// Publish date.
    pub pubdate: i64,
    /// Duration.
    pub duration: i64,
    /// Picture.
    pub picurl: String,
}

/// `get_mv_list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetMvListResponse {
    /// Total.
    pub total: i64,
    /// Items.
    #[json(alias = "list")]
    pub items: Vec<MvListItem>,
}

impl PageItems for GetMvListResponse {
    type Item = MvListItem;
    fn into_items(self) -> Vec<MvListItem> {
        self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn mv_models() {
        let detail: GetMvDetailResponse =
            from_value(&json!({"v1": {"vid": "v1", "name": "n", "related_songs": [1]}})).unwrap();
        assert_eq!(detail.data["v1"].mv.name, "n");
        assert_eq!(detail.data["v1"].related_songs, vec![1]);
        let urls: GetMvUrlsResponse =
            from_value(&json!({"v1": {"mp4": [{"url": ["u"], "fileSize": 5, "newFileType": 2}], "hls": []}})).unwrap();
        assert_eq!(urls.data["v1"].mp4[0].file_size, 5);
        let list: GetMvListResponse =
            from_value(&json!({"total": 1, "list": [{"vid": "v", "singers": [{"mid": "s"}]}]})).unwrap();
        assert_eq!(list.into_items()[0].singers[0].mid, "s");
    }
}
