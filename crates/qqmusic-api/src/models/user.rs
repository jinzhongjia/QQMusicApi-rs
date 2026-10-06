//! User models.

use serde::Serialize;
use serde_json::{Map, Value};

use super::base::{Album, Mv, Singer, SongList};
use crate::FromJson;
use crate::pagination::PageItems;

/// Playlist created by a user.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserPlaylistSummary {
    /// Playlist.
    #[json(flatten)]
    #[serde(flatten)]
    pub songlist: SongList,
    /// Create time.
    #[json(alias = "createTime")]
    pub create_time: i64,
    /// Update time.
    #[json(alias = "updateTime")]
    pub update_time: i64,
    /// Owner uin.
    pub uin: String,
    /// Owner nickname.
    pub nick: String,
    /// Big picture.
    #[json(alias = "bigpicUrl")]
    pub bigpic_url: String,
    /// Album picture.
    #[json(alias = "albumPicUrl")]
    pub album_pic_url: String,
    /// Avatar.
    pub avatar: String,
    /// Identity icon.
    #[json(alias = "identIcon")]
    pub ident_icon: String,
    /// Layer URL.
    #[json(alias = "layerUrl")]
    pub layer_url: String,
    /// Invalid.
    pub invalid: bool,
    /// Show flag.
    #[json(alias = "dirShow")]
    pub dir_show: i64,
    /// Favourite count.
    #[json(alias = "fav_cnt")]
    pub create_fav_cnt: i64,
    /// Play count.
    pub play_cnt: i64,
    /// Comment count.
    pub comment_cnt: i64,
    /// Operation type.
    #[json(alias = "opType")]
    pub op_type: i64,
    /// Sort weight.
    #[json(alias = "sortWeight")]
    pub sort_weight: i64,
}

/// `get_created_songlist` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserCreatedSonglistResponse {
    /// Total.
    pub total: i64,
    /// Playlists.
    #[json(path = "$.v_playlist[*]")]
    pub playlists: Vec<UserPlaylistSummary>,
    /// Deleted ids.
    #[json(alias = "v_delTid")]
    pub deleted_ids: Vec<i64>,
    /// Finished.
    #[json(alias = "bFinish")]
    pub finished: bool,
}

/// Favourite playlist.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFavSonglistItem {
    /// Playlist.
    #[json(flatten)]
    #[serde(flatten)]
    pub songlist: SongList,
    /// Owner uin.
    pub uin: String,
    /// Owner nickname.
    pub nickname: String,
    /// Create time.
    #[json(alias = "createtime")]
    pub create_time: i64,
    /// Update time.
    #[json(alias = "updateTime")]
    pub update_time: i64,
    /// Favourite time.
    #[json(alias = "orderTime")]
    pub order_time: i64,
    /// Show flag.
    #[json(alias = "dirShow")]
    pub dir_show: i64,
    /// Dir type.
    #[json(alias = "dirType")]
    pub dir_type: i64,
    /// Edge mark.
    #[json(alias = "edgeMark")]
    pub edge_mark: String,
    /// Layer URL.
    #[json(alias = "layerUrl")]
    pub layer_url: String,
    /// Album picture.
    #[json(alias = "albumPicUrl")]
    pub album_pic_url: String,
    /// Operation type.
    #[json(alias = "opType")]
    pub op_type: i64,
    /// Sort weight.
    #[json(alias = "sortWeight")]
    pub sort_weight: i64,
    /// Read time.
    pub readtime: i64,
}

/// `get_fav_songlist` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFavSonglistResponse {
    /// Items in this page.
    pub number: i64,
    /// Total.
    pub total: i64,
    /// Has more.
    pub hasmore: i64,
    /// Hidden.
    pub hide: bool,
    /// Playlists.
    #[json(path = "$.v_list")]
    pub playlists: Vec<UserFavSonglistItem>,
    /// Deleted ids.
    #[json(alias = "v_delTids")]
    pub deleted_ids: Vec<i64>,
    /// Failed ids.
    #[json(alias = "v_failTids")]
    pub failed_ids: Vec<i64>,
}

impl PageItems for UserFavSonglistResponse {
    type Item = UserFavSonglistItem;
    fn into_items(self) -> Vec<UserFavSonglistItem> {
        self.playlists
    }
}

/// Favourite album.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFavAlbumItem {
    /// Album.
    #[json(flatten)]
    #[serde(flatten)]
    pub album: Album,
    /// Songs.
    pub songnum: i64,
    /// Publish time.
    pub pubtime: i64,
    /// Favourite time.
    pub ordertime: i64,
    /// Status.
    pub status: i64,
    /// loc.
    pub loc: i64,
    /// Singers.
    #[json(alias = "v_singer")]
    pub singers: Vec<Singer>,
}

