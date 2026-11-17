//! 界面渲染，布局参照 go-musicfox：
//!
//! * 顶部一条 `──── QQ 音乐 ────` 标题线；
//! * 页面标题与列表靠左留白，内容少时从窗口约 1/3 高度处开始，内容多时上移并占满可用高度；
//! * 歌词在列表与底部之间垂直居中；
//! * 倒数第 4 行为 `[模式] 音量 ♫ ♪ ♫ ♪ ♥ 歌名 歌手`，倒数第 2 行为整行渐变进度条，最后一行是提示与通知。
//!
//! 选中项用 ` => `、正在播放用 `♪` 标出，不只依赖颜色。

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph, Wrap};

use super::app::{App, Level, LyricState, Menu, Mode, More, Page, SearchBox, TOAST_SECS, View, Viewport};
use super::data::{self, Item, SearchKind, Source};
use super::input::{truncate, width, wrap};
use super::lyric;
use crate::protocol::{Activity, PlayMode, Status, Track, quality_label};

const ACCENT: Color = Color::Rgb(49, 194, 124);
/// 进度条渐变终点色。
const ACCENT_END: Color = Color::Rgb(0, 168, 214);
const DIM: Color = Color::DarkGray;
const LIKED: Color = Color::Rgb(234, 64, 63);
/// 底部固定区：歌曲信息、空行、进度条、提示行。
const END_ROW_MARGIN: u16 = 4;
/// 列表正文的最大宽度，超宽终端上不把时长甩到最右边。
const MAX_CONTENT: u16 = 96;

/// 按窗口大小与内容多少计算的布局。
struct Geometry {
    /// 正文起始列（选中前缀 ` => ` 画在其左侧 4 列）。
    start_col: u16,
    /// 正文宽度。
    cols: u16,
    title_row: u16,
    list_row: u16,
    /// 列表可见行数。
    rows: u16,
    /// 列表下方歌词占的行数（0 表示不显示）。
    lyric_rows: u16,
    /// 歌曲信息所在行。
    info_row: u16,
}

impl Geometry {
    /// `len` 是列表的条目数，`lyrics` 表示列表下方要不要留歌词。
    fn new(area: Rect, len: usize, lyrics: bool) -> Self {
        let (w, h) = (area.width, area.height);
        let start_col = match w {
            ..100 => w / 5,
            100..150 => w / 4,
            _ => w / 3,
        }
        .max(5);
        let cols = w.saturating_sub(start_col + start_col / 2).min(MAX_CONTENT);
        let info_row = h.saturating_sub(END_ROW_MARGIN);
        // 内容少：像菜单一样从约 1/3 处开始，留白；放不下时上移，把高度让给列表。
        let airy_title = ((h / 3).min(12)).saturating_sub(2).max(2);
        let room = |title_row: u16| info_row.saturating_sub(title_row + 2 + 1);
        let lyric_for = |space: u16| match space {
            _ if !lyrics => 0,
            ..8 => 0,
            8..14 => 3,
            _ => 5,
        };
        let fits = |title_row: u16| {
            let space = room(title_row);
            let lyric_rows = lyric_for(space);
            len as u16 <= space.saturating_sub(if lyric_rows > 0 { lyric_rows + 1 } else { 0 })
        };
        let title_row = if len <= u16::MAX as usize && fits(airy_title) { airy_title } else { 2.min(airy_title) };
        let space = room(title_row);
        let lyric_rows = lyric_for(space);
        let available = space.saturating_sub(if lyric_rows > 0 { lyric_rows + 1 } else { 0 });
        let rows = available.min(len.min(u16::MAX as usize) as u16).max(available.min(1));
        Self { start_col, cols, title_row, list_row: title_row + 2, rows, lyric_rows, info_row }
    }

    /// 带前缀的一整行（从 `start_col - 4` 开始）。
    fn line(&self, area: Rect, y: u16) -> Rect {
        row(area, y, self.start_col.saturating_sub(4), self.cols + 4)
    }

