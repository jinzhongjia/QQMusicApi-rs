//! 页面的数据来源与加载。
//!
//! 远程页面先取第一批立即展示，其余通过 [`Feed`] 按需继续加载；续页沿用底层分页器
//! 的游标与结束条件，失败后可以重试同一页。

use std::collections::HashSet;

use futures::future::BoxFuture;
use qqmusic_api::models::Song;
use qqmusic_api::models::login::QrLoginType;
use qqmusic_api::modules::search::{SearchOptions, SearchType};
use qqmusic_api::modules::singer::OrderType;
use qqmusic_api::modules::songlist::SonglistDetailOptions;
use qqmusic_api::pagination::Pager;
use qqmusic_api::{Client, FromJson, Paged};

use crate::paths::Paths;
use crate::protocol::{ErrorKind, RecommendationSource, Track};

/// 单个页面最多保留的条目数；达到后不再继续加载，并在界面上注明。
pub const MAX_ITEMS: usize = 2000;
/// 搜索每页条数。
const SEARCH_PAGE: i64 = 30;
/// 保存的搜索历史条数。
const HISTORY_LIMIT: usize = 20;

/// 加载失败：原因与分类（决定恢复提示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub kind: ErrorKind,
    pub message: String,
}

impl Failure {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into() }
    }
}

impl From<qqmusic_api::Error> for Failure {
    fn from(error: qqmusic_api::Error) -> Self {
        use qqmusic_api::Error;
        let kind = match &error {
            _ if error.is_credential_expired() => ErrorKind::Login,
            Error::CredentialInvalid(_) => ErrorKind::Login,
            Error::Network(_) | Error::Http { .. } => ErrorKind::Network,
            _ => ErrorKind::Other,
        };
        Self { kind, message: crate::redact(&error.to_string()) }
    }
}

/// 搜索类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchKind {
    /// 单曲。
    Song,
    /// 歌手。
    Singer,
    /// 专辑。
    Album,
    /// 歌单。
    Playlist,
}

impl SearchKind {
    pub const ALL: [Self; 4] = [Self::Song, Self::Singer, Self::Album, Self::Playlist];

    pub fn label(self) -> &'static str {
        match self {
            Self::Song => "单曲",
            Self::Singer => "歌手",
            Self::Album => "专辑",
            Self::Playlist => "歌单",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Song => Self::Singer,
            Self::Singer => Self::Album,
            Self::Album => Self::Playlist,
            Self::Playlist => Self::Song,
        }
    }

    fn search_type(self) -> SearchType {
        match self {
            Self::Song => SearchType::Song,
            Self::Singer => SearchType::Singer,
            Self::Album => SearchType::Album,
            Self::Playlist => SearchType::SongList,
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
    Search {
        keyword: String,
        kind: SearchKind,
    },
    Playlist(i64),
    Album(String),
    Toplist(i64),
    /// 歌手的歌曲（第一行是进入专辑列表的入口）。
    Singer(String),
    /// 歌手的专辑。
    SingerAlbums(String),
    Queue,
    Lyrics,
    Account,
}

impl Source {
    /// 是否需要异步加载。
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Root | Self::Queue | Self::Lyrics | Self::Account)
    }

    /// 内容是否随登录账号变化（切换账号后必须丢弃）。
    pub fn is_personal(&self) -> bool {
        matches!(self, Self::Guess | Self::Radar | Self::Liked | Self::MyPlaylists)
    }
}

/// 列表项的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Login(QrLoginType),
    Logout,
    /// 重新确认登录是否有效。
    CheckLogin,
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

/// 主菜单（宽屏时即左侧导航）。
pub fn root_items() -> Vec<Item> {
    vec![
        Item::Action { title: "搜索".into(), action: Action::Search },
        link("当前播放", "", Source::Lyrics),
        link("播放队列", "", Source::Queue),
        link("猜你喜欢", "", Source::Guess),
        link("雷达推荐", "", Source::Radar),
        link("我喜欢的音乐", "", Source::Liked),
        link("我的歌单", "", Source::MyPlaylists),
        link("推荐歌单", "", Source::RecommendPlaylists),
        link("排行榜", "", Source::Toplists),
        link("新歌速递", "", Source::NewSongs),
        link("账号", "", Source::Account),
    ]
}