/// `get_fav_album` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFavAlbumResponse {
    /// Items in this page.
    pub number: i64,
    /// Total.
    pub total: i64,
    /// Has more.
    pub hasmore: i64,
    /// Hidden.
    pub hide: bool,
    /// Albums.
    #[json(path = "$.v_list[*]")]
    pub albums: Vec<UserFavAlbumItem>,
    /// Failed ids.
    #[json(alias = "v_failAlbumId")]
    pub failed_album_ids: Vec<i64>,
}

impl PageItems for UserFavAlbumResponse {
    type Item = UserFavAlbumItem;
    fn into_items(self) -> Vec<UserFavAlbumItem> {
        self.albums
    }
}

/// Music gene card.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserInfoCard {
    /// Avatar.
    #[json(alias = "HeadUrl")]
    pub head_url: String,
    /// Nickname.
    #[json(alias = "NickName")]
    pub nick_name: String,
    /// Signature.
    #[json(alias = "Signature")]
    pub signature: String,
    /// Encrypted account.
    #[json(alias = "EncryptionAccount")]
    pub encryption_account: String,
    /// Preferences.
    #[json(alias = "Preferences")]
    pub preferences: Map<String, Value>,
}

/// Listening report.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct ListeningReport {
    /// Report entries.
    #[json(alias = "Report")]
    pub report: Vec<Value>,
}

/// `get_music_gene` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserMusicGeneResponse {
    /// Info card.
    #[json(alias = "UserInfoCard")]
    pub user_info_card: UserInfoCard,
    /// Listening report.
    #[json(alias = "ListeningReport")]
    pub listening_report: ListeningReport,
    /// Sort array.
    #[json(alias = "SortArray")]
    pub sort_array: Vec<i64>,
    /// Visiting another account.
    #[json(alias = "IsVisitAccount")]
    pub is_visit_account: bool,
}

/// Homepage base info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserHomepageBaseInfo {
    /// Encrypted uin.
    #[json(alias = "EncryptedUin")]
    pub encrypted_uin: String,
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
    /// Avatar.
    #[json(alias = "Avatar")]
    pub avatar: String,
    /// Background image.
    #[json(alias = "BackgroundImage")]
    pub background_image: String,
    /// User type.
    #[json(alias = "UserType")]
    pub user_type: i64,
}

/// `get_homepage` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserHomepageResponse {
    /// Base info.
    #[json(path = "$.Info.BaseInfo")]
    pub base_info: UserHomepageBaseInfo,
    /// Singer info (if the user is a singer).
    #[json(path = "$.Info.Singer")]
    pub singer: Map<String, Value>,
    /// Followed by me.
    #[json(path = "$.Info.IsFollowed")]
    pub is_followed: i64,
    /// Tab detail.
    #[json(alias = "TabDetail")]
    pub tab_detail: Map<String, Value>,
}

/// VIP identity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct VipIdentity {
    /// Green diamond.
    pub vip: i64,
    /// Huge VIP.
    #[json(alias = "HugeVip")]
    pub huge_vip: i64,
    /// Huge VIP start.
    #[json(alias = "HugeVipStart")]
    pub huge_vip_start: String,
    /// Huge VIP end.
    #[json(alias = "HugeVipEnd")]
    pub huge_vip_end: String,
    /// Yearly flag.
    #[json(alias = "yearflag")]
    pub year_flag: i64,
    /// Huge yearly flag.
    #[json(alias = "HugeYearFlag")]
    pub huge_year_flag: i64,
    /// Twelve-yuan pack.
    pub twelve: i64,
    /// Twelve start.
    #[json(alias = "twelveStart")]
    pub twelve_start: String,
    /// Twelve end.
    #[json(alias = "twelveEnd")]
    pub twelve_end: String,
    /// Child VIP.
    #[json(alias = "ChildVip")]
    pub child_vip: i64,
    /// Experience VIP.
    #[json(alias = "ExpVip")]
    pub exp_vip: i64,
    /// Group VIP.
    #[json(alias = "GroupVipFlag")]
    pub group_vip_flag: i64,
    /// Group VIP start.
    #[json(alias = "GroupVipStart")]
    pub group_vip_start: String,
    /// Group VIP end.
    #[json(alias = "GroupVipEnd")]
    pub group_vip_end: String,
    /// Couple VIP.
    #[json(alias = "CPLoverFlag")]
    pub cp_lover_flag: i64,
    /// Couple VIP start.
    #[json(alias = "CPLoverStart")]
    pub cp_lover_start: String,
    /// Couple VIP end.
    #[json(alias = "CPLoverEnd")]
    pub cp_lover_end: String,
    /// Ad-free VIP.
    #[json(alias = "AdVipFlag")]
    pub ad_vip_flag: i64,
    /// Eight-yuan pack.
    pub eight: i64,
    /// Eight start.
    #[json(alias = "eightStart")]
    pub eight_start: String,
    /// Eight end.
    #[json(alias = "eightEnd")]
    pub eight_end: String,
    /// Level.
    pub level: i64,
    /// Next level.
    #[json(alias = "nextlevel")]
    pub next_level: i64,
    /// Icon.
    pub icon: String,
    /// Purchase URL.
    #[json(alias = "purchaseUrl")]
    pub purchase_url: String,
}

