//! 菜单页的数据来源与加载。

use std::collections::HashSet;

use qqmusic_api::Client;
use qqmusic_api::models::login::QrLoginType;
use qqmusic_api::modules::search::{SearchOptions, SearchType};
use qqmusic_api::modules::songlist::SonglistDetailOptions;

use crate::protocol::Track;

/// 搜索类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    /// 单曲。
    Song,
    /// 歌单。
    Playlist,
    /// 专辑。
    Album,
}

impl SearchKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Song => "单曲",
            Self::Playlist => "歌单",
            Self::Album => "专辑",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Song => Self::Playlist,
            Self::Playlist => Self::Album,
            Self::Album => Self::Song,
        }
    }

    fn search_type(self) -> SearchType {
        match self {
            Self::Song => SearchType::Song,
            Self::Playlist => SearchType::SongList,
            Self::Album => SearchType::Album,
        }
    }
}

/// 页面数据来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Root,
    Guess,
    Radar,
    Liked,
    MyPlaylists,
    RecommendPlaylists,
    Toplists,
    NewSongs,
    Search { keyword: String, kind: SearchKind },
    Playlist(i64),
    Album(String),
    Toplist(i64),
    Queue,
    Account,
}

impl Source {
    /// 是否需要异步加载。
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Root | Self::Queue | Self::Account)
    }
}

/// 列表项的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Login(QrLoginType),
    Logout,
    Search,
}

/// 列表项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// 进入下一级页面。
    Link { title: String, sub: String, source: Source },
    /// 歌曲。
    Song(Track),
    /// 动作。
    Action { title: String, action: Action },
    /// 纯文本（分隔、说明）。
    Text(String),
}

fn link(title: impl Into<String>, sub: impl Into<String>, source: Source) -> Item {
    Item::Link { title: title.into(), sub: sub.into(), source }
}

/// 主菜单。
pub fn root_items() -> Vec<Item> {
    vec![
        link("猜你喜欢", "", Source::Guess),
        link("雷达推荐", "", Source::Radar),
        link("我喜欢的音乐", "", Source::Liked),
        link("我的歌单", "", Source::MyPlaylists),
        link("推荐歌单", "", Source::RecommendPlaylists),
        link("排行榜", "", Source::Toplists),
        link("新歌速递", "", Source::NewSongs),
        Item::Action { title: "搜索".into(), action: Action::Search },
        link("播放列表", "", Source::Queue),
        link("账号", "", Source::Account),
    ]
}

/// 账号页。
pub fn account_items(logged_in: Option<(i64, &str)>) -> Vec<Item> {
    match logged_in {
        Some((musicid, nick)) => vec![
            Item::Text(format!("已登录：{nick} ({musicid})")),
            Item::Action { title: "退出登录".into(), action: Action::Logout },
        ],
        None => vec![
            Item::Action { title: "QQ 扫码登录".into(), action: Action::Login(QrLoginType::Qq) },
            Item::Action { title: "微信扫码登录".into(), action: Action::Login(QrLoginType::Wx) },
            Item::Action { title: "QQ 音乐 App 扫码登录".into(), action: Action::Login(QrLoginType::Mobile) },
        ],
    }
}

/// 帮助弹窗内容。
pub const HELP: &[(&str, &str)] = &[
    ("j / k  ↑ / ↓", "上下移动"),
    ("h / l  ← / →", "左右移动 / 翻页"),
    ("n / Enter", "进入 / 播放"),
    ("b / Esc", "返回上一级"),
    ("g / G", "第一项 / 最后一项"),
    ("Space", "播放 / 暂停"),
    ("[ / ]", "上一首 / 下一首"),
    (", / .", "后退 / 前进 5 秒"),
    ("- / =", "音量 -/+"),
    ("m", "切换播放模式"),
    ("v", "选择音质"),
    ("/", "搜索（Tab 切换类型）"),
    ("a / A", "加入播放列表 / 下一首播放"),
    ("f / F", "喜欢当前播放 / 选中的歌曲"),
    ("c", "当前播放列表"),
    ("d", "从播放列表移除"),
    ("r", "刷新当前页"),
    ("q", "退出界面（音乐继续播放）"),
    ("Q", "退出并停止播放"),
];

fn songs<'a>(songs: impl IntoIterator<Item = &'a qqmusic_api::models::Song>) -> Vec<Item> {
    let mut seen = HashSet::new();
    songs
        .into_iter()
        .filter(|s| !s.mid.is_empty() && seen.insert(s.mid.clone()))
        .map(|s| Item::Song(Track::from(s)))
        .collect()
}

fn require_login(client: &Client) -> Result<qqmusic_api::Credential, String> {
    let credential = client.credential();
    if credential.is_valid() { Ok(credential) } else { Err("请先在「账号」中登录".into()) }
}

