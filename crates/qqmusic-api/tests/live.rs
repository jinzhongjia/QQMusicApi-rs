//! Live tests against the real QQ Music API.
//!
//! Ignored by default: `cargo test -p qqmusic-api --test live -- --ignored`.
//! Anonymous requests can be throttled by risk control (`2001`); such
//! responses are reported and tolerated.

use qqmusic_api::error::ApiErrorKind;
use qqmusic_api::modules::comment::{CommentPage, CommentTarget};
use qqmusic_api::modules::lyric::LyricOptions;
use qqmusic_api::modules::song::Quality;
use std::future::IntoFuture;

use qqmusic_api::{Client, Error, Result};

const SONG_MID: &str = "0039MnYb0qxYhV"; // 晴天
const SONG_ID: i64 = 97773;
const ALBUM_MID: &str = "000MkMni19ClKG"; // 叶惠美
const SINGER_MID: &str = "0025NhlN2yWrP4"; // 周杰伦
const SONGLIST_ID: i64 = 3805603854; // public official playlist
const TOP_ID: i64 = 26; // 热歌榜
const LYRIC_EXTRAS_SONG_ID: i64 = 200369396; // Shape of You

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

/// Like [`check`], but records failures so one run reports every endpoint.
#[derive(Default)]
struct SoftCheck(Vec<String>);

impl SoftCheck {
    fn check<T: std::fmt::Debug>(&mut self, name: &str, result: Result<T>, verify: impl FnOnce(&T)) {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(name, result, verify)));
        if let Err(panic) = outcome {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_default();
            self.0.push(if msg.starts_with(name) { msg } else { format!("{name}: {msg}") });
        }
    }

    fn finish(self) {
        assert!(self.0.is_empty(), "{} endpoint(s) failed:\n{}", self.0.len(), self.0.join("\n"));
    }
}

/// Client logged in with the credential JSON at `QQMUSIC_CREDENTIAL`
/// (e.g. `.qqmusic/credential.json` from the `qrcode_login` example).
fn auth_client() -> Option<(Client, qqmusic_api::Credential)> {
    let Some(path) = std::env::var_os("QQMUSIC_CREDENTIAL") else {
        println!("QQMUSIC_CREDENTIAL not set; skipping");
        return None;
    };
    let text = std::fs::read_to_string(&path).expect("read credential");
    let credential = qqmusic_api::Credential::from_json_str(&text).expect("credential json");
    let dir = std::env::temp_dir().join("qqmusic-api-live");
    let client =
        Client::builder().device_path(dir.join("device.json")).credential(credential.clone()).build().expect("client");
    Some((client, credential))
}

/// QQ-friend features answer 101010 for WeChat logins; treat that as "none".
fn friends_ok<T>(cred: &qqmusic_api::Credential, result: Result<T>) -> Result<Option<T>> {
    match result {
        Err(Error::Api(err)) if cred.login_type == 1 && err.code == 101_010 => {
            println!("  (no QQ friends for a WeChat login: 101010)");
            Ok(None)
        }
        other => other.map(Some),
    }
}

#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_read_only() {
    use qqmusic_api::modules::private_message::SessionListOptions;
    use qqmusic_api::modules::user::DislikeListKind;

    let Some((client, cred)) = auth_client() else { return };
    let euin = cred.encrypt_uin.as_str();
    let mut soft = SoftCheck::default();

    soft.check("check_expired", client.login().check_expired(None).await, |r| assert!(!r, "credential expired"));
    soft.check("vip info", client.user().get_vip_info(None).await, |_| {});
    soft.check("homepage", client.user().get_homepage(euin, None).await, |r| {
        assert!(!r.base_info.name.is_empty(), "{r:?}");
    });
    soft.check("follow singers", client.user().get_follow_singers(euin, 1, 10, None).await, |_| {});
    soft.check("fans", client.user().get_fans(euin, 1, 10, None).await, |_| {});
    soft.check("follow users", client.user().get_follow_user(euin, 1, 10, None).await, |_| {});
    soft.check("friends", friends_ok(&cred, client.user().get_friend(1, 10, None).await), |_| {});
    soft.check("created songlist", client.user().get_created_songlist(cred.musicid, None).await, |r| {
        assert!(!r.playlists.is_empty(), "every account has the \"I like\" playlist");
    });
    soft.check("fav songs", client.user().get_fav_song(euin, 1, 10, None).await, |_| {});
    soft.check("fav songlists", client.user().get_fav_songlist(euin, 1, 10, None).await, |_| {});
    soft.check("fav albums", client.user().get_fav_album(euin, 1, 10, None).await, |_| {});
    soft.check("fav mvs", client.user().get_fav_mv(euin, 1, 10, None).await, |_| {});
    soft.check("music gene", client.user().get_music_gene(euin, None).await, |_| {});
    soft.check("dislike songs", client.user().get_dislike_list(DislikeListKind::Songs, 1, 0, None).await, |r| {
        assert_eq!(r.retcode, 0, "{r:?}");
    });
    soft.check("guess recommend", client.recommend().get_guess_recommend(None).await, |_| {});
    let sound_power = client.sound_power().get_detail(None).await;
    let hugevip = sound_power.as_ref().is_ok_and(|r| r.is_hugevip != 0);
    soft.check("sound power", sound_power, |_| {});
    soft.check("hugevip rule", client.sound_power().get_hugevip_rule(None).await, |r| {
        assert!(!r.rules.is_empty() && (!hugevip || r.current_level > 0), "{r:?}");
    });
    soft.check(
        "friend rank",
        friends_ok(&cred, client.sound_power().get_friend_rank(0, 10, "", 0, None).await),
        |_| {},
    );
    soft.check("medal entry", client.sound_power().get_medal_entry(euin, None).await, |_| {});
    soft.check("tasks", client.sound_power().get_tasks(None, None, None).await, |_| {});
    soft.check("pm sessions", client.private_message().get_sessions(SessionListOptions::default(), None).await, |_| {});
    soft.check("pm safety hint", client.private_message().get_safety_hint(euin, 0, None).await, |_| {});
    // 晴天 is a VIP song; other accounts (and overseas IPs) get no URL.
    soft.check("vip song url", client.song().playable_url(SONG_MID, None, Quality::Lossless).await, |r| match r {
        Some(url) => println!("  -> {:?} {}", url.quality, url.filename),
        None => assert!(!hugevip, "a huge-VIP account should get a URL"),
    });
    soft.finish();
}