    /// 只有正文的一行（从 `start_col` 开始）。
    fn text(&self, area: Rect, y: u16) -> Rect {
        row(area, y, self.start_col, self.cols)
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    draw_title_bar(f, area);
    let page = app.page();
    let rows = match page.source {
        Source::Queue => draw_queue(f, app, area),
        Source::Lyrics => draw_lyric_page(f, app, area),
        _ => draw_page(f, app, page, area),
    };
    app.viewport.set(Viewport { rows: rows.max(1) });
    let geo = Geometry::new(area, 0, false);
    draw_song_info(f, app, &geo, area);
    draw_progress(f, app, &geo, area);
    draw_bottom(f, app, &geo, area);

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
            // 二维码很大，只清弹窗区域会在两侧露出半截菜单，干脆清掉整屏。
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

/// 第 `y` 行从第 `x` 列起、最多 `width` 列；超出窗口时为空矩形。
fn row(area: Rect, y: u16, x: u16, width: u16) -> Rect {
    if y >= area.height || x >= area.width {
        return Rect { x: area.x, y: area.y, width: 0, height: 0 };
    }
    Rect { x: area.x + x, y: area.y + y, width: width.min(area.width - x), height: 1 }
}

/// `inner` 内的第 `y` 行。
fn inner_row(inner: Rect, y: u16) -> Rect {
    row(inner, y, 0, inner.width)
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

fn draw_title_bar(f: &mut Frame, area: Rect) {
    if area.height < 6 {
        return;
    }
    let name = " QQ 音乐 ";
    let name_width = width(name) as u16;
    let prefix = area.width.saturating_sub(name_width) / 2;
    let suffix = area.width.saturating_sub(prefix + name_width);
    let line = Line::from(vec![
        Span::raw("─".repeat(prefix as usize)),
        Span::styled(name, Style::new().add_modifier(Modifier::BOLD)),
        Span::raw("─".repeat(suffix as usize)),
    ])
    .fg(ACCENT);
    f.render_widget(Paragraph::new(line), row(area, 0, 0, area.width));
}

/// 标题行与它下面的状态行。
fn draw_heading(f: &mut Frame, geo: &Geometry, area: Rect, title: Vec<Span<'static>>, status: Line<'static>) {
    if geo.title_row >= geo.info_row {
        return;
    }
    f.render_widget(Paragraph::new(Line::from(title)), geo.text(area, geo.title_row));
    if geo.title_row + 1 < geo.info_row {
        f.render_widget(Paragraph::new(status), geo.text(area, geo.title_row + 1));
    }
}

fn title_span(text: String) -> Span<'static> {
    Span::styled(text, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
}

/// 页面状态行：已加载数量、服务端总量、续页状态。出了问题时问题排在最前面，窄屏也不会被截掉。
fn page_status(page: &Page) -> Line<'static> {
    let dim = Style::new().fg(DIM);
    let red = Style::new().fg(Color::Red);
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
    if let More::Failed(e) = &page.more {
        return Line::from(vec![
            Span::styled("加载更多失败（r 重试）", red),
            Span::styled(format!(" · {loaded}"), dim),
            Span::styled(format!(" · {e}"), red),
        ]);
    }
    if let Some(error) = page.error.as_ref().filter(|_| !page.items.is_empty()) {
        return Line::from(Span::styled(format!("刷新失败，显示的是旧内容（r 重试）: {error}"), red));
    }
    if page.capped {
        let text = format!(" · 已达上限 {} 条，其余没有加载", data::MAX_ITEMS);
        return Line::from(vec![Span::styled(loaded, dim), Span::styled(text, Style::new().fg(Color::Yellow))]);
    }
    // 数量在前，类型切换在后：窄屏放不下时被截掉的是后者。
    let mut spans = vec![Span::styled(loaded, dim)];
    if page.loading && !page.items.is_empty() {
        spans.push(Span::styled(" · 刷新中…", dim));
    }
    if page.more == More::Loading {
        spans.push(Span::styled(" · 正在加载更多…", dim));
    }
    if let Source::Search { kind, .. } = &page.source {
        spans.push(Span::raw("  "));
        for k in SearchKind::ALL {
            let style = if k == *kind { Style::new().fg(ACCENT) } else { dim };
            let label = if k == *kind { format!("[{}]", k.label()) } else { format!(" {} ", k.label()) };
            spans.push(Span::styled(label, style));
        }
        spans.push(Span::styled(" t 切换", dim));
    }
    Line::from(spans)
}

/// 列表下方要不要留歌词：有歌词，或加载失败需要提示。
fn wants_lyrics(app: &App) -> bool {
    matches!(app.lyric, LyricState::Ready(_) | LyricState::Failed(_))
}

/// 普通页面：标题、状态、列表、歌词。返回列表一屏的行数。
fn draw_page(f: &mut Frame, app: &App, page: &Page, area: Rect) -> usize {
    let geo = Geometry::new(area, page.items.len(), wants_lyrics(app));
    let mut title = vec![title_span(page.title.clone())];
    if page.source == Source::Root {
        title.push(Span::styled(format!("  {}", app.account_label()), Style::new().fg(DIM)));
    }
    draw_heading(f, &geo, area, title, page_status(page));

    let bottom = geo.list_row + geo.rows;
    if page.items.is_empty() {
        let (text, style) = match &page.error {
            Some(error) => (format!("加载失败: {error}（r 重试）"), Style::new().fg(Color::Red)),
            None if page.loading => (String::new(), Style::new()),
            None if matches!(page.source, Source::Search { .. }) => {
                ("没有找到相关结果，按 t 换个类型或按 / 重新搜索".to_string(), Style::new().fg(DIM))
            }
            None => ("暂无内容".to_string(), Style::new().fg(DIM)),
        };
        let height = geo.info_row.saturating_sub(geo.list_row + 1).min(3);
        if height > 0 && geo.list_row < area.height {
            let hint = Rect { height, ..geo.text(area, geo.list_row) };
            f.render_widget(Paragraph::new(text).style(style).wrap(Wrap { trim: false }), hint);
        }
        draw_lyrics(f, app, &geo, area, bottom);
        return geo.rows as usize;
    }

    let rows = geo.rows as usize;
    let start = scroll_to(&page.offset, page.selected, rows, page.items.len());
    let index_width = page.items.len().to_string().len();
    let playing = app.state.track.as_ref().map(|t| t.mid.as_str());
    for (i, item) in page.items.iter().enumerate().skip(start).take(rows) {
        let selected = i == page.selected;
        let line = match item {
            Item::Song(track) => {
                let mark = if playing == Some(track.mid.as_str()) { Mark::Playing } else { Mark::None };
                song_line(app, track, i, index_width, selected, mark, geo.cols as usize)
            }
            Item::Link { title, sub, .. } => {
                let mut spans = entry_spans(i, index_width, selected, Mark::None, title.clone());
                if !sub.is_empty() {
                    spans.push(Span::styled(format!(" {sub}"), Style::new().fg(DIM)));
                }
                Line::from(spans)
            }
            Item::Action { title, .. } => Line::from(entry_spans(i, index_width, selected, Mark::None, title.clone())),
            Item::Text(text) => Line::from(Span::styled(format!("    {text}"), Style::new().fg(DIM))),
        };
        f.render_widget(Paragraph::new(line), geo.line(area, geo.list_row + (i - start) as u16));
    }
    draw_lyrics(f, app, &geo, area, bottom);
    rows
}

/// 行首的标记。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    None,
    Playing,
    /// 队列里确定的下一首。
    Next,
}

/// 前缀 + 序号 + 标题。
fn entry_spans(i: usize, index_width: usize, selected: bool, mark: Mark, title: String) -> Vec<Span<'static>> {
    let prefix = match (selected, mark) {
        (true, Mark::Playing) => " =>♪",
        (true, _) => " => ",
        (false, Mark::Playing) => "  ♪ ",
        (false, Mark::Next) => "  » ",
        (false, Mark::None) => "    ",
    };
    let style = match (selected, mark) {
        (true, _) => Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        (_, Mark::Playing) => Style::new().fg(ACCENT),
        _ => Style::new(),
    };
    vec![
        Span::styled(prefix, Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:>index_width$}. ", i + 1), style),
        Span::styled(title, style),
    ]
}

