//! Shared models (`Singer`, `Album`, `Song`, ...).

use serde::Serialize;

use crate::FromJson;

/// Supported cover sizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
pub enum CoverSize {
    /// 150x150
    S150,
    /// 300x300
    #[default]
    S300,
    /// 500x500
    S500,
    /// 800x800
    S800,
    /// 1200x1200
    S1200,
    /// 1500x1500
    S1500,
}

impl CoverSize {
    /// Pixel size.
    pub fn pixels(self) -> u32 {
        match self {
            Self::S150 => 150,
            Self::S300 => 300,
            Self::S500 => 500,
            Self::S800 => 800,
            Self::S1200 => 1200,
            Self::S1500 => 1500,
        }
    }

    /// Size from pixels.
    pub fn from_pixels(pixels: u32) -> Option<Self> {
        Some(match pixels {
            150 => Self::S150,
            300 => Self::S300,
            500 => Self::S500,
            800 => Self::S800,
            1200 => Self::S1200,
            1500 => Self::S1500,
            _ => return None,
        })
    }
}

/// `https://y.gtimg.cn/music/photo_new/{kind}R{n}x{n}M000{mid}.jpg`
pub fn photo_new_cover_url(kind: &str, mid: &str, size: CoverSize) -> String {
    let mid = mid.trim();
    if mid.is_empty() {
        return String::new();
    }
    let n = size.pixels();
    format!("https://y.gtimg.cn/music/photo_new/{kind}R{n}x{n}M000{mid}.jpg")
}

/// Singer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct Singer {
    /// Singer id.
    #[json(alias("id", "singerID", "singerId", "SingerID", "singer_id"))]
    pub id: i64,
    /// Singer mid.
    #[json(alias("mid", "singerMid", "singerMID", "SingerMid", "singer_mid"))]
    pub mid: String,
    /// Name.
    #[json(alias("name", "singerName", "singer_name"))]
    pub name: String,
    /// Title.
    #[json(alias("title", "singerName", "name"))]
    pub title: String,
    /// Type.
    #[json(alias("type", "SingerType", "vt"))]
    pub r#type: i64,
    /// Uin.
    pub uin: i64,
    /// Picture mid.
    #[json(alias("pmid", "singerPmid", "singer_pmid", "pic_mid"))]
    pub pmid: String,
}

impl Singer {
    /// Cover URL.
    pub fn cover_url(&self, size: CoverSize) -> String {
        let mid = if self.mid.is_empty() { &self.pmid } else { &self.mid };
        photo_new_cover_url("T001", mid, size)
    }
}

/// Album.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct Album {
    /// Album id.
    #[json(alias("id", "albumID"))]
    pub id: i64,
    /// Album mid.
    #[json(alias("mid", "albumMid", "albumMID", "albummid"))]
    pub mid: String,
    /// Name.
    #[json(alias("name", "albumName"))]
    pub name: String,
    /// Title.
    #[json(alias("title", "albumName", "name"))]
    pub title: String,
    /// Subtitle.
    #[json(alias("subtitle", "albumTranName"))]
    pub subtitle: String,
    /// Publish date.
    #[json(alias("time_public", "publish_date", "publishDate"))]
    pub time_public: String,
    /// Picture mid.
    #[json(alias("pmid", "logo"))]
    pub pmid: String,
}

impl Album {
    /// Cover URL.
    pub fn cover_url(&self, size: CoverSize) -> String {
        let mid = if self.mid.is_empty() { &self.pmid } else { &self.mid };
        photo_new_cover_url("T002", mid, size)
    }
}

/// Audio file sizes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct File {
    /// Media mid.
    pub media_mid: String,
    /// AAC 24k.
    pub size_24aac: i64,
    /// AAC 48k.
    pub size_48aac: i64,
    /// AAC 96k.
    pub size_96aac: i64,
    /// OGG 192k.
    pub size_192ogg: i64,
    /// AAC 192k.
    pub size_192aac: i64,
    /// MP3 128k.
    pub size_128mp3: i64,
    /// MP3 320k.
    pub size_320mp3: i64,
    /// FLAC.
    pub size_flac: i64,
    /// DTS.
    pub size_dts: i64,
    /// Preview.
    pub size_try: i64,
    /// Preview start.
    pub try_begin: i64,
    /// Preview end.
    pub try_end: i64,
    /// OGG 96k.
    pub size_96ogg: i64,
    /// Dolby.
    pub size_dolby: i64,
    /// New size list.
    pub size_new: Vec<i64>,
}

/// Pay information.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct Pay {
    /// Requires VIP.
    pub pay_month: i64,
    /// Track price.
    pub price_track: i64,
    /// Album price.
    pub price_album: i64,
    /// Pay to play.
    pub pay_play: i64,
    /// Pay to download.
    pub pay_down: i64,
    /// Pay status.
    pub pay_status: i64,
    /// Free time.
    pub time_free: i64,
}

/// MV reference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct Mv {
    /// Id.
    #[json(alias("id", "sid", "mvid", "singerId"))]
    pub id: i64,
    /// Vid.
    pub vid: String,
    /// Type.
    #[json(alias("vt", "type"))]
    pub r#type: i64,
    /// Name.
    #[json(alias("name", "mvname", "title"))]
    pub name: String,
    /// Title.
    #[json(alias("title", "title_main", "name"))]
    pub title: String,
}

