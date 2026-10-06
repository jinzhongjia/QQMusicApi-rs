//! Song models.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::base::{Mv, Singer, Song, SongList};
use crate::FromJson;
use crate::pagination::PageItems;

/// `query_song` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct QuerySongResponse {
    /// Songs.
    pub tracks: Vec<Song>,
}

/// One entry of a playback URL response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UrlInfo {
    /// Song mid.
    #[json(alias = "songmid")]
    pub mid: String,
    /// Requested file name.
    pub filename: String,
    /// URL path (empty when not available).
    pub purl: String,
    /// vkey.
    pub vkey: String,
    /// ekey (encrypted files).
    pub ekey: String,
    /// Result code.
    pub result: i64,
}

/// `get_song_urls` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSongUrlsResponse {
    /// Expiration.
    pub expiration: i64,
    /// URLs.
    #[json(alias = "midurlinfo")]
    pub data: Vec<UrlInfo>,
}

/// Detail content entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct ContentItem {
    /// Id.
    pub id: i64,
    /// Value.
    pub value: String,
    /// Show type.
    pub show_type: i64,
    /// Jump URL.
    pub jumpurl: String,
}

/// `get_detail` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSongDetailResponse {
    /// Company.
    #[json(path = "$.info.company.content")]
    pub company: Vec<ContentItem>,
    /// Genre.
    #[json(path = "$.info.genre.content")]
    pub genre: Vec<ContentItem>,
    /// Introduction.
    #[json(path = "$.info.intro.content")]
    pub intro: Vec<ContentItem>,
    /// Language.
    #[json(path = "$.info.lan.content")]
    pub lan: Vec<ContentItem>,
    /// Publish time.
    #[json(path = "$.info.pub_time.content")]
    pub pub_time: Vec<ContentItem>,
    /// Extras.
    pub extras: BTreeMap<String, String>,
    /// Song.
    #[json(alias = "track_info", required)]
    pub track: Song,
}

/// Group of similar songs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SimilarSongGroup {
    /// Title template.
    pub title_template: String,
    /// Title content.
    pub title_content: String,
    /// Songs.
    #[json(path = "$.songs[*].track")]
    pub song: Vec<Song>,
}

/// `get_similar_song` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSimilarSongResponse {
    /// Tags.
    #[json(alias = "songTagInfoList")]
    pub tag: Vec<Value>,
    /// Groups.
    #[json(path = "$.vecSongNew")]
    pub song: Vec<SimilarSongGroup>,
}

/// Song label.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongLabel {
    /// Id.
    pub id: i64,
    /// Text.
    #[json(alias = "tagTxt")]
    pub tag_txt: String,
    /// Icon.
    #[json(alias = "tagIcon")]
    pub tag_icon: String,
    /// URL.
    #[json(alias = "tagUrl")]
    pub tag_url: String,
    /// Type.
    #[json(alias = "tagType")]
    pub tag_type: i64,
    /// Species.
    pub species: i64,
}

/// `get_labels` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSongLabelsResponse {
    /// Labels.
    pub labels: Vec<SongLabel>,
}

/// Related playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RelatedPlaylist {
    /// Playlist.
    #[json(flatten)]
    #[serde(flatten)]
    pub songlist: SongList,
    /// Creator.
    pub creator: String,
}

/// `get_related_songlist` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetRelatedSonglistResponse {
    /// Has more.
    #[json(alias = "hasMore")]
    pub has_more: i64,
    /// Playlists.
    #[json(path = "$.vecPlaylistNew[*].playlists[*]")]
    pub songlist: Vec<RelatedPlaylist>,
}

impl PageItems for GetRelatedSonglistResponse {
    type Item = RelatedPlaylist;
    fn into_items(self) -> Vec<RelatedPlaylist> {
        self.songlist
    }
}

/// Singer of a related MV.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct MvSinger {
    /// Singer.
    #[json(flatten)]
    #[serde(flatten)]
    pub singer: Singer,
    /// Picture URL.
    pub picurl: String,
}

/// Related MV.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RelatedMv {
    /// MV.
    #[json(flatten)]
    #[serde(flatten)]
    pub mv: Mv,
    /// Picture URL.
    pub picurl: String,
    /// Play count.
    pub playcnt: i64,
    /// Singers.
    pub singers: Vec<MvSinger>,
}

/// `get_related_mv` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetRelatedMvResponse {
    /// Has more.
    #[json(alias = "hasmore")]
    pub has_more: i64,
    /// MVs.
    #[json(alias = "list")]
    pub mv: Vec<RelatedMv>,
}