/// 歌曲行：` =>  1. 歌名 VIP ♥ 歌手 · 专辑        04:29`，窄屏依次省掉专辑、时长。
fn song_line(
    app: &App,
    track: &Track,
    i: usize,
    index_width: usize,
    selected: bool,
    mark: Mark,
    cols: usize,
) -> Line<'static> {
    let liked = app.liked.contains(&track.id);
    let vip = track.vip && cols >= 36;
    let duration = if cols >= 44 { 6 } else { 0 };
    let flags = if vip { 4 } else { 0 } + if liked { 2 } else { 0 };
    let room = cols.saturating_sub(index_width + 2 + flags + duration);
    let sub = if cols >= 72 && !track.album.is_empty() {
        format!("{} · {}", track.singers, track.album)
    } else {
        track.singers.clone()
    };
    // 歌名优先，但给歌手至少留三分之一。
    let name = truncate(&track.name, room.saturating_sub((room / 3).min(width(&sub) + 1)).max(room.min(4)));
    let sub = truncate(&sub, room.saturating_sub(width(&name) + 1));
    let mut spans = entry_spans(i, index_width, selected, mark, name.clone());
    if vip {
        spans.push(Span::styled(" VIP", Style::new().fg(Color::Yellow)));
    }
    if liked {
        spans.push(Span::styled(" ♥", Style::new().fg(LIKED)));
    }
    let mut used = index_width + 2 + width(&name) + flags;
    if !sub.is_empty() {
        used += width(&sub) + 1;
        spans.push(Span::styled(format!(" {sub}"), Style::new().fg(DIM)));
    }
    if duration > 0 {
        let pad = cols.saturating_sub(used + 5);
        spans.push(Span::styled(format!("{}{}", " ".repeat(pad), mmss(track.duration)), Style::new().fg(DIM)));
    }
    Line::from(spans)
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

