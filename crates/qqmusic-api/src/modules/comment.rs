//! Comment APIs.

use serde_json::{Value, json};

use crate::credential::Credential;
use crate::error::Result;
pub use crate::models::comment::CommentBizType;
use crate::models::comment::{AddCommentResponse, CommentCountResponse, CommentListResponse, MomentCommentResponse};
use crate::pagination::{CursorStrategy, FnStrategy, Paged};
use crate::request::CgiRequest;

/// Target of a comment request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommentTarget {
    /// Resource id.
    pub biz_id: i64,
    /// Resource type.
    pub biz_type: CommentBizType,
    /// Sub type.
    pub biz_sub_type: Option<i64>,
}

impl CommentTarget {
    /// Song comments.
    pub fn song(song_id: i64) -> Self {
        Self::new(song_id, CommentBizType::Song)
    }

    /// Comments of any resource.
    pub fn new(biz_id: i64, biz_type: CommentBizType) -> Self {
        Self { biz_id, biz_type, biz_sub_type: None }
    }

    /// Set the sub type.
    #[must_use]
    pub fn sub_type(mut self, sub_type: i64) -> Self {
        self.biz_sub_type = Some(sub_type);
        self
    }
}

impl From<i64> for CommentTarget {
    fn from(song_id: i64) -> Self {
        Self::song(song_id)
    }
}

/// Paging of comment lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentPage {
    /// Page (1 based).
    pub page_num: i64,
    /// Page size.
    pub page_size: i64,
    /// Cursor (`SeqNo` of the last comment).
    pub last_comment_seq_no: String,
}

impl Default for CommentPage {
    fn default() -> Self {
        Self { page_num: 1, page_size: 15, last_comment_seq_no: String::new() }
    }
}

fn comment_strategy() -> FnStrategy<impl Fn(&Value, &CommentListResponse) -> Option<Value> + Send + Sync + 'static> {
    FnStrategy(|params: &Value, r: &CommentListResponse| {
        if r.has_more == 0 {
            return None;
        }
        let cursor = r.comments.last()?.seq_no.clone();
        let mut next = params.clone();
        next["PageNum"] = json!(params["PageNum"].as_i64().unwrap_or(0) + 1);
        next["LastCommentSeqNo"] = json!(cursor);
        Some(next)
    })
}

api_module! {
    /// Comment APIs.
    CommentApi
}

impl CommentApi {
    /// Comment count.
    pub fn get_comment_count(&self, target: impl Into<CommentTarget>) -> CgiRequest<CommentCountResponse> {
        let target = target.into();
        let mut request = json!({"biz_id": target.biz_id.to_string(), "biz_type": target.biz_type.code()});
        match target.biz_sub_type {
            Some(sub) => request["biz_sub_type"] = json!(sub),
            None if target.biz_type == CommentBizType::Song => request["biz_sub_type"] = json!(2),
            None => {}
        }
        self.cgi("music.globalComment.CommentCountSrv", "GetCmCount", json!({"request": request}))
    }

    fn list(
        &self,
        method: &str,
        target: CommentTarget,
        page: CommentPage,
        mut param: Value,
    ) -> Paged<CommentListResponse> {
        param["BizType"] = json!(target.biz_type.code());
        param["BizId"] = json!(target.biz_id.to_string());
        param["LastCommentSeqNo"] = json!(page.last_comment_seq_no);
        param["PageSize"] = json!(page.page_size);
        param["PageNum"] = json!(page.page_num - 1);
        if let Some(sub) = target.biz_sub_type {
            param["BizSubType"] = json!(sub);
        }
        Paged::new(self.cgi("music.globalComment.CommentRead", method, param), comment_strategy())
    }

    /// Hot comments.
    pub fn get_hot_comments(&self, target: impl Into<CommentTarget>, page: CommentPage) -> Paged<CommentListResponse> {
        self.list("GetHotCommentList", target.into(), page, json!({"HotType": 1, "WithAirborne": 0, "PicEnable": 1}))
    }

    /// Newest comments.
    pub fn get_new_comments(&self, target: impl Into<CommentTarget>, page: CommentPage) -> Paged<CommentListResponse> {
        self.list(
            "GetNewCommentList",
            target.into(),
            page,
            json!({"HashTagID": "", "PicEnable": 1, "SelfSeeEnable": 1, "AudioEnable": 1}),
        )
    }

    /// Recommended comments.
    pub fn get_recommend_comments(
        &self,
        target: impl Into<CommentTarget>,
        page: CommentPage,
    ) -> Paged<CommentListResponse> {
        self.list(
            "GetRecCommentList",
            target.into(),
            page,
            json!({"PicEnable": 1, "Flag": 1, "CmListUIVer": 1, "AudioEnable": 1}),
        )
    }

    /// Moment (timestamped) comments (cursor pagination).
    pub fn get_moment_comments(
        &self,
        target: impl Into<CommentTarget>,
        page_size: i64,
        last_pos: &str,
    ) -> Paged<MomentCommentResponse> {
        let target = target.into();
        let mut param = json!({
            "LastPos": last_pos,
            "HashTagID": "",
            "SeekTs": -1,
            "Size": page_size,
            "BizType": target.biz_type.code(),
            "BizId": target.biz_id.to_string(),
        });
        if let Some(sub) = target.biz_sub_type {
            param["BizSubType"] = json!(sub);
        }
        Paged::new(
            self.cgi("music.globalComment.SongTsComment", "GetSongTsCmList", param),
            CursorStrategy::new("LastPos", |r: &MomentCommentResponse| {
                (!r.next_pos.is_empty()).then(|| json!(r.next_pos))
            })
            .has_more(|r: &MomentCommentResponse| Some(r.has_more == 1)),
        )
    }

