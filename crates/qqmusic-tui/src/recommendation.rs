//! 推荐页面和后台续播共用的批次加载。

use std::collections::HashSet;

use qqmusic_api::Client;

use crate::protocol::{RecommendationSource, Track};

pub(crate) struct Batch {
    pub tracks: Vec<Track>,
    pub next: RecommendationSource,
}

pub(crate) async fn load(client: &Client, source: RecommendationSource) -> Result<Batch, String> {
    let (songs, next) = match source {
        RecommendationSource::Guess => {
            let mut songs = Vec::new();
            for _ in 0..4 {
                songs.extend(client.recommend().get_guess_recommend(None).await.map_err(|e| e.to_string())?.songs);
            }
            (songs, source)
        }
        RecommendationSource::Radar { next_page } => {
            let page = next_page.max(1);
            let pages =
                client.recommend().get_radar_recommend(page).collect(Some(2)).await.map_err(|e| e.to_string())?;
            let next_page = if pages.last().is_some_and(|p| p.has_more) { page + pages.len() as i64 } else { 1 };
            (pages.into_iter().flat_map(|p| p.songs).collect(), RecommendationSource::Radar { next_page })
        }
    };
    let mut seen = HashSet::new();
    let tracks = songs.iter().filter(|s| !s.mid.is_empty() && seen.insert(s.mid.clone())).map(Track::from).collect();
    Ok(Batch { tracks, next })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{client, reply};
    use qqmusic_api::transport::mock::MockTransport;
    use serde_json::json;

    #[tokio::test]
    async fn radar_continues_pages_and_refreshes_after_the_last_page() {
        let mock = MockTransport::new();
        let client = client(&mock);
        for mid in ["a", "b"] {
            mock.push_response(reply(json!({"VecSongs": [{"Track": {"id": 1, "mid": mid}}], "HasMore": true})));
        }
        let batch = load(&client, RecommendationSource::Radar { next_page: 3 }).await.unwrap();
        assert_eq!(batch.tracks.len(), 2);
        assert_eq!(batch.next, RecommendationSource::Radar { next_page: 5 });
        let pages: Vec<_> =
            mock.requests().iter().map(|r| r.json_body().unwrap()["req_0"]["param"]["Page"].clone()).collect();
        assert_eq!(pages, [json!(3), json!(4)]);
        mock.push_response(reply(json!({"VecSongs": [], "HasMore": false})));
        assert_eq!(load(&client, batch.next).await.unwrap().next, RecommendationSource::Radar { next_page: 1 });
    }

    #[tokio::test]
    async fn guess_filters_duplicates_and_songs_without_mids() {
        let mock = MockTransport::new();
        let client = client(&mock);
        for _ in 0..4 {
            mock.push_response(reply(
                json!({"tracks": [{"id": 1, "mid": "a"}, {"id": 1, "mid": ""}, {"id": 1, "mid": "b"}]}),
            ));
        }
        let batch = load(&client, RecommendationSource::Guess).await.unwrap();
        assert_eq!(mock.request_count(), 4);
        assert_eq!(batch.tracks.iter().map(|t| t.mid.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(batch.next, RecommendationSource::Guess);
    }
}