/// Create a playlist, add and remove a song, delete it.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_songlist_roundtrip() {
    let Some((client, _)) = auth_client() else { return };
    let songlist = client.songlist();
    let name = format!("qqmusic-api-live-{}", std::process::id());
    let created = songlist.create(&name, None).await.expect("create");
    assert_eq!(created.ret_code, 0, "{created:?}");
    assert!(created.dirid > 0, "{created:?}");

    let result = async {
        assert!(songlist.add_songs(created.dirid, &[(SONG_ID, 0)], 0, None).await?, "add_songs");
        let detail = songlist.get_detail(created.id).await?;
        assert!(detail.songs.iter().any(|s| s.id == SONG_ID), "song not in playlist: {detail:?}");
        assert!(songlist.del_songs(created.dirid, &[(SONG_ID, 0)], 0, None).await?, "del_songs");
        Ok::<_, Error>(())
    }
    .await;

    let deleted = songlist.delete(created.dirid, None).await.expect("delete");
    assert_eq!(deleted.ret_code, 0, "{deleted:?}");
    result.expect("songlist ops");
}

/// Revert a write made by a test, retrying so the account is not left
/// modified by a transient failure (back-to-back writes can trip risk control).
async fn undo<T, F: std::future::Future<Output = Result<T>>>(name: &str, mut op: impl FnMut() -> F) -> T {
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        match op().await {
            Ok(value) => return value,
            Err(err) => {
                println!("{name}: attempt {} failed: {err:?}", attempt + 1);
                last = Some(err);
            }
        }
    }
    panic!("{name}: could not revert, fix the account by hand: {last:?}");
}

/// Like/unlike a song and favourite/unfavourite an album. Items already in
/// the account are left alone so the test never removes user data.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_fav_roundtrip() {
    let Some((client, cred)) = auth_client() else { return };

    if client.songlist().like_song(&[(SONG_ID, 0)], None).await.expect("like_song") {
        let unliked =
            undo("unlike_song", || async { client.songlist().unlike_song(&[(SONG_ID, 0)], None).await }).await;
        assert!(unliked);
        println!("like/unlike: ok");
    } else {
        println!("like/unlike: song already liked, skipped");
    }

    let album_id = client.album().get_detail(ALBUM_MID).await.expect("album").album.album.id;
    let favs = client.user().get_fav_album(&cred.encrypt_uin, 1, 100, None).await.expect("fav albums");
    if favs.albums.iter().any(|a| a.album.id == album_id) {
        println!("fav/unfav album: already favourited, skipped");
        return;
    }
    let fav = client.album().fav_album(&[album_id], None).await.expect("fav_album");
    assert!(fav.failed_album_id.is_empty(), "{fav:?}");
    let favs = client.user().get_fav_album(&cred.encrypt_uin, 1, 100, None).await.expect("fav albums");
    let listed = favs.albums.iter().any(|a| a.album.id == album_id);
    let unfav = undo("del_fav_album", || async { client.album().del_fav_album(&[album_id], None).await }).await;
    assert!(unfav.failed_album_id.is_empty(), "{unfav:?}");
    assert!(listed, "favourited album not listed");
    println!("fav/unfav album: ok");
}

/// Post a comment on a song and delete it again.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_comment_roundtrip() {
    let Some((client, _)) = auth_client() else { return };
    let content = format!("qqmusic-api live test {}", std::process::id());
    let added = client.comment().add_comment(CommentTarget::song(SONG_ID), &content, None, None).await.expect("add");
    assert!(!added.id.is_empty(), "{added:?}");
    let deleted = undo("delete_comment", || async { client.comment().delete_comment(&added.id, None).await }).await;
    assert!(deleted, "delete_comment");
}

