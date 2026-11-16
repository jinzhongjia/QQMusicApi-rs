//! 界面渲染。
//!
//! ```text
//! 顶栏：应用名 · 账号状态 · 未读错误数 · 连接状态
//! 导航（宽屏） │ 主内容：标题、状态行、单列列表 │ 侧栏（更宽时）：歌词或队列
//! 播放栏：状态与歌曲 / 进度条 / 提示或通知
//! ```
//!
//! 窄屏只保留主内容与播放栏，首页就是菜单；列表行数由可用高度决定。
//! 选中项用 `›`、正在播放用 `♪` 标出，不只依赖颜色。

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Wrap};

use super::app::{App, Focus, Level, LyricState, Menu, Mode, More, Page, SearchBox, Side, TOAST_SECS, Viewport};
use super::data::{self, Item, SearchKind, Source};
use super::input::{truncate, width, wrap};
use super::lyric;
use crate::protocol::{Activity, PlayMode, Status, Track, quality_label};

const ACCENT: Color = Color::Rgb(49, 194, 124);
/// 进度条渐变终点色。
const ACCENT_END: Color = Color::Rgb(0, 168, 214);
const DIM: Color = Color::DarkGray;
const LIKED: Color = Color::Rgb(234, 64, 63);
/// 显示左侧导航所需的最小宽度。
const NAV_MIN_WIDTH: u16 = 100;
/// 显示右侧栏所需的最小宽度。
const SIDE_MIN_WIDTH: u16 = 120;
const NAV_WIDTH: u16 = 16;
const SIDE_WIDTH: u16 = 30;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    // 高度不够时先保播放栏，再保主内容，最后才是顶栏。
    let bar_height = area.height.min(3);
    let header_height = u16::from(area.height > bar_height + 1);
    let [header, body, bar] =
        Layout::vertical([Constraint::Length(header_height), Constraint::Min(0), Constraint::Length(bar_height)])
            .areas(area);
    let nav = area.width >= NAV_MIN_WIDTH && body.height >= 3;
    let side = area.width >= SIDE_MIN_WIDTH && body.height >= 3 && app.side != Side::Off;
    let [nav_area, content, side_area] = Layout::horizontal([
        Constraint::Length(if nav { NAV_WIDTH } else { 0 }),
        Constraint::Min(0),
        Constraint::Length(if side { SIDE_WIDTH } else { 0 }),
    ])
    .areas(body);
    // 先写入布局，下面取焦点时就已经按本帧的布局归一化了。
    app.viewport.set(Viewport { nav, side, rows: app.viewport.get().rows });
    let focus = app.focus();

    draw_header(f, app, header);
    if nav {
        draw_nav(f, app, nav_area, focus == Focus::Nav);
    }
    let rows = draw_content(f, app, content, focus == Focus::Content, nav);
    if side {
        draw_side(f, app, side_area, focus == Focus::Side);
    }
    app.viewport.set(Viewport { nav, side, rows: rows.max(1) });
    draw_bar(f, app, bar);

    match &app.mode {
        Mode::Normal => {}
        Mode::Search(search) => draw_search(f, search),
        Mode::Menu(menu) => draw_menu(f, menu),
        Mode::Help { scroll } => {
            let lines = data::HELP
                .iter()
                .map(|(key, desc)| {
                    if key.is_empty() {
                        Line::from(Span::styled(format!("— {desc} —"), Style::new().fg(DIM)))
                    } else {
                        Line::from(vec![Span::styled(format!(" {key:<18}"), Style::new().fg(ACCENT)), Span::raw(*desc)])
                    }
                })
                .collect();
            draw_scroll(f, app, " 帮助 ", lines, *scroll, 62);
        }
        Mode::Notices { scroll } => {
            let inner = f.area().width.saturating_sub(6).clamp(10, 76) as usize;
            let mut lines = Vec::new();
            for notice in &app.notices {
                let (mark, style) = match notice.level {
                    Level::Error => ("! ", Style::new().fg(Color::Red)),
                    Level::Info => ("· ", Style::new()),
                };
                let text = format!("{}  {}", ago(notice.at.elapsed().as_secs()), notice.text);
                for (i, part) in wrap(&text, inner.saturating_sub(2)).into_iter().enumerate() {
                    let prefix = if i == 0 { mark } else { "  " };
                    lines.push(Line::from(Span::styled(format!("{prefix}{part}"), style)));
                }
            }
            if lines.is_empty() {
                lines.push(Line::from("暂无通知").fg(DIM));
            }
            draw_scroll(f, app, " 通知与错误 ", lines, *scroll, inner as u16 + 2);
        }
        Mode::Login(view) => {
            let title = match view.kind {
                qqmusic_api::models::login::QrLoginType::Qq => " QQ 扫码登录 ",
                qqmusic_api::models::login::QrLoginType::Wx => " 微信扫码登录 ",
                qqmusic_api::models::login::QrLoginType::Mobile => " QQ 音乐 App 扫码登录 ",
            };
            let mut lines = view.qr.clone();
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(view.status.clone()).fg(ACCENT));
            if let Some(path) = &view.path {
                lines.push(Line::from(format!("二维码图片: {path}")).fg(DIM));
            }
            lines.push(Line::from("Esc 取消 · r 重新获取").fg(DIM));
            // 按最宽的一行定宽，超出屏幕的行（长提示、长路径）折行显示。
            let content = lines.iter().map(|l| l.width() as u16).max().unwrap_or(0);
            let inner = content.max(44).min(f.area().width.saturating_sub(4)).max(1);
            let height = lines.iter().map(|l| (l.width() as u16).div_ceil(inner).max(1)).sum::<u16>() + 2;
            // 二维码很大，只清弹窗区域会在两侧露出半截列表，干脆清掉整屏。
            f.render_widget(Clear, f.area());
            let area = centered(f.area(), inner + 4, height);
            let block = popup(title.into()).padding(Padding::horizontal(1));
            let paragraph = Paragraph::new(lines).alignment(Alignment::Center).wrap(Wrap { trim: false });
            f.render_widget(paragraph.block(block), area);
        }
    }
}