/// 帮助内容；键为空的行是分组标题。
pub const HELP: &[(&str, &str)] = &[
    ("", "浏览"),
    ("j / k  ↑ / ↓", "上下移动"),
    ("g / G", "第一项 / 最后一项"),
    ("Ctrl+d / Ctrl+u", "下翻 / 上翻一页（PgDn / PgUp）"),
    ("Enter / n", "进入；在歌曲上：播放并替换队列"),
    ("l / →", "进入；在歌曲上：打开操作菜单"),
    ("h / ← / b / Esc", "返回上一级"),
    ("Tab / Shift+Tab", "切换焦点：导航 / 内容 / 侧栏"),
    ("r", "刷新当前页；加载更多失败时重试"),
    ("", "搜索"),
    ("/", "搜索（↑ ↓ 选择补全或历史，Enter 确认）"),
    ("Ctrl+T", "切换类型：单曲 / 歌手 / 专辑 / 歌单"),
    ("t", "结果页切换类型（保留各类型的位置）"),
    ("Ctrl+X", "搜索框内清除搜索历史"),
    ("", "播放与队列"),
    ("Enter", "播放并替换队列（当前列表已加载的歌曲）"),
    ("A", "下一首播放（多次使用时后加的先播）"),
    ("a", "追加到队尾"),
    ("o", "歌曲操作菜单（播放、入队、喜欢、专辑、歌手）"),
    ("c", "打开播放队列；在队列里定位到当前歌曲"),
    ("d / Delete", "队列：移除选中项"),
    ("J / K", "队列：下移 / 上移选中项"),
    ("D", "队列：清空（需确认）"),
    ("Space", "播放 / 暂停"),
    ("[ / ]", "上一首 / 下一首"),
    (", / .", "后退 / 前进 5 秒"),
    ("- / =", "音量 -/+"),
    ("m", "切换播放模式"),
    ("v", "选择音质"),
    ("f / F", "喜欢当前播放 / 选中的歌曲"),
    ("", "歌词"),
    ("L", "歌词视图"),
    ("j / k", "歌词视图：手动滚动"),
    ("Enter", "歌词视图：跳到选中的那一行"),
    ("z", "歌词视图：恢复自动跟随"),
    ("t", "歌词视图：显示 / 隐藏翻译"),
    ("r", "歌词视图：重新加载歌词"),
    ("", "其他"),
    ("w", "侧栏：歌词 / 队列 / 关闭"),
    ("e", "通知与错误列表"),
    ("x", "关闭底部的错误提示"),
    ("?", "帮助（j / k 滚动）"),
    ("q", "退出界面（音乐继续播放）"),
    ("Q", "退出并停止后台播放"),
];

/// 一批条目。
pub struct Batch {
    pub items: Vec<Item>,
    /// 服务端报告的总数（没有时为 `None`）。
    pub total: Option<usize>,
}

/// 页面打开时的第一批数据。
pub struct Opened {
    pub items: Vec<Item>,
    pub total: Option<usize>,
    /// 还有后续页时用它继续加载。
    pub feed: Option<Feed>,
    pub recommendation: Option<RecommendationSource>,
}

trait FeedSource: Send {
    fn next(&mut self) -> BoxFuture<'_, Option<Result<Batch, Failure>>>;
    fn has_more(&self) -> bool;
}

/// 可继续加载的分页数据。
pub struct Feed(Box<dyn FeedSource>);

impl Feed {
    fn new<T, F>(paged: Paged<T>, map: F) -> Self
    where
        T: FromJson + Send + 'static,
        F: FnMut(T) -> Batch + Send + 'static,
    {
        Self(Box::new(PagerFeed { pager: paged.pager(None), map }))
    }

    /// 取下一批；没有更多时返回 `None`。失败后再次调用会重试同一页。
    pub async fn next(&mut self) -> Option<Result<Batch, Failure>> {
        self.0.next().await
    }

    /// 是否还可能有下一批。
    pub fn has_more(&self) -> bool {
        self.0.has_more()
    }
}

struct PagerFeed<T, F> {
    pager: Pager<T>,
    map: F,
}

impl<T, F> FeedSource for PagerFeed<T, F>
where
    T: FromJson + Send + 'static,
    F: FnMut(T) -> Batch + Send + 'static,
{
    fn next(&mut self) -> BoxFuture<'_, Option<Result<Batch, Failure>>> {
        Box::pin(async move {
            let page = self.pager.next_page().await?;
            Some(page.map(&mut self.map).map_err(Failure::from))
        })
    }

    fn has_more(&self) -> bool {
        self.pager.has_more()
    }
}

