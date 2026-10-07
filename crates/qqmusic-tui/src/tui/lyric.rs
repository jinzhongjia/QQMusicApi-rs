//! LRC 歌词解析。

/// 一行歌词。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricLine {
    /// 时间戳（毫秒）。
    pub time_ms: u64,
    /// 原文。
    pub text: String,
    /// 翻译。
    pub trans: Option<String>,
}

fn parse_time(tag: &str) -> Option<u64> {
    let (min, rest) = tag.split_once(':')?;
    let min: u64 = min.trim().parse().ok()?;
    let (sec, frac) = rest.split_once(['.', ':']).unwrap_or((rest, "0"));
    let sec: u64 = sec.trim().parse().ok()?;
    let frac = frac.trim();
    let ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u64>().ok()? * 100,
        2 => frac.parse::<u64>().ok()? * 10,
        _ => frac[..3].parse::<u64>().ok()?,
    };
    Some(min * 60_000 + sec * 1000 + ms)
}

fn parse_lrc(lrc: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for line in lrc.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            if let Some(t) = parse_time(&stripped[..end]) {
                times.push(t);
            }
            rest = &stripped[end + 1..];
        }
        let text = rest.trim();
        for t in times {
            out.push((t, text.to_string()));
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

/// 解析原文与翻译，按时间戳合并。
pub fn parse(lyric: &str, trans: &str) -> Vec<LyricLine> {
    let trans = parse_lrc(trans);
    parse_lrc(lyric)
        .into_iter()
        .map(|(time_ms, text)| {
            let trans = trans.iter().find(|(t, s)| *t == time_ms && !s.is_empty() && s != "//").map(|(_, s)| s.clone());
            LyricLine { time_ms, text, trans }
        })
        .collect()
}

/// 当前应高亮的行。
pub fn current(lines: &[LyricLine], position_ms: u64) -> Option<usize> {
    lines.partition_point(|l| l.time_ms <= position_ms).checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_merge() {
        let lines = parse("[ti:x]\n[00:01.50]a\n[00:03.123][00:05]b\n", "[00:01.50]A\n[00:03.123]//\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], LyricLine { time_ms: 1500, text: "a".into(), trans: Some("A".into()) });
        assert_eq!(lines[1].time_ms, 3123);
        assert_eq!(lines[1].trans, None);
        assert_eq!(lines[2].time_ms, 5000);
        assert_eq!(current(&lines, 0), None);
        assert_eq!(current(&lines, 4000), Some(1));
    }
}