fn popup(title: String) -> Block<'static> {
    Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().fg(ACCENT)).title(title)
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))]).flex(Flex::Center).areas(area);
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))]).flex(Flex::Center).areas(area);
    area
}

/// `area` 内第 `y` 行（超出时为空矩形）。
fn row(area: Rect, y: u16) -> Rect {
    if y >= area.height {
        return Rect { x: area.x, y: area.y, width: 0, height: 0 };
    }
    Rect { x: area.x, y: area.y + y, width: area.width, height: 1 }
}

/// 截断并用空格补到恰好 `cols` 列。
fn cell(text: &str, cols: usize) -> String {
    let mut out = truncate(text, cols);
    let pad = cols.saturating_sub(width(&out));
    out.extend(std::iter::repeat_n(' ', pad));
    out
}

fn mmss(secs: u64) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn ago(secs: u64) -> String {
    match secs {
        0..60 => "刚刚".into(),
        60..3600 => format!("{} 分钟前", secs / 60),
        _ => format!("{} 小时前", secs / 3600),
    }
}

/// 调整滚动位置使选中项可见，返回新的起始下标。
fn scroll_to(offset: &std::cell::Cell<usize>, selected: usize, rows: usize, len: usize) -> usize {
    let rows = rows.max(1);
    let mut start = offset.get().min(len.saturating_sub(rows));
    if selected < start {
        start = selected;
    } else if selected >= start + rows {
        start = selected + 1 - rows;
    }
    offset.set(start);
    start
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let mut right = vec![Span::styled(app.account_label(), Style::new().fg(DIM))];
    if app.unread > 0 {
        right.push(Span::styled(format!("  ! {} 条错误（e 查看）", app.unread), Style::new().fg(Color::Red)));
    }
    if !app.connected {
        right.push(Span::styled("  ○ 后台未连接", Style::new().fg(Color::Yellow)));
    }
    let right = Line::from(right);
    let right_width = (right.width() as u16).min(area.width);
    let [left_area, right_area] = Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).areas(area);
    let left = Line::from(Span::styled(" QQ 音乐 ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)));
    f.render_widget(Paragraph::new(left), left_area);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), right_area);
}