/// VIP user info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct VipUserInfo {
    /// Buy URL.
    #[json(alias("buy_url", "buyurl"))]
    pub buy_url: String,
    /// My VIP URL.
    #[json(alias("my_vip_url", "myvipurl"))]
    pub my_vip_url: String,
    /// Score.
    pub score: i64,
    /// Expire.
    pub expire: i64,
    /// Music level.
    pub music_level: i64,
}

/// `get_vip_info` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserVipInfoResponse {
    /// Auto download.
    #[json(alias("auto_down", "autoDown", "autodown"))]
    pub auto_down: i64,
    /// Can renew.
    #[json(alias = "canRenew")]
    pub can_renew: i64,
    /// Max playlists.
    #[json(alias("max_dir_num", "maxDirNum", "maxdirnum"))]
    pub max_dir_num: i64,
    /// Max songs.
    #[json(alias("max_song_num", "maxSongNum", "maxsongnum"))]
    pub max_song_num: i64,
    /// Song limit message.
    #[json(alias("song_limit_msg", "songLimitMsg"))]
    pub song_limit_msg: String,
    /// SVIP.
    pub svip: i64,
    /// Star.
    pub star: i64,
    /// Star start.
    #[json(alias = "starstart")]
    pub star_start: String,
    /// Star end.
    #[json(alias = "starend")]
    pub star_end: String,
    /// Yearly star.
    pub ystar: i64,
    /// Yearly star start.
    #[json(alias = "ystarstart")]
    pub ystar_start: String,
    /// Yearly star end.
    #[json(alias = "ystarend")]
    pub ystar_end: String,
    /// Identity.
    pub identity: VipIdentity,
    /// User info.
    pub userinfo: VipUserInfo,
}

/// User in a relation list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct RelationUser {
    /// Mid.
    #[json(alias = "MID")]
    pub mid: String,
    /// Encrypted uin.
    #[json(alias = "EncUin")]
    pub enc_uin: String,
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
    /// Description.
    #[json(alias = "Desc")]
    pub desc: String,
    /// Avatar.
    #[json(alias = "AvatarUrl")]
    pub avatar_url: String,
    /// Fans.
    #[json(alias = "FanNum")]
    pub fan_num: i64,
    /// Followed by me.
    #[json(alias = "IsFollow")]
    pub is_follow: bool,
}

/// Relation list response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserRelationListResponse {
    /// Total.
    #[json(alias = "Total")]
    pub total: i64,
    /// Users.
    #[json(path = "$.List[*]")]
    pub users: Vec<RelationUser>,
    /// Has more.
    #[json(alias = "HasMore")]
    pub has_more: bool,
    /// Last position.
    #[json(alias = "LastPos")]
    pub last_pos: String,
    /// Message.
    #[json(alias = "Msg")]
    pub msg: String,
    /// Lock flag.
    #[json(alias = "LockFlag")]
    pub lock_flag: i64,
    /// Lock message.
    #[json(alias = "LockMsg")]
    pub lock_msg: String,
}

impl PageItems for UserRelationListResponse {
    type Item = RelationUser;
    fn into_items(self) -> Vec<RelationUser> {
        self.users
    }
}

/// Friend.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct FriendEntry {
    /// Encrypted uin.
    #[json(alias = "EncryptUin")]
    pub encrypt_uin: String,
    /// Name.
    #[json(alias = "UserName")]
    pub user_name: String,
    /// Avatar.
    #[json(alias = "AvatarUrl")]
    pub avatar_url: String,
    /// Followed by me.
    #[json(alias = "IsFollow")]
    pub is_follow: bool,
}

/// `get_friend` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFriendListResponse {
    /// Friends.
    #[json(alias = "Friends")]
    pub friends: Vec<FriendEntry>,
    /// Has more.
    #[json(alias = "HasMore")]
    pub has_more: bool,
}

impl PageItems for UserFriendListResponse {
    type Item = FriendEntry;
    fn into_items(self) -> Vec<FriendEntry> {
        self.friends
    }
}

/// Favourite MV.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFavMvItem {
    /// MV.
    #[json(flatten)]
    #[serde(flatten)]
    pub mv: Mv,
    /// Picture.
    #[json(alias = "picUrl")]
    pub picurl: String,
    /// Play count.
    pub playcount: i64,
    /// Publish date.
    pub publish_date: i64,
    /// Singer id.
    #[json(alias = "singerId")]
    pub singer_id: i64,
    /// Singer mid.
    #[json(alias = "singerMid")]
    pub singer_mid: String,
    /// Singer name.
    #[json(alias = "singerName")]
    pub singer_name: String,
    /// Status.
    pub status: i64,
}

