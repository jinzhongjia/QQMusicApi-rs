//! Song APIs.

use std::borrow::Cow;

use serde::Serialize;
use serde_json::{Value, json};

use crate::credential::Credential;
use crate::error::{Error, Result};
use crate::models::song::{
    GetCdnDispatchResponse, GetFavNumResponse, GetOtherVersionResponse, GetProducerResponse, GetRelatedMvResponse,
    GetRelatedSonglistResponse, GetSheetResponse, GetSimilarSongResponse, GetSongDetailResponse, GetSongLabelsResponse,
    GetSongUrlsResponse, HasSheetMusicResponse, QuerySongResponse,
};
use crate::pagination::{CursorStrategy, Paged};
use crate::request::CgiRequest;
use crate::utils::get_guid;
use crate::versioning::Platform;

/// Maximum number of songs per `get_song_urls` call.
pub const MAX_URL_MIDS: usize = 100;

/// Audio file type: filename prefix + extension.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct SongFileType {
    /// Filename prefix (e.g. `M500`).
    pub code: Cow<'static, str>,
    /// Extension including the dot (e.g. `.mp3`).
    pub extension: Cow<'static, str>,
    /// Encrypted (`GetEVkey`) file.
    pub encrypted: bool,
}

macro_rules! file_types {
    ($($(#[$doc:meta])* $name:ident = ($code:literal, $ext:literal, $enc:literal);)*) => {
        impl SongFileType {
            $(
                $(#[$doc])*
                pub const $name: Self = Self::new_const($code, $ext, $enc);
            )*
        }
    };
}

file_types! {
    /// DTS:X.
    DTS_X = ("DT03", ".mp4", false);
    /// Master tape.
    MASTER = ("AI00", ".flac", false);
    /// Atmos 2.0.
    ATMOS_2 = ("Q000", ".flac", false);
    /// Atmos 5.1.
    ATMOS_51 = ("Q001", ".flac", false);
    /// Atmos 7.1.
    ATMOS_71 = ("Q003", ".ogg", false);
    /// Dolby Atmos.
    ATMOS_DB = ("D004", ".mp4", false);
    /// NAC.
    NAC = ("TL01", ".nac", false);
    /// FLAC.
    FLAC = ("F000", ".flac", false);
    /// OGG 640k.
    OGG_640 = ("O801", ".ogg", false);
    /// OGG 320k.
    OGG_320 = ("O800", ".ogg", false);
    /// OGG 192k.
    OGG_192 = ("O600", ".ogg", false);
    /// OGG 96k.
    OGG_96 = ("O400", ".ogg", false);
    /// MP3 320k.
    MP3_320 = ("M800", ".mp3", false);
    /// MP3 128k.
    MP3_128 = ("M500", ".mp3", false);
    /// AAC 192k.
    ACC_192 = ("C600", ".m4a", false);
    /// AAC 96k.
    ACC_96 = ("C400", ".m4a", false);
    /// AAC 48k.
    ACC_48 = ("C200", ".m4a", false);
    /// Encrypted DTS:X.
    ENC_DTS_X = ("DTM3", ".mmp4", true);
    /// Encrypted vinyl.
    ENC_VINYL = ("V0M0", ".mflac", true);
    /// Encrypted master tape.
    ENC_MASTER = ("AIM0", ".mflac", true);
    /// Encrypted Atmos 2.0.
    ENC_ATMOS_2 = ("Q0M0", ".mflac", true);
    /// Encrypted Atmos 5.1.
    ENC_ATMOS_51 = ("Q0M1", ".mflac", true);
    /// Encrypted Atmos 7.1.
    ENC_ATMOS_71 = ("Q0M3", ".mgg", true);
    /// Encrypted Dolby Atmos.
    ENC_ATMOS_DB = ("D0M4", ".mmp4", true);
    /// Encrypted NAC.
    ENC_NAC = ("TLM1", ".mnac", true);
    /// Encrypted FLAC.
    ENC_FLAC = ("F0M0", ".mflac", true);
    /// Encrypted OGG 640k.
    ENC_OGG_640 = ("O8M1", ".mgg", true);
    /// Encrypted OGG 320k.
    ENC_OGG_320 = ("O8M0", ".mgg", true);
    /// Encrypted OGG 192k.
    ENC_OGG_192 = ("O6M0", ".mgg", true);
    /// Encrypted OGG 96k.
    ENC_OGG_96 = ("O4M0", ".mgg", true);
    /// Preview.
    TRY = ("RS02", ".mp3", false);
    /// Preview OGG 640k.
    TRY_OGG_640 = ("O802", ".ogg", false);
    /// Accompaniment.
    ACCOM = ("O801", ".ogg", false);
    /// Multi track.
    MULTI = ("O601", ".ogg", false);
    /// Piano.
    PIANO = ("AI01", ".ogg", false);
    /// Bayin.
    BAYIN = ("AI02", ".ogg", false);
    /// Guzheng.
    GUZHENG = ("AI03", ".ogg", false);
    /// Qudi.
    QUDI = ("AI04", ".ogg", false);
    /// Hulusi.
    HULUSI = ("AI05", ".ogg", false);
    /// Suona.
    SUONA = ("AI06", ".ogg", false);
    /// Shoudie.
    SHOUDIE = ("AI07", ".ogg", false);
    /// Guitar.
    GUITAR = ("AI08", ".ogg", false);
    /// Drums.
    DRUMS = ("AI09", ".ogg", false);
    /// Kazoo.
    KAZOO = ("A200", ".ogg", false);
    /// Therapy.
    THERAPY = ("AA01", ".ogg", false);
    /// Ring tone 128k.
    RING_128 = ("R500", ".mp3", false);
    /// Ring tone 96k.
    RING_96 = ("R400", ".m4a", false);
    /// Ring tone 48k.
    RING_48 = ("R200", ".m4a", false);
}

impl SongFileType {
    const fn new_const(code: &'static str, extension: &'static str, encrypted: bool) -> Self {
        Self { code: Cow::Borrowed(code), extension: Cow::Borrowed(extension), encrypted }
    }

    /// Custom file type.
    pub fn custom(code: impl Into<String>, extension: impl Into<String>, encrypted: bool) -> Self {
        Self { code: Cow::Owned(code.into()), extension: Cow::Owned(extension.into()), encrypted }
    }

    /// File name for a song.
    pub fn filename(&self, mid: &str, media_mid: Option<&str>) -> String {
        match media_mid.filter(|m| !m.is_empty()) {
            Some(media_mid) => format!("{}{media_mid}{}", self.code, self.extension),
            None => format!("{}{mid}{mid}{}", self.code, self.extension),
        }
    }
}

impl Default for SongFileType {
    fn default() -> Self {
        Self::MP3_128
    }
}

/// Song requested by `get_song_urls`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SongFileInfo {
    /// Song mid.
    pub mid: String,
    /// File type override.
    pub file_type: Option<SongFileType>,
    /// Song type.
    pub song_type: Option<i64>,
    /// Media mid (`file.media_mid`).
    pub media_mid: Option<String>,
}

impl SongFileInfo {
    /// Song by mid.
    pub fn new(mid: impl Into<String>) -> Self {
        Self { mid: mid.into(), ..Self::default() }
    }

    /// Set the file type.
    #[must_use]
    pub fn file_type(mut self, file_type: SongFileType) -> Self {
        self.file_type = Some(file_type);
        self
    }

    /// Set the song type.
    #[must_use]
    pub fn song_type(mut self, song_type: i64) -> Self {
        self.song_type = Some(song_type);
        self
    }

    /// Set the media mid.
    #[must_use]
    pub fn media_mid(mut self, media_mid: impl Into<String>) -> Self {
        self.media_mid = Some(media_mid.into());
        self
    }
}

impl From<&str> for SongFileInfo {
    fn from(mid: &str) -> Self {
        Self::new(mid)
    }
}

impl From<String> for SongFileInfo {
    fn from(mid: String) -> Self {
        Self::new(mid)
    }
}

/// Song id or mid.
pub type SongRef = super::IdOrMid;

/// Song for `query_song`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SongQuery {
    /// Id or mid.
    pub song: SongRef,
    /// Song type.
    pub song_type: i64,
}

impl SongQuery {
    /// Query with a song type.
    pub fn new(song: impl Into<SongRef>, song_type: i64) -> Self {
        Self { song: song.into(), song_type }
    }
}

impl From<SongRef> for SongQuery {
    fn from(song: SongRef) -> Self {
        Self { song, song_type: 0 }
    }
}

impl From<i64> for SongQuery {
    fn from(id: i64) -> Self {
        SongRef::Id(id).into()
    }
}

impl From<&str> for SongQuery {
    fn from(value: &str) -> Self {
        SongRef::from(value).into()
    }
}

impl From<String> for SongQuery {
    fn from(value: String) -> Self {
        SongRef::from(value).into()
    }
}

/// Playback quality tier (bypass quality ladder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// Lossless (FLAC / OGG 640).
    Lossless,
    /// High (320k).
    #[default]
    High,
    /// Standard (128k).
    Standard,
}