async fn dislike_ids(client: &Client, kind: qqmusic_api::modules::user::DislikeListKind) -> Vec<i64> {
    let pages = client.user().get_dislike_list(kind, 1, 0, None).collect(Some(100)).await.expect("dislike list");
    let mut ids: Vec<i64> = pages
        .iter()
        .flat_map(|p| p.songs.iter().chain(&p.singers).chain(&p.styles))
        .map(|item| item.id.parse().expect("numeric dislike id"))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// Add/cancel a disliked song, then clear all disliked songs and restore
/// them from a snapshot (saved to the temp dir first in case restoring fails).
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_dislike_roundtrip() {
    use qqmusic_api::modules::user::{DislikeListKind, DislikeType};

    let Some((client, _)) = auth_client() else { return };
    let user = client.user();
    let songs = dislike_ids(&client, DislikeListKind::Songs).await;
    let singers = dislike_ids(&client, DislikeListKind::Singers).await;

    if songs.contains(&SONG_ID) {
        println!("add/cancel dislike: already disliked, skipped");
    } else {
        assert!(user.add_dislike(DislikeType::Song, &[SONG_ID], None).await.expect("add_dislike"));
        let listed = dislike_ids(&client, DislikeListKind::Songs).await.contains(&SONG_ID);
        let cancelled = undo("cancel_dislike", || user.cancel_dislike(DislikeType::Song, &[SONG_ID], None)).await;
        assert!(cancelled && listed, "listed={listed} cancelled={cancelled}");
        println!("add/cancel dislike: ok");
    }

    let backup = std::env::temp_dir().join("qqmusic-api-live/dislike-songs.json");
    std::fs::create_dir_all(backup.parent().unwrap()).unwrap();
    std::fs::write(&backup, serde_json::to_string(&songs).unwrap()).unwrap();
    if songs.is_empty() {
        user.add_dislike(DislikeType::Song, &[SONG_ID], None).await.expect("seed dislike");
    }
    assert!(user.cancel_all_dislike_song(None).await.expect("cancel_all_dislike_song"));
    let after_clear = dislike_ids(&client, DislikeListKind::Songs).await;
    for chunk in songs.chunks(50) {
        let restored = undo("restore dislikes", || user.add_dislike(DislikeType::Song, chunk, None)).await;
        assert!(restored, "restore failed; snapshot at {}", backup.display());
    }
    assert!(after_clear.is_empty(), "not cleared: {after_clear:?}");
    let restored = dislike_ids(&client, DislikeListKind::Songs).await;
    assert_eq!(restored, songs, "restore mismatch; snapshot at {}", backup.display());
    assert_eq!(dislike_ids(&client, DislikeListKind::Singers).await, singers, "singers must be untouched");
    println!("cancel all + restore: ok ({} songs)", songs.len());
}

/// Favourite someone else's playlist and remove it again.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_fav_songlist_roundtrip() {
    let Some((client, cred)) = auth_client() else { return };
    let favs: Vec<i64> = client
        .user()
        .get_fav_songlist(&cred.encrypt_uin, 1, 100, None)
        .collect(Some(20))
        .await
        .expect("fav songlists")
        .iter()
        .flat_map(|p| p.playlists.iter().map(|s| s.songlist.id))
        .collect();
    let candidates = client.recommend().get_recommend_songlist(1, 20).await.expect("recommend songlists");
    let id = candidates.songlists.iter().map(|s| s.id).find(|id| *id > 0 && !favs.contains(id)).expect("candidate");

    assert!(client.user().fav_songlist(id, None).await.expect("fav_songlist"));
    let unfav = undo("unfav_songlist", || async { client.user().unfav_songlist(id, None).await }).await;
    assert!(unfav, "unfav_songlist");
}

/// Toggle friend-rank privacy and like/unlike a friend in the ranking.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_friend_rank_writes() {
    let Some((client, cred)) = auth_client() else { return };
    let sp = client.sound_power();
    let rank = sp.get_friend_rank(0, 50, "", 0, None).await.expect("rank");

    if rank.my_rank_no > 0 {
        sp.set_rank_privacy(1, None).await.expect("hide");
        let hidden = sp.get_friend_rank(0, 50, "", 0, None).await.expect("rank").my_rank_no;
        undo("unhide", || sp.set_rank_privacy(2, None).into_future()).await;
        let shown = sp.get_friend_rank(0, 50, "", 0, None).await.expect("rank").my_rank_no;
        println!("privacy: rank {} -> hidden {hidden} -> shown {shown}", rank.my_rank_no);
        assert!(shown > 0, "privacy not restored");
    } else {
        println!("privacy: not ranked (hidden already?), skipped");
    }

    let own = cred.musicid.to_string();
    let Some(friend) = rank.ranks.iter().find(|r| r.like_status == 0 && r.uin != own) else {
        println!("like friend: no unliked friend, skipped");
        return;
    };
    // The ranking reflects likes after a short delay.
    let settle = || tokio::time::sleep(std::time::Duration::from_secs(2));
    sp.like_friend(&friend.uin, false, None).await.expect("like");
    settle().await;
    let liked = sp.get_friend_rank(0, 50, "", 0, None).await.expect("rank");
    undo("unlike friend", || sp.like_friend(&friend.uin, true, None).into_future()).await;
    settle().await;
    let status = |r: &qqmusic_api::models::sound_power::FriendRankResponse| {
        r.ranks.iter().find(|x| x.uin == friend.uin).map(|x| x.like_status)
    };
    let after = sp.get_friend_rank(0, 50, "", 0, None).await.expect("rank");
    println!("like friend: {:?} -> {:?}", status(&liked), status(&after));
    assert_eq!(status(&liked), Some(1));
    assert_eq!(status(&after), Some(0));
}

