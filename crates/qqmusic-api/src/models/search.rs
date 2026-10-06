//! Search models.

use serde::Serialize;
use serde_json::{Map, Value};

use super::base::{Album, Mv, Singer, Song, SongList};
use crate::FromJson;
use crate::pagination::PageItems;

/// Song search result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongSearch {
    /// Song.
    #[json(flatten)]
    #[serde(flatten)]
    pub song: Song,
    /// Highlighted title.
    pub search_title: String,
    /// Main title.
    pub title_main: String,
    /// Extra title.
    pub title_extra: String,
    /// Favourite count display.
    pub fav_show: String,
    /// Description.
    pub desc: String,
    /// Description icon.
    pub desc_icon: String,
    /// Hotness.
    pub hotness: Map<String, Value>,
    /// Hotness description.
    pub hotness_desc: String,
    /// Hotness entries.
    pub vec_hotness: Vec<Value>,
    /// Content.
    pub content: String,
    /// New status.
    #[json(alias = "newStatus")]
    pub new_status: i64,
    /// Protect flag.
    pub protect: i64,
    /// Related words.
    pub relatedword_group: Map<String, Value>,
}

/// Album ranking info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RankingInfo {
    /// Rank.
    pub rank: String,
    /// Top list.
    pub toplist: String,
}

/// Album search result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AlbumSearch {
    /// Album.
    #[json(flatten)]
    #[serde(flatten)]
    pub album: Album,
    /// Description detail.
    pub desc_detail: Map<String, Value>,
    /// Description.
    pub description: String,
    /// Description 2.
    pub description2: String,
    /// Album type.
    #[json(path = "$.core_album_config.album_type")]
    pub r#type: i64,
    /// Award label.
    #[json(path = "$.core_album_config.award_label")]
    pub award_label: String,
    /// Hotness.
    pub hotness: Map<String, Value>,
    /// Hotness description.
    pub hotness_desc: String,
    /// New label.
    pub label_new: Map<String, Value>,
    /// Audio play ranking.
    pub audio_play: RankingInfo,
    /// Picture.
    pub pic: String,
    /// Picture icon.
    pub pic_icon: String,
    /// Singer names.
    pub singer: String,
    /// Singers.
    pub singer_list: Vec<Singer>,
    /// Tags.
    pub tag_list: Vec<String>,
    /// URL.
    pub url: String,
}

/// Playlist search result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongListSearch {
    /// Playlist.
    #[json(flatten)]
    #[serde(flatten)]
    pub songlist: SongList,
    /// Creator nickname.
    pub nickname: String,
    /// Dir type.
    pub dirtype: i64,
}

/// Singer search result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerSearch {
    /// Singer.
    #[json(flatten)]
    #[serde(flatten)]
    pub singer: Singer,
    /// Picture.
    #[json(alias = "singerPic")]
    pub pic: String,
    /// Song count.
    #[json(alias = "songNum")]
    pub song_num: i64,
    /// Album count.
    #[json(alias = "albumNum")]
    pub album_num: i64,
    /// MV count.
    #[json(alias = "mvNum")]
    pub mv_num: i64,
    /// Subtitle.
    pub subtitle: String,
}

/// MV search result.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MvSearch {
    /// MV.
    #[json(flatten)]
    #[serde(flatten)]
    pub mv: Mv,
    /// Picture.
    pub pic: String,
    /// Play count.
    pub play_count: i64,
    /// Duration.
    pub duration: i64,
    /// Publish date.
    pub publish_date: String,
    /// Singer id.
    #[json(alias = "singerid")]
    pub singer_id: i64,
    /// Singer mid.
    #[json(alias = "singermid")]
    pub singer_mid: String,
    /// Singer name.
    #[json(alias = "singername")]
    pub singer_name: String,
}

/// One section of a general search.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GeneralSearchSection<T> {
    /// Estimated total.
    pub estimate_sum: i64,
    /// Total.
    pub total_num: i64,
    /// Items.
    pub items: Vec<T>,
    /// More info.
    pub more_info: Map<String, Value>,
}

/// Related search word.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RelatedSearchWord {
    /// Display text.
    #[json(alias = "display_word")]
    pub display: String,
    /// Search text.
    #[json(alias = "search_word")]
    pub search: String,
}