impl Quality {
    /// Name used by decky-music.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::High => "high",
            Self::Standard => "standard",
        }
    }

    /// Parse a name (unknown names return `None`).
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "lossless" | "sq" | "flac" => Some(Self::Lossless),
            "high" | "hq" | "320" => Some(Self::High),
            "standard" | "std" | "128" => Some(Self::Standard),
            _ => None,
        }
    }
}

/// Quality ladder (highest first) – playable, unencrypted formats only.
pub const DEFAULT_LADDER: [(Quality, SongFileType); 5] = [
    (Quality::Lossless, SongFileType::FLAC),
    (Quality::Lossless, SongFileType::OGG_640),
    (Quality::High, SongFileType::MP3_320),
    (Quality::High, SongFileType::OGG_320),
    (Quality::Standard, SongFileType::MP3_128),
];

/// Tiers from `max` downwards.
pub fn ladder_from(ladder: &[(Quality, SongFileType)], max: Quality) -> Vec<(Quality, SongFileType)> {
    ladder.iter().filter(|(q, _)| *q >= max).cloned().collect()
}

/// Result of [`SongApi::playable_url`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlayableUrl {
    /// Full URL.
    pub url: String,
    /// Quality tier obtained.
    pub quality: Quality,
    /// File type obtained.
    pub file_type: SongFileType,
    /// File name.
    pub filename: String,
}