/// Song list (playlist) reference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SongList {
    /// Id.
    #[json(alias("id", "tid", "dissid"))]
    pub id: i64,
    /// Dir id.
    #[json(alias("dirid", "dirId"))]
    pub dirid: i64,
    /// Title.
    #[json(alias("title", "dissname", "name", "dirName"))]
    pub title: String,
    /// Cover URL.
    #[json(alias("picurl", "cover", "logo", "picUrl"))]
    pub picurl: String,
    /// Description.
    #[json(alias("desc", "description"))]
    pub desc: String,
    /// Number of songs.
    #[json(alias("songnum", "songNum", "song_cnt"))]
    pub songnum: i64,
    /// Play count.
    #[json(alias("listennum", "playCnt", "play_cnt"))]
    pub listennum: i64,
}

/// Song.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct Song {
    /// Song id.
    #[json(required)]
    pub id: i64,
    /// Song mid.
    #[json(required)]
    pub mid: String,
    /// Name.
    pub name: String,
    /// Type.
    pub r#type: i64,
    /// Title.
    pub title: String,
    /// Subtitle.
    pub subtitle: String,
    /// Singers.
    pub singer: Vec<Singer>,
    /// Album.
    pub album: Album,
    /// MV.
    pub mv: Mv,
    /// File sizes.
    pub file: File,
    /// Pay info.
    pub pay: Pay,
    /// Duration in seconds.
    pub interval: i64,
    /// Exclusive flag.
    pub isonly: i64,
    /// Language.
    pub language: i64,
    /// Genre.
    pub genre: i64,
    /// CD index.
    pub index_cd: i64,
    /// Album index.
    pub index_album: i64,
    /// Publish date.
    pub time_public: String,
    /// Status.
    pub status: i64,
    /// Label.
    pub label: String,
    /// BPM.
    pub bpm: i64,
    /// ov.
    pub ov: i64,
    /// sa.
    pub sa: i64,
    /// es.
    pub es: String,
    /// vs.
    pub vs: Vec<String>,
    /// vi.
    pub vi: Vec<i64>,
    /// vf.
    pub vf: Vec<f64>,
}

impl Song {
    /// Cover URL (album cover, falling back to the first singer).
    pub fn cover_url(&self, size: CoverSize) -> String {
        if !self.album.mid.is_empty() || !self.album.pmid.is_empty() {
            return self.album.cover_url(size);
        }
        self.singer
            .iter()
            .find(|s| !s.mid.is_empty() || !s.pmid.is_empty())
            .map(|s| s.cover_url(size))
            .unwrap_or_default()
    }

    /// Whether a VIP subscription is required.
    pub fn requires_vip(&self) -> bool {
        self.pay.pay_play != 0 || self.pay.pay_month != 0
    }

    /// Singer names joined with `/`.
    pub fn singer_names(&self) -> String {
        self.singer.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("/")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn singer_aliases_and_cover() {
        let singer: Singer =
            from_value(&json!({"singerID": 4558, "singerMid": "0025NhlN2yWrP4", "singerName": "周杰伦"})).unwrap();
        assert_eq!(singer.id, 4558);
        assert_eq!(singer.name, "周杰伦");
        assert_eq!(singer.title, "周杰伦");
        assert_eq!(
            singer.cover_url(CoverSize::S300),
            "https://y.gtimg.cn/music/photo_new/T001R300x300M0000025NhlN2yWrP4.jpg"
        );
        let pmid_only = Singer { pmid: " p ".into(), ..Singer::default() };
        assert!(pmid_only.cover_url(CoverSize::S150).ends_with("R150x150M000p.jpg"));
        assert_eq!(Singer::default().cover_url(CoverSize::S300), "");
    }

    #[test]
    fn album_and_song() {
        let song: Song = from_value(&json!({
            "id": 1, "mid": "m", "name": "晴天", "type": 0,
            "singer": [{"id": 2, "mid": "", "name": "a"}, {"id": 3, "mid": "s3", "name": "b"}],
            "album": {"albumMid": "", "logo": ""},
            "pay": {"pay_play": 1},
            "file": {"media_mid": "mm", "size_flac": "123", "size_new": [1, 2]},
            "vf": [1.5, 2],
            "unknown": {"ignored": true}
        }))
        .unwrap();
        assert_eq!(song.file.size_flac, 123);
        assert_eq!(song.vf, vec![1.5, 2.0]);
        assert!(song.requires_vip());
        assert_eq!(song.singer_names(), "a/b");
        assert!(song.cover_url(CoverSize::S500).contains("T001R500x500M000s3"));
        let mut with_album = song.clone();
        with_album.album.mid = "am".into();
        assert!(with_album.cover_url(CoverSize::S800).contains("T002R800x800M000am"));
        assert!(from_value::<Song>(&json!({"mid": "x"})).is_err(), "id is required");
        let empty = Song { id: 1, mid: "m".into(), ..Song::default() };
        assert_eq!(empty.cover_url(CoverSize::S300), "");
    }

    #[test]
    fn misc_models() {
        let mv: Mv = from_value(&json!({"mvid": 9, "vid": "v", "vt": 2, "mvname": "n", "title_main": "t"})).unwrap();
        assert_eq!((mv.id, mv.r#type, mv.name.as_str(), mv.title.as_str()), (9, 2, "n", "t"));
        let list: SongList = from_value(&json!({"dissid": "7", "dissname": "x", "song_cnt": 3, "playCnt": 4})).unwrap();
        assert_eq!((list.id, list.songnum, list.listennum), (7, 3, 4));
        assert_eq!(CoverSize::from_pixels(1200), Some(CoverSize::S1200));
        assert_eq!(CoverSize::from_pixels(1), None);
        assert_eq!(CoverSize::default().pixels(), 300);
    }
}
