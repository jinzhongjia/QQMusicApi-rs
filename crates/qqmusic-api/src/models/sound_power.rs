//! Sound power (listening level) models.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::FromJson;

/// Equity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SoundPowerEquity {
    /// Type.
    #[json(alias = "eType")]
    pub etype: i64,
    /// Name.
    pub name: String,
    /// Icon.
    pub icon: String,
    /// Scheme.
    pub scheme: String,
}

/// Level info.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SoundPowerInfo {
    /// Level.
    pub level: i64,
    /// Value.
    pub value: i64,
    /// Name.
    pub name: String,
    /// Icon.
    pub icon: String,
    /// Current value.
    #[json(alias = "currentValue")]
    pub current_value: i64,
    /// Next value.
    #[json(alias = "nextValue")]
    pub next_value: i64,
    /// Max level reached.
    #[json(alias = "reachMaxLevel")]
    pub reach_max_level: i64,
    /// Huge VIP.
    #[json(alias = "isHugeVip")]
    pub is_huge_vip: bool,
    /// Next level numbers.
    #[json(alias = "nextLevelNums")]
    pub next_level_nums: i64,
    /// Next level icon.
    #[json(alias = "nextLevelIcon")]
    pub next_level_icon: String,
}

/// `get_detail` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SoundPowerDetailResponse {
    /// Level info.
    #[json(alias = "powerInfo")]
    pub power_info: SoundPowerInfo,
    /// Medals.
    #[json(alias = "medalNum")]
    pub medal_num: i64,
    /// Nickname.
    pub nick: String,
    /// Avatar.
    #[json(alias = "headPic")]
    pub head_pic: String,
    /// Listening time today.
    #[json(alias = "todayLT")]
    pub today_listen_time: i64,
    /// Equity text.
    #[json(alias = "equityText")]
    pub equity_text: String,
    /// Equities.
    pub equities: Vec<SoundPowerEquity>,
    /// Huge VIP.
    #[json(alias = "isHugevip")]
    pub is_hugevip: i64,
    /// Huge VIP power info.
    #[json(alias = "hugevipPowerInfo")]
    pub hugevip_power_info: Option<Map<String, Value>>,
    /// Medal info.
    #[json(alias = "medalInfo")]
    pub medal_info: Option<Map<String, Value>>,
}

/// Huge VIP level rule.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HugevipLevelRuleItem {
    /// Level.
    pub level: i64,
    /// Name.
    pub name: String,
    /// Icon.
    pub icon: String,
    /// Required duration.
    #[json(alias = "needDuration")]
    pub need_duration: i64,
    /// Percent.
    pub percent: String,
}

/// `get_hugevip_rule` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct HugevipLevelRuleResponse {
    /// Current duration.
    #[json(alias = "currentDuration")]
    pub current_duration: i64,
    /// Next level duration.
    #[json(alias = "nextLevelDuration")]
    pub next_level_duration: i64,
    /// Rules.
    pub rules: Vec<HugevipLevelRuleItem>,
}

/// Friend rank entry.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct FriendRankItem {
    /// Uin.
    pub uin: String,
    /// Nickname.
    pub nick: String,
    /// Avatar.
    pub pic: String,
    /// Play time.
    pub play_time: i64,
    /// Likes.
    pub like_num: i64,
    /// Liked by me.
    pub like_status: i64,
    /// Rank.
    pub no: i64,
    /// Sound power info.
    #[json(alias = "spInfo")]
    pub sp_info: Map<String, Value>,
    /// Medal info.
    #[json(alias = "mInfo")]
    pub medal_info: Map<String, Value>,
}

/// `get_friend_rank` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct FriendRankResponse {
    /// Ranks.
    pub ranks: Vec<FriendRankItem>,
    /// My rank.
    pub my_rank_no: i64,
    /// My play time.
    pub my_play_time: i64,
    /// My likes.
    pub my_like_num: i64,
    /// Has more.
    pub has_more: i64,
    /// Rank number cursor.
    pub rankno: i64,
}

/// `like_friend` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct LikeFriendResponse {
    /// Unused.
    #[json(alias = "NOUSE")]
    pub no_use: i64,
}

/// `set_rank_privacy` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SetRankPrivacyResponse {
    /// Status.
    pub status: i64,
}

/// Medal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SoundPowerMedalItem {
    /// Picture.
    #[json(alias = "PicURL")]
    pub pic_url: String,
    /// Scheme.
    #[json(alias = "Scheme")]
    pub scheme: String,
}

/// `get_medal_entry` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct SoundPowerMedalEntryResponse {
    /// Title.
    #[json(alias = "Title")]
    pub title: String,
    /// Medal count.
    #[json(alias = "MedalCnt")]
    pub medal_cnt: i64,
    /// Medals.
    #[json(alias = "MedalList")]
    pub medal_list: Vec<SoundPowerMedalItem>,
}

/// `get_tasks` response.
#[derive(Debug, Clone, Default, PartialEq, Serialize, FromJson)]
#[json(default)]
pub struct ActTaskModulesResponse {
    /// Return code.
    #[json(alias = "retCode")]
    pub ret_code: i64,
    /// Return message.
    #[json(alias = "retMsg")]
    pub ret_msg: String,
    /// Task modules.
    #[json(alias = "taskModules")]
    pub task_modules: Option<Vec<Value>>,
    /// Activity info.
    #[json(alias = "actInfo")]
    pub act_info: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::from_value;
    use serde_json::json;

    #[test]
    fn sound_power_models() {
        let detail: SoundPowerDetailResponse = from_value(&json!({"powerInfo": {"level": 3, "isHugeVip": true}, "todayLT": 60, "medalInfo": null})).unwrap();
        assert_eq!(detail.power_info.level, 3);
        assert!(detail.power_info.is_huge_vip);
        assert_eq!(detail.today_listen_time, 60);
        assert!(detail.medal_info.is_none());
        let rank: FriendRankResponse = from_value(&json!({"ranks": [{"uin": "1", "spInfo": {"a": 1}}], "has_more": 1})).unwrap();
        assert_eq!(rank.ranks[0].sp_info["a"], 1);
        let medal: SoundPowerMedalEntryResponse = from_value(&json!({"MedalCnt": 2, "MedalList": [{"PicURL": "u"}]})).unwrap();
        assert_eq!(medal.medal_list[0].pic_url, "u");
        let tasks: ActTaskModulesResponse = from_value(&json!({"retCode": 0, "taskModules": [{}]})).unwrap();
        assert_eq!(tasks.task_modules.unwrap().len(), 1);
    }
}