api_module! {
    /// Song APIs.
    SongApi
}

impl SongApi {
    /// Query songs by id or mid (`CgiGetTrackInfo`).
    pub fn query_song<I, Q>(&self, songs: I) -> Result<CgiRequest<QuerySongResponse>>
    where
        I: IntoIterator<Item = Q>,
        Q: Into<SongQuery>,
    {
        let mut ids = Vec::new();
        let mut mids = Vec::new();
        let mut types = Vec::new();
        for song in songs {
            let song = song.into();
            match song.song {
                SongRef::Id(id) => ids.push(id),
                SongRef::Mid(mid) => mids.push(mid),
            }
            types.push(song.song_type);
        }
        if types.is_empty() {
            return Err(Error::invalid_argument("song_info 不能为空"));
        }
        let mut param = json!({
            "ctx": 0,
            "client": 1,
            "types": types,
            "modify_stamp": vec![0; types.len()],
        });
        if !ids.is_empty() {
            param["ids"] = json!(ids);
        }
        if !mids.is_empty() {
            param["mids"] = json!(mids);
        }
        Ok(self.cgi("music.trackInfo.UniformRuleCtrl", "CgiGetTrackInfo", param))
    }

    /// CDN dispatch information.
    pub fn get_cdn_dispatch(&self) -> CgiRequest<GetCdnDispatchResponse> {
        self.cgi(
            "music.audioCdnDispatch.cdnDispatch",
            "GetCdnDispatch",
            json!({"guid": get_guid(), "uid": "0", "use_new_domain": 1, "use_ipv6": 1}),
        )
    }

