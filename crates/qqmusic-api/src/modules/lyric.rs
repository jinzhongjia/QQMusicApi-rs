//! Lyric APIs.

use serde_json::json;

use super::IdOrMid;
use crate::models::lyric::{
    BatchGetMultiStyleTransLyricResponse, GetAiDictResponse, GetLyricResponse, GetSingingAnnotationsInfoResponse,
    IsAiDictExistsResponse,
};
use crate::request::CgiRequest;

const MODULE: &str = "music.musichallSong.PlayLyricInfo";

/// Options of [`LyricApi::get_lyric_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LyricOptions {
    /// Song type.
    pub song_type: i64,
    /// Request QRC (word level) lyrics.
    pub qrc: bool,
    /// Request translation.
    pub trans: bool,
    /// Request romanization.
    pub roma: bool,
    /// Request singing annotations.
    pub singing_annotations: bool,
}

impl Default for LyricOptions {
    fn default() -> Self {
        Self { song_type: 1, qrc: false, trans: false, roma: false, singing_annotations: false }
    }
}

api_module! {
    /// Lyric APIs.
    LyricApi
}

impl LyricApi {
    /// Plain LRC lyric.
    pub fn get_lyric(&self, song: impl Into<IdOrMid>) -> CgiRequest<GetLyricResponse> {
        self.get_lyric_with(song, LyricOptions::default())
    }

    /// Lyric with options (decrypted automatically).
    pub fn get_lyric_with(&self, song: impl Into<IdOrMid>, options: LyricOptions) -> CgiRequest<GetLyricResponse> {
        let mut param = json!({
            "crypt": 1,
            "lrc_t": 0,
            "qrc": i64::from(options.qrc),
            "qrc_t": 0,
            "roma": i64::from(options.roma),
            "roma_t": 0,
            "trans": i64::from(options.trans),
            "trans_t": 0,
            "needSingingAnnotations": options.singing_annotations,
            "type": options.song_type,
        });
        song.into().insert(&mut param, "songId", "songMid");
        self.cgi(MODULE, "GetPlayLyricInfo", param).preserve_bool(true)
    }

    /// Singing annotation availability.
    pub fn get_singing_annotations_info(&self, songid: i64) -> CgiRequest<GetSingingAnnotationsInfoResponse> {
        self.cgi(MODULE, "GetSingingAnnotationsInfo", json!({"songID": songid, "needNum": false})).preserve_bool(true)
    }

    /// Translations in multiple styles.
    pub fn get_multi_style_trans_lyric(&self, songid: i64) -> CgiRequest<BatchGetMultiStyleTransLyricResponse> {
        self.cgi(MODULE, "BatchGetMultiStyleTransLyric", json!({"songID": songid}))
    }

    /// Whether an AI dictionary exists.
    pub fn is_ai_dict_exists(&self, songid: i64) -> CgiRequest<IsAiDictExistsResponse> {
        self.cgi(MODULE, "IsAIDictExists", json!({"songID": songid}))
    }

    /// AI dictionary.
    pub fn get_ai_dict(&self, songid: i64) -> CgiRequest<GetAiDictResponse> {
        self.cgi(MODULE, "GetAIDictInfo", json!({"songID": songid}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::algorithms::qrc_encrypt;
    use crate::testing::*;

    #[tokio::test]
    async fn get_lyric_payload_and_decrypt() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"songID": 1, "lyric": qrc_encrypt("[00:01.00]hi"), "trans": ""}));
        let lyric = client
            .lyric()
            .get_lyric_with("0039MnYb", LyricOptions { qrc: true, trans: true, ..LyricOptions::default() })
            .await
            .unwrap();
        assert_eq!(lyric.lyric, "[00:01.00]hi");
        let req = last_req0(&mock);
        assert_eq!(req["module"], MODULE);
        assert_eq!(req["param"]["songMid"], "0039MnYb");
        assert_eq!(req["param"]["qrc"], 1);
        assert_eq!(req["param"]["trans"], 1);
        assert_eq!(req["param"]["roma"], 0);
        assert_eq!(req["param"]["needSingingAnnotations"], false);
        assert_eq!(req["param"]["type"], 1);

        push_cgi(&mock, json!({"lyric": ""}));
        client.lyric().get_lyric(97773).await.unwrap();
        assert_eq!(last_req0(&mock)["param"]["songId"], 97773);
    }

    #[tokio::test]
    async fn other_lyric_endpoints() {
        let (client, mock) = mock_client();
        mock.route_url(
            "musicu",
            reply_all(json!({"hasSingingAnnotationsLyric": true, "exists": true, "lyrics": [], "dictList": []})),
        );
        assert!(client.lyric().get_singing_annotations_info(1).await.unwrap().has_singing_annotations_lyric);
        assert_eq!(last_req0(&mock)["param"], json!({"songID": 1, "needNum": false}));
        assert!(client.lyric().is_ai_dict_exists(2).await.unwrap().exists);
        assert_eq!(last_req0(&mock)["method"], "IsAIDictExists");
        client.lyric().get_ai_dict(3).await.unwrap();
        assert_eq!(last_req0(&mock)["method"], "GetAIDictInfo");
        client.lyric().get_multi_style_trans_lyric(4).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"songID": 4}));
    }
}
