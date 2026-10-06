//! Singer APIs.

use serde_json::{Value, json};

use crate::models::singer::{
    HomepageHeaderResponse, HomepageTabDetailResponse, SimilarSingerResponse, SingerAlbumListResponse,
    SingerDetailResponse, SingerIndexPageResponse, SingerMvListResponse, SingerMvTagResponse, SingerSongListResponse,
    SingerTypeListResponse,
};
use crate::pagination::{FnStrategy, OffsetStrategy, PageStrategy, Paged};
use crate::request::CgiRequest;
use crate::versioning::Platform;

macro_rules! int_enum {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident = $value:expr),* $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vdoc])* $variant,)*
        }

        impl $name {
            /// Numeric value sent to the API.
            pub fn code(self) -> i64 {
                match self {
                    $(Self::$variant => $value,)*
                }
            }

            /// Variant from a numeric value.
            pub fn from_code(code: i64) -> Option<Self> {
                match code {
                    $(v if v == $value => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

int_enum! {
    /// Singer area.
    AreaType { /** All. */ All = -100, /** Mainland China. */ China = 200, /** Taiwan. */ Taiwan = 2, /** Europe & America. */ America = 5, /** Japan. */ Japan = 4, /** Korea. */ Korea = 3 }
}

int_enum! {
    /// Singer genre.
    GenreType {
        /** All. */ All = -100, /** Pop. */ Pop = 7, /** Rap. */ Rap = 3, /** Chinese style. */ ChineseStyle = 19,
        /** Rock. */ Rock = 4, /** Electronic. */ Electronic = 2, /** Folk. */ Folk = 8, /** R&B. */ RAndB = 11,
        /** Ethnic. */ Ethnic = 37, /** Light music. */ LightMusic = 93, /** Jazz. */ Jazz = 14,
        /** Classical. */ Classical = 33, /** Country. */ Country = 13, /** Blues. */ Blues = 10
    }
}

int_enum! {
    /// Singer sex.
    SexType { /** All. */ All = -100, /** Male. */ Male = 0, /** Female. */ Female = 1, /** Group. */ Group = 2 }
}

int_enum! {
    /// Sort order.
    OrderType { /** Latest first. */ Latest = 0, /** Hottest first. */ Hot = 1 }
}

int_enum! {
    /// Album filter.
    AlbumFilterType { /** Studio album. */ Studio = 0, /** Live album. */ Live = 1, /** EP. */ Ep = 11 }
}

int_enum! {
    /// Name index.
    IndexType {
        /** A */ A = 1, /** B */ B = 2, /** C */ C = 3, /** D */ D = 4, /** E */ E = 5, /** F */ F = 6, /** G */ G = 7,
        /** H */ H = 8, /** I */ I = 9, /** J */ J = 10, /** K */ K = 11, /** L */ L = 12, /** M */ M = 13, /** N */ N = 14,
        /** O */ O = 15, /** P */ P = 16, /** Q */ Q = 17, /** R */ R = 18, /** S */ S = 19, /** T */ T = 20, /** U */ U = 21,
        /** V */ V = 22, /** W */ W = 23, /** X */ X = 24, /** Y */ Y = 25, /** Z */ Z = 26, /** All. */ All = -100,
        /** `#` */ Hash = 27
    }
}

/// Homepage tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TabType {
    /// Wiki.
    Wiki,
    /// Albums.
    Album,
    /// Composed songs.
    Composer,
    /// Written lyrics.
    Lyricist,
    /// Produced songs.
    Producer,
    /// Arranged songs.
    Arranger,
    /// Played songs.
    Musician,
    /// Sung songs.
    Song,
    /// Videos.
    Video,
}

impl TabType {
    /// Tab id.
    pub fn tab_id(self) -> &'static str {
        match self {
            Self::Wiki => "wiki",
            Self::Album => "album",
            Self::Composer => "song_composing",
            Self::Lyricist => "song_lyric",
            Self::Producer => "producer",
            Self::Arranger => "arranger",
            Self::Musician => "musician",
            Self::Song => "song_sing",
            Self::Video => "video",
        }
    }

    /// Tab name.
    pub fn tab_name(self) -> &'static str {
        match self {
            Self::Wiki => "IntroductionTab",
            Self::Album => "AlbumTab",
            Self::Video => "VideoTab",
            _ => "SongTab",
        }
    }

    /// Key of the extension parameter (e.g. `AlbumExtension`).
    pub fn extension_key(self) -> String {
        format!("{}Extension", self.tab_name().replace("Tab", ""))
    }
}

/// Singer list filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingerFilter {
    /// Area.
    pub area: AreaType,
    /// Sex.
    pub sex: SexType,
    /// Genre.
    pub genre: GenreType,
}