    /// Playback URLs (`UrlGetVkey` / `CgiGetEVkey` for encrypted types).
    ///
    /// Uses the client's [`BypassConfig`](crate::BypassConfig) when enabled
    /// (disable per request with `.bypass(false)`).
    pub fn get_song_urls<I, F>(
        &self,
        files: I,
        file_type: SongFileType,
        credential: Option<&Credential>,
    ) -> Result<CgiRequest<GetSongUrlsResponse>>
    where
        I: IntoIterator<Item = F>,
        F: Into<SongFileInfo>,
    {
        let files: Vec<SongFileInfo> = files.into_iter().map(Into::into).collect();
        if files.len() > MAX_URL_MIDS {
            return Err(Error::invalid_argument(format!("mid 数量不能超过 {MAX_URL_MIDS}, 当前为 {}", files.len())));
        }
        let mut songmid = Vec::with_capacity(files.len());
        let mut filename = Vec::with_capacity(files.len());
        let mut songtype = Vec::with_capacity(files.len());
        for file in &files {
            let ty = file.file_type.as_ref().unwrap_or(&file_type);
            filename.push(ty.filename(&file.mid, file.media_mid.as_deref()));
            songmid.push(file.mid.clone());
            songtype.push(file.song_type.unwrap_or(0));
        }
        let resolved = credential.cloned().unwrap_or_else(|| self.client.credential());
        let (module, method) = if file_type.encrypted {
            ("music.vkey.GetEVkey", "CgiGetEVkey")
        } else {
            ("music.vkey.GetVkey", "UrlGetVkey")
        };
        let request = self
            .cgi(
                module,
                method,
                json!({
                    "uin": resolved.musicid.to_string(),
                    "filename": filename,
                    "guid": get_guid(),
                    "songmid": songmid,
                    "songtype": songtype,
                    "ctx": 0,
                }),
            )
            .bypass(true)
            .credential_opt(credential.cloned());
        Ok(request)
    }

    /// Best playable URL up to `max_quality` (decky-music bypass ladder).
    ///
    /// All tiers are requested in a single call; the highest tier with a
    /// `purl` wins. Returns `None` when no tier is available (no copyright /
    /// VIP required).
    pub async fn playable_url(
        &self,
        mid: &str,
        media_mid: Option<&str>,
        max_quality: Quality,
    ) -> Result<Option<PlayableUrl>> {
        self.playable_url_with_ladder(mid, media_mid, &ladder_from(&DEFAULT_LADDER, max_quality)).await
    }

    /// [`playable_url`](Self::playable_url) with a custom ladder.
    pub async fn playable_url_with_ladder(
        &self,
        mid: &str,
        media_mid: Option<&str>,
        ladder: &[(Quality, SongFileType)],
    ) -> Result<Option<PlayableUrl>> {
        if ladder.is_empty() {
            return Err(Error::invalid_argument("quality ladder 不能为空"));
        }
        let file_mid = media_mid.filter(|m| !m.is_empty()).unwrap_or(mid);
        let filenames: Vec<String> =
            ladder.iter().map(|(_, ty)| format!("{}{file_mid}{file_mid}{}", ty.code, ty.extension)).collect();
        let response: GetSongUrlsResponse = self
            .cgi(
                "music.vkey.GetVkey",
                "UrlGetVkey",
                json!({
                    "filename": filenames,
                    "guid": get_guid(),
                    "songmid": vec![mid; ladder.len()],
                    "songtype": vec![0; ladder.len()],
                }),
            )
            .bypass(true)
            .send()
            .await?;
        let bypass = self.client.bypass();
        for ((quality, file_type), (info, filename)) in ladder.iter().zip(response.data.iter().zip(filenames)) {
            if !info.purl.is_empty() {
                return Ok(Some(PlayableUrl {
                    url: bypass.full_url(&info.purl),
                    quality: *quality,
                    file_type: file_type.clone(),
                    filename,
                }));
            }
        }
        Ok(None)
    }

