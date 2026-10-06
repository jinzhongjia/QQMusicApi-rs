//! Search APIs.

use serde_json::{Map, Value, json};

use crate::models::search::{
    CompleteResponse, GeneralSearchResponse, HotkeyResponse, QuickSearchResponse, SearchByTypeResponse, SearchSelector,
};
use crate::pagination::{FnStrategy, PageStrategy, Paged};
use crate::request::{CgiRequest, HttpRequest, HttpSpec};
use crate::transport::Method;
use crate::utils::get_search_id;
use crate::versioning::Platform;

/// Search type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(i64)]
pub enum SearchType {
    /// Songs.
    #[default]
    Song = 0,
    /// Singers.
    Singer = 1,
    /// Albums.
    Album = 2,
    /// Playlists.
    SongList = 3,
    /// MVs.
    Mv = 4,
    /// Lyrics.
    Lyric = 7,
    /// Users.
    User = 8,
    /// Ring tones.
    Ringtone = 10,
    /// Audio albums.
    AudioAlbum = 15,
    /// Audio.
    Audio = 18,
}

impl SearchType {
    /// Numeric value.
    pub fn code(self) -> i64 {
        self as i64
    }

    /// From a numeric value.
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => Self::Song,
            1 => Self::Singer,
            2 => Self::Album,
            3 => Self::SongList,
            4 => Self::Mv,
            7 => Self::Lyric,
            8 => Self::User,
            10 => Self::Ringtone,
            15 => Self::AudioAlbum,
            18 => Self::Audio,
            _ => return None,
        })
    }
}

/// Options of [`SearchApi::search_by_type_with`].
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    /// Search type.
    pub search_type: SearchType,
    /// Results per page.
    pub num: i64,
    /// Page (1 based).
    pub page: i64,
    /// Filters.
    pub selectors: Vec<SearchSelector>,
    /// Search id (random when `None`).
    pub searchid: Option<String>,
    /// Highlight matches.
    pub highlight: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self { search_type: SearchType::Song, num: 10, page: 1, selectors: Vec::new(), searchid: None, highlight: true }
    }
}

api_module! {
    /// Search APIs.
    SearchApi
}

impl SearchApi {
    /// Hot keywords.
    pub fn get_hotkey(&self) -> CgiRequest<HotkeyResponse> {
        self.cgi("music.musicsearch.HotkeyService", "GetHotkeyForQQMusicMobile", json!({"search_id": get_search_id()}))
    }

    /// Keyword completion.
    pub fn complete(&self, keyword: &str) -> CgiRequest<CompleteResponse> {
        self.cgi(
            "music.smartboxCgi.SmartBoxCgi",
            "GetSmartBoxResult",
            json!({"search_id": get_search_id(), "query": keyword, "num_per_page": 0, "page_idx": 0}),
        )
    }

    /// Quick search (`smartbox_new.fcg`).
    pub fn quick_search(&self, keyword: &str) -> HttpRequest<QuickSearchResponse> {
        self.client.http(
            HttpSpec::new(Method::Get, "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg")
                .query([("key", keyword)]),
        )
    }

    /// General (mixed) search with continuation paging.
    pub fn general_search(&self, keyword: &str, page: i64, num: i64) -> Paged<GeneralSearchResponse> {
        self.general_search_with(keyword, page, num, None, None, true)
    }

    /// General search with all options.
    pub fn general_search_with(
        &self,
        keyword: &str,
        page: i64,
        num: i64,
        searchid: Option<&str>,
        page_start: Option<Map<String, Value>>,
        highlight: bool,
    ) -> Paged<GeneralSearchResponse> {
        let mut param = json!({
            "searchid": searchid.map_or_else(get_search_id, str::to_string),
            "search_type": 100,
            "page_num": num,
            "query": keyword,
            "page_id": page,
            "highlight": highlight,
            "grp": true,
        });
        if let Some(start) = page_start {
            param["page_start"] = Value::Object(start);
        }
        let request = self.cgi("music.adaptor.SearchAdaptor", "do_search_v2", param);
        Paged::new(
            request,
            FnStrategy(|params: &Value, r: &GeneralSearchResponse| {
                if r.nextpage == -1 {
                    return None;
                }
                let mut next = params.clone();
                next["searchid"] = json!(r.searchid);
                next["page_id"] = json!(r.nextpage);
                next["page_start"] = Value::Object(r.nextpage_start.clone());
                Some(next)
            }),
        )
    }

    /// Typed search (songs by default).
    pub fn search_by_type(&self, keyword: &str, search_type: SearchType) -> Paged<SearchByTypeResponse> {
        self.search_by_type_with(keyword, SearchOptions { search_type, ..SearchOptions::default() })
    }