fn draw_queue(f: &mut Frame, app: &App, area: Rect) -> usize {
    let geo = Geometry::new(area, app.queue.len(), wants_lyrics(app));
    let title = vec![
        title_span("播放队列".into()),
        Span::styled(format!("  {} 首 · {}", app.queue.len(), app.state.mode.label()), Style::new().fg(DIM)),
    ];
    let status = Line::from(Span::styled(up_next(app).unwrap_or_default(), Style::new().fg(DIM)));
    draw_heading(f, &geo, area, title, status);
    let bottom = geo.list_row + geo.rows;
    if app.queue.is_empty() {
        let height = geo.info_row.saturating_sub(geo.list_row + 1).min(3);
        if height > 0 && geo.list_row < area.height {
            let hint = Rect { height, ..geo.text(area, geo.list_row) };
            let text = "播放队列为空：在歌曲上按 Enter 播放列表，或按 a 追加到队尾";
            f.render_widget(Paragraph::new(text).fg(DIM).wrap(Wrap { trim: false }), hint);
        }
        return geo.rows as usize;
    }
    let rows = geo.rows as usize;
    let start = scroll_to(&app.queue_offset, app.queue_sel, rows, app.queue.len());
    let index_width = app.queue.len().to_string().len();
    for (i, track) in app.queue.iter().enumerate().skip(start).take(rows) {
        let mark = match (app.state.index == Some(i), app.state.next == Some(i)) {
            (true, _) => Mark::Playing,
            (_, true) => Mark::Next,
            _ => Mark::None,
        };
        let line = song_line(app, track, i, index_width, i == app.queue_sel, mark, geo.cols as usize);
        f.render_widget(Paragraph::new(line), geo.line(area, geo.list_row + (i - start) as u16));
    }
    draw_lyrics(f, app, &geo, area, bottom);
    rows
}