/// 把歌曲转成条目，跨批去重。
fn song_batch<T>(
    songs: impl Fn(&T) -> Vec<&Song> + Send + 'static,
    total: impl Fn(&T) -> i64 + Send + 'static,
) -> impl FnMut(T) -> Batch + Send + 'static {
    let mut seen = HashSet::new();
    move |page: T| {
        let items = songs(&page)
            .into_iter()
            .filter(|s| !s.mid.is_empty() && seen.insert(s.mid.clone()))
            .map(|s| Item::Song(Track::from(s)))
            .collect();
        Batch { items, total: usize::try_from(total(&page)).ok().filter(|n| *n > 0) }
    }
}

fn require_login(client: &Client) -> Result<qqmusic_api::Credential, Failure> {
    let credential = client.credential();
    if credential.is_valid() {
        Ok(credential)
    } else {
        Err(Failure::new(ErrorKind::Login, "需要登录：请先在「账号」中登录"))
    }
}

/// 拉取「我喜欢」的全部歌曲 id。
pub async fn liked_ids(client: &Client) -> Result<HashSet<i64>, Failure> {
    let credential = require_login(client)?;
    let pages = client.user().get_fav_song(&credential.encrypt_uin, 1, 100, None).collect(Some(50)).await?;
    Ok(pages.iter().flat_map(|p| p.songs.iter().map(|s| s.id)).collect())
}

/// 搜索补全建议。
pub async fn suggestions(client: &Client, keyword: &str) -> Result<Vec<String>, Failure> {
    let resp = client.search().complete(keyword).await?;
    let mut seen = HashSet::new();
    Ok(resp
        .items
        .into_iter()
        .map(|item| item.hint)
        .filter(|hint| !hint.is_empty() && seen.insert(hint.clone()))
        .take(8)
        .collect())
}

