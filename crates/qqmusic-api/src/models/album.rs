//! Album models.

use serde::Serialize;

use super::base::{Album, Singer, Song};
use crate::FromJson;
use crate::pagination::PageItems;

/// Album detail.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AlbumDetail {
    /// Album.
    #[json(flatten)]
    #[serde(flatten)]
    pub album: Album,
    /// Description.
    pub desc: String,
    /// Language.
    pub language: String,
    /// Album type.
    #[json(alias = "albumType")]
    pub album_type: String,
    /// Genre.
    pub genre: String,
    /// Wiki URL.
    pub wikiurl: String,
}

/// Record company.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AlbumCompany {
    /// Id.
    #[json(alias = "ID")]
    pub id: i64,
    /// Name.
    pub name: String,
    /// Shown.
    #[json(alias = "isShow")]
    pub is_show: i64,
    /// Brief.
    pub brief: String,
}

/// `get_detail` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetAlbumDetailResponse {
    /// Album.
    #[json(alias = "basicInfo", required)]
    pub album: AlbumDetail,
    /// Company.
    pub company: AlbumCompany,
    /// Singers.
    #[json(path = "$.singer.singerList")]
    pub singers: Vec<Singer>,
}

/// `get_song` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetAlbumSongResponse {
    /// Album mid.
    #[json(alias = "albumMid")]
    pub album_mid: String,
    /// Total.
    #[json(alias = "totalNum")]
    pub total_num: i64,
    /// Songs.
    #[json(path = "$.songList[*].songInfo")]
    pub song_list: Vec<Song>,
}

impl PageItems for GetAlbumSongResponse {
    type Item = Song;
    fn into_items(self) -> Vec<Song> {
        self.song_list
    }
}

/// New album.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct NewAlbumItem {
    /// Album.
    #[json(flatten)]
    #[serde(flatten)]
    pub album: Album,
    /// Singers.
    pub singers: Vec<Singer>,
    /// Release time.
    pub release_time: String,
    /// Type.
    pub r#type: i64,
    /// Area.
    pub area: i64,
    /// Genre.
    pub genre: i64,
    /// Language.
    pub language: i64,
}

/// `get_new_album` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetNewAlbumResponse {
    /// Total.
    pub total: i64,
    /// Albums.
    pub albums: Vec<NewAlbumItem>,
}

impl PageItems for GetNewAlbumResponse {
    type Item = NewAlbumItem;
    fn into_items(self) -> Vec<NewAlbumItem> {
        self.albums
    }
}

/// Favourite write response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct AlbumFavWriteResponse {
    /// Result code.
    pub result: i64,
    /// Failed ids.
    #[json(alias = "v_failedAlbumId")]
    pub failed_album_id: Vec<i64>,
}

impl AlbumFavWriteResponse {
    /// Whether every album was written.
    pub fn success(&self) -> bool {
        self.result == 0 && self.failed_album_id.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn album_models() {
        let detail: GetAlbumDetailResponse = from_value(&json!({
            "basicInfo": {"albumID": 1, "albumMid": "m", "albumName": "叶惠美", "publishDate": "2003-07-31", "albumType": "录音室专辑"},
            "company": {"ID": 2, "name": "JVR", "isShow": 1},
            "singer": {"singerList": [{"mid": "s"}]}
        }))
        .unwrap();
        assert_eq!(detail.album.album.name, "叶惠美");
        assert_eq!(detail.album.album.time_public, "2003-07-31");
        assert_eq!(detail.album.album_type, "录音室专辑");
        assert_eq!(detail.company.id, 2);
        assert_eq!(detail.singers[0].mid, "s");
        let songs: GetAlbumSongResponse = from_value(&json!({"albumMid": "m", "totalNum": 2, "songList": [{"songInfo": {"id": 1, "mid": "a"}}]})).unwrap();
        assert_eq!(songs.into_items()[0].mid, "a");
        let fav: AlbumFavWriteResponse = from_value(&json!({"result": 0, "v_failedAlbumId": [3]})).unwrap();
        assert!(!fav.success());
        assert!(AlbumFavWriteResponse::default().success());
    }
}
