//! Top list models.

use serde::Serialize;
use serde_json::Value;

use super::base::Song;
use crate::FromJson;
use crate::pagination::PageItems;

/// Preview song of a top list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TopPreviewSong {
    /// Rank.
    pub rank: i64,
    /// Rank type.
    #[json(alias = "rankType")]
    pub rank_type: i64,
    /// Rank value.
    #[json(alias = "rankValue")]
    pub rank_value: String,
    /// Song id.
    #[json(alias = "songId")]
    pub id: i64,
    /// Title.
    #[json(alias = "title")]
    pub name: String,
    /// Singer name.
    #[json(alias = "singerName")]
    pub singer_name: String,
    /// Singer mid.
    #[json(alias = "singerMid")]
    pub singer_mid: String,
    /// Album mid.
    #[json(alias = "albumMid")]
    pub album_mid: String,
    /// Cover.
    pub cover: String,
    /// MV id.
    #[json(alias = "mvid")]
    pub mv_id: i64,
}

/// Top list summary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TopSummary {
    /// Id.
    #[json(alias = "topId")]
    pub id: i64,
    /// Name.
    #[json(alias = "title")]
    pub name: String,
    /// Detailed title.
    #[json(alias = "titleDetail")]
    pub title_detail: String,
    /// Sub title.
    #[json(alias = "titleSub")]
    pub title_sub: String,
    /// Introduction.
    pub intro: String,
    /// Period.
    pub period: String,
    /// Update time.
    #[json(alias = "updateTime")]
    pub update_time: String,
    /// Listen count.
    #[json(alias = "listenNum")]
    pub listen_num: i64,
    /// Total songs.
    #[json(alias = "totalNum")]
    pub total_num: i64,
    /// Preview songs.
    #[json(alias = "song")]
    pub songs: Vec<TopPreviewSong>,
    /// Front picture.
    #[json(alias = "frontPicUrl")]
    pub front_pic_url: String,
    /// Head picture.
    #[json(alias = "headPicUrl")]
    pub head_pic_url: String,
    /// H5 URL.
    #[json(alias = "h5JumpUrl")]
    pub h5_jump_url: String,
    /// Special scheme.
    #[json(alias = "specialScheme")]
    pub special_scheme: String,
}

/// Top list group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TopCategory {
    /// Group id.
    #[json(alias = "groupId")]
    pub id: i64,
    /// Group name.
    #[json(alias = "groupName")]
    pub name: String,
    /// Top lists.
    pub toplist: Vec<TopSummary>,
}

/// `get_category` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TopCategoryResponse {
    /// Groups.
    pub group: Vec<TopCategory>,
}

/// `get_detail` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TopDetailResponse {
    /// Summary.
    #[json(alias = "data")]
    pub info: TopSummary,
    /// Songs.
    #[json(alias = "songInfoList")]
    pub songs: Vec<Song>,
    /// Song tags.
    #[json(alias = "songTagInfoList")]
    pub song_tags: Vec<Value>,
    /// Extra info.
    #[json(alias = "extInfoList")]
    pub ext_info_list: Vec<Value>,
    /// Index info.
    #[json(alias = "indexInfoList")]
    pub index_info_list: Vec<Value>,
}

impl PageItems for TopDetailResponse {
    type Item = Song;
    fn into_items(self) -> Vec<Song> {
        self.songs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn top_models() {
        let cat: TopCategoryResponse = from_value(&json!({
            "group": [{"groupId": 0, "groupName": "巅峰榜", "toplist": [{"topId": 4, "title": "流行指数", "song": [{"rank": 1, "songId": 9, "title": "t"}]}]}]
        }))
        .unwrap();
        let top = &cat.group[0].toplist[0];
        assert_eq!((top.id, top.name.as_str(), top.songs[0].id), (4, "流行指数", 9));
        let detail: TopDetailResponse = from_value(&json!({"data": {"topId": 4, "totalNum": 100}, "songInfoList": [{"id": 1, "mid": "m"}], "songTagInfoList": null})).unwrap();
        assert_eq!(detail.info.total_num, 100);
        assert!(detail.song_tags.is_empty());
        assert_eq!(detail.into_items().len(), 1);
    }
}
