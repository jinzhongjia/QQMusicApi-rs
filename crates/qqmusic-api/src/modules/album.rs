//! Album APIs.

use serde_json::json;

use super::IdOrMid;
use crate::credential::Credential;
use crate::models::album::{AlbumFavWriteResponse, GetAlbumDetailResponse, GetAlbumSongResponse, GetNewAlbumResponse};
use crate::pagination::{OffsetStrategy, Paged};
use crate::request::CgiRequest;

api_module! {
    /// Album APIs.
    AlbumApi
}

impl AlbumApi {
    /// Album detail.
    pub fn get_detail(&self, album: impl Into<IdOrMid>) -> CgiRequest<GetAlbumDetailResponse> {
        self.cgi(
            "music.musichallAlbum.AlbumInfoServer",
            "GetAlbumDetail",
            album.into().param("albumId", "albumMId"),
        )
    }

    /// Album songs (offset pagination).
    pub fn get_song(&self, album: impl Into<IdOrMid>, num: i64, page: i64) -> Paged<GetAlbumSongResponse> {
        let mut param = json!({"begin": num * (page - 1), "num": num});
        album.into().insert(&mut param, "albumId", "albumMid");
        Paged::new(
            self.cgi("music.musichallAlbum.AlbumSongList", "GetAlbumSongList", param),
            OffsetStrategy::with_size_key("begin", "num")
                .total(|r: &GetAlbumSongResponse| Some(r.total_num))
                .count(|r: &GetAlbumSongResponse| Some(r.song_list.len() as i64)),
        )
    }

    /// New albums of an area (offset pagination).
    pub fn get_new_album(&self, area: i64, num: i64, page: i64) -> Paged<GetNewAlbumResponse> {
        Paged::new(
            self.cgi(
                "newalbum.NewAlbumServer",
                "get_new_album_info",
                json!({"area": area, "num": num, "start": num * (page - 1)}),
            ),
            OffsetStrategy::with_size_key("start", "num")
                .total(|r: &GetNewAlbumResponse| Some(r.total))
                .count(|r: &GetNewAlbumResponse| Some(r.albums.len() as i64)),
        )
    }

    /// Favourite albums (login required).
    pub fn fav_album(&self, album_ids: &[i64], credential: Option<Credential>) -> CgiRequest<AlbumFavWriteResponse> {
        self.fav_write("FavAlbum", album_ids, credential)
    }

    /// Remove favourite albums (login required).
    pub fn del_fav_album(&self, album_ids: &[i64], credential: Option<Credential>) -> CgiRequest<AlbumFavWriteResponse> {
        self.fav_write("CancelFavAlbum", album_ids, credential)
    }

    fn fav_write(&self, method: &str, ids: &[i64], credential: Option<Credential>) -> CgiRequest<AlbumFavWriteResponse> {
        let request = self
            .cgi("music.musicasset.AlbumFavWrite", method, json!({"v_albumId": ids}))
            .require_login(true);
        match credential {
            Some(credential) => request.credential(credential),
            None => request,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::testing::*;

    #[tokio::test]
    async fn detail_and_songs() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"basicInfo": {"albumMid": "m", "albumName": "n"}}));
        let detail = client.album().get_detail("002MAeob").await.unwrap();
        assert_eq!(detail.album.album.name, "n");
        assert_eq!(last_req0(&mock)["param"], json!({"albumMId": "002MAeob"}));

        push_cgi(&mock, json!({"totalNum": 3, "songList": [{"songInfo": {"id": 1, "mid": "a"}}, {"songInfo": {"id": 2, "mid": "b"}}]}));
        push_cgi(&mock, json!({"totalNum": 3, "songList": [{"songInfo": {"id": 3, "mid": "c"}}]}));
        let songs = client.album().get_song(42, 2, 1).collect_items(None).await.unwrap();
        assert_eq!(songs.len(), 3);
        let req = last_req0(&mock);
        assert_eq!(req["param"]["albumId"], 42);
        assert_eq!(req["param"]["begin"], 2);
        assert_eq!(mock.request_count(), 3);
    }

    #[tokio::test]
    async fn new_album_and_fav() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"total": 1, "albums": [{"mid": "x"}]}));
        let albums = client.album().get_new_album(1, 20, 2).collect_items(None).await.unwrap();
        assert_eq!(albums[0].album.mid, "x");
        assert_eq!(last_req0(&mock)["param"], json!({"area": 1, "num": 20, "start": 20}));

        let err = client.album().fav_album(&[1], None).await.unwrap_err();
        assert!(matches!(err, Error::CredentialInvalid(_)), "{err:?}");
        push_cgi(&mock, json!({"result": 0}));
        let res = client.album().del_fav_album(&[1, 2], Some(Credential::new(1, "k"))).await.unwrap();
        assert!(res.success());
        assert_eq!(last_req0(&mock)["method"], "CancelFavAlbum");
        assert_eq!(last_req0(&mock)["param"]["v_albumId"], json!([1, 2]));
    }
}
