//! 把登录二维码图片还原为模块矩阵，用半角方块字符画在终端里。

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// 解码二维码图片为模块矩阵（`true` 为深色）。
pub fn modules(data: &[u8]) -> Option<Vec<Vec<bool>>> {
    let img = image::load_from_memory(data).ok()?.to_luma8();
    let (w, h) = img.dimensions();
    let dark = |x: u32, y: u32| img.get_pixel(x, y)[0] < 128;

    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if dark(x, y) {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x > max_x || min_y > max_y {
        return None;
    }
    // 左上角定位图案宽 7 个模块。
    let mut run = 0;
    while min_x + run <= max_x && dark(min_x + run, min_y) {
        run += 1;
    }
    if run < 7 {
        return None;
    }
    // 定位块估算的尺寸吸附到合法的 QR 尺寸（17 + 4v），再按实际宽度重算模块大小，
    // 以适配模块宽度非整数像素的图片。
    let span = (max_x - min_x + 1) as f32;
    let estimate = span / (run as f32 / 7.0);
    let version = ((estimate - 17.0) / 4.0).round().clamp(1.0, 40.0) as usize;
    let n = 17 + 4 * version;
    let module = span / n as f32;
    let sample = |i: usize, base: u32, max: u32| (base as f32 + (i as f32 + 0.5) * module).min(max as f32) as u32;
    Some(
        (0..n)
            .map(|row| (0..n).map(|col| dark(sample(col, min_x, max_x), sample(row, min_y, max_y))).collect())
            .collect(),
    )
}

/// 渲染为终端行（每行字符表示两行模块，带 2 模块静区）。
pub fn render(grid: &[Vec<bool>]) -> Vec<Line<'static>> {
    const QUIET: usize = 2;
    let n = grid.len();
    let size = n + QUIET * 2;
    let at =
        |x: usize, y: usize| x >= QUIET && y >= QUIET && x < n + QUIET && y < n + QUIET && grid[y - QUIET][x - QUIET];
    let color = |dark: bool| if dark { Color::Black } else { Color::White };
    (0..size)
        .step_by(2)
        .map(|y| {
            Line::from(
                (0..size)
                    .map(|x| Span::styled("▀", Style::new().fg(color(at(x, y))).bg(color(at(x, y + 1)))))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use qqmusic_api::models::login::QrLoginType;

    /// 从真实接口获取三种登录二维码并确认都能还原为模块矩阵。
    #[tokio::test]
    #[ignore = "需要网络"]
    async fn live_qrcode_decode() {
        let dir = std::env::temp_dir().join("qqmusic-tui-qr-test");
        std::fs::create_dir_all(&dir).unwrap();
        let client = qqmusic_api::Client::builder().device_path(dir.join("device.json")).build().unwrap();
        for kind in [QrLoginType::Qq, QrLoginType::Wx, QrLoginType::Mobile] {
            let qr = client.login().get_qrcode(kind).await.unwrap();
            let grid = super::modules(&qr.data).unwrap_or_else(|| panic!("{kind:?} 二维码无法解析"));
            assert_eq!((grid.len() - 17) % 4, 0, "{kind:?}: 模块数 {} 不是合法的 QR 尺寸", grid.len());
            println!("{kind:?}: {} x {}", grid.len(), grid.len());
        }
    }
}