/// 区域标题：有焦点时用 `▍` 和强调色标出。
fn pane_title(text: String, focused: bool) -> Line<'static> {
    if focused {
        Line::from(vec![
            Span::styled("▍", Style::new().fg(ACCENT)),
            Span::styled(text, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(vec![Span::raw(" "), Span::styled(text, Style::new().add_modifier(Modifier::BOLD))])
    }
}

fn cursor(selected: bool) -> &'static str {
    if selected { "› " } else { "  " }
}

fn row_style(selected: bool, focused: bool) -> Style {
    match (selected, focused) {
        (true, true) => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        (true, false) => Style::new().add_modifier(Modifier::BOLD),
        _ => Style::new(),
    }
}

fn draw_nav(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let block = Block::new().borders(Borders::RIGHT).border_style(Style::new().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(pane_title("导航".into(), focused)), row(inner, 0));
    let root = app.root();
    let list = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
    let start = scroll_to(&root.offset, root.selected, list.height as usize, root.items.len());
    // 导航里标出主内容区当前属于哪一项。
    let open = app.stack.get(1).map(|p| &p.source);
    for (i, item) in root.items.iter().enumerate().skip(start).take(list.height as usize) {
        let (title, source) = match item {
            Item::Link { title, source, .. } => (title.as_str(), Some(source)),
            Item::Action { title, .. } => (title.as_str(), None),
            _ => continue,
        };
        let selected = i == root.selected;
        let mark = if source.is_some() && source == open { "•" } else { " " };
        let text = format!("{}{mark}{title}", cursor(selected));
        let line = Line::from(Span::styled(text, row_style(selected, focused)));
        f.render_widget(Paragraph::new(line), row(list, (i - start) as u16));
    }
}

/// 主内容区，返回列表一屏的行数。
fn draw_content(f: &mut Frame, app: &App, area: Rect, focused: bool, nav: bool) -> usize {
    if area.height == 0 || area.width == 0 {
        return 0;
    }
    let page = app.page();
    // 宽屏停在首页：导航已经在左边，这里显示当前播放。
    if page.source == Source::Root && nav {
        f.render_widget(Paragraph::new(pane_title("当前播放".into(), false)), row(area, 0));
        let body = Rect { y: area.y + 1, height: area.height - 1, ..area };
        return draw_lyrics(f, app, body, false, true);
    }
    match page.source {
        Source::Queue => {
            f.render_widget(Paragraph::new(pane_title(queue_title(app), focused)), row(area, 0));
            let body = Rect { y: area.y + 1, height: area.height - 1, ..area };
            draw_queue(f, app, body, focused)
        }
        Source::Lyrics => {
            f.render_widget(Paragraph::new(pane_title(page.title.clone(), focused)), row(area, 0));
            let body = Rect { y: area.y + 1, height: area.height - 1, ..area };
            draw_lyrics(f, app, body, focused, true)
        }
        _ => draw_page(f, app, page, area, focused),
    }
}

/// 页面状态行：已加载数量、服务端总量、续页状态。
fn page_status(page: &Page) -> Line<'static> {
    let mut spans = Vec::new();
    if let Source::Search { kind, .. } = &page.source {
        for k in SearchKind::ALL {
            let style =
                if k == *kind { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().fg(DIM) };
            let label = if k == *kind { format!("[{}]", k.label()) } else { format!(" {} ", k.label()) };
            spans.push(Span::styled(label, style));
        }
        spans.push(Span::styled(" t 切换  ", Style::new().fg(DIM)));
    }
    let songs = page.song_count();
    let (count, unit) = if songs > 0 {
        (songs, "首")
    } else {
        (page.items.iter().filter(|i| !matches!(i, Item::Text(_))).count(), "项")
    };
    let loaded = match page.total {
        _ if !page.source.is_remote() => String::new(),
        _ if page.loading && page.items.is_empty() => "加载中…".into(),
        Some(total) if page.incomplete() || total > count => format!("已加载 {count} / 共 {total} {unit}"),
        _ if page.incomplete() => format!("已加载 {count} {unit}，还有更多"),
        _ => format!("共 {count} {unit}"),
    };
    spans.push(Span::styled(loaded, Style::new().fg(DIM)));
    if page.loading && !page.items.is_empty() {
        spans.push(Span::styled(" · 刷新中…", Style::new().fg(DIM)));
    }
    match &page.more {
        More::Loading => spans.push(Span::styled(" · 正在加载更多…", Style::new().fg(DIM))),
        More::Failed(e) => {
            spans.push(Span::styled(format!(" · 加载更多失败（r 重试）: {e}"), Style::new().fg(Color::Red)));
        }
        _ if page.capped => {
            let text = format!(" · 已达上限 {} 条，其余没有加载", data::MAX_ITEMS);
            spans.push(Span::styled(text, Style::new().fg(Color::Yellow)));
        }
        _ => {}
    }
    if let Some(error) = &page.error
        && !page.items.is_empty()
    {
        spans
            .push(Span::styled(format!(" · 刷新失败，显示的是旧内容（r 重试）: {error}"), Style::new().fg(Color::Red)));
    }
    Line::from(spans)
}