    /// Song detail (web platform).
    pub fn get_detail(&self, song: impl Into<SongRef>) -> CgiRequest<GetSongDetailResponse> {
        self.cgi("music.pf_song_detail_svr", "get_song_detail_yqq", song.into().param("song_id", "song_mid"))
            .platform(Platform::Web)
    }

    /// Similar songs.
    pub fn get_similar_song(&self, songid: i64) -> CgiRequest<GetSimilarSongResponse> {
        self.cgi("music.recommend.TrackRelationServer", "GetSimilarSongs", json!({"songid": songid}))
    }

    /// Song labels.
    pub fn get_labels(&self, songid: i64) -> CgiRequest<GetSongLabelsResponse> {
        self.cgi("music.recommend.TrackRelationServer", "GetSongLabels", json!({"songid": songid}))
    }

    /// Related playlists (batch refresh pagination).
    pub fn get_related_songlist(&self, songid: i64, last: Vec<i64>) -> Paged<GetRelatedSonglistResponse> {
        let request = self.cgi(
            "music.recommend.TrackRelationServer",
            "GetRelatedPlaylist",
            json!({"songid": songid, "vecPlaylist": last}),
        );
        Paged::new(
            request,
            CursorStrategy::new("vecPlaylist", |r: &GetRelatedSonglistResponse| {
                (!r.songlist.is_empty()).then(|| json!(r.songlist.iter().map(|p| p.songlist.id).collect::<Vec<_>>()))
            })
            .has_more(|r: &GetRelatedSonglistResponse| Some(r.has_more != 0)),
        )
    }

    /// Related MVs (batch refresh pagination).
    pub fn get_related_mv(&self, songid: i64, last_mvid: Option<&str>) -> Paged<GetRelatedMvResponse> {
        let last: Value = match last_mvid {
            Some(id) if !id.is_empty() => json!(id),
            _ => json!(0),
        };
        let request = self.cgi(
            "MvService.MvInfoProServer",
            "GetSongRelatedMv",
            json!({"songid": songid.to_string(), "songtype": 1, "lastmvid": last}),
        );
        Paged::new(
            request,
            CursorStrategy::new("lastmvid", |r: &GetRelatedMvResponse| r.mv.last().map(|m| json!(m.mv.id)))
                .has_more(|r: &GetRelatedMvResponse| Some(r.has_more != 0)),
        )
    }

    /// Other versions of a song.
    pub fn get_other_version(&self, song: impl Into<SongRef>) -> CgiRequest<GetOtherVersionResponse> {
        self.cgi(
            "music.musichallSong.OtherVersionServer",
            "GetOtherVersionSongs",
            song.into().param("songid", "songmid"),
        )
    }

    /// Producers of a song.
    pub fn get_producer(&self, song: impl Into<SongRef>) -> CgiRequest<GetProducerResponse> {
        self.cgi("music.sociality.KolWorksTag", "SongProducer", song.into().param("songid", "songmid"))
    }

    /// Sheet music. `ttype`: 0 = default, 1 = guitar, 2 = ChongChong.
    pub fn get_sheet(&self, mid: &str, ttype: i64) -> CgiRequest<GetSheetResponse> {
        let mut comm = sheet_comm();
        if ttype == 2 {
            comm.push(("platform", "h5"));
            return self
                .cgi(
                    "music.mir.SheetMusicSvr",
                    "GetChongChongSheetMusic",
                    json!({"songMid": mid, "begin": 0, "end": 100, "scoreType": -1, "ttype": 1}),
                )
                .override_comm(comm)
                .sign(true)
                .allow_error_codes([10007], true);
        }
        let score_type = if ttype == 1 { -473 } else { -1 };
        self.cgi(
            "music.mir.SheetMusicSvr",
            "GetMoreSheetMusic",
            json!({"songMid": mid, "begin": 0, "end": 100, "scoreType": score_type, "ttype": ttype}),
        )
        .override_comm(comm)
        .allow_error_codes([10007], true)
    }

