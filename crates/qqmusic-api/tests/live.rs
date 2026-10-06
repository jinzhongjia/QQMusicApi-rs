//! Live tests against the real QQ Music API.
//!
//! Ignored by default: `cargo test -p qqmusic-api --test live -- --ignored`.
//! Anonymous requests can be throttled by risk control (`2001`); such
//! responses are reported and tolerated.

use qqmusic_api::error::ApiErrorKind;
use qqmusic_api::modules::comment::{CommentPage, CommentTarget};
use qqmusic_api::modules::lyric::LyricOptions;
use qqmusic_api::modules::song::Quality;
use qqmusic_api::{Client, Error, Result};

const SONG_MID: &str = "0039MnYb0qxYhV"; // 晴天
const SONG_ID: i64 = 97773;
const ALBUM_MID: &str = "000MkMni19ClKG"; // 叶惠美
const SINGER_MID: &str = "0025NhlN2yWrP4"; // 周杰伦

fn client() -> Client {
    let dir = std::env::temp_dir().join("qqmusic-api-live");
    Client::builder().device_path(dir.join("device.json")).build().expect("client")
}

fn check<T: std::fmt::Debug>(name: &str, result: Result<T>, verify: impl FnOnce(&T)) {
    match result {
        Ok(value) => {
            verify(&value);
            println!("{name}: ok");
        }
        Err(Error::Api(err)) if matches!(err.kind, ApiErrorKind::Ratelimited { .. }) => {
            println!("{name}: throttled by risk control ({})", err.code);
        }
        Err(err) => panic!("{name}: {err:?}"),
    }
}

#[tokio::test]
#[ignore = "requires network"]
async fn live_public_endpoints() {
    let client = client();
    check("hotkey", client.search().get_hotkey().await, |r| assert!(!r.vec_hotkey.is_empty()));
    check("song detail", client.song().get_detail(SONG_MID).await, |r| assert_eq!(r.track.mid, SONG_MID));
    check("lyric", client.lyric().get_lyric(SONG_MID).await, |r| assert!(!r.lyric.is_empty()));
    check("album", client.album().get_detail(ALBUM_MID).await, |r| assert_eq!(r.album.album.mid, ALBUM_MID));
    check("singer", client.singer().get_info(SINGER_MID).await, |r| assert_eq!(r.singer.mid, SINGER_MID));
    check("top category", client.top().get_category().await, |r| assert!(!r.group.is_empty()));
    check(
        "comments",
        client.comment().get_hot_comments(CommentTarget::song(SONG_ID), CommentPage::default()).await,
        |r| assert!(!r.comments.is_empty()),
    );
}

#[tokio::test]
#[ignore = "requires network"]
async fn live_qrc_lyric_is_decrypted() {
    let client = client();
    let options = LyricOptions { qrc: true, trans: true, ..LyricOptions::default() };
    check("qrc lyric", client.lyric().get_lyric_with(SONG_MID, options).await, |r| {
        assert!(
            r.lyric.contains("<QrcInfos>") || r.lyric.contains("LyricContent"),
            "{}",
            &r.lyric[..80.min(r.lyric.len())]
        );
    });
}

#[tokio::test]
#[ignore = "requires network"]
async fn live_playable_url_with_bypass() {
    // Without a VIP credential the result may legitimately be `None`; the
    // request itself must succeed. Set QQMUSIC_CREDENTIAL to a credential
    // JSON file to test VIP qualities.
    let mut builder = Client::builder().device_path(std::env::temp_dir().join("qqmusic-api-live/device.json"));
    if let Some(text) = std::env::var("QQMUSIC_CREDENTIAL").ok().and_then(|p| std::fs::read_to_string(p).ok()) {
        builder = builder.credential(qqmusic_api::Credential::from_json_str(&text).expect("credential json"));
    }
    let client = builder.build().unwrap();
    check("playable url", client.song().playable_url(SONG_MID, None, Quality::Lossless).await, |r| {
        if let Some(url) = r {
            assert!(url.url.starts_with("http"), "{}", url.url);
            println!("  -> {:?} {}", url.quality, url.filename);
        } else {
            println!("  -> no playable url for this account");
        }
    });
}

/// QIMEI registration exercises the RSA-encrypted key exchange end to end:
/// the service can only answer if it decrypted our PKCS#1 v1.5 ciphertext.
#[cfg(feature = "reqwest-transport")]
#[tokio::test]
#[ignore = "requires network"]
async fn live_qimei_registration() {
    use std::sync::Arc;

    use qqmusic_api::device::DeviceStore;
    use qqmusic_api::qimei::QimeiProvider;
    use qqmusic_api::transport::{ReqwestTransport, TransportConfig};
    use qqmusic_api::versioning::VersionProfile;

    let transport = Arc::new(ReqwestTransport::new(&TransportConfig::default()).expect("transport"));
    let provider = QimeiProvider::new(Arc::new(DeviceStore::ephemeral()), VersionProfile::android(), transport);
    let qimei = provider.get().await.expect("qimei");
    assert_eq!(qimei.q36.len(), 36, "{qimei:?}");
    println!("qimei: ok ({}…)", &qimei.q36[..8]);
}