/// 读取搜索历史（最近的在前）。
pub fn load_history(paths: &Paths) -> Vec<String> {
    std::fs::read_to_string(paths.search_history())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// 把 `keyword` 记到历史最前面并保存，返回新的历史。
pub fn remember_search(paths: &Paths, mut history: Vec<String>, keyword: &str) -> Vec<String> {
    history.retain(|k| k != keyword);
    history.insert(0, keyword.to_string());
    history.truncate(HISTORY_LIMIT);
    let _ = serde_json::to_string(&history).map(|text| std::fs::write(paths.search_history(), text));
    history
}

/// 清除搜索历史。
pub fn clear_history(paths: &Paths) -> std::io::Result<()> {
    match std::fs::remove_file(paths.search_history()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// 打开一个远程页面：返回第一批数据，以及继续加载用的 [`Feed`]。
pub async fn open(client: &Client, source: &Source) -> Result<Opened, Failure> {
    let mut items = Vec::new();
    let mut feed = match source {
        Source::Guess | Source::Radar => {
            let source = match source {
                Source::Guess => RecommendationSource::Guess,
                _ => RecommendationSource::Radar { next_page: 1 },
            };
            let batch = crate::recommendation::load(client, source)
                .await
                .map_err(|e| Failure::new(ErrorKind::Network, crate::redact(&e)))?;
            return Ok(Opened {
                items: batch.tracks.into_iter().map(Item::Song).collect(),
                total: None,
                feed: None,
                recommendation: Some(batch.next),
            });
        }
        Source::Liked => {
            let credential = require_login(client)?;
            Some(Feed::new(
                client.user().get_fav_song(&credential.encrypt_uin, 1, 100, None),
                song_batch(
                    |p: &qqmusic_api::models::songlist::GetSonglistDetailResponse| p.songs.iter().collect(),
                    |p| p.total,
                ),
            ))
        }
        Source::MyPlaylists => {
            let credential = require_login(client)?;
            let created = client.user().get_created_songlist(credential.musicid, None).await?;
            items.push(Item::Text("— 创建的歌单 —".into()));
            items.extend(created.playlists.iter().filter(|p| p.songlist.id != 0).map(|p| {
                link(&p.songlist.title, format!("{} 首", p.songlist.songnum), Source::Playlist(p.songlist.id))
            }));
            items.push(Item::Text("— 收藏的歌单 —".into()));
            Some(Feed::new(client.user().get_fav_songlist(&credential.encrypt_uin, 1, 100, None), |page| Batch {
                items: page
                    .playlists
                    .iter()
                    .map(|p| {
                        link(
                            &p.songlist.title,
                            format!("{} 首 · {}", p.songlist.songnum, p.nickname),
                            Source::Playlist(p.songlist.id),
                        )
                    })
                    .collect(),
                total: None,
            }))
        }
        Source::RecommendPlaylists => Some(Feed::new(client.recommend().get_recommend_songlist(1, 40), |page| Batch {
            items: page.songlists.iter().map(|p| link(&p.title, "", Source::Playlist(p.id))).collect(),
            total: None,
        })),
        Source::Toplists => {
            let resp = client.top().get_category().await?;
            for group in &resp.group {
                items.push(Item::Text(format!("— {} —", group.name)));
                items.extend(group.toplist.iter().map(|t| link(&t.name, &t.update_time, Source::Toplist(t.id))));
            }
            None
        }
        Source::NewSongs => {
            let resp = client.recommend().get_recommend_newsong(5).await?;
            items = song_batch(|songs: &Vec<Song>| songs.iter().collect(), |_| 0)(resp.songs).items;
            None
        }
        Source::Search { keyword, kind } => {
            let kind = *kind;
            let options = SearchOptions {
                search_type: kind.search_type(),
                num: SEARCH_PAGE,
                highlight: false,
                ..Default::default()
            };
            let mut songs = song_batch(
                |p: &qqmusic_api::models::search::SearchByTypeResponse| p.song.iter().map(|s| &s.song).collect(),
                |p| p.total_num,
            );
            Some(Feed::new(client.search().search_by_type_with(keyword, options), move |page| {
                let total = usize::try_from(page.total_num).ok().filter(|n| *n > 0);
                let items = match kind {
                    SearchKind::Song => return songs(page),
                    SearchKind::Singer => page
                        .singer
                        .iter()
                        .filter(|s| !s.singer.mid.is_empty())
                        .map(|s| {
                            link(
                                &s.singer.name,
                                format!("{} 首 · {} 张专辑", s.song_num, s.album_num),
                                Source::Singer(s.singer.mid.clone()),
                            )
                        })
                        .collect(),
                    SearchKind::Album => page
                        .album
                        .iter()
                        .map(|a| {
                            let sub = [a.singer.as_str(), a.album.time_public.as_str()]
                                .into_iter()
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                                .join(" · ");
                            link(&a.album.name, sub, Source::Album(a.album.mid.clone()))
                        })
                        .collect(),
                    SearchKind::Playlist => page
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
                };
                Batch { items, total }
            }))
        }
        Source::Playlist(id) => {
            let options =
                SonglistDetailOptions { num: 100, onlysong: true, tag: false, userinfo: false, ..Default::default() };
            Some(Feed::new(
                client.songlist().get_detail_with(*id, options),
                song_batch(
                    |p: &qqmusic_api::models::songlist::GetSonglistDetailResponse| p.songs.iter().collect(),
                    |p| p.total,
                ),
            ))
        }
        Source::Album(mid) => Some(Feed::new(
            client.album().get_song(mid.as_str(), 100, 1),
            song_batch(
                |p: &qqmusic_api::models::album::GetAlbumSongResponse| p.song_list.iter().collect(),
                |p| p.total_num,
            ),
        )),
        Source::Toplist(id) => Some(Feed::new(
            client.top().get_detail(*id, 100, 1, false),
            song_batch(
                |p: &qqmusic_api::models::top::TopDetailResponse| p.songs.iter().collect(),
                |p| p.info.total_num,
            ),
        )),
        Source::Singer(mid) => {
            items.push(link("全部专辑", "", Source::SingerAlbums(mid.clone())));
            Some(Feed::new(
                client.singer().get_songs_list(mid, 50, 1, OrderType::Hot),
                song_batch(
                    |p: &qqmusic_api::models::singer::SingerSongListResponse| p.song_list.iter().collect(),
                    |p| p.total_num,
                ),
            ))
        }
        Source::SingerAlbums(mid) => {
            Some(Feed::new(client.singer().get_album_list(mid, 50, 1, OrderType::Latest, None), |page| Batch {
                total: usize::try_from(page.total).ok().filter(|n| *n > 0),
                items: page
                    .album_list
                    .iter()
                    .map(|a| link(&a.name, &a.time_public, Source::Album(a.mid.clone())))
                    .collect(),
            }))
        }
        Source::Root | Source::Queue | Source::Lyrics | Source::Account => None,
    };
    let mut total = None;
    if let Some(feed) = &mut feed
        && let Some(batch) = feed.next().await
    {
        let batch = batch?;
        items.extend(batch.items);
        total = batch.total;
    }
    Ok(Opened { items, total, feed: feed.filter(Feed::has_more), recommendation: None })
}