    /// Post a comment (login required).
    pub fn add_comment(
        &self,
        target: impl Into<CommentTarget>,
        content: &str,
        reply_cmt_id: Option<&str>,
        credential: Option<Credential>,
    ) -> CgiRequest<AddCommentResponse> {
        let target = target.into();
        let mut param = json!({
            "Content": content,
            "BizType": target.biz_type.code(),
            "BizId": target.biz_id.to_string(),
        });
        if let Some(reply) = reply_cmt_id {
            param["RepliedCmId"] = json!(reply);
        }
        if let Some(sub) = target.biz_sub_type {
            param["BizSubType"] = json!(sub);
        }
        self.cgi("music.globalComment.CommentWriteServer", "AddComment", param)
            .require_login(true)
            .credential_opt(credential)
    }

    /// Delete a comment (login required).
    pub async fn delete_comment(&self, cm_id: &str, credential: Option<Credential>) -> Result<bool> {
        let data: Value = self
            .cgi("music.globalComment.CommentWriteServer", "DelComment", json!({"CommentId": cm_id}))
            .require_login(true)
            .credential_opt(credential)
            .send()
            .await?;
        Ok(data.get("SubCode").and_then(Value::as_i64).unwrap_or(0) == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[tokio::test]
    async fn count_params() {
        let (client, mock) = mock_client();
        mock.route_url("musicu", reply_all(json!({"response": {"count": 3}})));
        assert_eq!(client.comment().get_comment_count(97773).await.unwrap().count, 3);
        assert_eq!(
            last_req0(&mock)["param"],
            json!({"request": {"biz_id": "97773", "biz_type": 1, "biz_sub_type": 2}})
        );
        client.comment().get_comment_count(CommentTarget::new(5, CommentBizType::Album)).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"request": {"biz_id": "5", "biz_type": 2}}));
        client.comment().get_comment_count(CommentTarget::new(5, CommentBizType::Mv).sub_type(9)).await.unwrap();
        assert_eq!(last_req0(&mock)["param"]["request"]["biz_sub_type"], 9);
    }

    #[tokio::test]
    async fn list_pagination() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"CommentList": {"HasMore": 1, "Comments": [{"CmId": "a", "SeqNo": "s1"}]}}));
        push_cgi(&mock, json!({"CommentList": {"HasMore": 1, "Comments": []}}));
        let comments = client.comment().get_hot_comments(1, CommentPage::default()).collect_items(None).await.unwrap();
        assert_eq!(comments.len(), 1);
        let req = last_req0(&mock);
        assert_eq!(req["method"], "GetHotCommentList");
        assert_eq!(req["param"]["PageNum"], 1);
        assert_eq!(req["param"]["LastCommentSeqNo"], "s1");
        assert_eq!(req["param"]["HotType"], 1);
        assert_eq!(req["param"]["BizId"], "1");
        assert_eq!(mock.request_count(), 2);

        push_cgi(&mock, json!({"CommentList": {"HasMore": 0}}));
        client
            .comment()
            .get_new_comments(CommentTarget::song(1).sub_type(3), CommentPage { page_num: 2, ..CommentPage::default() })
            .await
            .unwrap();
        let req = last_req0(&mock);
        assert_eq!(req["param"]["PageNum"], 1);
        assert_eq!(req["param"]["BizSubType"], 3);
        assert_eq!(req["param"]["SelfSeeEnable"], 1);
        push_cgi(&mock, json!({}));
        client.comment().get_recommend_comments(1, CommentPage::default()).await.unwrap();
        assert_eq!(last_req0(&mock)["param"]["CmListUIVer"], 1);
    }

    #[tokio::test]
    async fn moment_cursor() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"HasMore": 1, "NextPos": "p2", "CmList": [{"CmId": "a"}]}));
        push_cgi(&mock, json!({"HasMore": 0, "NextPos": "p3", "CmList": [{"CmId": "b"}]}));
        let items = client.comment().get_moment_comments(1, 15, "").collect_items(None).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(last_req0(&mock)["param"]["LastPos"], "p2");
        assert_eq!(last_req0(&mock)["param"]["SeekTs"], -1);
    }

    #[tokio::test]
    async fn write_comments() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"AddedCmId": "new"}));
        let added = client.comment().add_comment(1, "好听", Some("p"), None).await.unwrap();
        assert_eq!(added.id, "new");
        assert_eq!(
            last_req0(&mock)["param"],
            json!({"Content": "好听", "BizType": 1, "BizId": "1", "RepliedCmId": "p"})
        );
        push_cgi(&mock, json!({"SubCode": 0}));
        assert!(client.comment().delete_comment("new", None).await.unwrap());
        push_cgi(&mock, json!({"SubCode": 5}));
        assert!(!client.comment().delete_comment("new", None).await.unwrap());
    }
}