/// Upload a 16x16 PNG as a playlist cover object (not attached to anything).
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_upload() {
    use qqmusic_api::modules::helper::UploadBusiness;

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00,
        0x00, 0x10, 0x00, 0x00, 0x00, 0x10, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x91, 0x68, 0x36, 0x00, 0x00, 0x00,
        0x16, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x90, 0x9b, 0xf0, 0x9f, 0x24, 0xc4, 0x30, 0xaa, 0x61, 0x54,
        0xc3, 0xf0, 0xd5, 0x00, 0x00, 0x30, 0x80, 0xad, 0x10, 0x47, 0x9b, 0x37, 0xa9, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    let Some((client, _)) = auth_client() else { return };
    let path = std::env::temp_dir().join("qqmusic-api-live/cover.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, PNG).unwrap();
    let objects = client.helper().upload_session(UploadBusiness::Songlist).upload(&[&path]).await.expect("upload");
    assert_eq!(objects.len(), 1);
    assert!(objects[0].url.url.starts_with("http"), "{objects:?}");
    println!("upload: {}", objects[0].url.url);
}

/// Refresh the musickey and write the new credential back to
/// `QQMUSIC_CREDENTIAL` so later runs keep working.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_refresh_credential() {
    let Some((client, old)) = auth_client() else { return };
    let refreshed = client.login().refresh_credential(None).await.expect("refresh");
    assert!(!refreshed.musickey.is_empty());
    assert_eq!(refreshed.musicid, old.musicid);
    assert_eq!(refreshed.unionid, old.unionid, "fields missing from the response must be kept");
    std::fs::write(std::env::var_os("QQMUSIC_CREDENTIAL").unwrap(), refreshed.to_json_string()).unwrap();
    assert!(!client.login().check_expired(Some(refreshed.clone())).await.expect("check new"));
    println!("refresh: key changed = {}", refreshed.musickey != old.musickey);
}

/// Log out a throwaway credential (`QQMUSIC_LOGOUT_CREDENTIAL`).
///
/// Observed: `Logout` succeeds but does not revoke the musickey (it stays
/// usable afterwards), so validity is only reported. Note that a fresh QR
/// login for the same account does expire the previous session, so obtaining
/// the throwaway credential invalidates the main one.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_LOGOUT_CREDENTIAL"]
async fn live_auth_logout() {
    let Some(path) = std::env::var_os("QQMUSIC_LOGOUT_CREDENTIAL") else {
        println!("QQMUSIC_LOGOUT_CREDENTIAL not set; skipping");
        return;
    };
    let credential = qqmusic_api::Credential::from_json_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let client = client();
    assert!(!client.login().check_expired(Some(credential.clone())).await.expect("check before"));
    client.login().logout(Some(credential.clone())).await.expect("logout");
    let expired = client.login().check_expired(Some(credential)).await.expect("check after");
    println!("logout: ok (credential expired afterwards: {expired})");
}

/// Fetch two pages and require both to be non-empty and different.
///
/// Retried once: some feeds (e.g. the home feed) occasionally answer a page
/// with nothing.
async fn check_two_pages<T: qqmusic_api::FromJson + std::fmt::Debug + Send + 'static>(
    soft: &mut SoftCheck,
    name: &str,
    paged: qqmusic_api::pagination::Paged<T>,
    items: impl Fn(&T) -> Vec<String>,
) {
    let verify = |pages: &Vec<T>| -> std::result::Result<(), String> {
        if pages.len() != 2 {
            return Err(format!("expected two pages, got {}", pages.len()));
        }
        let (first, second) = (items(&pages[0]), items(&pages[1]));
        if first.is_empty() || second.is_empty() {
            return Err(format!("empty page: {} / {}", first.len(), second.len()));
        }
        if first == second {
            return Err("second page repeats the first".into());
        }
        Ok(())
    };
    let mut result = paged.clone().collect(Some(2)).await;
    if result.as_ref().map_or(true, |pages| verify(pages).is_err()) {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        result = paged.collect(Some(2)).await;
    }
    soft.check(name, result, |pages| {
        if let Err(msg) = verify(pages) {
            panic!("{msg}");
        }
    });
}

