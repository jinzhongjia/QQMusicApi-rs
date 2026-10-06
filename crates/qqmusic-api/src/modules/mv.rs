//! MV APIs.

use serde_json::json;

use crate::models::mv::{GetMvDetailResponse, GetMvListResponse, GetMvUrlsResponse};
use crate::pagination::{OffsetStrategy, Paged};
use crate::request::CgiRequest;
use crate::utils::get_guid;

/// Fields requested by [`MvApi::get_detail`].
pub const DETAIL_FIELDS: [&str; 22] = [
    "vid",
    "type",
    "sid",
    "cover_pic",
    "duration",
    "singers",
    "video_switch",
    "msg",
    "name",
    "desc",
    "playcnt",
    "pubdate",
    "isfav",
    "gmid",
    "uploader_headurl",
    "uploader_nick",
    "uploader_encuin",
    "uploader_uin",
    "uploader_hasfollow",
    "uploader_follower_num",
    "uploader_hasfollow",
    "related_songs",
];

api_module! {
    /// MV APIs.
    MvApi
}

impl MvApi {
    /// MV details.
    pub fn get_detail(&self, vids: &[&str]) -> CgiRequest<GetMvDetailResponse> {
        self.cgi(
            "video.VideoDataServer",
            "get_video_info_batch",
            json!({"vidlist": vids, "required": DETAIL_FIELDS}),
        )
    }

    /// MV stream URLs.
    pub fn get_mv_urls(&self, vids: &[&str]) -> CgiRequest<GetMvUrlsResponse> {
        self.cgi(
            "music.stream.MvUrlProxy",
            "GetMvUrls",
            json!({
                "vids": vids,
                "request_type": 10003,
                "guid": get_guid(),
                "videoformat": 1,
                "format": 265,
                "dolby": 1,
                "use_new_domain": 1,
                "use_ipv6": 1,
            }),
        )
    }

    /// MV list (offset pagination). Upstream defaults: area 15, version 7, order 0.
    pub fn get_mv_list(&self, area: i64, version: i64, order: i64, num: i64, page: i64) -> Paged<GetMvListResponse> {
        Paged::new(
            self.cgi(
                "MvService.MvInfoProServer",
                "GetAllocMvInfo",
                json!({"area": area, "version": version, "order": order, "start": num * (page - 1), "size": num}),
            ),
            OffsetStrategy::with_size_key("start", "size")
                .total(|r: &GetMvListResponse| Some(r.total))
                .count(|r: &GetMvListResponse| Some(r.items.len() as i64)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[tokio::test]
    async fn mv_requests() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"v1": {"vid": "v1", "name": "n"}}));
        let detail = client.mv().get_detail(&["v1"]).await.unwrap();
        assert_eq!(detail.data["v1"].mv.name, "n");
        let req = last_req0(&mock);
        assert_eq!(req["param"]["vidlist"], json!(["v1"]));
        assert_eq!(req["param"]["required"].as_array().unwrap().len(), DETAIL_FIELDS.len());

        push_cgi(&mock, json!({"v1": {"mp4": [{"url": ["u"]}]}}));
        let urls = client.mv().get_mv_urls(&["v1"]).await.unwrap();
        assert_eq!(urls.data["v1"].mp4[0].url, vec!["u"]);
        assert_eq!(last_req0(&mock)["param"]["request_type"], 10003);

        push_cgi(&mock, json!({"total": 1, "list": [{"vid": "a"}]}));
        let items = client.mv().get_mv_list(15, 7, 0, 10, 1).collect_items(None).await.unwrap();
        assert_eq!(items[0].mv.vid, "a");
        assert_eq!(last_req0(&mock)["param"], json!({"area": 15, "version": 7, "order": 0, "start": 0, "size": 10}));
    }
}