impl Default for SingerFilter {
    fn default() -> Self {
        Self { area: AreaType::All, sex: SexType::All, genre: GenreType::All }
    }
}

api_module! {
    /// Singer APIs.
    SingerApi
}

impl SingerApi {
    /// Hot singers by type.
    pub fn get_singer_list(&self, filter: SingerFilter) -> CgiRequest<SingerTypeListResponse> {
        self.cgi(
            "music.musichallSinger.SingerList",
            "GetSingerList",
            json!({"hastag": 0, "area": filter.area.code(), "sex": filter.sex.code(), "genre": filter.genre.code()}),
        )
    }

    /// Singers by name index (continuation pagination, 80 per page upstream).
    pub fn get_singer_list_index(
        &self,
        filter: SingerFilter,
        index: IndexType,
        page: i64,
        num: i64,
    ) -> Paged<SingerIndexPageResponse> {
        let request = self.cgi(
            "music.musichallSinger.SingerList",
            "GetSingerListIndex",
            json!({
                "area": filter.area.code(),
                "sex": filter.sex.code(),
                "genre": filter.genre.code(),
                "index": index.code(),
                "sin": (page - 1) * num,
                "cur_page": page,
            }),
        );
        Paged::new(
            request,
            FnStrategy(|params: &Value, r: &SingerIndexPageResponse| {
                let sin = params["sin"].as_i64().unwrap_or(0);
                let count = r.singerlist.len() as i64;
                if count == 0 || sin + count >= r.total {
                    return None;
                }
                let mut next = params.clone();
                next["sin"] = json!(sin + count);
                next["cur_page"] = json!(params["cur_page"].as_i64().unwrap_or(1) + 1);
                Some(next)
            }),
        )
    }

    /// Homepage header (Android platform).
    pub fn get_info(&self, mid: &str) -> CgiRequest<HomepageHeaderResponse> {
        self.cgi("music.UnifiedHomepage.UnifiedHomepageSrv", "GetHomepageHeader", json!({"SingerMid": mid}))
            .platform(Platform::Android)
    }

    /// Homepage tab detail (page pagination).
    ///
    /// `extension` is sent as `AlbumExtension` / `VideoExtension` / ... e.g.
    /// `{"IsNeedFilterType": 1, "FilterType": 0}` for albums or
    /// `{"TagID": 0, "IsNeedTagList": 1}` for videos.
    pub fn get_tab_detail(
        &self,
        mid: &str,
        tab: TabType,
        page: i64,
        num: i64,
        order: OrderType,
        extension: Option<Value>,
    ) -> Paged<HomepageTabDetailResponse> {
        let mut param = json!({
            "SingerMid": mid,
            "IsQueryTabDetail": 1,
            "TabID": tab.tab_id(),
            "PageNum": page - 1,
            "PageSize": num,
            "Order": order.code(),
        });
        if let Some(extension) = extension.filter(|e| !e.is_null()) {
            param[tab.extension_key()] = extension;
        }
        Paged::new(
            self.cgi("music.UnifiedHomepage.UnifiedHomepageSrv", "GetHomepageTabDetail", param),
            PageStrategy::new("PageNum")
                .page_size(num)
                .start_page(page - 1)
                .has_more(|r: &HomepageTabDetailResponse| Some(r.has_more != 0)),
        )
    }