    /// Whether sheet music exists.
    pub fn has_sheet(&self, mid: &str) -> CgiRequest<HasSheetMusicResponse> {
        self.cgi("music.mir.SheetMusicSvr", "HasSheetMusic", json!({"songMid": mid})).override_comm(sheet_comm())
    }

    /// Favourite counts.
    pub fn get_fav_num(&self, song_ids: &[i64]) -> CgiRequest<GetFavNumResponse> {
        self.cgi("music.musicasset.SongFavRead", "GetSongFansNumberById", json!({"v_songId": song_ids}))
    }
}

fn sheet_comm() -> Vec<(&'static str, &'static str)> {
    vec![
        ("g_tk", "5381"),
        ("uin", ""),
        ("format", "json"),
        ("inCharset", "utf-8"),
        ("outCharset", "utf-8"),
        ("notice", "0"),
        ("needNewCode", "1"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bypass::{BypassConfig, HIGH_QUALITY_CT};
    use crate::testing::*;

    #[test]
    fn file_types_and_refs() {
        assert_eq!(SongFileType::MP3_128.filename("m", None), "M500mm.mp3");
        assert_eq!(SongFileType::FLAC.filename("m", Some("media")), "F000media.flac");
        assert_eq!(SongFileType::FLAC.filename("m", Some("")), "F000mm.flac");
        assert!(SongFileType::ENC_FLAC.filename("m", None).ends_with(".mflac"));
        assert_eq!(SongFileType::default(), SongFileType::MP3_128);
        assert_eq!(SongFileType::custom("X1", ".x", false).filename("a", None), "X1aa.x");
        assert_eq!(SongRef::from("123"), SongRef::Id(123));
        assert_eq!(SongRef::from("0039Mn"), SongRef::Mid("0039Mn".into()));
        assert_eq!(SongRef::from(""), SongRef::Mid(String::new()));
        assert_eq!(SongRef::from(String::from("7")), SongRef::Id(7));
        assert_eq!(Quality::parse("SQ"), Some(Quality::Lossless));
        assert_eq!(Quality::parse("x"), None);
        assert_eq!(Quality::default().as_str(), "high");
    }

    #[test]
    fn ladder_selection() {
        let ladder = ladder_from(&DEFAULT_LADDER, Quality::High);
        assert_eq!(ladder.len(), 3);
        assert_eq!(ladder[0].1, SongFileType::MP3_320);
        assert_eq!(ladder_from(&DEFAULT_LADDER, Quality::Lossless).len(), 5);
        assert_eq!(ladder_from(&DEFAULT_LADDER, Quality::Standard).len(), 1);
    }

    #[tokio::test]
    async fn query_song_payload() {
        let (client, mock) = mock_client();
        assert!(client.song().query_song(Vec::<i64>::new()).is_err());
        push_cgi(&mock, json!({"tracks": [{"id": 1, "mid": "a"}]}));
        let res = client
            .song()
            .query_song([SongQuery::from(1), SongQuery::from("abc"), SongQuery::new(2, 1)])
            .unwrap()
            .await
            .unwrap();
        assert_eq!(res.tracks[0].mid, "a");
        let req = last_req0(&mock);
        assert_eq!(req["module"], "music.trackInfo.UniformRuleCtrl");
        assert_eq!(req["param"]["ids"], json!([1, 2]));
        assert_eq!(req["param"]["mids"], json!(["abc"]));
        assert_eq!(req["param"]["types"], json!([0, 0, 1]));
        assert_eq!(req["param"]["modify_stamp"], json!([0, 0, 0]));
    }

    #[tokio::test]
    async fn song_urls_use_bypass_comm() {
        let (client, mock) = logged_in_client();
        let too_many: Vec<String> = (0..101).map(|i| i.to_string()).collect();
        assert!(client.song().get_song_urls(too_many, SongFileType::MP3_128, None).is_err());

        push_cgi(&mock, json!({"midurlinfo": [{"songmid": "m", "purl": "p"}]}));
        let res = client.song().get_song_urls(["m", "n"], SongFileType::FLAC, None).unwrap().await.unwrap();
        assert_eq!(res.data[0].purl, "p");
        let body = last_body(&mock);
        let device = client.device().await.unwrap();
        let ct = body["comm"]["ct"].as_str().unwrap().parse::<i64>().unwrap();
        assert!(HIGH_QUALITY_CT.contains(&ct));
        assert_eq!(body["comm"]["cv"], "0");
        assert_eq!(body["comm"]["qq"], "10001");
        assert_eq!(body["comm"]["authst"], "Q_H_L_key");
        assert!(body["comm"].get("QIMEI36").is_none());
        assert_eq!(body["req_0"]["module"], "music.vkey.GetVkey");
        assert_eq!(body["req_0"]["param"]["filename"], json!(["F000mm.flac", "F000nn.flac"]));
        assert_eq!(body["req_0"]["param"]["guid"], device.open_udid);
        assert_eq!(body["req_0"]["param"]["uin"], "10001");
        assert!(mock.last_request().unwrap().header("user-agent").unwrap().starts_with("Mozilla"));
    }

    #[tokio::test]
    async fn song_urls_without_bypass_and_encrypted() {
        let (client, mock) = mock_client();
        client.set_bypass(BypassConfig::disabled());
        push_cgi(&mock, json!({}));
        let other = Credential::new(5, "k");
        client
            .song()
            .get_song_urls([SongFileInfo::new("m").media_mid("mm").song_type(1)], SongFileType::ENC_FLAC, Some(&other))
            .unwrap()
            .await
            .unwrap();
        let body = last_body(&mock);
        assert_eq!(body["comm"]["ct"], "11");
        assert_eq!(body["comm"]["qq"], "5");
        assert_eq!(body["req_0"]["module"], "music.vkey.GetEVkey");
        assert_eq!(body["req_0"]["method"], "CgiGetEVkey");
        assert_eq!(body["req_0"]["param"]["filename"], json!(["F0M0mm.mflac"]));
        assert_eq!(body["req_0"]["param"]["songtype"], json!([1]));
        assert_eq!(body["req_0"]["param"]["uin"], "5");
        assert_ne!(body["req_0"]["param"]["guid"], client.device().await.unwrap().open_udid);
    }

    #[tokio::test]
    async fn playable_url_picks_highest_available() {
        let (client, mock) = logged_in_client();
        client.set_bypass(BypassConfig::default().with_fixed_ct(13));
        push_cgi(&mock, json!({"midurlinfo": [{"purl": ""}, {"purl": ""}, {"purl": "M800x.mp3?vkey=1"}]}));
        let url = client.song().playable_url("mid1", None, Quality::Lossless).await.unwrap().unwrap();
        assert_eq!(url.quality, Quality::High);
        assert_eq!(url.file_type, SongFileType::MP3_320);
        assert_eq!(url.url, "https://isure.stream.qqmusic.qq.com/M800x.mp3?vkey=1");
        assert_eq!(url.filename, "M800mid1mid1.mp3");
        let body = last_body(&mock);
        assert_eq!(body["comm"], json!({"ct": "13", "cv": "0", "qq": "10001", "authst": "Q_H_L_key"}));
        assert_eq!(body["req_0"]["param"]["songmid"].as_array().unwrap().len(), 5);
        assert!(body["req_0"]["param"].get("uin").is_none());

        push_cgi(&mock, json!({"midurlinfo": [{"purl": ""}]}));
        assert!(client.song().playable_url("m", Some("media"), Quality::Standard).await.unwrap().is_none());
        assert_eq!(last_req0(&mock)["param"]["filename"], json!(["M500mediamedia.mp3"]));
        assert!(client.song().playable_url_with_ladder("m", None, &[]).await.is_err());
    }

    #[tokio::test]
    async fn detail_and_misc_requests() {
        let (client, mock) = mock_client();
        mock.route_url("musicu", reply_all(json!({"track_info": {"id": 1, "mid": "m"}})));
        let detail = client.song().get_detail("0039MnYb").await.unwrap();
        assert_eq!(detail.track.id, 1);
        let body = last_body(&mock);
        assert_eq!(body["req_0"]["param"], json!({"song_mid": "0039MnYb"}));
        assert_eq!(body["comm"]["platform"], "yqq.json");
        let _ = client.song().get_detail(42).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"song_id": 42}));

        let _ = client.song().get_other_version("12").await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"songid": 12}));
        let _ = client.song().get_producer("abc").await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"songmid": "abc"}));
        let _ = client.song().get_similar_song(1).await.unwrap();
        let _ = client.song().get_labels(1).await.unwrap();
        let _ = client.song().get_fav_num(&[1, 2]).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"v_songId": [1, 2]}));
        let _ = client.song().get_cdn_dispatch().await.unwrap();
        assert_eq!(last_req0(&mock)["param"]["use_new_domain"], 1);
        let _ = client.song().has_sheet("m").await.unwrap();
        let body = last_body(&mock);
        assert_eq!(body["comm"]["g_tk"], "5381");
        assert!(body["comm"].get("ct").is_none());
    }

    #[tokio::test]
    async fn sheet_variants() {
        let (client, mock) = mock_client();
        push_cgi_code(&mock, 10007, json!({"result": null, "totalMap": {}}));
        let sheet = client.song().get_sheet("m", 1).await.unwrap();
        assert!(sheet.result.is_empty());
        let req = mock.last_request().unwrap();
        assert!(req.url.ends_with("musicu.fcg"));
        assert_eq!(last_req0(&mock)["param"]["scoreType"], -473);

        push_cgi(&mock, json!({"result": [{"scoreName": "s"}]}));
        let sheet = client.song().get_sheet("m", 2).await.unwrap();
        assert_eq!(sheet.result[0].score_name, "s");
        let req = mock.last_request().unwrap();
        assert!(req.url.ends_with("musics.fcg"));
        assert_eq!(last_body(&mock)["comm"]["platform"], "h5");
        assert_eq!(last_req0(&mock)["param"]["ttype"], 1);
    }

    #[tokio::test]
    async fn related_pagination() {
        let (client, mock) = mock_client();
        push_cgi(&mock, json!({"hasMore": 1, "vecPlaylistNew": [{"playlists": [{"tid": 1}, {"tid": 2}]}]}));
        push_cgi(&mock, json!({"hasMore": 0, "vecPlaylistNew": [{"playlists": [{"tid": 3}]}]}));
        let items = client.song().get_related_songlist(9, vec![]).collect_items(None).await.unwrap();
        assert_eq!(items.iter().map(|p| p.songlist.id).collect::<Vec<_>>(), [1, 2, 3]);
        assert_eq!(last_req0(&mock)["param"]["vecPlaylist"], json!([1, 2]));
        assert_eq!(mock.request_count(), 2);

        push_cgi(&mock, json!({"hasmore": 1, "list": [{"vid": "a", "mvid": 7}]}));
        push_cgi(&mock, json!({"hasmore": 1, "list": []}));
        let pages = client.song().get_related_mv(9, None).collect(Some(5)).await.unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(last_req0(&mock)["param"]["lastmvid"], 7);
        assert_eq!(last_req0(&mock)["param"]["songid"], "9");
    }
}
