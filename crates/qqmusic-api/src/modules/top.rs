//! Top list APIs.

use serde_json::json;

use crate::models::top::{TopCategoryResponse, TopDetailResponse};
use crate::pagination::{OffsetStrategy, Paged};
use crate::request::CgiRequest;

api_module! {
    /// Top list APIs.
    TopApi
}

impl TopApi {
    /// All top lists by group.
    pub fn get_category(&self) -> CgiRequest<TopCategoryResponse> {
        self.cgi("music.musicToplist.Toplist", "GetAll", json!({}))
    }

    /// Songs of a top list (offset pagination).
    pub fn get_detail(&self, top_id: i64, num: i64, page: i64, tag: bool) -> Paged<TopDetailResponse> {
        let mut param = json!({"topId": top_id, "offset": num * (page - 1), "num": num});
        if tag {
            param["withTags"] = json!(true);
        }
        Paged::new(
            self.cgi("music.musicToplist.Toplist", "GetDetail", param).preserve_bool(tag),
            OffsetStrategy::with_size_key("offset", "num")
                .total(|r: &TopDetailResponse| Some(r.info.total_num))
                .count(|r: &TopDetailResponse| Some(r.songs.len() as i64)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[tokio::test]
    async fn top_requests() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"group": [{"groupId": 1, "toplist": []}]}));
        assert_eq!(client.top().get_category().await.unwrap().group[0].id, 1);
        assert_eq!(last_req0(&mock)["method"], "GetAll");

        push_cgi(&mock, json!({"data": {"totalNum": 3}, "songInfoList": [{"id": 1, "mid": "a"}, {"id": 2, "mid": "b"}]}));
        push_cgi(&mock, json!({"data": {"totalNum": 3}, "songInfoList": [{"id": 3, "mid": "c"}]}));
        let songs = client.top().get_detail(26, 2, 1, true).collect_items(None).await.unwrap();
        assert_eq!(songs.len(), 3);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["offset"], 2);
        assert_eq!(req["param"]["withTags"], true);

        push_cgi(&mock, json!({"data": {"totalNum": 0}}));
        client.top().get_detail(26, 10, 1, false).await.unwrap();
        assert!(last_req0(&mock)["param"].get("withTags").is_none());
    }
}