/// `get_fav_mv` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct UserFavMvResponse {
    /// Code.
    pub code: i64,
    /// Sub code.
    #[json(alias("subCode", "subcode"))]
    pub sub_code: i64,
    /// Message.
    pub msg: String,
    /// MVs.
    #[json(alias = "mvlist")]
    pub mv_list: Vec<UserFavMvItem>,
}

/// Disliked item.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct DislikeItem {
    /// Id.
    #[json(alias = "ID")]
    pub id: String,
    /// Name.
    #[json(alias = "Name")]
    pub name: String,
    /// Image.
    #[json(alias = "Img")]
    pub img: String,
    /// Id type.
    #[json(alias = "IdType")]
    pub id_type: i64,
    /// Time.
    #[json(alias = "Time")]
    pub time: i64,
}

/// `get_dislike_list` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct DislikeListData {
    /// Return code.
    #[json(alias = "Retcode")]
    pub retcode: i64,
    /// Message.
    #[json(alias = "Msg")]
    pub msg: String,
    /// Singers.
    #[json(alias = "Singers")]
    pub singers: Vec<DislikeItem>,
    /// Songs.
    #[json(alias = "Songs")]
    pub songs: Vec<DislikeItem>,
    /// Styles.
    #[json(alias = "Styles")]
    pub styles: Vec<DislikeItem>,
    /// Page.
    #[json(alias = "Page")]
    pub page: i64,
    /// Token.
    #[json(alias = "Token")]
    pub token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn playlist_models() {
        let created: UserCreatedSonglistResponse = from_value(&json!({
            "total": 1, "v_playlist": [{"tid": 5, "dirName": "我喜欢", "fav_cnt": 3, "invalid": false}], "bFinish": true, "v_delTid": [1]
        }))
        .unwrap();
        assert_eq!(created.playlists[0].songlist.id, 5);
        assert_eq!(created.playlists[0].songlist.title, "我喜欢");
        assert_eq!(created.playlists[0].create_fav_cnt, 3);
        assert!(created.finished);
        let fav: UserFavSonglistResponse =
            from_value(&json!({"v_list": [{"tid": 1, "nickname": "n", "createtime": 9}], "hasmore": 1})).unwrap();
        assert_eq!(fav.playlists[0].create_time, 9);
        let albums: UserFavAlbumResponse =
            from_value(&json!({"v_list": [{"albumMid": "a", "v_singer": [{"mid": "s"}]}]})).unwrap();
        assert_eq!(albums.albums[0].album.mid, "a");
        assert_eq!(albums.into_items()[0].singers[0].mid, "s");
    }

    #[test]
    fn profile_models() {
        let vip: UserVipInfoResponse = from_value(
            &json!({"maxDirNum": 10, "identity": {"HugeVip": 1, "nextlevel": 2}, "userinfo": {"buyurl": "u"}}),
        )
        .unwrap();
        assert_eq!((vip.max_dir_num, vip.identity.huge_vip, vip.identity.next_level), (10, 1, 2));
        assert_eq!(vip.userinfo.buy_url, "u");
        let home: UserHomepageResponse =
            from_value(&json!({"Info": {"BaseInfo": {"Name": "n"}, "IsFollowed": 1}, "TabDetail": {"x": 1}})).unwrap();
        assert_eq!((home.base_info.name.as_str(), home.is_followed), ("n", 1));
        let gene: UserMusicGeneResponse = from_value(
            &json!({"UserInfoCard": {"NickName": "n"}, "ListeningReport": {"Report": [{}]}, "IsVisitAccount": true}),
        )
        .unwrap();
        assert_eq!(gene.user_info_card.nick_name, "n");
        assert_eq!(gene.listening_report.report.len(), 1);
        let rel: UserRelationListResponse =
            from_value(&json!({"Total": 2, "List": [{"MID": "m", "IsFollow": true}], "HasMore": true})).unwrap();
        assert!(rel.users[0].is_follow);
        let friends: UserFriendListResponse =
            from_value(&json!({"Friends": [{"UserName": "f"}], "HasMore": false})).unwrap();
        assert_eq!(friends.friends[0].user_name, "f");
        let mvs: UserFavMvResponse =
            from_value(&json!({"subcode": 0, "mvlist": [{"vid": "v", "picUrl": "p"}]})).unwrap();
        assert_eq!(mvs.mv_list[0].picurl, "p");
        let dislike: DislikeListData =
            from_value(&json!({"Retcode": 0, "Songs": [{"ID": 12, "Name": "s"}], "Page": 1})).unwrap();
        assert_eq!(dislike.songs[0].id, "12");
    }
}
