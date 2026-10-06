//! Playlist APIs.

use serde_json::{Value, json};

use crate::credential::Credential;
use crate::error::Result;
use crate::models::songlist::{CreateDeleteSonglistResp, GetSonglistDetailResponse};
use crate::pagination::{OffsetStrategy, Paged};
use crate::request::CgiRequest;

/// Dir id of the "I like" playlist.
pub const LIKE_DIRID: i64 = 201;

/// Error code returned when the songs are already (not) in the playlist.
const NOOP_CODE: i64 = 80092;

/// Options of [`SonglistApi::get_detail_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SonglistDetailOptions {
    /// Dir id.
    pub dirid: i64,
    /// Songs per page.
    pub num: i64,
    /// Page (1 based).
    pub page: i64,
    /// Only return songs.
    pub onlysong: bool,
    /// Include tags.
    pub tag: bool,
    /// Include creator info.
    pub userinfo: bool,
}

impl Default for SonglistDetailOptions {
    fn default() -> Self {
        Self {
            dirid: 0,
            num: 10,
            page: 1,
            onlysong: false,
            tag: true,
            userinfo: true,
        }
    }
}

fn oper_param(dirid: i64, songs: &[(i64, i64)], tid: i64) -> Value {
    json!({
        "dirId": dirid,
        "tid": tid,
        "bFmtUtf8": true,
        "v_songInfo": songs
            .iter()
            .map(|(id, ty)| json!({"songId": id, "songType": ty}))
            .collect::<Vec<_>>(),
    })
}

api_module! {
    /// Playlist APIs.
    SonglistApi
}

impl SonglistApi {
    /// Playlist detail (offset pagination over songs).
    pub fn get_detail(&self, songlist_id: i64) -> Paged<GetSonglistDetailResponse> {
        self.get_detail_with(songlist_id, SonglistDetailOptions::default())
    }

    /// Playlist detail with options.
    pub fn get_detail_with(&self, songlist_id: i64, options: SonglistDetailOptions) -> Paged<GetSonglistDetailResponse> {
        Paged::new(
            self.cgi(
                "music.srfDissInfo.DissInfo",
                "CgiGetDiss",
                json!({
                    "disstid": songlist_id,
                    "dirid": options.dirid,
                    "tag": options.tag,
                    "song_begin": options.num * (options.page - 1),
                    "song_num": options.num,
                    "userinfo": options.userinfo,
                    "orderlist": true,
                    "onlysonglist": options.onlysong,
                }),
            ),
            OffsetStrategy::with_size_key("song_begin", "song_num")
                .has_more(|r: &GetSonglistDetailResponse| Some(r.hasmore != 0))
                .total(|r: &GetSonglistDetailResponse| Some(r.total))
                .count(|r: &GetSonglistDetailResponse| Some(r.songs.len() as i64)),
        )
    }

    /// Create a playlist (login required).
    pub fn create(&self, dirname: &str, credential: Option<Credential>) -> CgiRequest<CreateDeleteSonglistResp> {
        self.cgi("music.musicasset.PlaylistBaseWrite", "AddPlaylist", json!({"dirName": dirname}))
            .require_login(true)
            .credential_opt(credential)
    }

    /// Delete a playlist (login required).
    pub fn delete(&self, dirid: i64, credential: Option<Credential>) -> CgiRequest<CreateDeleteSonglistResp> {
        self.cgi("music.musicasset.PlaylistBaseWrite", "DelPlaylist", json!({"dirId": dirid}))
            .require_login(true)
            .credential_opt(credential)
    }

    /// Add `(song_id, song_type)` pairs. `Ok(false)` when nothing changed.
    pub async fn add_songs(&self, dirid: i64, songs: &[(i64, i64)], tid: i64, credential: Option<Credential>) -> Result<bool> {
        let request = self
            .cgi::<Value>("music.musicasset.PlaylistDetailWrite", "AddSonglist", oper_param(dirid, songs, tid))
            .require_login(true)
            .preserve_bool(true);
        ret_code_ok(request.credential_opt(credential).send().await)
    }

