//! Playlist models.

use serde::Serialize;

use super::base::{Song, SongList};
use crate::FromJson;
use crate::pagination::PageItems;

/// Playlist creator.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SonglistCreator {
    /// Creator uin.
    pub musicid: i64,
    /// Nickname.
    pub nick: String,
    /// Avatar.
    pub headurl: String,
    /// Encrypted uin.
    pub encrypt_uin: String,
}

/// Playlist info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SonglistInfo {
    /// Playlist.
    #[json(flatten)]
    #[serde(flatten)]
    pub songlist: SongList,
    /// Creator.
    pub creator: SonglistCreator,
}

/// `get_detail` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSonglistDetailResponse {
    /// Code.
    pub code: i64,
    /// Sub code.
    pub subcode: i64,
    /// Message.
    pub msg: String,
    /// Info.
    #[json(alias = "dirinfo")]
    pub info: SonglistInfo,
    /// Number of songs in this page.
    #[json(alias = "songlist_size")]
    pub size: i64,
    /// Songs.
    #[json(alias = "songlist")]
    pub songs: Vec<Song>,
    /// Total songs.
    #[json(alias = "total_song_num")]
    pub total: i64,
    /// Has more.
    pub hasmore: i64,
}

impl PageItems for GetSonglistDetailResponse {
    type Item = Song;
    fn into_items(self) -> Vec<Song> {
        self.songs
    }
}

/// `create` / `delete` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CreateDeleteSonglistResp {
    /// Return code.
    #[json(alias = "retCode")]
    pub ret_code: i64,
    /// Playlist id.
    #[json(path = "$.result.tid")]
    pub id: i64,
    /// Dir id.
    #[json(path = "$.result.dirId")]
    pub dirid: i64,
    /// Name.
    #[json(path = "$.result.dirName")]
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn songlist_models() {
        let detail: GetSonglistDetailResponse = from_value(&json!({
            "dirinfo": {"id": 7, "title": "t", "creator": {"musicid": 1, "nick": "n"}},
            "songlist_size": 1,
            "songlist": [{"id": 1, "mid": "a"}],
            "total_song_num": 9,
            "hasmore": 1
        }))
        .unwrap();
        assert_eq!(detail.info.songlist.id, 7);
        assert_eq!(detail.info.creator.nick, "n");
        assert_eq!((detail.size, detail.total, detail.hasmore), (1, 9, 1));
        assert_eq!(detail.into_items()[0].mid, "a");
        let created: CreateDeleteSonglistResp =
            from_value(&json!({"retCode": 0, "result": {"tid": 5, "dirId": 3, "dirName": "x"}})).unwrap();
        assert_eq!((created.id, created.dirid, created.name.as_str()), (5, 3, "x"));
    }
}
