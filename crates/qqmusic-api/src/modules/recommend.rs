//! Recommendation APIs.

use serde_json::{Value, json};

use crate::credential::Credential;
use crate::models::recommend::{
    GuessRecommendResponse, RadarRecommendResponse, RecommendFeedCardResponse, RecommendNewSongResponse,
    RecommendSonglistResponse,
};
use crate::pagination::{CursorStrategy, FnStrategy, PageStrategy, Paged};
use crate::request::CgiRequest;

api_module! {
    /// Recommendation APIs.
    RecommendApi
}

impl RecommendApi {
    /// Home feed (continuation pagination with shelf de-duplication).
    pub fn get_home_feed(
        &self,
        page: i64,
        direction: i64,
        s_num: i64,
        v_cache: Vec<String>,
    ) -> Paged<RecommendFeedCardResponse> {
        Paged::new(
            self.cgi(
                "music.recommend.RecommendFeed",
                "get_recommend_feed",
                json!({"direction": direction, "page": page, "s_num": s_num, "v_cache": v_cache}),
            ),
            FnStrategy(|params: &Value, r: &RecommendFeedCardResponse| {
                if r.shelves.is_empty() {
                    return None;
                }
                let mut seen: Vec<String> = params["v_cache"]
                    .as_array()
                    .map(|items| {
                        items.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_string)).collect()
                    })
                    .unwrap_or_default();
                for shelf in &r.shelves {
                    let id = shelf.id.to_string();
                    if !seen.contains(&id) {
                        seen.push(id);
                    }
                }
                let mut next = params.clone();
                next["direction"] = json!(1);
                next["page"] = json!(params["page"].as_i64().unwrap_or(1) + 1);
                next["s_num"] = json!(params["s_num"].as_i64().unwrap_or(0) + r.shelves.len() as i64);
                next["v_cache"] = json!(seen);
                Some(next)
            }),
        )
    }

    /// "Guess you like" radio.
    pub fn get_guess_recommend(&self, credential: Option<Credential>) -> CgiRequest<GuessRecommendResponse> {
        self.cgi(
            "music.radioProxy.MbTrackRadioSvr",
            "get_radio_track",
            json!({"id": 99, "num": 5, "from": 0, "scene": 0, "song_ids": []}),
        )
        .credential_opt(credential)
    }

    /// Radar recommendations (page pagination).
    pub fn get_radar_recommend(&self, page: i64) -> Paged<RadarRecommendResponse> {
        Paged::new(
            self.cgi(
                "music.recommend.TrackRelationServer",
                "GetRadarSong",
                json!({"Page": page, "ReqType": 0, "FavSongs": [], "EntranceSongs": []}),
            ),
            PageStrategy::new("Page").start_page(page).has_more(|r: &RadarRecommendResponse| Some(r.has_more)),
        )
    }

    /// Recommended playlists (cursor pagination).
    pub fn get_recommend_songlist(&self, page: i64, num: i64) -> Paged<RecommendSonglistResponse> {
        Paged::new(
            self.cgi(
                "music.playlist.PlaylistSquare",
                "GetRecommendFeed",
                json!({"From": num * (page - 1), "Size": num}),
            ),
            CursorStrategy::new("From", |r: &RecommendSonglistResponse| Some(json!(r.from_limit)))
                .has_more(|r: &RecommendSonglistResponse| Some(r.has_more)),
        )
    }

    /// New songs of a type (upstream default 5).
    pub fn get_recommend_newsong(&self, song_type: i64) -> CgiRequest<RecommendNewSongResponse> {
        self.cgi("newsong.NewSongServer", "get_new_song_info", json!({"type": song_type}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[tokio::test]
    async fn home_feed_continuation() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"v_shelf": [{"id": 1}, {"id": 2}]}));
        push_cgi(&mock, json!({"v_shelf": [{"id": 2}, {"id": 3}]}));
        push_cgi(&mock, json!({"v_shelf": []}));
        let shelves = client.recommend().get_home_feed(1, 0, 0, vec!["9".into()]).collect_items(None).await.unwrap();
        assert_eq!(shelves.len(), 4);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["page"], 3);
        assert_eq!(req["param"]["direction"], 1);
        assert_eq!(req["param"]["s_num"], 4);
        assert_eq!(req["param"]["v_cache"], json!(["9", "1", "2", "3"]));
    }

    #[tokio::test]
    async fn other_recommendations() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"tracks": [{"id": 1, "mid": "a"}]}));
        assert_eq!(client.recommend().get_guess_recommend(None).await.unwrap().songs.len(), 1);
        assert_eq!(last_req0(&mock)["param"]["id"], 99);

        push_cgi(&mock, json!({"HasMore": true, "VecSongs": [{"Track": {"id": 1, "mid": "a"}}]}));
        push_cgi(&mock, json!({"HasMore": false, "VecSongs": [{"Track": {"id": 2, "mid": "b"}}]}));
        let songs = client.recommend().get_radar_recommend(1).collect_items(None).await.unwrap();
        assert_eq!(songs.len(), 2);
        assert_eq!(last_req0(&mock)["param"]["Page"], 2);

        push_cgi(&mock, json!({"HasMore": true, "FromLimit": 25, "List": [{"Playlist": {"basic": {"tid": 1}}}]}));
        push_cgi(&mock, json!({"HasMore": false, "FromLimit": 50, "List": []}));
        let lists = client.recommend().get_recommend_songlist(1, 25).collect_items(None).await.unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(last_req0(&mock)["param"]["From"], 25);

        push_cgi(&mock, json!({"type": 5}));
        client.recommend().get_recommend_newsong(5).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"type": 5}));
    }
}