fn draw_page(f: &mut Frame, app: &App, page: &Page, area: Rect, focused: bool) -> usize {
    f.render_widget(Paragraph::new(pane_title(page.title.clone(), focused)), row(area, 0));
    let mut top = 1;
    if area.height >= 4 {
        let status = Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..row(area, 1) };
        f.render_widget(Paragraph::new(page_status(page)), status);
        top = 2;
    }
    // 没有侧栏时在列表下方留一行当前歌词。
    let strip = !app.viewport.get().side && area.height >= 14 && !app.lyrics().is_empty();
    let list_height = area.height.saturating_sub(top + if strip { 2 } else { 0 });
    let list = Rect { y: area.y + top, height: list_height, ..area };
    if strip {
        draw_lyric_strip(f, app, row(area, area.height - 1));
    }

    if page.items.is_empty() {
        let (text, style) = match &page.error {
            Some(error) => (format!("加载失败: {error}（r 重试）"), Style::new().fg(Color::Red)),
            None if page.loading => ("加载中…".to_string(), Style::new().fg(DIM)),
            None if matches!(page.source, Source::Search { .. }) => {
                ("没有找到相关结果，按 t 换个类型或按 / 重新搜索".to_string(), Style::new().fg(DIM))
            }
            None => ("暂无内容".to_string(), Style::new().fg(DIM)),
        };
        let hint = Rect { x: list.x + 2, width: list.width.saturating_sub(2), ..list };
        f.render_widget(Paragraph::new(text).style(style).wrap(Wrap { trim: false }), hint);
        return list.height as usize;
    }

    let rows = list.height as usize;
    let start = scroll_to(&page.offset, page.selected, rows, page.items.len());
    let columns = Columns::new(list.width as usize, page.items.len());
    let playing = app.state.track.as_ref().map(|t| t.mid.as_str());
    for (i, item) in page.items.iter().enumerate().skip(start).take(rows) {
        let selected = i == page.selected;
        let line = match item {
            Item::Song(track) => {
                let marks = Marks { playing: playing == Some(track.mid.as_str()), next: false };
                columns.song(app, track, i, selected, focused, marks)
            }
            Item::Link { title, sub, .. } => {
                let style = row_style(selected, focused);
                let mut spans = vec![Span::styled(format!("{}  {title}", cursor(selected)), style)];
                if !sub.is_empty() {
                    spans.push(Span::styled(format!("  {sub}"), Style::new().fg(DIM)));
                }
                Line::from(spans)
            }
            Item::Action { title, .. } => {
                Line::from(Span::styled(format!("{}  {title}", cursor(selected)), row_style(selected, focused)))
            }
            Item::Text(text) => Line::from(Span::styled(format!("    {text}"), Style::new().fg(DIM))),
        };
        f.render_widget(Paragraph::new(line), row(list, (i - start) as u16));
    }
    rows
}

/// 歌曲行前的标记。
struct Marks {
    playing: bool,
    /// 队列里确定的下一首。
    next: bool,
}

/// 歌曲行的列宽：窄屏只留歌名与歌手，变宽后依次加上时长、序号、专辑。
struct Columns {
    index: usize,
    title: usize,
    singer: usize,
    album: usize,
    duration: usize,
    flags: usize,
}

impl Columns {
    fn new(total: usize, len: usize) -> Self {
        let index = if total >= 70 { len.to_string().len() + 1 } else { 0 };
        let singer = if total >= 30 { total * 26 / 100 } else { 0 };
        let album = if total >= 72 { total * 22 / 100 } else { 0 };
        let duration = if total >= 56 { 6 } else { 0 };
        let flags = if total >= 56 { 6 } else { 2 };
        // 光标 2 列 + 播放标记 2 列。
        let title = total.saturating_sub(4 + index + singer + album + duration + flags);
        Self { index, title, singer, album, duration, flags }
    }