impl PageItems for GetRelatedMvResponse {
    type Item = RelatedMv;
    fn into_items(self) -> Vec<RelatedMv> {
        self.mv
    }
}

/// `get_other_version` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetOtherVersionResponse {
    /// Songs.
    #[json(alias = "versionList")]
    pub data: Vec<Song>,
}

/// Producer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongProducer {
    /// Type.
    #[json(alias = "Type")]
    pub r#type: i64,
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
    /// Icon.
    #[json(alias = "Icon")]
    pub icon: String,
    /// Scheme.
    #[json(alias = "Scheme")]
    pub scheme: String,
    /// Singer mid.
    #[json(alias = "SingerMid")]
    pub singer_mid: String,
    /// Follow flag.
    #[json(alias = "Follow")]
    pub follow: i64,
}

/// Producer group.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongProducerGroup {
    /// Title.
    #[json(alias = "Title")]
    pub title: String,
    /// Producers.
    #[json(alias = "Producers")]
    pub producers: Vec<SongProducer>,
    /// Type.
    #[json(alias = "Type")]
    pub r#type: i64,
}

/// `get_producer` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetProducerResponse {
    /// Groups.
    #[json(alias = "Lst")]
    pub data: Vec<SongProducerGroup>,
    /// Message.
    #[json(alias = "ReinforceMsg")]
    pub reinforce_msg: String,
}

/// Sheet music.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SheetMusic {
    /// Score mid.
    #[json(alias = "scoreMID")]
    pub score_mid: String,
    /// Score name.
    #[json(alias = "scoreName")]
    pub score_name: String,
    /// Picture URLs.
    #[json(alias = "picURLs")]
    pub pic_urls: Vec<String>,
    /// Version.
    pub version: String,
    /// Tonality.
    pub tonality: i64,
    /// Score type.
    #[json(alias = "scoreType")]
    pub score_type: i64,
    /// Score type text.
    #[json(alias = "strScoreType")]
    pub score_type_text: String,
    /// Uploader.
    pub uploader: String,
    /// Views.
    #[json(alias = "viewFrequency")]
    pub view_frequency: i64,
    /// Tonality 2.
    pub tonality2: i64,
    /// Author.
    pub author: String,
    /// Composer.
    pub composer: String,
    /// Lyricist.
    pub lyricist: String,
    /// Singer.
    pub singer: String,
    /// Performer.
    pub performer: String,
    /// Song mid.
    #[json(alias = "songMID")]
    pub song_mid: String,
    /// Sub name.
    #[json(alias = "subName")]
    pub sub_name: String,
    /// URL.
    pub url: String,
    /// Album URL.
    #[json(alias = "albumURL")]
    pub album_url: String,
    /// Instrument type.
    #[json(alias = "insType")]
    pub ins_type: i64,
    /// Instrument type text.
    #[json(alias = "strInsType")]
    pub ins_type_text: String,
    /// Cover URL.
    #[json(alias = "coverURL")]
    pub cover_url: String,
    /// Difficulty.
    pub difficulty: String,
    /// Sheet file.
    #[json(alias = "sheetFile")]
    pub sheet_file: String,
}

/// `get_sheet` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetSheetResponse {
    /// Sheets.
    pub result: Vec<SheetMusic>,
    /// Totals by type.
    #[json(alias = "totalMap")]
    pub total_map: BTreeMap<String, i64>,
}

/// `has_sheet` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HasSheetMusicResponse {
    /// Guitar sheets.
    #[json(alias = "hasGuitar")]
    pub has_guitar: bool,
    /// More sheets.
    #[json(alias = "hasMore")]
    pub has_more: bool,
    /// LDY sheets.
    #[json(alias = "hasLDY")]
    pub has_ldy: bool,
    /// QRCX sheets.
    #[json(alias = "hasQRCX")]
    pub has_qrcx: bool,
    /// ChongChong sheets.
    #[json(alias = "hasChongChong")]
    pub has_chong_chong: bool,
}

/// `get_fav_num` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetFavNumResponse {
    /// Numbers by song id.
    #[json(alias = "m_numbers")]
    pub numbers: BTreeMap<String, i64>,
    /// Display strings by song id.
    #[json(alias = "m_show")]
    pub show: BTreeMap<String, String>,
}