/// Search filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
#[json(default)]
pub struct SearchSelector {
    /// Id.
    pub id: i64,
    /// Name.
    pub name: String,
    /// Type.
    pub r#type: i64,
}

/// `search_by_type` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SearchByTypeResponse {
    /// Search id.
    #[json(path = "$.meta.searchid")]
    pub searchid: String,
    /// Items per page.
    #[json(path = "$.meta.perpage")]
    pub perpage: i64,
    /// Next page (`-1` = none).
    #[json(path = "$.meta.nextpage")]
    pub nextpage: i64,
    /// Estimated total.
    #[json(path = "$.meta.estimate_sum")]
    pub estimate_sum: i64,
    /// Total.
    #[json(path = "$.meta.sum")]
    pub total_num: i64,
    /// Songs.
    #[json(path = "$.body.item_song")]
    pub song: Vec<SongSearch>,
    /// Singers.
    #[json(path = "$.body.singer")]
    pub singer: Vec<SingerSearch>,
    /// Albums.
    #[json(path = "$.body.item_album")]
    pub album: Vec<AlbumSearch>,
    /// Playlists.
    #[json(path = "$.body.item_songlist")]
    pub songlist: Vec<SongListSearch>,
    /// Users.
    #[json(path = "$.body.item_user")]
    pub user: Vec<Value>,
    /// Audio albums.
    #[json(path = "$.body.item_audio")]
    pub audio_album: Vec<AlbumSearch>,
    /// MVs.
    #[json(path = "$.body.item_mv")]
    pub mv: Vec<MvSearch>,
    /// Filters.
    #[json(path = "$.body.multi_extern_info.selectors")]
    pub selectors: Vec<Vec<SearchSelector>>,
}

/// Item of a typed search.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "item", rename_all = "snake_case")]
pub enum SearchItem {
    /// Song (also lyric / audio / ringtone searches).
    Song(Box<SongSearch>),
    /// Singer.
    Singer(Box<SingerSearch>),
    /// Album (also audio albums).
    Album(Box<AlbumSearch>),
    /// Playlist.
    SongList(Box<SongListSearch>),
    /// MV.
    Mv(Box<MvSearch>),
    /// User (raw).
    User(Value),
}

impl SearchItem {
    /// Song, if this is a song.
    pub fn as_song(&self) -> Option<&SongSearch> {
        match self {
            Self::Song(song) => Some(song),
            _ => None,
        }
    }
}

impl PageItems for SearchByTypeResponse {
    type Item = SearchItem;

    fn into_items(self) -> Vec<SearchItem> {
        fn boxed<T>(items: Vec<T>, f: impl Fn(Box<T>) -> SearchItem) -> Vec<SearchItem> {
            items.into_iter().map(|i| f(Box::new(i))).collect()
        }
        if !self.song.is_empty() {
            boxed(self.song, SearchItem::Song)
        } else if !self.singer.is_empty() {
            boxed(self.singer, SearchItem::Singer)
        } else if !self.album.is_empty() {
            boxed(self.album, SearchItem::Album)
        } else if !self.songlist.is_empty() {
            boxed(self.songlist, SearchItem::SongList)
        } else if !self.mv.is_empty() {
            boxed(self.mv, SearchItem::Mv)
        } else if !self.user.is_empty() {
            self.user.into_iter().map(SearchItem::User).collect()
        } else {
            boxed(self.audio_album, SearchItem::Album)
        }
    }
}