    /// Singer descriptions (all sections enabled).
    pub fn get_desc(&self, mids: &[&str]) -> CgiRequest<SingerDetailResponse> {
        self.cgi(
            "music.musichallSinger.SingerInfoInter",
            "GetSingerDetail",
            json!({
                "singer_mids": mids,
                "group_singer": true,
                "wiki_singer": true,
                "ex_singer": true,
                "pic": true,
                "photos": true,
            }),
        )
    }

    /// Similar singers.
    pub fn get_similar(&self, mid: &str, number: i64) -> CgiRequest<SimilarSingerResponse> {
        self.cgi("music.SimilarSingerSvr", "GetSimilarSingerList", json!({"singerMid": mid, "number": number}))
    }

    /// Songs (offset pagination).
    pub fn get_songs_list(&self, mid: &str, num: i64, page: i64, order: OrderType) -> Paged<SingerSongListResponse> {
        Paged::new(
            self.cgi(
                "musichall.song_list_server",
                "GetSingerSongList",
                json!({"singerMid": mid, "order": order.code(), "number": num, "begin": (page - 1) * num}),
            ),
            OffsetStrategy::with_size_key("begin", "number")
                .total(|r: &SingerSongListResponse| Some(r.total_num))
                .count(|r: &SingerSongListResponse| Some(r.song_list.len() as i64)),
        )
    }