/// 歌词：左对齐、在列表底部与歌曲信息之间垂直居中，空间够时 5 行，否则 3 行。
fn draw_lyrics(f: &mut Frame, app: &App, geo: &Geometry, area: Rect, list_bottom: u16) {
    let lines = geo.lyric_rows;
    if lines == 0 || geo.info_row < list_bottom + 1 + lines {
        return;
    }
    let top = (list_bottom + 1 + geo.info_row - lines) / 2;
    let center = (lines / 2) as usize;
    if let LyricState::Failed(_) = &app.lyric {
        let text = "歌词加载失败（按 L 查看并重试）";
        f.render_widget(Paragraph::new(text).fg(DIM), geo.text(area, top + center as u16));
        return;
    }
    let lyrics = app.lyrics();
    let current = lyric::current(lyrics, app.position_ms());
    for i in 0..lines as usize {
        let index = match current {
            Some(cur) => (cur + i).checked_sub(center),
            None => i.checked_sub(center + 1),
        };
        let Some(line) = index.and_then(|i| lyrics.get(i)) else { continue };
        let highlight = i == center && current.is_some();
        let style = if highlight { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().fg(DIM) };
        let mut spans = vec![Span::styled(line.text.clone(), style)];
        if app.show_trans
            && let Some(trans) = &line.trans
        {
            let style = if highlight { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
            spans.push(Span::styled(format!("  {trans}"), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), geo.text(area, top + i as u16));
    }
}

/// 歌词视图：整页歌词，可手动滚动。返回一屏的行数。
fn draw_lyric_page(f: &mut Frame, app: &App, area: Rect) -> usize {
    // 借用列表的布局，把列表区整个给歌词。
    let geo = Geometry::new(area, usize::MAX, false);
    let title = match &app.state.track {
        Some(track) => {
            vec![title_span(track.name.clone()), Span::styled(format!("  {}", track.singers), Style::new().fg(DIM))]
        }
        None => vec![title_span("当前播放".into())],
    };
    let album = app.state.track.as_ref().map(|t| t.album.clone()).unwrap_or_default();
    draw_heading(f, &geo, area, title, Line::from(Span::styled(album, Style::new().fg(DIM))));
    let rows = geo.rows as usize;
    if rows == 0 || geo.list_row >= area.height {
        return 0;
    }
    let message = match &app.lyric {
        LyricState::Ready(_) => None,
        LyricState::Idle => Some(("没有正在播放的歌曲".to_string(), DIM)),
        LyricState::Loading => Some(("歌词加载中…".to_string(), DIM)),
        LyricState::Empty => Some(("这首歌没有歌词".to_string(), DIM)),
        LyricState::Failed(e) => Some((format!("歌词加载失败: {e}（r 重试）"), Color::Red)),
    };
    if let Some((text, color)) = message {
        let hint = Rect { height: geo.rows.min(3), ..geo.text(area, geo.list_row) };
        f.render_widget(Paragraph::new(text).fg(color).wrap(Wrap { trim: false }), hint);
        return rows;
    }

    let lines = app.lyrics();
    let current = lyric::current(lines, app.position_ms());
    let manual = app.lyric_sel;
    // 手动滚动时最后一行说明如何恢复跟随。
    let hint = manual.is_some() && rows >= 4;
    let visible = rows - usize::from(hint);
    let text_width = (geo.cols as usize).max(4);
    // 长行折成多行；翻译紧跟在原文下面。
    let mut out: Vec<Line> = Vec::new();
    let mut anchor = 0;
    for (i, line) in lines.iter().enumerate() {
        let is_current = current == Some(i);
        if manual.or(current) == Some(i) {
            anchor = out.len();
        }
        let style =
            if is_current { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new().fg(DIM) };
        let prefix = match (manual == Some(i), is_current) {
            (true, _) => "  › ",
            (_, true) => "  ♪ ",
            _ => "    ",
        };
        for (n, part) in wrap(&line.text, text_width).into_iter().enumerate() {
            let prefix = if n == 0 { prefix } else { "    " };
            out.push(Line::from(vec![Span::styled(prefix, Style::new().fg(ACCENT)), Span::styled(part, style)]));
        }
        if app.show_trans
            && let Some(trans) = &line.trans
        {
            let style = if is_current { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
            for part in wrap(trans, text_width) {
                out.push(Line::from(Span::styled(format!("    {part}"), style)));
            }
        }
    }
    let start = anchor.saturating_sub(visible / 2).min(out.len().saturating_sub(visible));
    for (y, line) in out.into_iter().skip(start).take(visible).enumerate() {
        f.render_widget(Paragraph::new(line), geo.line(area, geo.list_row + y as u16));
    }
    if hint {
        let text = "手动滚动中 · Enter 跳到这一行 · z 恢复跟随";
        f.render_widget(Paragraph::new(text).fg(Color::Yellow), geo.text(area, geo.list_row + visible as u16));
    }
    visible
}

/// 播放状态：暂停、停止、加载、缓冲各不相同，且都带文字。
fn status_span(app: &App) -> Span<'static> {
    let state = &app.state;
    let warn = Style::new().fg(Color::Yellow);
    if !app.connected {
        return Span::styled("○ 后台未连接 ", warn);
    }
    match (state.status, state.activity) {
        (Status::Stopped, _) => Span::styled("■ 已停止 ", Style::new().fg(DIM)),
        (Status::Paused, Activity::Loading) => Span::styled("_ z Z Z 已暂停（加载中） ", warn),
        (Status::Paused, _) => Span::styled("_ z Z Z 已暂停 ", warn),
        (_, Activity::Buffering) => Span::styled("· · · · 缓冲中 ", Style::new().fg(ACCENT)),
        (Status::Loading, _) => Span::styled("· · · · 加载中 ", Style::new().fg(ACCENT)),
        (Status::Playing, _) => Span::styled("♫ ♪ ♫ ♪ ", Style::new().fg(ACCENT)),
    }
}

/// `[模式] 音量 状态 ♥ 歌名 歌手  [音质]`
fn draw_song_info(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) {
    if area.height < 4 {
        return;
    }
    let state = &app.state;
    let dim = Style::new().fg(DIM);
    let mut spans = vec![
        Span::styled(format!("[{}] ", state.mode.label()), dim),
        Span::styled(format!("{}% ", state.volume), dim),
        status_span(app),
    ];
    // 实际音质与期望不同时两个都写出来。
    let quality = match state.playing_quality {
        Some(actual) if actual != state.quality => {
            format!("{}（期望{}，已降级）", quality_label(actual), quality_label(state.quality))
        }
        Some(actual) => quality_label(actual).to_string(),
        None => quality_label(state.quality).to_string(),
    };
    if let Some(track) = &state.track {
        let liked = app.liked.contains(&track.id);
        spans.push(Span::styled("♥ ", Style::new().fg(if liked { LIKED } else { DIM })));
        spans.push(Span::styled(track.name.clone(), Style::new().fg(ACCENT)));
        spans.push(Span::styled(format!(" {}", track.singers), dim));
        spans.push(Span::styled(format!("  [{quality}]"), dim));
    } else {
        spans.push(Span::styled(format!("[{quality}]"), dim));
    }
    let x = geo.start_col.saturating_sub(4);
    f.render_widget(Paragraph::new(Line::from(spans)), row(area, geo.info_row, x, area.width));
}

/// 整行渐变进度条：`####······ 01:23/04:56`。
fn draw_progress(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) {
    let duration = app.state.duration_ms;
    if app.state.track.is_none() || duration == 0 || area.height < 2 || area.width < 16 {
        return;
    }
    let position = app.position_ms();
    let bar = area.width.saturating_sub(14) as usize;
    let filled = ((bar as f64) * (position as f64 / duration as f64).clamp(0.0, 1.0)).round() as usize;
    let mut spans: Vec<Span> = (0..filled).map(|i| Span::styled("#", Style::new().fg(gradient(i, bar)))).collect();
    spans.push(Span::styled("·".repeat(bar - filled), Style::new().fg(DIM)));
    let times = format!(" {}/{}", mmss(position / 1000), mmss(duration / 1000));
    spans.push(Span::styled(times, Style::new().fg(ACCENT)));
    f.render_widget(Paragraph::new(Line::from(spans)), row(area, geo.info_row + 2, 0, area.width));
}

fn gradient(i: usize, width: usize) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (ACCENT, ACCENT_END) else { return ACCENT };
    let t = if width > 1 { i as f32 / (width - 1) as f32 } else { 0.0 };
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
    Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
}

/// 最后一行：普通提示（几秒后消失）、未关闭的错误、连接状态，或按键提示。
fn draw_bottom(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) {
    if area.height == 0 {
        return;
    }
    let toast = app.toast.as_ref().filter(|(_, at)| at.elapsed().as_secs() < TOAST_SECS);
    let (text, style) = match (toast, &app.alert) {
        (Some((message, _)), _) => (message.clone(), Style::new().fg(Color::Yellow)),
        (None, Some(alert)) => {
            // 操作说明放前面，长错误在窄屏被截断时也看得到怎么关。
            let count = if app.unread > 1 { format!("{} 条", app.unread) } else { String::new() };
            (format!("! {count}错误（x 关闭 · e 详情）{alert}"), Style::new().fg(Color::Red))
        }
        (None, None) if !app.connected => ("正在连接后台播放进程…".to_string(), Style::new().fg(Color::Yellow)),
        (None, None) => (hints(app).to_string(), Style::new().fg(DIM)),
    };
    let x = geo.start_col.saturating_sub(4);
    let line = row(area, area.height - 1, x, area.width);
    let next = up_next(app).filter(|_| toast.is_none() && app.alert.is_none()).unwrap_or_default();
    let next_width = if width(&text) + width(&next) + 3 <= line.width as usize { width(&next) as u16 + 1 } else { 0 };
    let [text_area, next_area] = Layout::horizontal([Constraint::Min(0), Constraint::Length(next_width)]).areas(line);
    f.render_widget(Paragraph::new(truncate(&text, text_area.width as usize)).style(style), text_area);
    f.render_widget(Paragraph::new(format!("{next} ")).fg(DIM).alignment(Alignment::Right), next_area);
}

/// 与当前视图相关的按键提示。
fn hints(app: &App) -> &'static str {
    match app.view() {
        View::Queue => "? 帮助 · Enter 播放 · d 移除 · J/K 移动 · D 清空 · c 定位当前",
        View::Lyrics => "? 帮助 · j/k 滚动 · Enter 跳转 · z 跟随 · t 翻译 · r 重试",
        View::List => match app.page().items.get(app.page().selected) {
            Some(Item::Song(_)) => "? 帮助 · Enter 播放并替换队列 · A 下一首播放 · a 追加 · o 菜单",
            _ => "? 帮助",
        },
    }
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
    f.render_widget(Paragraph::new(format!("> {text}")), inner_row(inner, 0));
    f.set_cursor_position(Position::new(inner.x + 2 + column as u16, inner.y));
    if options == 0 {
        return;
    }
    let label =
        if search.from_history { "历史（↑ ↓ 选择 · Ctrl+X 清除）" } else { "建议（↑ ↓ 选择）" };
    f.render_widget(Paragraph::new(label).fg(DIM), inner_row(inner, 1));
    for (i, option) in search.options.iter().take(8).enumerate() {
        let picked = search.picked == Some(i);
        let text = format!("{}{}", cursor(picked), truncate(option, inner.width as usize - 2));
        f.render_widget(Paragraph::new(text).style(picked_style(picked)), inner_row(inner, 2 + i as u16));
    }
}

fn cursor(selected: bool) -> &'static str {
    if selected { "› " } else { "  " }
}

fn picked_style(selected: bool) -> Style {
    if selected { Style::new().fg(ACCENT).add_modifier(Modifier::BOLD) } else { Style::new() }
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
        f.render_widget(Paragraph::new(text).style(picked_style(selected)), inner_row(inner, (i - start) as u16));
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
