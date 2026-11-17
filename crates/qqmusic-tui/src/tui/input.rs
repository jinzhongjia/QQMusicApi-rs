//! 单行文本输入：光标移动、插入、删除，以及按显示宽度的横向滚动。

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// 字符串的显示宽度（中文等宽字符占 2 列）。
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// 截断到不超过 `max` 列，超出时以 `…` 结尾。
pub fn truncate(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    if max > 0 {
        out.push('…');
    }
    out
}

/// 按显示宽度折行（不在宽字符中间断开）。
pub fn wrap(text: &str, max: usize) -> Vec<String> {
    let max = max.max(2);
    let mut lines = vec![String::new()];
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > max {
            lines.push(String::new());
            used = 0;
        }
        lines.last_mut().expect("at least one line").push(c);
        used += w;
    }
    lines
}

/// 单行输入框。光标是字节下标，始终落在「字符 + 其后的组合字符」的边界上。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
}

impl TextInput {
    pub fn new(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.len() }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn insert(&mut self, c: char) {
        if c.is_control() {
            return;
        }
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// 插入一段文本（粘贴）：换行与制表符变成空格，其余控制字符丢弃。
    pub fn insert_str(&mut self, text: &str) {
        for c in text.chars() {
            self.insert(if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c });
        }
    }

    /// 光标前一个字符（连同它的组合字符）的起点。
    fn prev_boundary(&self) -> usize {
        let mut at = self.cursor;
        for (i, c) in self.text[..self.cursor].char_indices().rev() {
            at = i;
            if c.width().unwrap_or(0) > 0 {
                break;
            }
        }
        at
    }

    /// 光标处字符（连同它的组合字符）的终点。
    fn next_boundary(&self) -> usize {
        let mut chars = self.text[self.cursor..].char_indices();
        if chars.next().is_none() {
            return self.cursor;
        }
        chars.find(|(_, c)| c.width().unwrap_or(0) > 0).map_or(self.text.len(), |(i, _)| self.cursor + i)
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn backspace(&mut self) {
        let start = self.prev_boundary();
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    pub fn delete(&mut self) {
        let end = self.next_boundary();
        self.text.replace_range(self.cursor..end, "");
    }

    /// 删除光标之前的全部内容。
    pub fn kill_to_start(&mut self) {
        self.text.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// 在 `max` 列内可见的片段，以及光标在其中的列。输入过长时横向滚动，保证光标可见。
    pub fn visible(&self, max: usize) -> (String, usize) {
        let max = max.max(1);
        // 从头开始丢字符，直到光标（自身占一列）能落在可见范围内。
        let mut start = 0;
        while width(&self.text[start..self.cursor]) >= max {
            start += self.text[start..].chars().next().map_or(1, char::len_utf8);
        }
        let mut out = String::new();
        let mut used = 0;
        for c in self.text[start..].chars() {
            let w = c.width().unwrap_or(0);
            if used + w > max {
                break;
            }
            out.push(c);
            used += w;
        }
        (out, width(&self.text[start..self.cursor]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editing_moves_by_whole_characters() {
        let mut input = TextInput::default();
        input.insert_str("周杰伦abc");
        input.home();
        input.right();
        input.insert('·');
        assert_eq!(input.text(), "周·杰伦abc");
        input.end();
        input.backspace();
        input.left();
        input.left();
        input.delete();
        assert_eq!(input.text(), "周·杰伦b");
        input.left();
        input.backspace();
        assert_eq!(input.text(), "周·伦b");
        input.home();
        input.backspace();
        input.left();
        input.end();
        input.delete();
        input.right();
        assert_eq!(input.text(), "周·伦b");
        input.left();
        input.kill_to_start();
        assert_eq!(input.text(), "b");
    }

    #[test]
    fn combining_characters_stay_attached() {
        // e + 组合尖音符：一次移动 / 删除处理整个字符。
        let mut input = TextInput::new("ae\u{301}b");
        input.left();
        input.left();
        assert_eq!(&input.text()[input.cursor..], "e\u{301}b");
        input.delete();
        assert_eq!(input.text(), "ab");
        let mut input = TextInput::new("e\u{301}");
        input.backspace();
        assert!(input.is_empty());
    }

    #[test]
    fn paste_is_sanitised() {
        let mut input = TextInput::default();
        input.insert_str("晴天\n周杰伦\tq\u{1b}[A\u{7}");
        assert_eq!(input.text(), "晴天 周杰伦 q[A");
    }

    #[test]
    fn long_input_scrolls_to_keep_the_cursor_visible() {
        let mut input = TextInput::new("一二三四五六七八九十");
        let (text, column) = input.visible(9);
        assert!(width(&text) <= 9 && text.ends_with('十'), "{text}");
        assert_eq!(column, width(&text));
        input.home();
        let (text, column) = input.visible(9);
        assert_eq!((text.as_str(), column), ("一二三四", 0));
        for _ in 0..6 {
            input.right();
        }
        let (text, column) = input.visible(9);
        assert!(width(&text) <= 9 && column < 9, "{text} {column}");
        assert!(text.contains('六'), "{text}");
        // 极窄的输入框也不 panic。
        for max in 0..4 {
            let (text, column) = input.visible(max);
            assert!(width(&text) <= max.max(1) && column < max.max(1), "{max}: {text} {column}");
        }
    }

    #[test]
    fn truncate_and_wrap_respect_display_width() {
        assert_eq!(truncate("晴天", 10), "晴天");
        assert_eq!(truncate("故事的小黄花", 7), "故事的…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(wrap("从出生那年就飘着", 8), ["从出生那", "年就飘着"]);
        assert_eq!(wrap("", 8), [""]);
        assert!(wrap("童年的荡秋千", 1).iter().all(|l| width(l) <= 2));
    }
}