    /// Albums (offset pagination).
    pub fn get_album_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
        order: OrderType,
        filter: Option<AlbumFilterType>,
    ) -> Paged<SingerAlbumListResponse> {
        let mut param = json!({"singerMid": mid, "order": order.code(), "number": num, "begin": (page - 1) * num});
        if let Some(filter) = filter {
            param["types"] = json!([filter.code()]);
        }
        Paged::new(
            self.cgi("music.musichallAlbum.AlbumListServer", "GetAlbumList", param),
            OffsetStrategy::with_size_key("begin", "number")
                .total(|r: &SingerAlbumListResponse| Some(r.total))
                .count(|r: &SingerAlbumListResponse| Some(r.album_list.len() as i64)),
        )
    }

    /// MV tags.
    pub fn get_mv_tag(&self, mid: &str) -> CgiRequest<SingerMvTagResponse> {
        self.cgi("MvService.MvInfoProServer", "GetSingerMvTag", json!({"singermid": mid}))
    }

    /// MVs (offset pagination).
    pub fn get_mv_list(
        &self,
        mid: &str,
        num: i64,
        page: i64,
        order: OrderType,
        tag_id: Option<i64>,
    ) -> Paged<SingerMvListResponse> {
        let mut param = json!({"singermid": mid, "order": order.code(), "count": num, "start": (page - 1) * num});
        if let Some(tag_id) = tag_id {
            param["tagid"] = json!(tag_id);
        }
        Paged::new(
            self.cgi("MvService.MvInfoProServer", "GetSingerMvList", param),
            OffsetStrategy::with_size_key("start", "count")
                .total(|r: &SingerMvListResponse| Some(r.total))
                .count(|r: &SingerMvListResponse| Some(r.mv_list.len() as i64)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[test]
    fn enums() {
        assert_eq!(AreaType::China.code(), 200);
        assert_eq!(GenreType::from_code(93), Some(GenreType::LightMusic));
        assert_eq!(IndexType::Hash.code(), 27);
        assert_eq!(IndexType::from_code(-100), Some(IndexType::All));
        assert_eq!(SexType::from_code(9), None);
        assert_eq!(TabType::Album.extension_key(), "AlbumExtension");
        assert_eq!(TabType::Wiki.extension_key(), "IntroductionExtension");
        assert_eq!(TabType::Lyricist.tab_id(), "song_lyric");
        assert_eq!(TabType::Producer.tab_name(), "SongTab");
        assert_eq!(OrderType::Hot.code(), 1);
        assert_eq!(AlbumFilterType::Ep.code(), 11);
    }

    #[tokio::test]
    async fn singer_list_and_index_paging() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"singerlist": [{"singer_mid": "a"}]}));
        let list = client
            .singer()
            .get_singer_list(SingerFilter { area: AreaType::Korea, ..SingerFilter::default() })
            .await
            .unwrap();
        assert_eq!(list.singerlist[0].mid, "a");
        assert_eq!(last_req0(&mock)["param"], json!({"hastag": 0, "area": 3, "sex": -100, "genre": -100}));

        push_cgi(&mock, json!({"total": 3, "singerlist": [{"singer_mid": "a"}, {"singer_mid": "b"}]}));
        push_cgi(&mock, json!({"total": 3, "singerlist": [{"singer_mid": "c"}]}));
        let items = client
            .singer()
            .get_singer_list_index(SingerFilter::default(), IndexType::J, 1, 2)
            .collect_items(None)
            .await
            .unwrap();
        assert_eq!(items.len(), 3);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["sin"], 2);
        assert_eq!(req["param"]["cur_page"], 2);
        assert_eq!(req["param"]["index"], 10);
        assert_eq!(mock.request_count(), 3);
    }

    #[tokio::test]
    async fn info_and_tabs() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"Status": 0, "Info": {"Singer": {"SingerMid": "m"}}}));
        let info = client.singer().get_info("m").await.unwrap();
        assert_eq!(info.singer.mid, "m");
        assert_eq!(last_body(&mock)["comm"]["ct"], "11");

        push_cgi(&mock, json!({"HasMore": 1, "AlbumTab": {"AlbumList": [{"albumMid": "a"}]}}));
        push_cgi(&mock, json!({"HasMore": 0}));
        let pages = client
            .singer()
            .get_tab_detail(
                "m",
                TabType::Album,
                1,
                10,
                OrderType::Latest,
                Some(json!({"IsNeedFilterType": 1, "FilterType": 0})),
            )
            .collect(None)
            .await
            .unwrap();
        assert_eq!(pages.len(), 2);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["PageNum"], 1);
        assert_eq!(req["param"]["TabID"], "album");
        assert_eq!(req["param"]["AlbumExtension"], json!({"IsNeedFilterType": 1, "FilterType": 0}));
    }

    #[tokio::test]
    async fn desc_similar_and_lists() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"singer_list": [{"basic_info": {"singer_mid": "m"}}]}));
        let desc = client.singer().get_desc(&["m"]).await.unwrap();
        assert_eq!(desc.singer_list[0].basic_info.mid, "m");
        assert_eq!(last_req0(&mock)["param"]["photos"], 1);

        push_cgi(&mock, json!({"singerlist": []}));
        client.singer().get_similar("m", 5).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"singerMid": "m", "number": 5}));

        push_cgi(&mock, json!({"totalNum": 1, "songList": [{"songInfo": {"id": 1, "mid": "s"}}]}));
        let songs = client.singer().get_songs_list("m", 10, 1, OrderType::Hot).collect_items(None).await.unwrap();
        assert_eq!(songs[0].mid, "s");

        push_cgi(&mock, json!({"total": 1, "albumList": [{"albumMid": "a"}]}));
        client
            .singer()
            .get_album_list("m", 10, 2, OrderType::Latest, Some(AlbumFilterType::Live))
            .collect(None)
            .await
            .unwrap();
        let req = last_req0(&mock);
        assert_eq!(req["param"]["types"], json!([1]));
        assert_eq!(req["param"]["begin"], 10);

        push_cgi(&mock, json!({"list": [{"id": 0, "name": "全部"}]}));
        assert_eq!(client.singer().get_mv_tag("m").await.unwrap().tags.len(), 1);

        push_cgi(&mock, json!({"total": 1, "list": [{"mvid": 3}]}));
        let mvs = client.singer().get_mv_list("m", 10, 1, OrderType::Hot, Some(2)).collect_items(None).await.unwrap();
        assert_eq!(mvs[0].id, 3);
        assert_eq!(last_req0(&mock)["param"]["tagid"], 2);
    }
}