/// Every public (no login) endpoint not covered by `live_public_endpoints`.
#[tokio::test]
#[ignore = "requires network"]
async fn live_public_endpoints_full() {
    use qqmusic_api::modules::search::{SearchOptions, SearchType};
    use qqmusic_api::modules::singer::{AlbumFilterType, IndexType, OrderType, SingerFilter, TabType};
    use qqmusic_api::modules::song::{Quality, SongFileType};
    use qqmusic_api::modules::songlist::SonglistDetailOptions;

    let client = client();
    let mut soft = SoftCheck::default();
    let dbg = |v: &dyn std::fmt::Debug| format!("{v:?}");

    // search
    soft.check("search complete", client.search().complete("周杰伦").await, |r| {
        assert!(!r.items.is_empty(), "{r:?}")
    });
    soft.check("quick search", client.search().quick_search("晴天").await, |r| {
        assert!(!r.song.itemlist.is_empty(), "{r:?}");
    });
    soft.check("general search", client.search().general_search("周杰伦", 1, 10).await, |r| {
        assert!(!r.song.items.is_empty() || !r.singer.items.is_empty(), "{r:?}");
    });
    check_two_pages(&mut soft, "general search pages", client.search().general_search("周杰伦", 1, 10), |r| {
        r.song.items.iter().map(|s| dbg(s)).collect()
    })
    .await;
    for (name, ty) in [
        ("song", SearchType::Song),
        ("singer", SearchType::Singer),
        ("album", SearchType::Album),
        ("songlist", SearchType::SongList),
        ("mv", SearchType::Mv),
        ("lyric", SearchType::Lyric),
    ] {
        soft.check(&format!("search type {name}"), client.search().search_by_type("周杰伦", ty).await, |r| {
            let n = r.song.len() + r.singer.len() + r.album.len() + r.songlist.len() + r.mv.len();
            assert!(n > 0, "{r:?}");
        });
    }
    let options = SearchOptions { search_type: SearchType::Song, num: 10, ..SearchOptions::default() };
    check_two_pages(&mut soft, "search type pages", client.search().search_by_type_with("周杰伦", options), |r| {
        r.song.iter().map(|s| dbg(s)).collect()
    })
    .await;

    // singer
    soft.check("singer list", client.singer().get_singer_list(SingerFilter::default()).await, |r| {
        assert!(!r.singerlist.is_empty() || !r.hotlist.is_empty(), "{r:?}");
    });
    check_two_pages(
        &mut soft,
        "singer index pages",
        client.singer().get_singer_list_index(SingerFilter::default(), IndexType::All, 1, 20),
        |r| r.singerlist.iter().map(|s| dbg(s)).collect(),
    )
    .await;
    for tab in [TabType::Wiki, TabType::Album, TabType::Composer, TabType::Lyricist] {
        let result = client.singer().get_tab_detail(SINGER_MID, tab, 1, 10, OrderType::Hot, None).await;
        soft.check(&format!("singer tab {tab:?}"), result, |r| assert!(!r.tab_id.is_empty(), "{r:?}"));
    }
    soft.check("singer desc", client.singer().get_desc(&[SINGER_MID]).await, |r| assert!(!r.singer_list.is_empty()));
    soft.check("similar singers", client.singer().get_similar(SINGER_MID, 10).await, |r| {
        assert!(!r.singerlist.is_empty(), "{r:?}");
    });
    check_two_pages(
        &mut soft,
        "singer songs",
        client.singer().get_songs_list(SINGER_MID, 10, 1, OrderType::Hot),
        |r| r.song_list.iter().map(|s| s.mid.clone()).collect(),
    )
    .await;
    check_two_pages(
        &mut soft,
        "singer albums",
        client.singer().get_album_list(SINGER_MID, 10, 1, OrderType::Latest, None),
        |r| r.album_list.iter().map(|a| dbg(a)).collect(),
    )
    .await;
    soft.check(
        "singer albums (studio)",
        client.singer().get_album_list(SINGER_MID, 10, 1, OrderType::Latest, Some(AlbumFilterType::Studio)).await,
        |r| assert!(!r.album_list.is_empty(), "{r:?}"),
    );
    soft.check("singer mv tags", client.singer().get_mv_tag(SINGER_MID).await, |r| assert!(!r.tags.is_empty()));
    let mvs = client.singer().get_mv_list(SINGER_MID, 10, 1, OrderType::Hot, None).await;
    let vid = mvs.as_ref().ok().and_then(|r| r.mv_list.first()).map(|v| v.vid.clone()).unwrap_or_default();
    soft.check("singer mvs", mvs, |r| assert!(!r.mv_list.is_empty() && !vid.is_empty(), "{r:?}"));
    check_two_pages(
        &mut soft,
        "singer mv pages",
        client.singer().get_mv_list(SINGER_MID, 10, 1, OrderType::Hot, None),
        |r| r.mv_list.iter().map(|v| v.vid.clone()).collect(),
    )
    .await;

    // mv
    soft.check("mv detail", client.mv().get_detail(&[&vid]).await, |r| assert!(r.data.contains_key(&vid), "{r:?}"));
    soft.check("mv urls", client.mv().get_mv_urls(&[&vid]).await, |r| assert!(r.data.contains_key(&vid), "{r:?}"));
    check_two_pages(&mut soft, "mv list", client.mv().get_mv_list(15, 7, 0, 10, 1), |r| {
        r.items.iter().map(|m| dbg(m)).collect()
    })
    .await;

    // song
    soft.check("query song", client.song().query_song([SONG_MID]).unwrap().await, |r| {
        assert_eq!(r.tracks.first().map(|t| t.mid.as_str()), Some(SONG_MID));
    });
    soft.check("cdn dispatch", client.song().get_cdn_dispatch().await, |r| assert!(!r.sip.is_empty(), "{r:?}"));
    soft.check("song urls", client.song().get_song_urls([SONG_MID], SongFileType::MP3_128, None).unwrap().await, |r| {
        assert_eq!(r.data.len(), 1, "{r:?}");
    });
    soft.check(
        "playable url ladder",
        client.song().playable_url_with_ladder(SONG_MID, None, &[(Quality::Standard, SongFileType::MP3_128)]).await,
        |_| {},
    );
    soft.check("similar songs", client.song().get_similar_song(SONG_ID).await, |r| assert!(!r.song.is_empty()));
    soft.check("song labels", client.song().get_labels(SONG_ID).await, |r| assert!(!r.labels.is_empty(), "{r:?}"));
    check_two_pages(&mut soft, "related songlists", client.song().get_related_songlist(SONG_ID, Vec::new()), |r| {
        r.songlist.iter().map(|s| dbg(s)).collect()
    })
    .await;
    check_two_pages(&mut soft, "related mvs", client.song().get_related_mv(SONG_ID, None), |r| {
        r.mv.iter().map(|m| dbg(m)).collect()
    })
    .await;
    soft.check("other versions", client.song().get_other_version(SONG_MID).await, |r| assert!(!r.data.is_empty()));
    soft.check("producer", client.song().get_producer(SONG_MID).await, |r| assert!(!r.data.is_empty(), "{r:?}"));
    soft.check("has sheet", client.song().has_sheet(SONG_MID).await, |r| assert!(r.has_guitar, "{r:?}"));
    soft.check("sheet", client.song().get_sheet(SONG_MID, 1).await, |r| assert!(!r.result.is_empty(), "{r:?}"));
    soft.check("fav num", client.song().get_fav_num(&[SONG_ID]).await, |r| {
        assert!(r.numbers.contains_key(&SONG_ID.to_string()), "{r:?}");
    });

    // album / songlist / top
    check_two_pages(&mut soft, "album songs", client.album().get_song(ALBUM_MID, 5, 1), |r| {
        r.song_list.iter().map(|s| s.mid.clone()).collect()
    })
    .await;
    check_two_pages(&mut soft, "new albums", client.album().get_new_album(1, 10, 1), |r| {
        r.albums.iter().map(|a| dbg(a)).collect()
    })
    .await;
    let options = SonglistDetailOptions { num: 5, onlysong: true, ..SonglistDetailOptions::default() };
    check_two_pages(&mut soft, "songlist detail pages", client.songlist().get_detail_with(SONGLIST_ID, options), |r| {
        r.songs.iter().map(|s| s.mid.clone()).collect()
    })
    .await;
    check_two_pages(&mut soft, "top detail", client.top().get_detail(TOP_ID, 10, 1, true), |r| {
        r.songs.iter().map(|s| s.mid.clone()).collect()
    })
    .await;

    // comments
    let target = CommentTarget::song(SONG_ID);
    soft.check("comment count", client.comment().get_comment_count(target).await, |r| assert!(r.count > 0));
    check_two_pages(
        &mut soft,
        "hot comments",
        client.comment().get_hot_comments(target, CommentPage::default()),
        |r| r.comments.iter().map(|c| dbg(c)).collect(),
    )
    .await;
    check_two_pages(
        &mut soft,
        "new comments",
        client.comment().get_new_comments(target, CommentPage::default()),
        |r| r.comments.iter().map(|c| dbg(c)).collect(),
    )
    .await;
    soft.check(
        "recommend comments",
        client.comment().get_recommend_comments(target, CommentPage::default()).await,
        |r| assert!(!r.comments.is_empty(), "{r:?}"),
    );
    check_two_pages(&mut soft, "moment comments", client.comment().get_moment_comments(target, 10, ""), |r| {
        r.comments.iter().map(|c| dbg(c)).collect()
    })
    .await;

    // lyric extras (晴天 has none of these; Shape of You has all of them)
    let lyric = client.lyric();
    soft.check("singing annotations", lyric.get_singing_annotations_info(LYRIC_EXTRAS_SONG_ID).await, |r| {
        assert!(r.has_singing_annotations_lyric);
    });
    soft.check("multi style trans", lyric.get_multi_style_trans_lyric(LYRIC_EXTRAS_SONG_ID).await, |r| {
        let first = r.lyrics.first().expect("styles");
        assert!(!first.style_name.is_empty() && first.lyric.contains("[00:"), "not decrypted: {first:?}");
    });
    soft.check("ai dict exists", lyric.is_ai_dict_exists(LYRIC_EXTRAS_SONG_ID).await, |r| assert!(r.exists));
    soft.check("ai dict", lyric.get_ai_dict(LYRIC_EXTRAS_SONG_ID).await, |r| {
        assert!(r.dict_list.first().is_some_and(|d| !d.phrase.is_empty() && !d.explain.is_empty()), "{r:?}");
    });

    // recommend
    check_two_pages(&mut soft, "home feed", client.recommend().get_home_feed(1, 0, 0, Vec::new()), |r| {
        r.shelves.iter().map(|s| dbg(s)).collect()
    })
    .await;
    soft.check("radar", client.recommend().get_radar_recommend(1).await, |r| assert!(!r.songs.is_empty(), "{r:?}"));
    soft.check("new songs", client.recommend().get_recommend_newsong(5).await, |r| assert!(!r.songs.is_empty()));

    soft.finish();
}