    fn song(&self, app: &App, track: &Track, i: usize, selected: bool, focused: bool, marks: Marks) -> Line<'static> {
        let base = row_style(selected, focused);
        let title_style = if marks.playing && !selected { Style::new().fg(ACCENT) } else { base };
        let dim = if selected { base } else { Style::new().fg(DIM) };
        let mark = match (marks.playing, marks.next) {
            (true, _) => "♪ ",
            (_, true) => "» ",
            _ => "  ",
        };
        let mut spans = vec![Span::styled(cursor(selected), base), Span::styled(mark, Style::new().fg(ACCENT))];
        if self.index > 0 {
            spans.push(Span::styled(format!("{:>w$} ", i + 1, w = self.index - 1), dim));
        }
        spans.push(Span::styled(cell(&track.name, self.title), title_style));
        if self.singer > 0 {
            spans.push(Span::styled(format!(" {}", cell(&track.singers, self.singer - 1)), dim));
        }
        if self.album > 0 {
            spans.push(Span::styled(format!(" {}", cell(&track.album, self.album - 1)), dim));
        }
        if self.duration > 0 {
            spans.push(Span::styled(format!(" {}", mmss(track.duration)), dim));
        }
        let liked = app.liked.contains(&track.id);
        spans.push(Span::styled(if liked { " ♥" } else { "  " }, Style::new().fg(LIKED)));
        if self.flags > 2 {
            spans.push(Span::styled(if track.vip { " VIP" } else { "    " }, Style::new().fg(Color::Yellow)));
        }
        Line::from(spans)
    }
}

/// 「接下来」的说明：只说确定的事，随机挑选时不编造下一首。
fn up_next(app: &App) -> Option<String> {
    let state = &app.state;
    state.index?;
    if state.protocol < 2 {
        return None;
    }
    Some(match state.next {
        Some(next) if Some(next) == state.index => "接下来：重复当前歌曲（单曲循环）".into(),
        Some(next) => format!("接下来：{}", app.queue.get(next)?.name),
        None if state.mode == PlayMode::Shuffle => "接下来：随机挑选".into(),
        None => "接下来：没有了，播完即停".into(),
    })
}

fn queue_title(app: &App) -> String {
    let mut title = format!("播放队列 · {} 首 · {}", app.queue.len(), app.state.mode.label());
    if let Some(next) = up_next(app) {
        title.push_str(&format!(" · {next}"));
    }
    title
}

fn draw_queue(f: &mut Frame, app: &App, area: Rect, focused: bool) -> usize {
    if app.queue.is_empty() {
        let hint = Rect { x: area.x + 2, width: area.width.saturating_sub(2), ..area };
        let text = "播放队列为空：在歌曲上按 Enter 播放列表，或按 a 追加到队尾";
        f.render_widget(Paragraph::new(text).fg(DIM).wrap(Wrap { trim: false }), hint);
        return area.height as usize;
    }
    let rows = area.height as usize;
    let start = scroll_to(&app.queue_offset, app.queue_sel, rows, app.queue.len());
    let columns = Columns::new(area.width as usize, app.queue.len());
    for (i, track) in app.queue.iter().enumerate().skip(start).take(rows) {
        let playing = app.state.index == Some(i);
        let marks = Marks { playing, next: !playing && app.state.next == Some(i) };
        let line = columns.song(app, track, i, i == app.queue_sel, focused, marks);
        f.render_widget(Paragraph::new(line), row(area, (i - start) as u16));
    }
    rows
}

fn draw_side(f: &mut Frame, app: &App, area: Rect, focused: bool) {
    let block = Block::new().borders(Borders::LEFT).border_style(Style::new().fg(DIM));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let title = match app.side {
        Side::Queue => format!("队列 · {} 首", app.queue.len()),
        _ => "歌词".to_string(),
    };
    f.render_widget(Paragraph::new(pane_title(format!("{title}（w 切换）"), focused)), row(inner, 0));
    let body = Rect { y: inner.y + 1, height: inner.height.saturating_sub(1), ..inner };
    match app.side {
        Side::Queue => {
            draw_queue(f, app, body, focused);
        }
        _ => {
            draw_lyrics(f, app, body, focused, false);
        }
    }
}

