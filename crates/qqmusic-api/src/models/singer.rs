//! Singer models.

use serde::Serialize;
use serde_json::{Map, Value};

use super::base::{CoverSize, Song, photo_new_cover_url};
use crate::FromJson;
use crate::pagination::PageItems;

/// Filter option.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, FromJson)]
#[json(default)]
pub struct TagOption {
    /// Id.
    pub id: i64,
    /// Name.
    pub name: String,
}

/// Singer in a list.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerBrief {
    /// Id.
    #[json(alias("singer_id", "singerId", "id"), default = -1)]
    pub id: i64,
    /// Mid.
    #[json(alias("singer_mid", "singerMid", "mid"))]
    pub mid: String,
    /// Name.
    #[json(alias("singer_name", "singerName", "name"))]
    pub name: String,
    /// Picture mid.
    #[json(alias("singer_pmid", "singerPmid", "pmid"))]
    pub pmid: String,
    /// Area id.
    #[json(default = -1)]
    pub area_id: i64,
    /// Country id.
    #[json(default = -1)]
    pub country_id: i64,
    /// Country.
    pub country: String,
    /// Other name.
    pub other_name: String,
    /// Spelling.
    pub spell: String,
    /// Trend.
    pub trend: i64,
    /// Followers.
    #[json(alias = "concernNum")]
    pub concern_num: i64,
    /// Picture URL.
    pub singer_pic: String,
}

impl SingerBrief {
    /// Cover URL.
    pub fn cover_url(&self, size: CoverSize) -> String {
        let mid = if self.mid.is_empty() { &self.pmid } else { &self.mid };
        photo_new_cover_url("T001", mid, size)
    }
}

/// Available filters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerTagData {
    /// Areas.
    pub area: Vec<TagOption>,
    /// Genres.
    pub genre: Vec<TagOption>,
    /// Sexes.
    pub sex: Vec<TagOption>,
    /// Indexes.
    pub index: Vec<TagOption>,
}

/// `get_singer_list` response.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerTypeListResponse {
    /// Area.
    #[json(default = -100)]
    pub area: i64,
    /// Sex.
    #[json(default = -100)]
    pub sex: i64,
    /// Genre.
    #[json(default = -100)]
    pub genre: i64,
    /// Singers.
    pub singerlist: Vec<SingerBrief>,
    /// Code.
    pub code: i64,
    /// Hot singers.
    pub hotlist: Vec<SingerBrief>,
    /// Filters.
    pub tags: SingerTagData,
}

/// `get_singer_list_index` response.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerIndexPageResponse {
    /// Area.
    #[json(default = -100)]
    pub area: i64,
    /// Sex.
    #[json(default = -100)]
    pub sex: i64,
    /// Genre.
    #[json(default = -100)]
    pub genre: i64,
    /// Index.
    #[json(default = -100)]
    pub index: i64,
    /// Total.
    pub total: i64,
    /// Singers.
    pub singerlist: Vec<SingerBrief>,
    /// Code.
    pub code: i64,
    /// Hot singers.
    pub hotlist: Vec<SingerBrief>,
    /// Filters.
    pub tags: SingerTagData,
}

impl PageItems for SingerIndexPageResponse {
    type Item = SingerBrief;
    fn into_items(self) -> Vec<SingerBrief> {
        self.singerlist
    }
}

/// Homepage base info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HomepageBaseInfo {
    /// Encrypted uin.
    #[json(alias = "EncryptedUin")]
    pub encrypted_uin: String,
    /// Background image.
    #[json(alias = "BackgroundImage")]
    pub background_image: String,
    /// Avatar.
    #[json(alias = "Avatar")]
    pub avatar: String,
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
    /// Host flag.
    #[json(alias = "IsHost")]
    pub is_host: i64,
    /// Singer flag.
    #[json(alias = "IsSinger")]
    pub is_singer: i64,
    /// User type.
    #[json(alias = "UserType")]
    pub user_type: i64,
}

/// Special name display.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerNameSpecialDisplay {
    /// Display type.
    #[json(alias = "DisplayType")]
    pub display_type: i64,
    /// Picture file.
    #[json(alias = "PicFile")]
    pub pic_file: String,
    /// Overlap ratio.
    #[json(alias = "SignatureNameOverlapRatio")]
    pub signature_name_overlap_ratio: f64,
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
}

/// Singer on the homepage.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HomepageSinger {
    /// Id.
    #[json(alias("SingerID", "singerID", "singer_id", "id"), default = -1)]
    pub id: i64,
    /// Mid.
    #[json(alias("SingerMid", "singerMid", "singer_mid", "mid"))]
    pub mid: String,
    /// Name.
    #[json(alias("Name", "name", "singerName"))]
    pub name: String,
    /// Type.
    #[json(alias("SingerType", "type"), default = -1)]
    pub r#type: i64,
    /// Picture.
    #[json(alias = "SingerPic")]
    pub singer_pic: String,
    /// Picture mid.
    #[json(alias = "SingerPMid")]
    pub singer_pmid: String,
    /// Special display.
    #[json(alias("SingerNameSpecialDisplay", "singerNameSpecialDisplay"))]
    pub name_special_display: Option<SingerNameSpecialDisplay>,
}