/// 拉取「我喜欢」的全部歌曲 id。
pub async fn liked_ids(client: &Client) -> Result<HashSet<i64>, String> {
    let credential = require_login(client)?;
    let pages = client
        .user()
        .get_fav_song(&credential.encrypt_uin, 1, 100, None)
        .collect(Some(50))
        .await
        .map_err(|e| e.to_string())?;
    Ok(pages.iter().flat_map(|p| p.songs.iter().map(|s| s.id)).collect())
}

/// 异步加载一个页面。
pub async fn load(client: &Client, source: &Source) -> Result<Vec<Item>, String> {
    let err = |e: qqmusic_api::Error| e.to_string();
    Ok(match source {
        Source::Guess => {
            let mut all = Vec::new();
            for _ in 0..4 {
                all.extend(client.recommend().get_guess_recommend(None).await.map_err(err)?.songs);
            }
            songs(&all)
        }
        Source::Radar => {
            let pages = client.recommend().get_radar_recommend(1).collect(Some(2)).await.map_err(err)?;
            songs(pages.iter().flat_map(|p| &p.songs))
        }
        Source::Liked => {
            let credential = require_login(client)?;
            let pages = client
                .user()
                .get_fav_song(&credential.encrypt_uin, 1, 100, None)
                .collect(Some(50))
                .await
                .map_err(err)?;
            songs(pages.iter().flat_map(|p| &p.songs))
        }
        Source::MyPlaylists => {
            let credential = require_login(client)?;
            let created = client.user().get_created_songlist(credential.musicid, None).await.map_err(err)?;
            let fav =
                client.user().get_fav_songlist(&credential.encrypt_uin, 1, 100, None).send().await.map_err(err)?;
            let mut items = vec![Item::Text("— 创建的歌单 —".into())];
            items.extend(created.playlists.iter().filter(|p| p.songlist.id != 0).map(|p| {
                link(&p.songlist.title, format!("{} 首", p.songlist.songnum), Source::Playlist(p.songlist.id))
            }));
            items.push(Item::Text("— 收藏的歌单 —".into()));
            items.extend(fav.playlists.iter().map(|p| {
                link(
                    &p.songlist.title,
                    format!("{} 首 · {}", p.songlist.songnum, p.nickname),
                    Source::Playlist(p.songlist.id),
                )
            }));
            items
        }
        Source::RecommendPlaylists => {
            let resp = client.recommend().get_recommend_songlist(1, 40).send().await.map_err(err)?;
            resp.songlists.iter().map(|p| link(&p.title, "", Source::Playlist(p.id))).collect()
        }
        Source::Toplists => {
            let resp = client.top().get_category().await.map_err(err)?;
            let mut items = Vec::new();
            for group in &resp.group {
                items.push(Item::Text(format!("— {} —", group.name)));
                items.extend(group.toplist.iter().map(|t| link(&t.name, &t.update_time, Source::Toplist(t.id))));
            }
            items
        }
        Source::NewSongs => songs(&client.recommend().get_recommend_newsong(5).await.map_err(err)?.songs),
        Source::Search { keyword, kind } => {
            let options =
                SearchOptions { search_type: kind.search_type(), num: 50, highlight: false, ..Default::default() };
            let resp = client.search().search_by_type_with(keyword, options).send().await.map_err(err)?;
            match kind {
                SearchKind::Song => songs(resp.song.iter().map(|s| &s.song)),
                SearchKind::Playlist => resp
                    .songlist
                    .iter()
                    .map(|p| {
                        link(
                            &p.songlist.title,
                            format!("{} 首 · {}", p.songlist.songnum, p.nickname),
                            Source::Playlist(p.songlist.id),
                        )
                    })
                    .collect(),
                SearchKind::Album => resp
                    .album
                    .iter()
                    .map(|a| link(&a.album.name, &a.album.time_public, Source::Album(a.album.mid.clone())))
                    .collect(),
            }
        }
        Source::Playlist(id) => {
            let options =
                SonglistDetailOptions { num: 100, onlysong: true, tag: false, userinfo: false, ..Default::default() };
            let pages = client.songlist().get_detail_with(*id, options).collect(Some(30)).await.map_err(err)?;
            songs(pages.iter().flat_map(|p| &p.songs))
        }
        Source::Album(mid) => {
            let pages = client.album().get_song(mid.as_str(), 100, 1).collect(Some(10)).await.map_err(err)?;
            songs(pages.iter().flat_map(|p| &p.song_list))
        }
        Source::Toplist(id) => {
            let pages = client.top().get_detail(*id, 100, 1, false).collect(Some(5)).await.map_err(err)?;
            songs(pages.iter().flat_map(|p| &p.songs))
        }
        Source::Root | Source::Queue | Source::Account => Vec::new(),
    })
}