/// 歌词视图；`heading` 为真时先显示歌名与歌手。返回一屏的行数。
fn draw_lyrics(f: &mut Frame, app: &App, area: Rect, focused: bool, heading: bool) -> usize {
    let mut area = Rect { x: area.x + 1, width: area.width.saturating_sub(2), ..area };
    if area.height == 0 || area.width == 0 {
        return 0;
    }
    if heading
        && area.height >= 6
        && let Some(track) = &app.state.track
    {
        let name = Line::from(Span::styled(track.name.clone(), Style::new().add_modifier(Modifier::BOLD)));
        f.render_widget(Paragraph::new(name), row(area, 0));
        let sub = [track.singers.as_str(), track.album.as_str()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        f.render_widget(Paragraph::new(sub).fg(DIM), row(area, 1));
        area = Rect { y: area.y + 3, height: area.height - 3, ..area };
    }
    let message = match &app.lyric {
        LyricState::Ready(_) => None,
        LyricState::Idle => Some(("没有正在播放的歌曲".to_string(), DIM)),
        LyricState::Loading => Some(("歌词加载中…".to_string(), DIM)),
        LyricState::Empty => Some(("这首歌没有歌词".to_string(), DIM)),
        LyricState::Failed(e) => Some((format!("歌词加载失败: {e}（按 L 进入歌词视图后按 r 重试）"), Color::Red)),
    };
    if let Some((text, color)) = message {
        f.render_widget(Paragraph::new(text).fg(color).wrap(Wrap { trim: false }), area);
        return area.height as usize;
    }

    let lines = app.lyrics();
    let current = lyric::current(lines, app.position_ms());
    let manual = app.lyric_sel.filter(|_| focused);
    // 手动滚动时底部留一行说明如何恢复跟随。
    let hint = manual.is_some() && area.height >= 4;
    let rows = (area.height as usize).saturating_sub(usize::from(hint));
    let text_width = (area.width as usize).saturating_sub(2);
    // 长行折成多行；翻译紧跟在原文下面。
    let mut out: Vec<Line> = Vec::new();
    let mut anchor = 0;
    for (i, line) in lines.iter().enumerate() {
        let is_current = current == Some(i);
        let is_selected = manual == Some(i);
        if manual.or(current) == Some(i) {
            anchor = out.len();
        }
        let style =
            if is_current { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().fg(DIM) };
        let prefix = match (is_selected, is_current) {
            (true, _) => "› ",
            (_, true) => "♪ ",
            _ => "  ",
        };
        for (n, part) in wrap(&line.text, text_width).into_iter().enumerate() {
            let prefix = if n == 0 { prefix } else { "  " };
            out.push(Line::from(vec![Span::styled(prefix, Style::new().fg(ACCENT)), Span::styled(part, style)]));
        }
        if app.show_trans
            && let Some(trans) = &line.trans
        {
            let style = if is_current { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
            for part in wrap(trans, text_width) {
                out.push(Line::from(Span::styled(format!("  {part}"), style)));
            }
        }
    }
    let start = anchor.saturating_sub(rows / 2).min(out.len().saturating_sub(rows));
    for (y, line) in out.into_iter().skip(start).take(rows).enumerate() {
        f.render_widget(Paragraph::new(line), row(area, y as u16));
    }
    if hint {
        let text = "手动滚动中 · Enter 跳到这一行 · z 恢复跟随";
        f.render_widget(Paragraph::new(text).fg(Color::Yellow), row(area, area.height - 1));
    }
    rows
}

/// 列表下方的一行当前歌词。
fn draw_lyric_strip(f: &mut Frame, app: &App, area: Rect) {
    let lines = app.lyrics();
    let Some(line) = lyric::current(lines, app.position_ms()).and_then(|i| lines.get(i)) else { return };
    let mut spans =
        vec![Span::styled("  ♪ ", Style::new().fg(ACCENT)), Span::styled(line.text.clone(), Style::new().fg(ACCENT))];
    if app.show_trans
        && let Some(trans) = &line.trans
    {
        spans.push(Span::styled(format!("  {trans}"), Style::new().fg(DIM)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 播放状态的符号与文字：暂停、停止、加载、缓冲各不相同。
fn status_label(app: &App) -> (&'static str, Color) {
    let state = &app.state;
    if !app.connected {
        return ("○ 未连接", Color::Yellow);
    }
    match (state.status, state.activity) {
        (Status::Stopped, _) => ("■ 已停止", DIM),
        (Status::Paused, Activity::Loading) => ("‖ 已暂停（加载中）", Color::Yellow),
        (Status::Paused, _) => ("‖ 已暂停", Color::Yellow),
        (_, Activity::Buffering) => ("… 缓冲中", Color::Yellow),
        (Status::Loading, _) => ("… 加载中", Color::Yellow),
        (Status::Playing, _) => ("▶ 播放中", ACCENT),
    }
}

fn draw_bar(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let state = &app.state;
    // 第一行：状态、歌曲；右侧：模式、音量、音质。
    let quality = match state.playing_quality {
        Some(actual) if actual != state.quality => {
            format!("{}（期望{}，已降级）", quality_label(actual), quality_label(state.quality))
        }
        Some(actual) => quality_label(actual).to_string(),
        None => quality_label(state.quality).to_string(),
    };
    let right = format!("{} · 音量 {}% · {quality} ", state.mode.label(), state.volume);
    let (label, color) = status_label(app);
    let mut left = vec![Span::styled(format!(" {label}  "), Style::new().fg(color))];
    if let Some(track) = &state.track {
        if app.liked.contains(&track.id) {
            left.push(Span::styled("♥ ", Style::new().fg(LIKED)));
        }
        left.push(Span::styled(track.name.clone(), Style::new().add_modifier(Modifier::BOLD)));
        if !track.singers.is_empty() {
            left.push(Span::styled(format!(" - {}", track.singers), Style::new().fg(DIM)));
        }
    } else {
        left.push(Span::styled("没有正在播放的歌曲", Style::new().fg(DIM)));
    }
    // 窄屏优先保证状态和歌名，右侧信息放不下就不显示。
    let left_width = Line::from(left.clone()).width();
    let right_width = if left_width + width(&right) + 2 <= area.width as usize { width(&right) as u16 } else { 0 };
    let [left_area, right_area] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(right_width)]).areas(row(area, 0));
    f.render_widget(Paragraph::new(Line::from(left)), left_area);
    f.render_widget(Paragraph::new(right).fg(DIM).alignment(Alignment::Right), right_area);

    // 第二行：进度。
    if area.height >= 2 {
        draw_progress(f, app, row(area, 1));
    }

    // 第三行：通知优先，其次是按键提示。
    if area.height >= 3 {
        let line = row(area, 2);
        let toast = app.toast.as_ref().filter(|(_, at)| at.elapsed().as_secs() < TOAST_SECS);
        let (text, style) = match (toast, &app.alert) {
            (Some((message, _)), _) => (format!(" {message}"), Style::new().fg(Color::Yellow)),
            (None, Some(alert)) => (format!(" ! {alert}（x 关闭 · e 详情）"), Style::new().fg(Color::Red)),
            (None, None) if !app.connected => (" 正在连接后台播放进程…".to_string(), Style::new().fg(Color::Yellow)),
            (None, None) => (format!(" {}", hints(app)), Style::new().fg(DIM)),
        };
        let next = up_next(app).filter(|_| toast.is_none() && app.alert.is_none()).unwrap_or_default();
        let next_width =
            if width(&text) + width(&next) + 3 <= line.width as usize { width(&next) as u16 + 1 } else { 0 };
        let [text_area, next_area] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(next_width)]).areas(line);
        f.render_widget(Paragraph::new(truncate(&text, text_area.width as usize)).style(style), text_area);
        f.render_widget(Paragraph::new(format!("{next} ")).fg(DIM).alignment(Alignment::Right), next_area);
    }
}

/// 与当前视图相关的按键提示。
fn hints(app: &App) -> &'static str {
    use super::app::View;
    match app.view() {
        View::Nav => "Enter 打开 · Tab 切换区域 · / 搜索 · ? 帮助",
        View::Queue => "Enter 播放 · d 移除 · J/K 移动 · D 清空 · c 定位当前 · ? 帮助",
        View::Lyrics => "j/k 滚动 · Enter 跳转 · z 跟随 · t 翻译 · r 重试 · ? 帮助",
        View::List => match app.page().items.get(app.page().selected) {
            Some(Item::Song(_)) => "Enter 播放并替换队列 · A 下一首播放 · a 追加 · o 菜单 · F 喜欢 · ? 帮助",
            _ => "Enter 进入 · Esc 返回 · / 搜索 · c 队列 · L 歌词 · ? 帮助",
        },
    }
}

/// 渐变进度条：`━━━━━─────  01:23 / 04:56`。
fn draw_progress(f: &mut Frame, app: &App, area: Rect) {
    let duration = app.state.duration_ms;
    if app.state.track.is_none() || duration == 0 || area.width < 16 {
        return;
    }
    let position = app.position_ms();
    let times = format!(" {} / {} ", mmss(position / 1000), mmss(duration / 1000));
    let bar = (area.width as usize).saturating_sub(width(&times) + 1);
    let filled = ((bar as f64) * (position as f64 / duration as f64).clamp(0.0, 1.0)).round() as usize;
    let mut spans = vec![Span::raw(" ")];
    spans.extend((0..filled).map(|i| Span::styled("━", Style::new().fg(gradient(i, bar)))));
    spans.push(Span::styled("─".repeat(bar - filled), Style::new().fg(DIM)));
    spans.push(Span::styled(times, Style::new().fg(ACCENT)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn gradient(i: usize, width: usize) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (ACCENT, ACCENT_END) else { return ACCENT };
    let t = if width > 1 { i as f32 / (width - 1) as f32 } else { 0.0 };
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
    Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
}

fn draw_search(f: &mut Frame, search: &SearchBox) {
    let options = search.options.len().min(8) as u16;
    let heading = u16::from(options > 0);
    let area = centered(f.area(), 64, 3 + heading + options);
    f.render_widget(Clear, area);
    let mut title = vec![Span::raw(" 搜索 ")];
    for kind in SearchKind::ALL {
        title.push(if kind == search.kind {
            Span::styled(format!("[{}]", kind.label()), Style::new().add_modifier(Modifier::BOLD))
        } else {
            Span::styled(format!(" {} ", kind.label()), Style::new().fg(DIM))
        });
    }
    title.push(Span::styled(" Tab 切换 ", Style::new().fg(DIM)));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Line::from(title));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height == 0 || inner.width < 3 {
        return;
    }
    // 输入行：过长时横向滚动，光标始终可见。
    let (text, column) = search.input.visible(inner.width as usize - 2);
    f.render_widget(Paragraph::new(format!("> {text}")), row(inner, 0));
    f.set_cursor_position(Position::new(inner.x + 2 + column as u16, inner.y));
    if options == 0 {
        return;
    }
    let label =
        if search.from_history { "历史（↑ ↓ 选择 · Ctrl+X 清除）" } else { "建议（↑ ↓ 选择）" };
    f.render_widget(Paragraph::new(label).fg(DIM), row(inner, 1));
    for (i, option) in search.options.iter().take(8).enumerate() {
        let picked = search.picked == Some(i);
        let text = format!("{}{}", cursor(picked), truncate(option, inner.width as usize - 2));
        f.render_widget(Paragraph::new(text).style(row_style(picked, true)), row(inner, 2 + i as u16));
    }
}

fn draw_menu(f: &mut Frame, menu: &Menu) {
    let content = menu.items.iter().map(|(label, _)| width(label)).max().unwrap_or(0).max(width(&menu.title));
    let area = centered(f.area(), content as u16 + 6, menu.items.len() as u16 + 2);
    f.render_widget(Clear, area);
    let block = popup(truncate(&menu.title, area.width.saturating_sub(2) as usize));
    let inner = block.inner(area);
    f.render_widget(block, area);
    // 项目多于可见行数时跟着选中项滚动。
    let rows = inner.height as usize;
    let start = (menu.selected + 1).saturating_sub(rows);
    for (i, (label, _)) in menu.items.iter().enumerate().skip(start).take(rows) {
        let selected = i == menu.selected;
        let text = format!("{}{}", cursor(selected), truncate(label, inner.width.saturating_sub(2) as usize));
        f.render_widget(Paragraph::new(text).style(row_style(selected, true)), row(inner, (i - start) as u16));
    }
}

/// 可滚动的文本弹窗（帮助、通知）：小窗口里也能看完全部内容。
fn draw_scroll(f: &mut Frame, app: &App, title: &str, lines: Vec<Line<'static>>, scroll: usize, content_width: u16) {
    let area = centered(f.area(), content_width + 4, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    let rows = area.height.saturating_sub(2) as usize;
    let max = lines.len().saturating_sub(rows);
    app.scroll_max.set(max);
    let start = scroll.min(max);
    let position = if max == 0 {
        "Esc 关闭".to_string()
    } else {
        format!("{}-{} / {} · j/k 滚动 · Esc 关闭", start + 1, (start + rows).min(lines.len()), lines.len())
    };
    let block = popup(title.into())
        .title_bottom(Line::from(Span::styled(format!(" {position} "), Style::new().fg(DIM))))
        .padding(Padding::horizontal(1));
    let visible: Vec<Line> = lines.into_iter().skip(start).take(rows).collect();
    f.render_widget(Paragraph::new(visible).block(block), area);
}