    /// Typed search with options.
    pub fn search_by_type_with(&self, keyword: &str, options: SearchOptions) -> Paged<SearchByTypeResponse> {
        let selectors: Map<String, Value> =
            options.selectors.iter().map(|s| (s.r#type.to_string(), Value::String(s.id.to_string()))).collect();
        let vec_selectors: Vec<Value> =
            options.selectors.iter().map(|s| json!({"type": s.r#type, "name": s.name, "id": s.id})).collect();
        let request = self
            .cgi(
                "music.search.SearchCgiService",
                "DoSearchForQQMusicMobile",
                json!({
                    "searchid": options.searchid.clone().unwrap_or_else(get_search_id),
                    "query": keyword,
                    "search_type": options.search_type.code(),
                    "num_per_page": options.num,
                    "page_num": options.page,
                    "highlight": options.highlight,
                    "grp": true,
                    "selectors": selectors,
                    "vec_selectors": vec_selectors,
                }),
            )
            .platform(Platform::Android);
        Paged::new(
            request,
            PageStrategy::new("page_num")
                .page_size(options.num)
                .start_page(options.page)
                .has_more(|r: &SearchByTypeResponse| Some(r.nextpage != -1))
                .total(|r: &SearchByTypeResponse| Some(r.total_num)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pagination::PageItems;
    use crate::testing::*;
    use crate::transport::Response;

    #[test]
    fn search_type_codes() {
        assert_eq!(SearchType::AudioAlbum.code(), 15);
        assert_eq!(SearchType::from_code(18), Some(SearchType::Audio));
        assert_eq!(SearchType::from_code(5), None);
        for code in [0, 1, 2, 3, 4, 7, 8, 10, 15, 18] {
            assert_eq!(SearchType::from_code(code).unwrap().code(), code);
        }
    }

    #[tokio::test]
    async fn by_type_paginates() {
        let (client, mock) = mock_client();
        push_cgi(
            &mock,
            json!({"meta": {"nextpage": 2, "sum": 3}, "body": {"item_song": [{"id": 1, "mid": "a"}, {"id": 2, "mid": "b"}]}}),
        );
        push_cgi(&mock, json!({"meta": {"nextpage": -1, "sum": 3}, "body": {"item_song": [{"id": 3, "mid": "c"}]}}));
        let items = client
            .search()
            .search_by_type_with(
                "jay",
                SearchOptions {
                    num: 2,
                    selectors: vec![SearchSelector { id: 9, name: "n".into(), r#type: 4 }],
                    ..SearchOptions::default()
                },
            )
            .collect_items(None)
            .await
            .unwrap();
        assert_eq!(items.len(), 3);
        let req = last_req0(&mock);
        assert_eq!(req["module"], "music.search.SearchCgiService");
        assert_eq!(req["param"]["page_num"], 2);
        assert_eq!(req["param"]["num_per_page"], 2);
        assert_eq!(req["param"]["highlight"], 1);
        assert_eq!(req["param"]["selectors"], json!({"4": "9"}));
        assert_eq!(req["param"]["vec_selectors"], json!([{"type": 4, "name": "n", "id": 9}]));
        assert_eq!(mock.request_count(), 2);

        push_cgi(&mock, json!({"meta": {"nextpage": 2}, "body": {"singer": [{"singerMID": "x"}]}}));
        let page = client.search().search_by_type("jay", SearchType::Singer).await.unwrap();
        assert_eq!(page.into_items().len(), 1);
        assert_eq!(last_req0(&mock)["param"]["search_type"], 1);
    }

    #[tokio::test]
    async fn general_search_continuation() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"meta": {"sid": "sid1", "nextpage": 2, "nextpage_start": {"song": 15}}}));
        push_cgi(&mock, json!({"meta": {"sid": "sid1", "nextpage": -1}}));
        let pages = client.search().general_search("jay", 1, 15).collect(None).await.unwrap();
        assert_eq!(pages.len(), 2);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["searchid"], "sid1");
        assert_eq!(req["param"]["page_id"], 2);
        assert_eq!(req["param"]["page_start"], json!({"song": 15}));
        assert_eq!(req["param"]["search_type"], 100);

        let mut start = Map::new();
        start.insert("x".into(), json!(1));
        push_cgi(&mock, json!({}));
        client.search().general_search_with("k", 3, 5, Some("fixed"), Some(start), false).await.unwrap();
        let req = last_req0(&mock);
        assert_eq!(req["param"]["searchid"], "fixed");
        assert_eq!(req["param"]["page_start"], json!({"x": 1}));
        assert_eq!(req["param"]["highlight"], 0);
    }

    #[tokio::test]
    async fn hotkey_complete_quick() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"vec_hotkey": [{"query": "q"}]}));
        assert_eq!(client.search().get_hotkey().await.unwrap().vec_hotkey[0].query, "q");
        push_cgi(&mock, json!({"items": [{"hint": "h"}]}));
        assert_eq!(client.search().complete("j").await.unwrap().items[0].hint, "h");
        assert_eq!(last_req0(&mock)["param"]["query"], "j");
        mock.push_response(Response::json(&json!({"code": 0, "data": {"song": {"itemlist": [{"name": "晴天"}]}}})));
        let quick = client.search().quick_search("晴天").await.unwrap();
        assert_eq!(quick.song.itemlist[0].name, "晴天");
        let req = mock.last_request().unwrap();
        assert_eq!(req.query_param("key"), Some("晴天"));
        assert!(req.url.contains("smartbox_new.fcg"));
    }
}