/// CDN entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct CdnDispatchSipInfo {
    /// CDN.
    pub cdn: String,
    /// QUIC.
    pub quic: i64,
    /// IP stack.
    pub ipstack: i64,
    /// QUIC host.
    pub quichost: String,
    /// Plaintext QUIC.
    #[json(alias = "plaintextquic")]
    pub plaintext_quic: i64,
    /// Encrypted QUIC.
    #[json(alias = "encryptquic")]
    pub encrypt_quic: i64,
}

/// `get_cdn_dispatch` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct GetCdnDispatchResponse {
    /// Return code.
    pub retcode: i64,
    /// CDN hosts.
    pub sip: Vec<String>,
    /// CDN details.
    pub sipinfo: Vec<CdnDispatchSipInfo>,
    /// Keep-alive test file.
    #[json(alias = "keepalivefile")]
    pub test_file: String,
    /// Expiration.
    pub expiration: i64,
    /// Refresh time.
    #[json(alias = "refreshTime")]
    pub refresh_time: i64,
    /// Cache time.
    #[json(alias = "cacheTime")]
    pub cache_time: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn detail_with_paths() {
        let detail: GetSongDetailResponse = from_value(&json!({
            "info": {
                "company": {"content": [{"id": 1, "value": "JVR", "show_type": 0, "jumpurl": ""}]},
                "lan": {"content": [{"value": "国语"}]}
            },
            "extras": {"name": "晴天"},
            "track_info": {"id": 1, "mid": "m"}
        }))
        .unwrap();
        assert_eq!(detail.company[0].value, "JVR");
        assert_eq!(detail.lan[0].value, "国语");
        assert!(detail.genre.is_empty());
        assert_eq!(detail.extras["name"], "晴天");
        assert_eq!(detail.track.mid, "m");
        assert!(from_value::<GetSongDetailResponse>(&json!({})).is_err());
    }

    #[test]
    fn related_and_similar() {
        let related: GetRelatedSonglistResponse = from_value(&json!({
            "hasMore": 1,
            "vecPlaylistNew": [{"playlists": [{"tid": 1, "creator": "a"}]}, {"playlists": [{"tid": 2}]}]
        }))
        .unwrap();
        assert_eq!(related.songlist.len(), 2);
        assert_eq!(related.songlist[0].songlist.id, 1);
        assert_eq!(related.songlist[0].creator, "a");
        let similar: GetSimilarSongResponse = from_value(&json!({
            "vecSongNew": [{"title_template": "t", "songs": [{"track": {"id": 5, "mid": "x"}}]}]
        }))
        .unwrap();
        assert_eq!(similar.song[0].song[0].id, 5);
        let mvs: GetRelatedMvResponse = from_value(&json!({
            "hasmore": 0,
            "list": [{"vid": "v", "picurl": "p", "singers": [{"mid": "s", "picurl": "sp"}]}]
        }))
        .unwrap();
        assert_eq!(mvs.mv[0].mv.vid, "v");
        assert_eq!(mvs.mv[0].singers[0].singer.mid, "s");
        assert_eq!(mvs.into_items().len(), 1);
    }

    #[test]
    fn urls_and_misc() {
        let urls: GetSongUrlsResponse = from_value(&json!({
            "expiration": 80400,
            "midurlinfo": [{"songmid": "m", "filename": "M500mm.mp3", "purl": "p", "vkey": "k", "ekey": "", "result": 0}]
        }))
        .unwrap();
        assert_eq!(urls.data[0].mid, "m");
        let sheet: GetSheetResponse = from_value(&json!({"result": null, "totalMap": {"1": 2}})).unwrap();
        assert!(sheet.result.is_empty());
        assert_eq!(sheet.total_map["1"], 2);
        let has: HasSheetMusicResponse = from_value(&json!({"hasGuitar": 1, "hasMore": false})).unwrap();
        assert!(has.has_guitar && !has.has_more);
        let fav: GetFavNumResponse = from_value(&json!({"m_numbers": {"1": 10}, "m_show": {"1": "10"}})).unwrap();
        assert_eq!(fav.numbers["1"], 10);
        let cdn: GetCdnDispatchResponse = from_value(&json!({"sip": ["https://a/"], "keepalivefile": "k", "refreshTime": 1, "sipinfo": [{"plaintextquic": 1}]})).unwrap();
        assert_eq!(cdn.test_file, "k");
        assert_eq!(cdn.sipinfo[0].plaintext_quic, 1);
        let producer: GetProducerResponse = from_value(&json!({"Lst": [{"Title": "t", "Producers": [{"Name": "n", "Type": 1}]}]})).unwrap();
        assert_eq!(producer.data[0].producers[0].name, "n");
    }
}
