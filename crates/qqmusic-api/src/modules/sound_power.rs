//! Sound power (listening level) APIs. All endpoints require login.

use serde_json::json;

use crate::credential::Credential;
use crate::models::sound_power::{
    ActTaskModulesResponse, FriendRankResponse, HugevipLevelRuleResponse, LikeFriendResponse, SetRankPrivacyResponse,
    SoundPowerDetailResponse, SoundPowerMedalEntryResponse,
};
use crate::request::CgiRequest;

/// Default activity id of [`SoundPowerApi::get_tasks`].
pub const DEFAULT_ACT_ID: &str = "1nsAQf";
/// Default task module id of [`SoundPowerApi::get_tasks`].
pub const DEFAULT_TASK_MODULE_ID: &str = "Z1jtHy7";

api_module! {
    /// Sound power APIs.
    SoundPowerApi
}

impl SoundPowerApi {
    fn login_cgi<T: crate::FromJson + Send + 'static>(
        &self,
        module: &str,
        method: &str,
        param: serde_json::Value,
        credential: Option<Credential>,
    ) -> CgiRequest<T> {
        self.cgi(module, method, param).require_login(true).credential_opt(credential)
    }

    /// Level detail.
    pub fn get_detail(&self, credential: Option<Credential>) -> CgiRequest<SoundPowerDetailResponse> {
        self.login_cgi("music.soundPower.SoundPowerSvr", "QueryLevelDetailPage", json!({}), credential)
    }

    /// Huge VIP level rules.
    pub fn get_hugevip_rule(&self, credential: Option<Credential>) -> CgiRequest<HugevipLevelRuleResponse> {
        // Unsigned requests are rejected with 500031.
        self.login_cgi("music.soundPower.SoundPowerSvr", "QueryHugevipLevelRule", json!({}), credential).sign(true)
    }

    /// Friend ranking.
    ///
    /// Built on the QQ friend chain: WeChat logins have none and get 101010.
    pub fn get_friend_rank(
        &self,
        offset: i64,
        limit: i64,
        last_uin: &str,
        rankno: i64,
        credential: Option<Credential>,
    ) -> CgiRequest<FriendRankResponse> {
        self.login_cgi(
            "music.activeCenter.FriendRankSvr",
            "GetRank",
            json!({"rank_type": 1, "offset": offset, "limit": limit, "last_uin": last_uin, "rankno": rankno}),
            credential,
        )
    }

    /// Like (or un-like) a friend in the ranking.
    pub fn like_friend(
        &self,
        uin: &str,
        cancel: bool,
        credential: Option<Credential>,
    ) -> CgiRequest<LikeFriendResponse> {
        self.login_cgi(
            "music.activeCenter.FriendRankSvr",
            "Like",
            json!({"rank_type": 1, "uin": uin, "cancel": i64::from(cancel)}),
            credential,
        )
    }

    /// Set ranking privacy: `1` hides the user from friends' rankings, `2`
    /// shows them again (`0` is rejected with `24270103`).
    pub fn set_rank_privacy(&self, status: i64, credential: Option<Credential>) -> CgiRequest<SetRankPrivacyResponse> {
        self.login_cgi("music.activeCenter.FriendRankSvr", "SetPrivacy", json!({"status": status}), credential)
    }

    /// Medal hall entry.
    pub fn get_medal_entry(
        &self,
        enc_uin: &str,
        credential: Option<Credential>,
    ) -> CgiRequest<SoundPowerMedalEntryResponse> {
        self.login_cgi(
            "music.medalHall.MedalHallEntrySrv",
            "GetSoundPowerEntry",
            json!({"EncUin": enc_uin}),
            credential,
        )
    }

    /// Task modules (defaults: [`DEFAULT_ACT_ID`], [`DEFAULT_TASK_MODULE_ID`]).
    pub fn get_tasks(
        &self,
        act_id: Option<&str>,
        task_module_ids: Option<&[&str]>,
        credential: Option<Credential>,
    ) -> CgiRequest<ActTaskModulesResponse> {
        self.login_cgi(
            "music.activeCenter.ActTaskNewSvr",
            "GetTaskModules",
            json!({
                "actID": act_id.unwrap_or(DEFAULT_ACT_ID),
                "taskModuleIDs": task_module_ids.unwrap_or(&[DEFAULT_TASK_MODULE_ID]),
            }),
            credential,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::testing::*;

    #[tokio::test]
    async fn requires_login() {
        let (client, mock) = mock_client();
        assert!(matches!(client.sound_power().get_detail(None).await, Err(Error::CredentialInvalid(_))));
        assert_eq!(mock.request_count(), 0);
    }

    #[tokio::test]
    async fn sound_power_requests() {
        let (client, mock) = logged_in_client();
        mock.route_url("musicu", reply_all(json!({"status": 1})));
        mock.route_url("musics", reply_all(json!({"status": 1})));
        client.sound_power().get_detail(None).await.unwrap();
        assert_eq!(last_req0(&mock)["method"], "QueryLevelDetailPage");
        client.sound_power().get_hugevip_rule(None).await.unwrap();
        assert_eq!(last_req0(&mock)["method"], "QueryHugevipLevelRule");
        assert!(mock.last_request().unwrap().query_param("sign").is_some());
        client.sound_power().get_friend_rank(10, 5, "u", 3, None).await.unwrap();
        assert_eq!(
            last_req0(&mock)["param"],
            json!({"rank_type": 1, "offset": 10, "limit": 5, "last_uin": "u", "rankno": 3})
        );
        client.sound_power().like_friend("2", true, None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"]["cancel"], 1);
        assert_eq!(client.sound_power().set_rank_privacy(1, None).await.unwrap().status, 1);
        client.sound_power().get_medal_entry("e", None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"EncUin": "e"}));
        client.sound_power().get_tasks(None, None, None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"actID": "1nsAQf", "taskModuleIDs": ["Z1jtHy7"]}));
        client.sound_power().get_tasks(Some("a"), Some(&["b", "c"]), None).await.unwrap();
        assert_eq!(last_req0(&mock)["param"], json!({"actID": "a", "taskModuleIDs": ["b", "c"]}));
    }
}