impl Default for HomepageSinger {
    fn default() -> Self {
        Self {
            id: -1,
            mid: String::new(),
            name: String::new(),
            r#type: -1,
            singer_pic: String::new(),
            singer_pmid: String::new(),
            name_special_display: None,
        }
    }
}

/// Tab metadata.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TabMeta {
    /// Tab id.
    #[json(alias = "TabID")]
    pub tab_id: String,
    /// Tab name.
    #[json(alias = "TabName")]
    pub tab_name: String,
    /// Title.
    #[json(alias = "Title")]
    pub title: String,
}

/// Album in a singer's album list.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AlbumBrief {
    /// Id.
    #[json(alias("albumID", "id"), default = -1)]
    pub id: i64,
    /// Mid.
    #[json(alias("albumMid", "mid"))]
    pub mid: String,
    /// Name.
    #[json(alias("albumName", "name"))]
    pub name: String,
    /// Subtitle.
    #[json(alias("albumTranName", "subtitle"))]
    pub subtitle: String,
    /// Publish date.
    #[json(alias("publishDate", "time_public"))]
    pub time_public: String,
    /// Song count.
    #[json(alias = "totalNum")]
    pub total_num: i64,
    /// Album type.
    #[json(alias = "albumType")]
    pub album_type: String,
    /// Singer name.
    #[json(alias = "singerName")]
    pub singer_name: String,
    /// Tags.
    pub tags: Vec<String>,
}

impl AlbumBrief {
    /// Cover URL.
    pub fn cover_url(&self, size: CoverSize) -> String {
        photo_new_cover_url("T002", &self.mid, size)
    }
}

/// Video in a singer's list.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct VideoBrief {
    /// Id.
    #[json(alias("mvid", "id"), default = -1)]
    pub id: i64,
    /// Vid.
    pub vid: String,
    /// Type.
    #[json(default = -1)]
    pub r#type: i64,
    /// Title.
    pub title: String,
    /// Picture URL.
    pub picurl: String,
    /// Picture format.
    pub picformat: i64,
    /// Duration.
    pub duration: i64,
    /// Play count.
    pub playcnt: i64,
    /// Publish date.
    pub pubdate: i64,
    /// Icon type.
    pub icon_type: i64,
}

/// Filter item.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TabFilterItem {
    /// Id.
    #[json(alias("ID", "id"), default = -1)]
    pub id: i64,
    /// Title.
    #[json(alias("Title", "name", "title"))]
    pub title: String,
}

/// Filter list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct TabFilterList {
    /// Default id.
    #[json(alias = "DefaultID")]
    pub default_id: i64,
    /// Items.
    #[json(alias = "ItemList")]
    pub items: Vec<TabFilterItem>,
}

/// Song tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongTabInfo {
    /// Songs.
    #[json(path = "$.List[*]")]
    pub songs: Vec<Song>,
    /// Tags.
    #[json(alias = "SongTagInfoList")]
    pub tag_info_list: Vec<Value>,
}

/// Album tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AlbumTabInfo {
    /// Type filters.
    #[json(alias = "TypeList")]
    pub type_list: TabFilterList,
    /// Albums.
    #[json(path = "$.AlbumList[*]")]
    pub albums: Vec<AlbumBrief>,
}

/// Video tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct VideoTabInfo {
    /// Videos.
    #[json(path = "$.VideoList[*]")]
    pub videos: Vec<VideoBrief>,
    /// Tags.
    #[json(alias = "TagList")]
    pub tag_list: Vec<TabFilterItem>,
}

/// `get_tab_detail` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HomepageTabDetailResponse {
    /// Tab id.
    #[json(alias = "TabID")]
    pub tab_id: String,
    /// Has more.
    #[json(alias = "HasMore")]
    pub has_more: i64,
    /// Show tab.
    #[json(alias = "NeedShowTab")]
    pub need_show_tab: i64,
    /// Order.
    #[json(alias = "Order")]
    pub order: i64,
    /// Tabs.
    #[json(alias = "TabList")]
    pub tab_list: Vec<TabMeta>,
    /// Song tab.
    #[json(alias = "SongTab")]
    pub song_tab: SongTabInfo,
    /// Album tab.
    #[json(alias = "AlbumTab")]
    pub album_tab: AlbumTabInfo,
    /// Video tab.
    #[json(alias = "VideoTab")]
    pub video_tab: VideoTabInfo,
    /// Introduction.
    #[json(path = "$.IntroductionTab.List")]
    pub introduction_tab: Vec<Value>,
}

