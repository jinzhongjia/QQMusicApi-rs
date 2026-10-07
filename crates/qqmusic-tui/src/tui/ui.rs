//! 界面渲染，布局参照 go-musicfox：
//!
//! * 顶部一条 `──── QQ 音乐 ────` 标题线；
//! * 菜单从窗口约 1/3 高度处开始，起始列随窗口宽度变化，宽度足够时双列显示，每页 10 项；
//! * 歌词在菜单与底部之间垂直居中；
//! * 倒数第 4 行为 `[模式] 音量 ♫ ♪ ♫ ♪ ♥ 歌名 歌手`，倒数第 2 行为整行渐变进度条。

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use super::app::{App, MenuLayout, Mode, QUALITIES};
use super::data::{self, Item, Source};
use super::lyric;
use crate::protocol::{Status, quality_label};

const ACCENT: Color = Color::Rgb(49, 194, 124);
/// 进度条渐变终点色。
const ACCENT_END: Color = Color::Rgb(0, 168, 214);
const DIM: Color = Color::DarkGray;
const LIKED: Color = Color::Rgb(234, 64, 63);
/// 单页最多条目数（与 go-musicfox 一致）。
const PAGE_SIZE: u16 = 10;
/// 底部固定区：歌曲信息、空行、进度条、提示行。
const END_ROW_MARGIN: u16 = 4;
/// 双列时左列最大宽度。
const MAX_LEFT_COLUMN: u16 = 44;

/// 按窗口大小计算的布局。
struct Geometry {
    /// 菜单起始列（选中前缀 `=> ` 画在其左侧 4 列）。
    start_col: u16,
    title_row: u16,
    list_row: u16,
    rows: u16,
    columns: u16,
    /// 歌曲信息所在行。
    info_row: u16,
}

impl Geometry {
    fn new(area: Rect) -> Self {
        let (w, h) = (area.width, area.height);
        let dual = w >= 75;
        let start_col = match (dual, w) {
            (true, ..100) => w / 5,
            (true, ..150) => w / 4,
            (true, _) => w / 3,
            (false, ..100) => w / 3,
            (false, _) => w * 2 / 5,
        }
        .max(5);
        let menu_start_row = (h / 3).min(12);
        let title_row = menu_start_row.saturating_sub(2).max(2);
        let list_row = title_row + 2;
        let info_row = h.saturating_sub(END_ROW_MARGIN);
        let columns = if dual { 2 } else { 1 };
        let max_rows = info_row.saturating_sub(list_row + 1).max(1);
        let rows = (PAGE_SIZE / columns).min(max_rows);
        Self { start_col, title_row, list_row, rows, columns, info_row }
    }

    fn page_size(&self) -> usize {
        (self.rows * self.columns) as usize
    }

