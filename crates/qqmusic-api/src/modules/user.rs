//! User APIs.

use serde_json::{Value, json};

use crate::credential::Credential;
use crate::error::Result;
use crate::models::songlist::GetSonglistDetailResponse;
use crate::models::user::{
    DislikeListData, UserCreatedSonglistResponse, UserFavAlbumResponse, UserFavMvResponse, UserFavSonglistResponse,
    UserFriendListResponse, UserHomepageResponse, UserMusicGeneResponse, UserRelationListResponse, UserVipInfoResponse,
};
use crate::modules::songlist::LIKE_DIRID;
use crate::pagination::{FnStrategy, OffsetStrategy, PageStrategy, Paged};
use crate::request::CgiRequest;

/// Section of [`UserApi::get_dislike_list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DislikeListKind {
    /// Disliked singers.
    Singers,
    /// Disliked songs.
    #[default]
    Songs,
    /// Disliked styles.
    Styles,
}

impl DislikeListKind {
    /// `Cmd` value.
    pub fn cmd(self) -> i64 {
        match self {
            Self::Singers => 2,
            Self::Songs => 3,
            Self::Styles => 4,
        }
    }

    fn lastid_key(self) -> &'static str {
        match self {
            Self::Singers => "SingersLastid",
            Self::Songs => "SongLastid",
            Self::Styles => "StyleLastid",
        }
    }
}

/// Item type of [`UserApi::add_dislike`] / [`UserApi::cancel_dislike`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DislikeType {
    /// Songs.
    Song,
    /// Singers.
    Singer,
    /// Styles.
    Style,
}

impl DislikeType {
    /// `IdType` value.
    pub fn id_type(self) -> i64 {
        match self {
            Self::Song => 1,
            Self::Singer => 2,
            Self::Style => 3,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Song => "Songs",
            Self::Singer => "Singers",
            Self::Style => "Styles",
        }
    }
}

/// Placeholder credential used by [`UserApi::get_homepage`] when not logged in
/// (the endpoint rejects anonymous requests but accepts any well-formed login).
pub fn placeholder_credential() -> Credential {
    let mut credential = Credential::new(1, "placeholder-musickey");
    credential.str_musicid = "1".into();
    credential.encrypt_uin = "0".repeat(32);
    credential.login_type = 1;
    credential
}

fn relation_strategy() -> OffsetStrategy<UserRelationListResponse> {
    OffsetStrategy::with_size_key("From", "Size")
        .has_more(|r: &UserRelationListResponse| Some(r.has_more))
        .total(|r: &UserRelationListResponse| Some(r.total))
        .count(|r: &UserRelationListResponse| Some(r.users.len() as i64))
}

fn dislike_body(kind: DislikeType, values: &[i64]) -> Value {
    let items: Vec<Value> = values
        .iter()
        .map(|v| json!({"ID": v.to_string(), "IdType": kind.id_type()}))
        .collect();
    let mut body = json!({});
    body[kind.key()] = json!(items);
    body
}

fn retcode_ok(data: &Value) -> bool {
    data.get("Retcode").and_then(Value::as_i64) == Some(0)
}

api_module! {
    /// User APIs.
    UserApi
}

impl UserApi {
    fn resolve_placeholder(&self, credential: Option<Credential>) -> Credential {
        credential.unwrap_or_else(|| {
            let current = self.client.credential();
            if current.is_valid() { current } else { placeholder_credential() }
        })
    }