/// Private-message read endpoints. Only sessions without unread messages are
/// opened, and the unread counters must be unchanged afterwards.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_private_message_read() {
    use qqmusic_api::modules::private_message::{MessageListOptions, SessionListOptions};

    let Some((client, _)) = auth_client() else { return };
    let pm = client.private_message();
    let mut soft = SoftCheck::default();
    let unread = |r: &qqmusic_api::models::private_message::PrivateSessionListResponse| {
        r.sessions.iter().map(|s| (s.session_id.clone(), s.new_msg_cnt)).collect::<Vec<_>>()
    };

    let before = pm.get_sessions(SessionListOptions::default(), None).await.expect("sessions");
    // Sessions are sorted newest first, so this is the most recent read one.
    let Some(session) = before.sessions.iter().find(|s| s.new_msg_cnt == 0 && s.user.is_some()) else {
        println!("no read session to inspect; skipping");
        return;
    };
    let peer = session.user.as_ref().unwrap();
    let options =
        MessageListOptions { session_id: session.session_id.clone(), size: 10, ..MessageListOptions::default() };
    // The server only keeps roughly the last 90 days of messages, so a session
    // whose last message is older legitimately comes back empty.
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    let recent = session.new_msg.as_ref().is_some_and(|m| now - m.time < 80 * 86_400);
    soft.check("messages", pm.get_messages(options, None).await, |r| {
        assert_eq!(r.session.as_ref().map(|s| s.session_id.as_str()), Some(session.session_id.as_str()));
        if recent {
            let first = r.messages.first().expect("recent session has messages");
            assert!(!first.id.is_empty() && first.time > 0, "{first:?}");
            assert!(first.meta_data.as_ref().is_some_and(|m| !m.content.is_empty()), "{first:?}");
        } else {
            println!("  (last message older than 80 days; content not checked)");
        }
    });
    let msg_ids = session.new_msg.iter().map(|m| m.id.clone()).collect();
    soft.check("media details", pm.get_media_message_details(&session.session_id, msg_ids, None).await, |_| {});
    soft.check("config", pm.get_config(0, "", None).await, |_| {});
    soft.check("chat entries", pm.get_chat_entries(vec![1], None, None, None, None).await, |r| {
        assert!(r.entries.get("1").is_some_and(|e| !e.is_empty()), "{r:?}");
    });
    soft.check("musician card", pm.get_musician_message_card(&peer.encrypt_uin, None).await, |_| {});
    soft.check("safety hint", pm.get_safety_hint(&peer.encrypt_uin, 0, None).await, |_| {});
    soft.check("friendship badge", pm.get_friendship_badge(&peer.encrypt_uin, None).await, |r| {
        assert!(r.get("FloatingIcon").is_some(), "{r}");
    });

    let after = pm.get_sessions(SessionListOptions::default(), None).await.expect("sessions");
    assert_eq!(unread(&before), unread(&after), "reading changed unread counters");
    soft.finish();
}