    /// 第 `column` 列的起点与宽度（含 4 列前缀）。
    fn column(&self, width: u16, column: u16) -> (u16, u16) {
        let x = self.start_col - 4;
        let total = width.saturating_sub(x);
        if self.columns == 1 {
            return (x, total);
        }
        let left = if width <= 88 { total.saturating_sub(4) / 2 } else { MAX_LEFT_COLUMN };
        if column == 0 { (x, left) } else { (x + left + 4, total.saturating_sub(left + 4)) }
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let geo = Geometry::new(area);
    app.layout.set(MenuLayout { page_size: geo.page_size(), columns: geo.columns as usize });

    draw_title_bar(f, area);
    let menu_bottom = draw_menu(f, app, &geo, area);
    draw_lyrics(f, app, &geo, area, menu_bottom);
    draw_song_info(f, app, &geo, area);
    draw_progress(f, app, &geo, area);
    draw_toast(f, app, &geo, area);

    match &app.mode {
        Mode::Normal => {}
        Mode::Search { input, kind } => {
            let area = centered(f.area(), 50, 3);
            f.render_widget(Clear, area);
            let block = popup(format!(" 搜索 · {}（Tab 切换） ", kind.label()));
            f.render_widget(Paragraph::new(format!("{input}▏")).block(block), area);
        }
        Mode::Quality { selected } => {
            let area = centered(f.area(), 30, QUALITIES.len() as u16 + 2);
            f.render_widget(Clear, area);
            let lines: Vec<Line> = QUALITIES
                .iter()
                .enumerate()
                .map(|(i, q)| {
                    let mark = if *q == app.state.quality { " ✓" } else { "" };
                    let text = format!("{} {}{mark}", if i == *selected { "›" } else { " " }, quality_label(*q));
                    if i == *selected { Line::from(text).fg(ACCENT).bold() } else { Line::from(text) }
                })
                .collect();
            f.render_widget(Paragraph::new(lines).block(popup(" 选择音质 ".into())), area);
        }
        Mode::Help => {
            let lines: Vec<Line> = data::HELP
                .iter()
                .map(|(key, desc)| {
                    Line::from(vec![Span::styled(format!(" {key:<16}"), Style::new().fg(ACCENT)), Span::raw(*desc)])
                })
                .collect();
            let area = centered(f.area(), 46, lines.len() as u16 + 2);
            f.render_widget(Clear, area);
            f.render_widget(Paragraph::new(lines).block(popup(" 帮助（任意键关闭） ".into())), area);
        }
        Mode::Login(view) => {
            let qr_width = view.qr.first().map_or(0, |l| l.width() as u16);
            let width = qr_width.max(44) + 4;
            let height = view.qr.len() as u16 + 6;
            let area = centered(f.area(), width, height);
            f.render_widget(Clear, area);
            let title = match view.kind {
                qqmusic_api::models::login::QrLoginType::Qq => " QQ 扫码登录 ",
                qqmusic_api::models::login::QrLoginType::Wx => " 微信扫码登录 ",
                qqmusic_api::models::login::QrLoginType::Mobile => " QQ 音乐 App 扫码登录 ",
            };
            let mut lines = view.qr.clone();
            lines.push(Line::default());
            lines.push(Line::from(view.status.clone()).fg(ACCENT));
            if let Some(path) = &view.path {
                lines.push(Line::from(format!("二维码图片: {path}")).fg(DIM));
            }
            lines.push(Line::from("Esc 取消 · r 重新获取").fg(DIM));
            f.render_widget(Paragraph::new(lines).alignment(Alignment::Center).block(popup(title.into())), area);
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

fn row(area: Rect, y: u16, x: u16, width: u16) -> Rect {
    Rect { x: area.x + x, y: area.y + y, width: width.min(area.width.saturating_sub(x)), height: 1 }
}

fn draw_title_bar(f: &mut Frame, area: Rect) {
    let name = " QQ 音乐 ";
    let name_width = Line::from(name).width() as u16;
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

/// 菜单标题与列表，返回菜单底部所在行。
fn draw_menu(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) -> u16 {
    let page = app.page();
    let page_size = geo.page_size();
    let pages = page.items.len().div_ceil(page_size).max(1);
    let cur_page = page.selected / page_size;

    let mut title = vec![Span::styled(page.title.clone(), Style::new().fg(ACCENT).bold())];
    if page.source == Source::Root
        && let Some(nick) = &app.nick
    {
        title.push(Span::styled(format!("  {nick}"), Style::new().fg(DIM)));
    }
    if pages > 1 {
        title.push(Span::styled(format!("  {}/{pages}", cur_page + 1), Style::new().fg(DIM)));
    }
    if page.loading {
        title.push(Span::styled("  加载中…", Style::new().fg(DIM)));
    }
    let x = geo.start_col;
    f.render_widget(Paragraph::new(Line::from(title)), row(area, geo.title_row, x, area.width));

    let bottom = geo.list_row + geo.rows;
    if let Some(error) = &page.error {
        let text = format!("加载失败: {error}（r 重试）");
        f.render_widget(Paragraph::new(text).fg(Color::Red), row(area, geo.list_row, x, area.width));
        return bottom;
    }
    if page.items.is_empty() {
        if !page.loading {
            let hint = if page.source == Source::Queue { "播放列表为空" } else { "暂无内容" };
            f.render_widget(Paragraph::new(hint).fg(DIM), row(area, geo.list_row, x, area.width));
        }
        return bottom;
    }

    let start = cur_page * page_size;
    let end = (start + page_size).min(page.items.len());
    let index_width = end.to_string().len();
    let playing_mid = app.state.track.as_ref().map(|t| t.mid.as_str());
    for (offset, item) in page.items[start..end].iter().enumerate() {
        let index = start + offset;
        let columns = geo.columns as usize;
        let (x, width) = geo.column(area.width, (offset % columns) as u16);
        let y = geo.list_row + (offset / columns) as u16;
        let selected = index == page.selected;
        let playing = match item {
            Item::Song(_) if page.source == Source::Queue => app.state.index == Some(index),
            Item::Song(track) => playing_mid == Some(track.mid.as_str()),
            _ => false,
        };
        let (title, subtitle) = match item {
            Item::Song(track) => (track.name.clone(), track.singers.clone()),
            Item::Link { title, sub, .. } => (title.clone(), sub.clone()),
            Item::Action { title, .. } => (title.clone(), String::new()),
            Item::Text(text) => (text.clone(), String::new()),
        };
        let title_style = match (selected, playing, item) {
            (true, ..) => Style::new().fg(ACCENT).bold(),
            (_, true, _) => Style::new().fg(ACCENT),
            (_, _, Item::Text(_)) => Style::new().fg(DIM),
            _ => Style::new(),
        };
        let prefix = if selected { " => " } else { "    " };
        let mut spans = vec![
            Span::styled(prefix, Style::new().fg(ACCENT).bold()),
            Span::styled(format!("{:>index_width$}. ", index + 1), title_style),
            Span::styled(title, title_style),
        ];
        if let Item::Song(track) = item {
            if track.vip {
                spans.push(Span::styled(" VIP", Style::new().fg(Color::Yellow)));
            }
            if app.liked.contains(&track.id) {
                spans.push(Span::styled(" ♥", Style::new().fg(LIKED)));
            }
        }
        if !subtitle.is_empty() {
            spans.push(Span::styled(format!(" {subtitle}"), Style::new().fg(DIM)));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), row(area, y, x, width));
    }
    bottom
}

/// 歌词：左对齐、在菜单底部与歌曲信息之间垂直居中，空间够时 5 行，否则 3 行。
fn draw_lyrics(f: &mut Frame, app: &App, geo: &Geometry, area: Rect, menu_bottom: u16) {
    let space = geo.info_row.saturating_sub(menu_bottom + 1);
    if app.state.track.is_none() || space < 3 {
        return;
    }
    let lines: u16 = if space >= 5 { 5 } else { 3 };
    let top = (menu_bottom + geo.info_row - lines) / 2;
    let x = if geo.columns == 2 { geo.start_col + 3 } else { geo.start_col.saturating_sub(4) };
    let width = area.width.saturating_sub(x + 4);
    let center = (lines / 2) as usize;

    if app.lyrics.is_empty() {
        let rect = row(area, top + center as u16, x, width);
        f.render_widget(Paragraph::new("暂无歌词").fg(DIM), rect);
        return;
    }
    let current = lyric::current(&app.lyrics, app.position_ms());
    for i in 0..lines as usize {
        let index = match current {
            Some(cur) => (cur + i).checked_sub(center),
            None => i.checked_sub(center + 1),
        };
        let Some(line) = index.and_then(|i| app.lyrics.get(i)) else { continue };
        let highlight = i == center && current.is_some();
        let style = if highlight { Style::new().fg(ACCENT).bold() } else { Style::new().fg(DIM) };
        let mut spans = vec![Span::styled(line.text.clone(), style)];
        if let Some(trans) = &line.trans {
            let style = if highlight { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
            spans.push(Span::styled(format!("  {trans}"), style));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), row(area, top + i as u16, x, width));
    }
}

fn draw_song_info(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) {
    let state = &app.state;
    let mut spans = Vec::new();
    spans.push(Span::styled(format!("[{}] ", state.mode.label()), Style::new().fg(DIM)));
    spans.push(Span::styled(format!("{}% ", state.volume), Style::new().fg(DIM)));
    let status = match state.status {
        Status::Playing => Span::styled("♫ ♪ ♫ ♪ ", Style::new().fg(ACCENT)),
        Status::Loading => Span::styled("· · · · ", Style::new().fg(ACCENT)),
        Status::Paused | Status::Stopped => Span::styled("_ z Z Z ", Style::new().fg(Color::Yellow)),
    };
    spans.push(status);
    if let Some(track) = &state.track {
        let liked = app.liked.contains(&track.id);
        spans.push(Span::styled("♥ ", Style::new().fg(if liked { LIKED } else { DIM })));
        spans.push(Span::styled(track.name.clone(), Style::new().fg(ACCENT)));
        spans.push(Span::styled(format!(" {}", track.singers), Style::new().fg(DIM)));
        let quality = match state.playing_quality {
            Some(q) => quality_label(q),
            None => quality_label(state.quality),
        };
        spans.push(Span::styled(format!("  [{quality}]"), Style::new().fg(DIM)));
    } else {
        spans.push(Span::styled(format!("[{}]", quality_label(state.quality)), Style::new().fg(DIM)));
    }
    let x = geo.start_col.saturating_sub(4);
    f.render_widget(Paragraph::new(Line::from(spans)), row(area, geo.info_row, x, area.width));
}

/// 整行渐变进度条：`####······ 01:23/04:56`。
fn draw_progress(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) {
    let duration = app.state.duration_ms;
    if app.state.track.is_none() || duration == 0 {
        return;
    }
    let position = app.position_ms();
    let width = area.width.saturating_sub(14) as usize;
    let filled = ((width as f64) * (position as f64 / duration as f64).clamp(0.0, 1.0)).round() as usize;
    let mut spans: Vec<Span> = (0..filled).map(|i| Span::styled("#", Style::new().fg(gradient(i, width)))).collect();
    spans.push(Span::styled("·".repeat(width - filled), Style::new().fg(DIM)));
    let (p, d) = (position / 1000, duration / 1000);
    spans.push(Span::styled(
        format!(" {:02}:{:02}/{:02}:{:02}", p / 60, p % 60, d / 60, d % 60),
        Style::new().fg(ACCENT),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), row(area, geo.info_row + 2, 0, area.width));
}

fn gradient(i: usize, width: usize) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (ACCENT, ACCENT_END) else { return ACCENT };
    let t = if width > 1 { i as f32 / (width - 1) as f32 } else { 0.0 };
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
    Color::Rgb(mix(r1, r2), mix(g1, g2), mix(b1, b2))
}

/// 最后一行：提示信息（4 秒后消失）或连接状态。
fn draw_toast(f: &mut Frame, app: &App, geo: &Geometry, area: Rect) {
    let line = match &app.toast {
        Some((message, at)) if at.elapsed().as_secs() < 4 => Line::from(message.clone()).fg(Color::Yellow),
        _ if !app.connected => Line::from("正在连接播放进程…").fg(Color::Yellow),
        _ => Line::from("? 帮助").fg(DIM),
    };
    let x = geo.start_col.saturating_sub(4);
    f.render_widget(Paragraph::new(line), row(area, geo.info_row + 3, x, area.width));
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::paths::Paths;
    use crate::protocol::{Event, PlayerState, Track};
    use crate::tui::app::Msg;

    fn track(i: usize) -> Track {
        Track {
            id: i as i64,
            mid: format!("m{i}"),
            media_mid: String::new(),
            song_type: 0,
            name: format!("歌曲 {i}"),
            singers: "周杰伦".into(),
            album: "叶惠美".into(),
            cover: String::new(),
            duration: 269,
            vip: i.is_multiple_of(3),
        }
    }

    fn render(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        // 宽字符后面的占位格跳过，便于断言与阅读。
        let buffer = terminal.backend().buffer();
        let mut lines = Vec::new();
        for row in buffer.content.chunks(width as usize) {
            let mut line = String::new();
            let mut skip = 0;
            for cell in row {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                line.push_str(cell.symbol());
                skip = Line::from(cell.symbol()).width().saturating_sub(1);
            }
            lines.push(line.trim_end().to_string());
        }
        lines.join("\n")
    }

    #[tokio::test]
    async fn render_layouts() {
        let dir = std::env::temp_dir().join(format!("qqmusic-tui-test-{}", std::process::id()));
        let paths = Paths::with_root(&dir).unwrap();
        let client = qqmusic_api::Client::builder().device_path(paths.device()).build().unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(paths, client, tx, cmd_tx);
        let tracks: Vec<Track> = (1..=25).map(track).collect();
        let state = PlayerState {
            status: Status::Playing,
            index: Some(2),
            track: Some(tracks[2].clone()),
            position_ms: 61_000,
            duration_ms: 269_000,
            volume: 80,
            queue_len: tracks.len(),
            ..Default::default()
        };
        app.handle_msg(Msg::Player(Event::State(Box::new(state))));
        app.handle_msg(Msg::Player(Event::Queue { tracks }));
        app.handle_msg(Msg::Lyric {
            mid: "m3".into(),
            lines: lyric::parse("[00:50.00]故事的小黄花\n[01:00.00]从出生那年就飘着\n[01:10.00]童年的荡秋千", ""),
        });

        let root = render(&app, 120, 36);
        assert!(root.contains(" =>  1. 猜你喜欢"), "{root}");
        assert!(root.contains("从出生那年就飘着"));

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        for (w, h) in [(60, 20), (120, 36), (200, 50)] {
            let screen = render(&app, w, h);
            assert!(screen.contains("歌曲 3"), "{screen}");
        }
        // 双列：l 移到右列，再 l 翻到下一页。
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
        assert!(render(&app, 120, 36).contains("2/3"));

        app.mode = Mode::Help;
        assert!(render(&app, 120, 36).contains("帮助"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