/// `get_info` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HomepageHeaderResponse {
    /// Status.
    #[json(alias = "Status")]
    pub status: i64,
    /// Singer.
    #[json(path = "$.Info.Singer")]
    pub singer: HomepageSinger,
    /// Base info.
    #[json(path = "$.Info.BaseInfo")]
    pub base_info: HomepageBaseInfo,
    /// Tab detail.
    #[json(alias = "TabDetail")]
    pub tab_detail: HomepageTabDetailResponse,
    /// Prompt.
    #[json(alias = "Prompt")]
    pub prompt: Map<String, Value>,
}

/// Singer basic info.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerBasicInfo {
    /// Id.
    #[json(alias = "singer_id", default = -1)]
    pub id: i64,
    /// Mid.
    #[json(alias = "singer_mid")]
    pub mid: String,
    /// Name.
    pub name: String,
    /// Type.
    #[json(default = -1)]
    pub r#type: i64,
    /// Picture mid.
    #[json(alias = "singer_pmid")]
    pub pmid: String,
    /// Has photo.
    pub has_photo: i64,
    /// Wiki URL.
    pub wikiurl: String,
}

impl Default for SingerBasicInfo {
    fn default() -> Self {
        Self {
            id: -1,
            mid: String::new(),
            name: String::new(),
            r#type: -1,
            pmid: String::new(),
            has_photo: 0,
            wikiurl: String::new(),
        }
    }
}

/// Extra info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerExtraInfo {
    /// Area.
    #[json(with = "crate::json::convert::none_or_zero_to_empty_string")]
    pub area: String,
    /// Description.
    pub desc: String,
    /// Tag.
    pub tag: String,
    /// Identity.
    #[json(with = "crate::json::convert::none_or_zero_to_empty_string")]
    pub identity: String,
    /// Instrument.
    #[json(with = "crate::json::convert::none_or_zero_to_empty_string")]
    pub instrument: String,
    /// Genre.
    #[json(with = "crate::json::convert::none_or_zero_to_empty_string")]
    pub genre: String,
    /// Foreign name.
    pub foreign_name: String,
    /// Birthday.
    pub birthday: String,
    /// Debut.
    #[json(with = "crate::json::convert::none_or_zero_to_empty_string")]
    pub enter: String,
    /// Blog flag.
    #[json(alias = "blogFlag")]
    pub blog_flag: i64,
}

/// Singer pictures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerPic {
    /// Big black.
    pub big_black: String,
    /// Big white.
    pub big_white: String,
    /// Picture.
    pub pic: String,
}

/// Photo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerPhotoItem {
    /// Big.
    pub big: String,
    /// Small.
    pub small: String,
}

/// Singer detail.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerDetail {
    /// Basic info.
    #[json(required)]
    pub basic_info: SingerBasicInfo,
    /// Extra info.
    pub ex_info: SingerExtraInfo,
    /// Wiki.
    pub wiki: String,
    /// Group list.
    pub group_list: Vec<Value>,
    /// Pictures.
    pub pic: SingerPic,
    /// Photos.
    pub photos: Vec<SingerPhotoItem>,
    /// Group info.
    pub group_info: Vec<Value>,
}

/// `get_desc` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerDetailResponse {
    /// Singers.
    pub singer_list: Vec<SingerDetail>,
}

/// Similar singer.
#[derive(Debug, Clone, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SimilarSinger {
    /// Id.
    #[json(alias = "singerId", default = -1)]
    pub id: i64,
    /// Mid.
    #[json(alias = "singerMid")]
    pub mid: String,
    /// Name.
    #[json(alias = "singerName")]
    pub name: String,
    /// Picture mid.
    #[json(alias = "pic_mid")]
    pub pmid: String,
    /// Picture.
    #[json(alias = "singerPic")]
    pub singer_pic: String,
    /// Trace.
    pub trace: String,
    /// abt.
    pub abt: String,
    /// tf.
    pub tf: String,
}

/// `get_similar` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SimilarSingerResponse {
    /// Singers.
    pub singerlist: Vec<SimilarSinger>,
    /// Code.
    pub code: i64,
    /// Error message.
    #[json(alias = "errMsg")]
    pub err_msg: String,
}

/// `get_songs_list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerSongListResponse {
    /// Singer mid.
    #[json(alias = "singerMid")]
    pub singer_mid: String,
    /// Total.
    #[json(alias = "totalNum")]
    pub total_num: i64,
    /// Songs.
    #[json(path = "$.songList[*].songInfo")]
    pub song_list: Vec<Song>,
}