/// QR login up to the point a human has to scan: the image is valid and an
/// unscanned code reports `Scan` (the MQTT stream for the app code subscribes
/// and then times out).
#[tokio::test]
#[ignore = "requires network"]
async fn live_qrcode_unscanned() {
    use futures::StreamExt;
    use qqmusic_api::models::login::{QrCodeLoginEvent, QrLoginType};

    let client = client();
    let login = client.login();
    for ty in [QrLoginType::Qq, QrLoginType::Wx, QrLoginType::Mobile] {
        let qr = login.get_qrcode(ty).await.unwrap_or_else(|e| panic!("{ty:?} qrcode: {e:?}"));
        let is_image = qr.data.starts_with(b"\x89PNG") || qr.data.starts_with(b"\xff\xd8");
        assert!(is_image && !qr.identifier.is_empty(), "{ty:?}: {} bytes, {}", qr.data.len(), qr.mimetype);
        let event = if ty == QrLoginType::Mobile {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            let events: Vec<_> = login.mobile_qrcode_events(&qr, Some(deadline)).collect().await;
            let events: Vec<_> = events.into_iter().map(|e| e.expect("mobile event").event).collect();
            assert_eq!(events.last(), Some(&QrCodeLoginEvent::Timeout), "{events:?}");
            events[0]
        } else {
            login.check_qrcode(&qr).await.unwrap_or_else(|e| panic!("{ty:?} check: {e:?}")).event
        };
        assert_eq!(event, QrCodeLoginEvent::Scan, "{ty:?}");
        println!("{ty:?} qrcode: ok ({} bytes)", qr.data.len());
    }
}