    /// Remove `(song_id, song_type)` pairs. `Ok(false)` when nothing changed.
    pub async fn del_songs(&self, dirid: i64, songs: &[(i64, i64)], tid: i64, credential: Option<Credential>) -> Result<bool> {
        let request = self
            .cgi::<Value>("music.musicasset.PlaylistDetailWrite", "DelSonglist", oper_param(dirid, songs, tid))
            .require_login(true);
        ret_code_ok(request.credential_opt(credential).send().await)
    }

    /// Add songs to "I like".
    pub async fn like_song(&self, songs: &[(i64, i64)], credential: Option<Credential>) -> Result<bool> {
        self.add_songs(LIKE_DIRID, songs, 0, credential).await
    }

    /// Remove songs from "I like".
    pub async fn unlike_song(&self, songs: &[(i64, i64)], credential: Option<Credential>) -> Result<bool> {
        self.del_songs(LIKE_DIRID, songs, 0, credential).await
    }
}

fn ret_code_ok(result: Result<Value>) -> Result<bool> {
    match result {
        Ok(data) => Ok(data.get("retCode").and_then(Value::as_i64) == Some(0)),
        Err(err) if err.code() == Some(NOOP_CODE) => Ok(false),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::testing::*;

    #[tokio::test]
    async fn detail_paging() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"hasmore": 1, "total_song_num": 3, "songlist": [{"id": 1, "mid": "a"}, {"id": 2, "mid": "b"}]}));
        push_cgi(&mock, json!({"hasmore": 0, "total_song_num": 3, "songlist": [{"id": 3, "mid": "c"}]}));
        let songs = client
            .songlist()
            .get_detail_with(7, SonglistDetailOptions { num: 2, ..SonglistDetailOptions::default() })
            .collect_items(None)
            .await
            .unwrap();
        assert_eq!(songs.len(), 3);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["disstid"], 7);
        assert_eq!(req["param"]["song_begin"], 2);
        assert_eq!(req["param"]["tag"], 1);
        assert_eq!(req["param"]["onlysonglist"], 0);
    }

    #[tokio::test]
    async fn create_delete_and_songs() {
        let (client, mock) = logged_in_client();
        push_cgi(&mock, json!({"retCode": 0, "result": {"tid": 9, "dirId": 3, "dirName": "x"}}));
        let created = client.songlist().create("x", None).await.unwrap();
        assert_eq!(created.id, 9);
        push_cgi(&mock, json!({"retCode": 0}));
        client.songlist().delete(3, None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"dirId": 3}));

        push_cgi(&mock, json!({"retCode": 0}));
        assert!(client.songlist().like_song(&[(1, 0), (2, 1)], None).await.unwrap());
        let req = last_req0(&mock);
        assert_eq!(req["method"], "AddSonglist");
        assert_eq!(req["param"]["dirId"], 201);
        assert_eq!(req["param"]["bFmtUtf8"], true);
        assert_eq!(req["param"]["v_songInfo"], json!([{"songId": 1, "songType": 0}, {"songId": 2, "songType": 1}]));

        push_cgi_code(&mock, NOOP_CODE, json!({}));
        assert!(!client.songlist().unlike_song(&[(1, 0)], None).await.unwrap());
        assert_eq!(last_req0(&mock)["param"]["bFmtUtf8"], 1);

        push_cgi(&mock, json!({"retCode": 3}));
        assert!(!client.songlist().add_songs(5, &[(1, 0)], 0, None).await.unwrap());
        push_cgi_code(&mock, 2000, json!({}));
        assert!(client.songlist().del_songs(5, &[(1, 0)], 0, None).await.is_err());
    }

    #[tokio::test]
    async fn requires_login() {
        let (client, mock) = mock_client();
        let err = client.songlist().like_song(&[(1, 0)], None).await.unwrap_err();
        assert!(matches!(err, Error::CredentialInvalid(_)));
        assert_eq!(mock.request_count(), 0);
        push_cgi(&mock, json!({"retCode": 0}));
        assert!(client.songlist().like_song(&[(1, 0)], Some(Credential::new(1, "k"))).await.unwrap());
    }
}