    fn login_cgi<T: crate::FromJson + Send + 'static>(
        &self,
        module: &str,
        method: &str,
        param: Value,
        credential: Option<Credential>,
    ) -> CgiRequest<T> {
        self.cgi(module, method, param).require_login(true).credential_opt(credential)
    }

    /// User homepage by encrypted uin.
    pub fn get_homepage(&self, euin: &str, credential: Option<Credential>) -> CgiRequest<UserHomepageResponse> {
        self.cgi(
            "music.UnifiedHomepage.UnifiedHomepageSrv",
            "GetHomepageHeader",
            json!({"uin": euin, "IsQueryTabDetail": 1}),
        )
        .credential(self.resolve_placeholder(credential))
    }

    /// VIP info of the current user.
    pub fn get_vip_info(&self, credential: Option<Credential>) -> CgiRequest<UserVipInfoResponse> {
        self.login_cgi("VipLogin.VipLoginInter", "vip_login_base", json!({}), credential)
    }

    fn relation(&self, method: &str, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserRelationListResponse> {
        Paged::new(
            self.login_cgi(
                "music.concern.RelationList",
                method,
                json!({"HostUin": euin, "From": (page - 1) * num, "Size": num}),
                credential,
            ),
            relation_strategy(),
        )
    }

    /// Followed singers.
    pub fn get_follow_singers(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserRelationListResponse> {
        self.relation("GetFollowSingerList", euin, page, num, credential)
    }

    /// Fans.
    pub fn get_fans(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserRelationListResponse> {
        self.relation("GetFansList", euin, page, num, credential)
    }

    /// Followed users.
    pub fn get_follow_user(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserRelationListResponse> {
        self.relation("GetFollowUserList", euin, page, num, credential)
    }

    /// Friends of the current user (page pagination).
    pub fn get_friend(&self, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserFriendListResponse> {
        Paged::new(
            self.login_cgi(
                "music.homepage.Friendship",
                "GetFriendList",
                json!({"PageSize": num, "Page": page - 1}),
                credential,
            ),
            PageStrategy::new("Page")
                .page_size(num)
                .start_page(page - 1)
                .has_more(|r: &UserFriendListResponse| Some(r.has_more)),
        )
    }

    /// Playlists created by `uin`.
    pub fn get_created_songlist(&self, uin: i64, credential: Option<Credential>) -> CgiRequest<UserCreatedSonglistResponse> {
        self.cgi(
            "music.musicasset.PlaylistBaseRead",
            "GetPlaylistByUin",
            json!({"uin": uin.to_string()}),
        )
        .credential_opt(credential)
    }

    /// Liked songs of `euin` (offset pagination).
    pub fn get_fav_song(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<GetSonglistDetailResponse> {
        Paged::new(
            self.cgi(
                "music.srfDissInfo.DissInfo",
                "CgiGetDiss",
                json!({
                    "disstid": 0,
                    "dirid": LIKE_DIRID,
                    "tag": true,
                    "song_begin": num * (page - 1),
                    "song_num": num,
                    "userinfo": true,
                    "orderlist": true,
                    "enc_host_uin": euin,
                }),
            )
            .credential_opt(credential),
            OffsetStrategy::with_size_key("song_begin", "song_num")
                .has_more(|r: &GetSonglistDetailResponse| Some(r.hasmore != 0))
                .total(|r: &GetSonglistDetailResponse| Some(r.total))
                .count(|r: &GetSonglistDetailResponse| Some(r.songs.len() as i64)),
        )
    }

    /// Favourite playlists of `euin` (offset pagination).
    pub fn get_fav_songlist(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserFavSonglistResponse> {
        Paged::new(
            self.cgi(
                "music.musicasset.PlaylistFavRead",
                "CgiGetPlaylistFavInfo",
                json!({"uin": euin, "offset": (page - 1) * num, "size": num}),
            )
            .credential_opt(credential),
            OffsetStrategy::with_size_key("offset", "size")
                .has_more(|r: &UserFavSonglistResponse| Some(r.hasmore != 0))
                .total(|r: &UserFavSonglistResponse| Some(r.total))
                .count(|r: &UserFavSonglistResponse| Some(r.playlists.len() as i64)),
        )
    }

    async fn playlist_fav_write(&self, method: &str, songlist_id: i64, credential: Option<Credential>) -> Result<bool> {
        let uin = credential
            .as_ref()
            .map_or_else(|| self.client.credential().encrypt_uin, |c| c.encrypt_uin.clone());
        let data: Value = self
            .login_cgi(
                "music.musicasset.PlaylistFavWrite",
                method,
                json!({"uin": uin, "v_playlistId": [songlist_id]}),
                credential,
            )
            .send()
            .await?;
        let failed = data
            .get("v_failedPlaylistId")
            .and_then(Value::as_array)
            .is_some_and(|ids| ids.iter().any(|id| id.as_i64() == Some(songlist_id)));
        Ok(data.get("result").and_then(Value::as_i64) == Some(0) && !failed)
    }

    /// Favourite a playlist.
    pub async fn fav_songlist(&self, songlist_id: i64, credential: Option<Credential>) -> Result<bool> {
        self.playlist_fav_write("FavPlaylist", songlist_id, credential).await
    }

    /// Un-favourite a playlist.
    pub async fn unfav_songlist(&self, songlist_id: i64, credential: Option<Credential>) -> Result<bool> {
        self.playlist_fav_write("CancelFavPlaylist", songlist_id, credential).await
    }

    /// Favourite albums of `euin` (offset pagination).
    pub fn get_fav_album(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> Paged<UserFavAlbumResponse> {
        Paged::new(
            self.cgi(
                "music.musicasset.AlbumFavRead",
                "CgiGetAlbumFavInfo",
                json!({"euin": euin, "offset": (page - 1) * num, "size": num}),
            )
            .credential_opt(credential),
            OffsetStrategy::with_size_key("offset", "size")
                .has_more(|r: &UserFavAlbumResponse| Some(r.hasmore != 0))
                .total(|r: &UserFavAlbumResponse| Some(r.total))
                .count(|r: &UserFavAlbumResponse| Some(r.albums.len() as i64)),
        )
    }

    /// Favourite MVs of `euin`.
    pub fn get_fav_mv(&self, euin: &str, page: i64, num: i64, credential: Option<Credential>) -> CgiRequest<UserFavMvResponse> {
        self.login_cgi(
            "music.musicasset.MVFavRead",
            "getMyFavMV_v2",
            json!({"encuin": euin, "pagesize": num, "num": page - 1}),
            credential,
        )
    }

    /// Music gene report of `euin`.
    pub fn get_music_gene(&self, euin: &str, credential: Option<Credential>) -> CgiRequest<UserMusicGeneResponse> {
        self.cgi(
            "music.recommend.UserProfileSettingSvr",
            "GetProfileReport",
            json!({"VisitAccount": euin}),
        )
        .credential_opt(credential)
    }

    /// Dislike list (signed request, continuation pagination).
    pub fn get_dislike_list(&self, kind: DislikeListKind, page: i64, lastid: i64, credential: Option<Credential>) -> Paged<DislikeListData> {
        let mut param = json!({"Cmd": kind.cmd(), "Page": page});
        if lastid != 0 {
            param[kind.lastid_key()] = json!(lastid);
        }
        Paged::new(
            self.login_cgi("music.feedback.FeedbackBlack", "GetDislikeList", param, credential)
                .sign(true),
            FnStrategy(|params: &Value, r: &DislikeListData| {
                if r.singers.is_empty() && r.songs.is_empty() && r.styles.is_empty() {
                    return None;
                }
                let mut next = params.clone();
                next["Page"] = json!(params["Page"].as_i64().unwrap_or(1) + 1);
                if let Some(last) = r.songs.last() {
                    next["SongLastid"] = json!(last.id);
                }
                if let Some(last) = r.singers.last() {
                    next["SingersLastid"] = json!(last.id);
                }
                if let Some(last) = r.styles.last() {
                    next["StyleLastid"] = json!(last.id);
                }
                Some(next)
            }),
        )
    }

    /// Add items to the dislike list.
    pub async fn add_dislike(&self, kind: DislikeType, values: &[i64], credential: Option<Credential>) -> Result<bool> {
        let data: Value = self
            .login_cgi("music.feedback.FeedbackBlack", "AddDislike", dislike_body(kind, values), credential)
            .send()
            .await?;
        Ok(retcode_ok(&data))
    }

    /// Remove items from the dislike list.
    pub async fn cancel_dislike(&self, kind: DislikeType, values: &[i64], credential: Option<Credential>) -> Result<bool> {
        let data: Value = self
            .login_cgi("music.feedback.FeedbackBlack", "CancelDislike", dislike_body(kind, values), credential)
            .send()
            .await?;
        Ok(retcode_ok(&data))
    }

    /// Clear all disliked songs (fetches a token first).
    pub async fn cancel_all_dislike_song(&self, credential: Option<Credential>) -> Result<bool> {
        let token: Value = self
            .login_cgi(
                "music.feedback.FeedbackBlack",
                "CancelAllDislike",
                json!({"ISOnlyGetToken": true}),
                credential.clone(),
            )
            .preserve_bool(true)
            .send()
            .await?;
        let token = token.get("Token").and_then(Value::as_str).unwrap_or_default().to_string();
        let data: Value = self
            .login_cgi(
                "music.feedback.FeedbackBlack",
                "CancelAllDislike",
                json!({"DelType": 3, "Token": token}),
                credential,
            )
            .send()
            .await?;
        Ok(retcode_ok(&data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::*;

    #[tokio::test]
    async fn homepage_uses_placeholder_when_anonymous() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"Info": {"BaseInfo": {"Name": "n"}}}));
        let home = client.user().get_homepage("euin", None).await.unwrap();
        assert_eq!(home.base_info.name, "n");
        let body = last_body(&mock);
        assert_eq!(body["comm"]["qq"], "1");
        assert_eq!(body["comm"]["authst"], "placeholder-musickey");
        assert_eq!(body["req_0"]["param"], json!({"uin": "euin", "IsQueryTabDetail": 1}));

        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({}));
        client.user().get_homepage("euin", None).await.unwrap();
        assert_ne!(last_body(&mock)["comm"]["authst"], "placeholder-musickey");
    }

    #[tokio::test]
    async fn relation_and_friend_paging() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"Total": 3, "HasMore": true, "List": [{"MID": "a"}, {"MID": "b"}]}));
        push_cgi(&mock, json!({"Total": 3, "HasMore": false, "List": [{"MID": "c"}]}));
        let users = client.user().get_fans("e", 1, 2, None).collect_items(None).await.unwrap();
        assert_eq!(users.len(), 3);
        let req = last_req0(&mock);
        assert_eq!(req["method"], "GetFansList");
        assert_eq!(req["param"]["From"], 2);

        push_cgi(&mock, json!({"List": []}));
        client.user().get_follow_singers("e", 1, 10, None).await.unwrap();
        assert_eq!(last_req0(&mock)["method"], "GetFollowSingerList");
        push_cgi(&mock, json!({"List": []}));
        client.user().get_follow_user("e", 1, 10, None).await.unwrap();
        assert_eq!(last_req0(&mock)["method"], "GetFollowUserList");

        push_cgi(&mock, json!({"Friends": [{"UserName": "a"}], "HasMore": true}));
        push_cgi(&mock, json!({"Friends": [{"UserName": "b"}], "HasMore": false}));
        let friends = client.user().get_friend(1, 1, None).collect_items(None).await.unwrap();
        assert_eq!(friends.len(), 2);
        assert_eq!(last_req0(&mock)["param"]["Page"], 1);
    }

    #[tokio::test]
    async fn favourites() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"total": 1, "v_playlist": [{"tid": 1}]}));
        client.user().get_created_songlist(42, None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"uin": "42"}));

        push_cgi(&mock, json!({"hasmore": 0, "total_song_num": 1, "songlist": [{"id": 1, "mid": "a"}]}));
        let songs = client.user().get_fav_song("e", 1, 10, None).collect_items(None).await.unwrap();
        assert_eq!(songs.len(), 1);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["dirid"], 201);
        assert_eq!(req["param"]["enc_host_uin"], "e");

        push_cgi(&mock, json!({"hasmore": 0, "v_list": [{"tid": 1}]}));
        assert_eq!(client.user().get_fav_songlist("e", 1, 10, None).collect_items(None).await.unwrap().len(), 1);
        push_cgi(&mock, json!({"hasmore": 0, "v_list": [{"albumMid": "a"}]}));
        assert_eq!(client.user().get_fav_album("e", 1, 10, None).collect_items(None).await.unwrap().len(), 1);
        push_cgi(&mock, json!({"mvlist": [{"vid": "v"}]}));
        client.user().get_fav_mv("e", 2, 10, None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"encuin": "e", "pagesize": 10, "num": 1}));
        push_cgi(&mock, json!({"IsVisitAccount": true}));
        assert!(client.user().get_music_gene("e", None).await.unwrap().is_visit_account);
        push_cgi(&mock, json!({"identity": {"HugeVip": 1}}));
        assert_eq!(client.user().get_vip_info(None).await.unwrap().identity.huge_vip, 1);

        push_cgi(&mock, json!({"result": 0, "v_failedPlaylistId": []}));
        assert!(client.user().fav_songlist(7, None).await.unwrap());
        let req = last_req0(&mock);
        assert_eq!(req["param"]["v_playlistId"], json!([7]));
        assert_eq!(req["param"]["uin"], client.credential().encrypt_uin);
        push_cgi(&mock, json!({"result": 0, "v_failedPlaylistId": [7]}));
        assert!(!client.user().unfav_songlist(7, None).await.unwrap());
        assert_eq!(last_req0(&mock)["method"], "CancelFavPlaylist");
    }

    #[tokio::test]
    async fn dislike() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"Songs": [{"ID": "5"}, {"ID": "6"}], "Page": 1}));
        push_cgi(&mock, json!({"Songs": [], "Page": 2}));
        let pages = client.user().get_dislike_list(DislikeListKind::Songs, 1, 0, None).collect(None).await.unwrap();
        assert_eq!(pages.len(), 2);
        let req = last_req0(&mock);
        assert_eq!(req["param"], json!({"Cmd": 3, "Page": 2, "SongLastid": "6"}));
        assert!(mock.last_request().unwrap().query_param("sign").is_some());

        push_cgi(&mock, json!({}));
        client.user().get_dislike_list(DislikeListKind::Singers, 1, 9, None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"]["SingersLastid"], 9);

        push_cgi(&mock, json!({"Retcode": 0}));
        assert!(client.user().add_dislike(DislikeType::Singer, &[1, 2], None).await.unwrap());
        assert_eq!(last_req0(&mock)["param"], json!({"Singers": [{"ID": "1", "IdType": 2}, {"ID": "2", "IdType": 2}]}));
        push_cgi(&mock, json!({"Retcode": 1}));
        assert!(!client.user().cancel_dislike(DislikeType::Song, &[1], None).await.unwrap());

        push_cgi(&mock, json!({"Token": "tk"}));
        push_cgi(&mock, json!({"Retcode": 0}));
        assert!(client.user().cancel_all_dislike_song(None).await.unwrap());
        assert_eq!(last_req0(&mock)["param"], json!({"DelType": 3, "Token": "tk"}));
        let first = mock.requests()[mock.request_count() - 2].json_body().unwrap();
        assert_eq!(first["req_0"]["param"]["ISOnlyGetToken"], true);
    }
}