/// `general_search` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GeneralSearchResponse {
    /// Search id.
    #[json(path = "$.meta.sid")]
    pub searchid: String,
    /// Items per page.
    #[json(path = "$.meta.perpage")]
    pub perpage: i64,
    /// Next page (`-1` = none).
    #[json(path = "$.meta.nextpage")]
    pub nextpage: i64,
    /// Continuation data.
    #[json(path = "$.meta.nextpage_start")]
    pub nextpage_start: Map<String, Value>,
    /// Songs.
    #[json(path = "$.body.item_song")]
    pub song: GeneralSearchSection<SongSearch>,
    /// Singers.
    #[json(path = "$.body.singer")]
    pub singer: GeneralSearchSection<SingerSearch>,
    /// MVs.
    #[json(path = "$.body.item_mv")]
    pub mv: GeneralSearchSection<MvSearch>,
    /// Albums.
    #[json(path = "$.body.item_album")]
    pub album: GeneralSearchSection<AlbumSearch>,
    /// Playlists.
    #[json(path = "$.body.item_songlist")]
    pub songlist: GeneralSearchSection<SongListSearch>,
    /// Audio.
    #[json(path = "$.body.item_audio")]
    pub audio: GeneralSearchSection<AlbumSearch>,
    /// Direct results.
    #[json(path = "$.body.direct_result.direct_group")]
    pub direct: Vec<Value>,
    /// Related words.
    #[json(path = "$.body.item_related")]
    pub related: GeneralSearchSection<RelatedSearchWord>,
}

/// Quick search item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct QuickSearchItem {
    /// Doc id.
    pub docid: String,
    /// Id.
    pub id: String,
    /// Mid.
    pub mid: String,
    /// Name.
    pub name: String,
    /// Singer.
    pub singer: String,
    /// Picture.
    pub pic: String,
    /// Vid.
    pub vid: String,
}

/// Quick search category.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct QuickSearchCategory {
    /// Count.
    pub count: i64,
    /// Items.
    pub itemlist: Vec<QuickSearchItem>,
    /// Name.
    pub name: String,
    /// Order.
    pub order: i64,
    /// Type.
    pub r#type: i64,
}

/// `quick_search` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct QuickSearchResponse {
    /// Songs.
    #[json(path = "$.data.song")]
    pub song: QuickSearchCategory,
    /// Singers.
    #[json(path = "$.data.singer")]
    pub singer: QuickSearchCategory,
    /// Albums.
    #[json(path = "$.data.album")]
    pub album: QuickSearchCategory,
    /// MVs.
    #[json(path = "$.data.mv")]
    pub mv: QuickSearchCategory,
}

/// Hot keyword.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct Hotkey {
    /// Id.
    pub hotkey_id: String,
    /// Query.
    pub query: String,
    /// Title.
    pub title: String,
    /// Score.
    pub score: String,
    /// Kind.
    pub kind: i64,
    /// Type.
    pub r#type: i64,
    /// Source.
    pub source: i64,
    /// Pinned.
    pub need_top: i64,
    /// Sub position.
    pub subpos: i64,
    /// Song type.
    pub song_type: i64,
    /// Direct id.
    pub direct_id: i64,
    /// Jump tab.
    #[json(default = "0".to_string())]
    pub jump_tab: String,
    /// Jump URL.
    pub jump_url: String,
    /// Cover picture URL.
    pub cover_pic_url: String,
    /// Picture URL.
    pub pic_url: String,
    /// Description.
    pub description: String,
}

/// `get_hotkey` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HotkeyResponse {
    /// Return code.
    pub ret_code: i64,
    /// Hotkey time.
    pub hotkey_time: String,
    /// Track list id.
    pub track_list_id: String,
    /// Hot keywords.
    pub vec_hotkey: Vec<Hotkey>,
    /// Recommended keywords.
    pub vec_reckey: Vec<Value>,
}

/// Completion item.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CompleteItem {
    /// Hint.
    pub hint: String,
    /// Highlighted hint.
    pub hint_hilight: String,
    /// Doc id.
    pub docid: String,
    /// Type.
    pub r#type: i64,
    /// Result type.
    pub res_type: String,
    /// Score.
    pub score: f64,
    /// Pre search.
    pub pre_search: bool,
    /// Icon.
    pub icon: String,
    /// Icon type.
    pub icon_type: i64,
    /// Jump tab.
    #[json(default = -1)]
    pub jumptab: i64,
    /// Jump type.
    pub jump_type: i64,
    /// Jump URL.
    pub jump_url: String,
    /// Picture URL.
    pub pic_url: String,
}

