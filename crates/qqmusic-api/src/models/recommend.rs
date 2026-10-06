//! Recommendation models.

use serde::Serialize;
use serde_json::{Map, Value};

use super::base::Song;
use crate::FromJson;
use crate::pagination::PageItems;

/// Niche of a feed shelf.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendNiche {
    /// Id.
    pub id: i64,
    /// Title template.
    pub title_template: String,
    /// Title content.
    pub title_content: String,
    /// Cards.
    #[json(alias = "v_card")]
    pub cards: Vec<Value>,
}

/// Feed shelf.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendShelf {
    /// Id.
    pub id: i64,
    /// Title template.
    pub title_template: String,
    /// Title content.
    pub title_content: String,
    /// More link.
    pub more: Map<String, Value>,
    /// Niches.
    #[json(alias = "v_niche")]
    pub niches: Vec<RecommendNiche>,
}

/// `get_home_feed` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendFeedCardResponse {
    /// Return code.
    pub retcode: i64,
    /// Message.
    pub msg: String,
    /// Prompt.
    pub prompt: String,
    /// d_num.
    pub d_num: i64,
    /// Load mark.
    pub load_mark: i64,
    /// Shelves.
    #[json(alias = "v_shelf")]
    pub shelves: Vec<RecommendShelf>,
}

impl PageItems for RecommendFeedCardResponse {
    type Item = RecommendShelf;
    fn into_items(self) -> Vec<RecommendShelf> {
        self.shelves
    }
}

/// `get_guess_recommend` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GuessRecommendResponse {
    /// Songs.
    #[json(alias = "tracks")]
    pub songs: Vec<Song>,
}

/// `get_radar_recommend` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RadarRecommendResponse {
    /// Songs.
    #[json(path = "$.VecSongs[*].Track")]
    pub songs: Vec<Song>,
    /// Recommended ids.
    #[json(alias = "RecommendSongIds")]
    pub recommend_song_ids: Vec<i64>,
    /// Base ids.
    #[json(alias = "BaseSongIds")]
    pub base_song_ids: Vec<i64>,
    /// Has more.
    #[json(alias = "HasMore")]
    pub has_more: bool,
    /// Toast.
    pub toast: String,
    /// Timestamp.
    #[json(alias = "TimeStamp")]
    pub timestamp: i64,
    /// Video cards.
    #[json(alias = "VideoCards")]
    pub video_cards: Map<String, Value>,
}

impl PageItems for RadarRecommendResponse {
    type Item = Song;
    fn into_items(self) -> Vec<Song> {
        self.songs
    }
}

/// Recommended playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendSonglistItem {
    /// Id.
    #[json(alias("id", "tid", "dissid"))]
    pub id: i64,
    /// Dir id.
    #[json(alias("dirid", "dirId"))]
    pub dirid: i64,
    /// Title.
    #[json(alias("title", "dissname", "name", "dirName"))]
    pub title: String,
    /// Description.
    #[json(alias("desc", "description"))]
    pub desc: String,
    /// Cover URL.
    #[json(path = "$.cover.default_url")]
    pub picurl: String,
    /// Songs.
    #[json(alias("song_cnt", "songnum", "songNum"))]
    pub songnum: i64,
    /// Play count.
    #[json(alias("play_cnt", "listennum", "playCnt"))]
    pub listennum: i64,
    /// Creator nickname.
    #[json(path = "$.creator.nick")]
    pub creator_nick: String,
}

/// `get_recommend_songlist` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendSonglistResponse {
    /// Playlists.
    #[json(path = "$.List[*].Playlist.basic")]
    pub songlists: Vec<RecommendSonglistItem>,
    /// Has more.
    #[json(alias = "HasMore")]
    pub has_more: bool,
    /// Next offset.
    #[json(alias = "FromLimit")]
    pub from_limit: i64,
    /// Message.
    #[json(alias = "Msg")]
    pub msg: String,
}

impl PageItems for RecommendSonglistResponse {
    type Item = RecommendSonglistItem;
    fn into_items(self) -> Vec<RecommendSonglistItem> {
        self.songlists
    }
}

/// New song tag.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendNewSongTag {
    /// Id.
    pub id: i64,
    /// Tag id.
    pub tagid: i64,
    /// Tag.
    pub tag: String,
    /// Link.
    pub link: String,
    /// From type.
    pub from_type: i64,
}

/// `get_recommend_newsong` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RecommendNewSongResponse {
    /// Languages.
    pub lanlist: Vec<Value>,
    /// Language.
    pub lan: String,
    /// Songs.
    #[json(alias = "songlist")]
    pub songs: Vec<Song>,
    /// Message.
    pub ret_msg: String,
    /// Type.
    pub r#type: i64,
    /// Tags.
    #[json(alias = "songTagInfoList")]
    pub song_tags: Vec<RecommendNewSongTag>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn recommend_models() {
        let feed: RecommendFeedCardResponse =
            from_value(&json!({"v_shelf": [{"id": 1, "v_niche": [{"id": 2, "v_card": [{}]}]}]})).unwrap();
        assert_eq!(feed.shelves[0].niches[0].cards.len(), 1);
        let radar: RadarRecommendResponse =
            from_value(&json!({"VecSongs": [{"Track": {"id": 1, "mid": "a"}}], "HasMore": true, "TimeStamp": 5}))
                .unwrap();
        assert!(radar.has_more);
        assert_eq!(radar.songs[0].mid, "a");
        let lists: RecommendSonglistResponse = from_value(&json!({
            "List": [{"Playlist": {"basic": {"tid": 7, "title": "t", "cover": {"default_url": "u"}, "play_cnt": 10, "creator": {"nick": "n"}}}}],
            "HasMore": true, "FromLimit": 25
        }))
        .unwrap();
        let first = &lists.songlists[0];
        assert_eq!((first.id, first.picurl.as_str(), first.listennum, first.creator_nick.as_str()), (7, "u", 10, "n"));
        let new: RecommendNewSongResponse =
            from_value(&json!({"songlist": [{"id": 1, "mid": "a"}], "type": 5, "songTagInfoList": [{"tag": "x"}]}))
                .unwrap();
        assert_eq!((new.r#type, new.song_tags[0].tag.as_str()), (5, "x"));
    }
}