/// Private-message writes against the official "QQ音乐小秘书" account:
/// send a text, delete it, flip two settings and restore them, then clear and
/// delete the session if it holds nothing but our own messages.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_private_message_write() {
    use qqmusic_api::modules::private_message::{MessageListOptions, SendMessageOptions, SessionListOptions};
    use serde_json::json;

    const PEER_UIN: &str = "2738155675"; // QQ音乐小秘书
    let Some((client, cred)) = auth_client() else { return };
    let pm = client.private_message();
    let session_id = format!("{}_{PEER_UIN}", cred.musicid);
    let list = MessageListOptions { session_id: session_id.clone(), size: 50, ..MessageListOptions::default() };

    let content = format!("qqmusic-api live test {}", std::process::id());
    let options = SendMessageOptions {
        session_id: session_id.clone(),
        meta_data: Some(json!({"content": content})),
        client_key: format!("qqmusic-api-{}", std::process::id()),
        ..SendMessageOptions::default()
    };
    let sent = pm.send_message(PEER_UIN, 0, options, None).await.expect("send");
    let msg = sent.messages.first().expect("sent message echoed").clone();
    assert_eq!(msg.meta_data.as_ref().map(|m| m.content.as_str()), Some(content.as_str()), "{sent:?}");

    let listed = pm.get_messages(list.clone(), None).await.expect("messages");
    let deleted = undo("delete_message", || async { pm.delete_message(&session_id, &msg.id, 0, None).await }).await;
    assert!(listed.messages.iter().any(|m| m.id == msg.id), "sent message not listed");
    println!("send/delete message: ok ({:?})", deleted.tips);
    let remaining = pm.get_messages(list.clone(), None).await.expect("messages");
    assert!(!remaining.messages.iter().any(|m| m.id == msg.id), "message not deleted");

    // Type 1 is an integer setting, type 5 a string one: flip each, read it
    // back and restore the original value.
    for (ty, integer) in [(1, true), (5, false)] {
        let original = pm.get_config(ty, "", None).await.expect("get_config");
        let current = if integer { original.config_value.to_string() } else { original.config_value_str.clone() };
        let flipped = if current == "1" { "0" } else { "1" };
        pm.set_config(ty, flipped, None).await.expect("set_config");
        let changed = pm.get_config(ty, "", None).await.expect("get_config");
        undo("restore config", || async { pm.set_config(ty, &current, None).await }).await;
        let restored = pm.get_config(ty, "", None).await.expect("get_config");
        let value = |c: &qqmusic_api::models::private_message::PrivateConfigResponse| {
            if integer { c.config_value.to_string() } else { c.config_value_str.clone() }
        };
        assert_eq!(value(&changed), flipped, "config {ty} not updated");
        assert_eq!(value(&restored), current, "config {ty} not restored");
        println!("config {ty}: {current} -> {flipped} -> {current}");
    }

    let own = cred.musicid.to_string();
    if remaining.messages.iter().all(|m| m.from_user.as_ref().is_some_and(|u| u.uin == own)) {
        let probe = SendMessageOptions {
            session_id: session_id.clone(),
            meta_data: Some(json!({"content": format!("{content} (clear)")})),
            client_key: format!("qqmusic-api-{}-clear", std::process::id()),
            ..SendMessageOptions::default()
        };
        pm.send_message(PEER_UIN, 0, probe, None).await.expect("send");
        pm.clear_session(&session_id, 0, None).await.expect("clear_session");
        let cleared = pm.get_messages(list.clone(), None).await.expect("messages");
        let listed = pm.get_sessions(SessionListOptions::default(), None).await.expect("sessions");
        assert!(cleared.messages.is_empty(), "not cleared: {:?}", cleared.messages);
        assert!(listed.sessions.iter().any(|s| s.session_id == session_id), "clear removed the session");
        pm.delete_session(&session_id, 0, None).await.expect("delete_session");
        let sessions = pm.get_sessions(SessionListOptions::default(), None).await.expect("sessions");
        assert!(!sessions.sessions.iter().any(|s| s.session_id == session_id), "session still listed");
        println!("clear/delete session: ok");
    } else {
        println!("clear/delete session: session has messages from the peer, skipped");
    }
}

/// Mark every private message as read. Clears the unread counters of all
/// sessions (not undoable), so it also needs `QQMUSIC_ALLOW_MARK_ALL_READ=1`.
#[tokio::test]
#[ignore = "requires network, QQMUSIC_CREDENTIAL and QQMUSIC_ALLOW_MARK_ALL_READ"]
async fn live_auth_mark_all_read() {
    use qqmusic_api::modules::private_message::SessionListOptions;

    if std::env::var("QQMUSIC_ALLOW_MARK_ALL_READ").as_deref() != Ok("1") {
        println!("QQMUSIC_ALLOW_MARK_ALL_READ not set; skipping");
        return;
    }
    let Some((client, cred)) = auth_client() else { return };
    let pm = client.private_message();
    pm.mark_all_messages_read(1, &cred.encrypt_uin, None).await.expect("mark all read");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let sessions = pm.get_sessions(SessionListOptions::default(), None).await.expect("sessions");
    assert_eq!(sessions.new_msg_cnt, 0, "unread left: {sessions:?}");
    assert!(sessions.sessions.iter().all(|s| s.new_msg_cnt == 0));
}

/// Desktop and web requests authenticate through cookies.
#[tokio::test]
#[ignore = "requires network and QQMUSIC_CREDENTIAL"]
async fn live_auth_desktop_and_web() {
    use qqmusic_api::Platform;

    let Some((client, cred)) = auth_client() else { return };
    let mut soft = SoftCheck::default();
    for platform in [Platform::Desktop, Platform::Web] {
        let user = client.user();
        let vip = user.get_vip_info(None).platform(platform).await;
        soft.check(&format!("{platform:?} vip info"), vip, |_| {});
        let fans = user.get_fans(&cred.encrypt_uin, 1, 5, None).map_request(|r| r.platform(platform)).await;
        soft.check(&format!("{platform:?} fans"), fans, |_| {});
        let fav = user.get_fav_mv(&cred.encrypt_uin, 1, 5, None).platform(platform).await;
        soft.check(&format!("{platform:?} fav mv"), fav, |_| {});
        let detail = client.sound_power().get_detail(None).platform(platform).await;
        soft.check(&format!("{platform:?} sound power"), detail, |_| {});
    }
    soft.finish();
}