/// `complete` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CompleteResponse {
    /// Items.
    pub items: Vec<CompleteItem>,
    /// Total.
    pub total_num: i64,
    /// Search id.
    pub search_id: String,
    /// Expire time.
    pub expire_time: i64,
    /// Default search.
    pub use_default_search: i64,
    /// Debug info.
    pub debug_info: String,
    /// Experiment id.
    pub expid: String,
    /// History.
    pub history_items: Vec<Value>,
    /// Direct items.
    pub vec_direct_items: Vec<Value>,
    /// Related items.
    pub vec_related_items: Vec<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn by_type_items() {
        let res: SearchByTypeResponse = from_value(&json!({
            "meta": {"searchid": "s", "nextpage": 2, "sum": 30, "perpage": 10},
            "body": {
                "item_song": [{"id": 1, "mid": "m", "title_main": "t", "newStatus": 2}],
                "multi_extern_info": {"selectors": [[{"id": 1, "name": "n", "type": 3}]]}
            }
        }))
        .unwrap();
        assert_eq!((res.searchid.as_str(), res.nextpage, res.total_num), ("s", 2, 30));
        assert_eq!(res.selectors[0][0].r#type, 3);
        let items = res.into_items();
        assert_eq!(items[0].as_song().unwrap().song.mid, "m");
        assert_eq!(items[0].as_song().unwrap().new_status, 2);

        let singers: SearchByTypeResponse =
            from_value(&json!({"body": {"singer": [{"singerMID": "x", "singerPic": "p", "songNum": 3}]}})).unwrap();
        match &singers.into_items()[0] {
            SearchItem::Singer(s) => assert_eq!((s.singer.mid.as_str(), s.pic.as_str(), s.song_num), ("x", "p", 3)),
            other => panic!("unexpected {other:?}"),
        }
        let albums: SearchByTypeResponse =
            from_value(&json!({"body": {"item_audio": [{"albumMID": "a", "core_album_config": {"album_type": 5}}]}}))
                .unwrap();
        match &albums.into_items()[0] {
            SearchItem::Album(a) => assert_eq!((a.album.mid.as_str(), a.r#type), ("a", 5)),
            other => panic!("unexpected {other:?}"),
        }
        let users: SearchByTypeResponse = from_value(&json!({"body": {"item_user": [{"name": "u"}]}})).unwrap();
        assert!(matches!(&users.into_items()[0], SearchItem::User(_)));
        let empty: SearchByTypeResponse = from_value(&json!({})).unwrap();
        assert!(empty.into_items().is_empty());
        let mvs: SearchByTypeResponse =
            from_value(&json!({"body": {"item_mv": [{"vid": "v", "singername": "n"}]}})).unwrap();
        assert!(matches!(&mvs.into_items()[0], SearchItem::Mv(m) if m.singer_name == "n"));
        let lists: SearchByTypeResponse =
            from_value(&json!({"body": {"item_songlist": [{"dissid": 3, "nickname": "n"}]}})).unwrap();
        assert!(matches!(&lists.into_items()[0], SearchItem::SongList(l) if l.songlist.id == 3));
    }

    #[test]
    fn general_quick_hotkey_complete() {
        let res: GeneralSearchResponse = from_value(&json!({
            "meta": {"sid": "x", "nextpage": -1, "nextpage_start": {"a": 1}},
            "body": {"item_song": {"total_num": 1, "items": [{"id": 1, "mid": "m"}]}, "item_related": {"items": [{"display_word": "d", "search_word": "s"}]}}
        }))
        .unwrap();
        assert_eq!(res.song.items[0].song.id, 1);
        assert_eq!(res.related.items[0].search, "s");
        assert_eq!(res.nextpage_start["a"], 1);
        let quick: QuickSearchResponse =
            from_value(&json!({"data": {"song": {"count": 1, "itemlist": [{"mid": "m", "id": 12}]}}})).unwrap();
        assert_eq!(quick.song.itemlist[0].id, "12");
        let hot: HotkeyResponse = from_value(&json!({"vec_hotkey": [{"query": "q"}]})).unwrap();
        assert_eq!(hot.vec_hotkey[0].jump_tab, "0");
        let complete: CompleteResponse = from_value(&json!({"items": [{"hint": "h"}]})).unwrap();
        assert_eq!(complete.items[0].jumptab, -1);
    }
}