impl PageItems for SingerSongListResponse {
    type Item = Song;
    fn into_items(self) -> Vec<Song> {
        self.song_list
    }
}

/// `get_album_list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerAlbumListResponse {
    /// Singer mid.
    #[json(alias = "singerMid")]
    pub singer_mid: String,
    /// Total.
    pub total: i64,
    /// Albums.
    #[json(alias = "albumList")]
    pub album_list: Vec<AlbumBrief>,
}

impl PageItems for SingerAlbumListResponse {
    type Item = AlbumBrief;
    fn into_items(self) -> Vec<AlbumBrief> {
        self.album_list
    }
}

/// `get_mv_tag` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerMvTagResponse {
    /// Tags.
    #[json(path = "$.list[*]")]
    pub tags: Vec<TagOption>,
}

/// `get_mv_list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SingerMvListResponse {
    /// Total.
    pub total: i64,
    /// MVs.
    #[json(alias = "list")]
    pub mv_list: Vec<VideoBrief>,
}

impl PageItems for SingerMvListResponse {
    type Item = VideoBrief;
    fn into_items(self) -> Vec<VideoBrief> {
        self.mv_list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn brief_defaults_and_aliases() {
        let brief: SingerBrief = from_value(&json!({"singer_mid": "m", "concernNum": 5})).unwrap();
        assert_eq!((brief.id, brief.area_id, brief.concern_num), (-1, -1, 5));
        assert!(brief.cover_url(CoverSize::S150).contains("M000m.jpg"));
        let list: SingerTypeListResponse =
            from_value(&json!({"singerlist": [{"singer_id": 1}], "tags": {"area": null}})).unwrap();
        assert_eq!((list.area, list.singerlist[0].id), (-100, 1));
        assert!(list.tags.area.is_empty());
    }

    #[test]
    fn homepage_and_tabs() {
        let header: HomepageHeaderResponse = from_value(&json!({
            "Status": 1,
            "Info": {"Singer": {"SingerMid": "s", "SingerNameSpecialDisplay": {"DisplayType": 2}}, "BaseInfo": {"Name": "n"}},
            "TabDetail": {
                "HasMore": 1,
                "SongTab": {"List": [{"id": 1, "mid": "a"}]},
                "AlbumTab": {"TypeList": {"ItemList": [{"ID": 3, "Title": "t"}]}, "AlbumList": [{"albumMid": "am"}]},
                "VideoTab": {"VideoList": [{"mvid": 9}]},
                "IntroductionTab": {"List": [{"x": 1}]}
            }
        }))
        .unwrap();
        assert_eq!(header.singer.mid, "s");
        assert_eq!(header.singer.r#type, -1);
        assert_eq!(header.singer.name_special_display.as_ref().unwrap().display_type, 2);
        assert_eq!(header.base_info.name, "n");
        assert_eq!(header.tab_detail.song_tab.songs[0].mid, "a");
        assert_eq!(header.tab_detail.album_tab.type_list.items[0].id, 3);
        assert_eq!(header.tab_detail.album_tab.albums[0].mid, "am");
        assert_eq!(header.tab_detail.video_tab.videos[0].id, 9);
        assert_eq!(header.tab_detail.introduction_tab.len(), 1);
        assert_eq!(HomepageSinger::default().id, -1);
    }

    #[test]
    fn detail_and_lists() {
        let detail: SingerDetailResponse = from_value(&json!({
            "singer_list": [{"basic_info": {"singer_mid": "m", "name": "n"}, "ex_info": {"area": 0, "genre": null, "desc": "d"}}]
        }))
        .unwrap();
        let first = &detail.singer_list[0];
        assert_eq!(first.basic_info.mid, "m");
        assert_eq!(first.basic_info.id, -1);
        assert_eq!(first.ex_info.area, "");
        assert_eq!(first.ex_info.genre, "");
        assert_eq!(first.ex_info.desc, "d");
        assert!(from_value::<SingerDetail>(&json!({})).is_err());
        let similar: SimilarSingerResponse =
            from_value(&json!({"singerlist": [{"singerMid": "x", "pic_mid": "p"}], "errMsg": "ok"})).unwrap();
        assert_eq!(similar.singerlist[0].pmid, "p");
        let albums: SingerAlbumListResponse =
            from_value(&json!({"albumList": [{"albumMid": "a", "tags": null}]})).unwrap();
        assert!(albums.album_list[0].cover_url(CoverSize::S300).contains("T002"));
        let mvs: SingerMvListResponse = from_value(&json!({"total": 1, "list": [{"mvid": 2}]})).unwrap();
        assert_eq!(mvs.into_items()[0].id, 2);
        let tags: SingerMvTagResponse = from_value(&json!({"list": [{"id": 1, "name": "全部"}]})).unwrap();
        assert_eq!(tags.tags[0].name, "全部");
    }
}
