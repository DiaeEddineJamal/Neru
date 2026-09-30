//! Terminal rendering for the `neru` CLI: colors, Markdown with VS Code Dark+ code highlighting,
//! a footer that is redrawn in place (status spinner, input box, suggestions), a line editor with
//! history and completion, and a type-to-filter picker.

use std::{
    io::{Write, stdout},
    time::{Duration, Instant},
};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal,
};

// ---------- colors ----------

pub const MOSS: &str = "\x1b[38;2;122;154;128m";
pub const SAGE: &str = "\x1b[38;2;142;162;145m";
pub const CREAM: &str = "\x1b[38;2;242;240;233m";
pub const DIM: &str = "\x1b[38;2;128;132;124m";
pub const FAINT: &str = "\x1b[38;2;90;94;88m";
pub const RED: &str = "\x1b[38;2;224;120;107m";
pub const GREEN: &str = "\x1b[38;2;142;190;140m";
pub const YELLOW: &str = "\x1b[38;2;217;184;112m";
pub const CYAN: &str = "\x1b[38;2;127;181;173m";
pub const BOLD: &str = "\x1b[1m";
pub const STRIKE: &str = "\x1b[9m";
pub const RESET: &str = "\x1b[0m";
const DIFF_ADD_BG: &str = "\x1b[48;2;28;52;34m";
const DIFF_DEL_BG: &str = "\x1b[48;2;64;30;30m";

pub fn width() -> usize {
    terminal::size().map(|(w, _)| w as usize).unwrap_or(100).clamp(40, 240)
}

/// Visible width of text with ANSI codes removed (CJK counts double).
pub fn visible(text: &str) -> usize {
    let mut count = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if ('\u{1100}'..='\u{115f}').contains(&c) || ('\u{2e80}'..='\u{a4cf}').contains(&c) || ('\u{ac00}'..='\u{d7a3}').contains(&c) || ('\u{f900}'..='\u{faff}').contains(&c) || ('\u{ff00}'..='\u{ff60}').contains(&c) {
            count += 2;
        } else {
            count += 1;
        }
    }
    count
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ---------- the screen: printed lines above, a footer redrawn below ----------

pub struct Screen {
    footer_lines: usize,
    /// Row (from the top of the footer) and column where the cursor should rest.
    cursor: Option<(usize, usize)>,
    pub color: bool,
}

impl Screen {
    pub fn new(color: bool) -> Self {
        Self { footer_lines: 0, cursor: None, color }
    }

    fn clear_footer(&mut self, out: &mut impl Write) {
        if self.footer_lines == 0 {
            return;
        }
        // The cursor may be parked inside the footer; go to its first row, then wipe downwards.
        let row = self.cursor.map_or(self.footer_lines - 1, |(row, _)| row);
        let _ = write!(out, "\r");
        if row > 0 {
            let _ = write!(out, "\x1b[{row}A");
        }
        let _ = write!(out, "\x1b[0J");
        self.footer_lines = 0;
        self.cursor = None;
    }

    /// Prints finished lines above the footer.
    pub fn print(&mut self, text: &str) {
        let mut out = stdout().lock();
        self.clear_footer(&mut out);
        for line in text.split('\n') {
            let _ = write!(out, "{}\r\n", if self.color { line.to_string() } else { strip(line) });
        }
        let _ = out.flush();
    }

    /// Redraws the footer; `cursor` places the caret (row within the footer, column).
    pub fn footer(&mut self, lines: &[String], cursor: Option<(usize, usize)>) {
        let mut out = stdout().lock();
        self.clear_footer(&mut out);
        let width = width();
        for (index, line) in lines.iter().enumerate() {
            let line = if visible(line) > width { strip_to(line, width) } else { line.clone() };
            let _ = write!(out, "{line}");
            if index + 1 < lines.len() {
                let _ = write!(out, "\r\n");
            }
        }
        self.footer_lines = lines.len();
        match cursor {
            Some((row, column)) => {
                let up = lines.len().saturating_sub(1).saturating_sub(row);
                if up > 0 {
                    let _ = write!(out, "\x1b[{up}A");
                }
                let _ = write!(out, "\r");
                if column > 0 {
                    let _ = write!(out, "\x1b[{column}C");
                }
                let _ = write!(out, "{}", cursor::Show);
                self.cursor = Some((row, column));
            }
            None => {
                let _ = write!(out, "{}", cursor::Hide);
                self.cursor = Some((lines.len().saturating_sub(1), 0));
            }
        }
        let _ = out.flush();
    }

    pub fn clear(&mut self) {
        let mut out = stdout().lock();
        self.clear_footer(&mut out);
        let _ = write!(out, "{}", cursor::Show);
        let _ = out.flush();
    }
}

pub fn strip(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Cuts a colored line to `width` visible columns, keeping its escapes.
fn strip_to(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut shown = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            out.push(c);
            for next in chars.by_ref() {
                out.push(next);
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            if shown + 1 >= width {
                out.push('…');
                break;
            }
            out.push(c);
            shown += 1;
        }
    }
    out.push_str(RESET);
    out
}

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn elapsed(since: Instant) -> String {
    let seconds = since.elapsed().as_secs();
    if seconds < 60 { format!("{seconds}s") } else { format!("{}m {:02}s", seconds / 60, seconds % 60) }
}

// ---------- welcome ----------

/// Neru's mascot, a paper-cut lump of dough, traced from the app icon. Each character is one pixel:
/// two pixel rows make one terminal row (drawn with ▀ and a background color).
/// L highlight, B body, D base, E eyes, S shadow.
const MASCOT: [&str; 16] = [
    "                    ",
    "            LBB     ",
    "          LLBBBB    ",
    "         LBBBBBB    ",
    "        LBBBBBBBB   ",
    "       LBBBBBBBBB   ",
    "      LBBBBBBBBBBB  ",
    "      LBBBBEBBEBBB  ",
    "     LBBBBBEBBEBBB  ",
    "    LBBBBBBEBBEBBBB ",
    "   LBBBBBBBBBBBBBBB ",
    " LLBBBBBBBBBBBBBBBB ",
    " BBBBBBBBBBBBBBBBBBB",
    " BBBBBBBBBBBBBBBBBB ",
    "SSSSSDDDDDDDDDDDDDSS",
    "   SSSSSSSSSSSSSS   ",
];

/// The same figure at half size, for narrow terminals.
const MASCOT_SMALL: [&str; 8] = [
    "     LB    ",
    "    LBBB   ",
    "   LBEBEB  ",
    "  LBBEBEBB ",
    " LBBBBBBBB ",
    "LBBBBBBBBBB",
    "SSDDDDDDDSS",
    " SSSSSSSSS ",
];

fn pixel(c: char) -> Option<(u8, u8, u8)> {
    Some(match c {
        'L' => (152, 180, 156),
        'B' => (122, 154, 128),
        'D' => (96, 124, 102),
        'E' => (242, 240, 233),
        'S' => (54, 58, 52),
        _ => return None,
    })
}

/// Terminal rows for a pixel figure. With `sparkle`, a small star sits by the top right, as in the icon.
fn figure(rows: &[&str], sparkle: bool) -> Vec<String> {
    let mut lines = Vec::new();
    for (row, pair) in rows.chunks(2).enumerate() {
        let top: Vec<char> = pair[0].chars().collect();
        let bottom: Vec<char> = pair.get(1).map_or(Vec::new(), |row| row.chars().collect());
        let mut line = String::new();
        for (index, &up) in top.iter().enumerate() {
            let down = bottom.get(index).copied().unwrap_or(' ');
            if sparkle && row == 0 && index + 3 == top.len() && pixel(up).is_none() && pixel(down).is_none() {
                line.push_str(&format!("{CREAM}✦{RESET}"));
                continue;
            }
            match (pixel(up), pixel(down)) {
                (None, None) => line.push(' '),
                (Some((r, g, b)), None) => line.push_str(&format!("\x1b[38;2;{r};{g};{b}m▀{RESET}")),
                (None, Some((r, g, b))) => line.push_str(&format!("\x1b[38;2;{r};{g};{b}m▄{RESET}")),
                (Some((r, g, b)), Some((r2, g2, b2))) => line.push_str(&format!("\x1b[38;2;{r};{g};{b};48;2;{r2};{g2};{b2}m▀{RESET}")),
            }
        }
        lines.push(line);
    }
    lines
}

/// What the welcome box shows.
pub struct Welcome<'a> {
    pub cwd: &'a str,
    pub model: &'a str,
    pub mode: &'a str,
    pub version: &'a str,
    /// Seen Neru here before: "Welcome back" instead of "Welcome to Neru".
    pub returning: bool,
    pub tips: Vec<String>,
    /// (title, when) of recent sessions in this folder.
    pub recent: Vec<(String, String)>,
}

fn center(text: &str, width: usize) -> String {
    let shown = visible(text);
    let left = width.saturating_sub(shown) / 2;
    format!("{}{text}{}", " ".repeat(left), " ".repeat(width.saturating_sub(shown + left)))
}

fn pad(text: &str, width: usize) -> String {
    let text = if visible(text) > width { strip_to(text, width) } else { text.to_string() };
    let shown = visible(&text);
    format!("{text}{}", " ".repeat(width.saturating_sub(shown)))
}

/// Shortens a path from the left, keeping its end: …\projects\neru.
fn tail(path: &str, max: usize) -> String {
    let count = path.chars().count();
    if count <= max {
        return path.to_string();
    }
    format!("…{}", path.chars().skip(count + 1 - max).collect::<String>())
}

/// The welcome box, like Claude Code's: the mascot and where you are on the left, tips and recent
/// sessions on the right, the version in the top border. Narrow terminals get a compact stack.
pub fn welcome(info: &Welcome) -> String {
    let width = width();
    let title = if info.returning { "Welcome back" } else { "Welcome to Neru" };
    if width < 78 {
        let art = figure(&MASCOT_SMALL, false);
        let inner = width.saturating_sub(16);
        let side = [
            format!("{BOLD}{CREAM}{title}{RESET} {DIM}v{}{RESET}", info.version),
            format!("{DIM}練る · think, build, refine{RESET}"),
            String::new(),
            format!("{SAGE}{}{RESET}", truncate(info.model, inner)),
        ];
        let mut text = String::new();
        for (index, line) in art.iter().enumerate() {
            text.push_str(&format!(" {line}   {}\n", side.get(index).cloned().unwrap_or_default()));
        }
        text.push_str(&format!(" {DIM}{}{RESET}\n {FAINT}/help commands · @ files · ! shell · shift+tab mode{RESET}\n", tail(info.cwd, width.saturating_sub(2))));
        return text;
    }
    let total = width.min(112);
    let inner = total - 2;
    let left = 34.min(inner / 2 - 2);
    let right = inner - left - 3;
    let label = format!(" {MOSS}✻{RESET} {BOLD}Neru{RESET} {DIM}v{}{RESET} ", info.version);
    let border = |text: &str| format!("{SAGE}{text}{RESET}");
    let mut left_col = vec![String::new(), center(&format!("{BOLD}{CREAM}{title}{RESET}"), left), String::new()];
    left_col.extend(figure(&MASCOT, true).iter().map(|line| center(line, left)));
    left_col.push(String::new());
    left_col.push(center(&format!("{SAGE}{}{RESET}", truncate(info.model, left)), left));
    left_col.push(center(&format!("{DIM}{}{RESET}", tail(info.cwd, left)), left));

    let heading = |text: &str| format!("{MOSS}{BOLD}{text}{RESET}");
    let mut right_col = vec![String::new(), heading("Tips for getting started")];
    for tip in &info.tips {
        right_col.push(truncate_visible(tip, right));
    }
    right_col.push(format!("{FAINT}{}{RESET}", "─".repeat(right.min(48))));
    right_col.push(heading("Recent activity"));
    if info.recent.is_empty() {
        right_col.push(format!("{DIM}No sessions in this folder yet{RESET}"));
    }
    for (title, when) in info.recent.iter().take(3) {
        let when = format!("{when:>8}");
        right_col.push(format!("{DIM}{when}{RESET}  {}", truncate(title, right.saturating_sub(11))));
    }
    right_col.push(String::new());
    right_col.push(format!("{DIM}mode{RESET}  {}", info.mode));

    let rows = left_col.len().max(right_col.len());
    let fill = inner.saturating_sub(visible(&label) + 3);
    let mut text = format!("{}{label}{}\n", border("╭───"), border(&format!("{}╮", "─".repeat(fill))));
    for index in 0..rows {
        let l = left_col.get(index).map_or(" ".repeat(left), |line| pad(line, left));
        let r = right_col.get(index).map_or(" ".repeat(right), |line| pad(line, right));
        text.push_str(&format!("{} {l} {} {r}{}\n", border("│"), border("│"), border("│")));
    }
    text.push_str(&format!("{}\n", border(&format!("╰{}╯", "─".repeat(inner)))));
    text
}

/// Truncates text that may hold color codes to `max` visible columns.
fn truncate_visible(text: &str, max: usize) -> String {
    if visible(text) <= max { text.to_string() } else { strip_to(text, max) }
}

// ---------- Markdown ----------

/// Renders one complete Markdown line; `fence` tracks whether we are inside a code block.
pub fn markdown_line(line: &str, fence: &mut Option<String>) -> String {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        if fence.is_some() {
            *fence = None;
            return format!("{FAINT}  └{}{RESET}", "─".repeat(20));
        }
        let lang = trimmed.trim_start_matches('`').trim().to_string();
        let label = if lang.is_empty() { String::new() } else { format!(" {lang} ") };
        *fence = Some(lang);
        return format!("{FAINT}  ┌─{label}{}{RESET}", "─".repeat(18usize.saturating_sub(label.len())));
    }
    if let Some(lang) = fence {
        return format!("{FAINT}  │{RESET} {}", highlight(line, lang));
    }
    if trimmed.is_empty() {
        return String::new();
    }
    let indent = &line[..line.len() - trimmed.len()];
    if let Some(rest) = trimmed.strip_prefix("### ").or_else(|| trimmed.strip_prefix("#### ")) {
        return format!("{BOLD}{}{RESET}", inline(rest));
    }
    if let Some(rest) = trimmed.strip_prefix("## ").or_else(|| trimmed.strip_prefix("# ")) {
        return format!("{BOLD}{CREAM}{}{RESET}", inline(rest));
    }
    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        let (mark, rest) = if let Some(rest) = rest.strip_prefix("[x] ").or_else(|| rest.strip_prefix("[X] ")) {
            (format!("{GREEN}☒{RESET}"), rest)
        } else if let Some(rest) = rest.strip_prefix("[ ] ") {
            ("☐".to_string(), rest)
        } else {
            (format!("{DIM}•{RESET}"), rest)
        };
        return format!("{indent}{mark} {}", inline(rest));
    }
    if trimmed.starts_with('>') {
        return format!("{DIM}▎ {}{RESET}", inline(trimmed.trim_start_matches('>').trim_start()));
    }
    if trimmed.chars().all(|c| matches!(c, '-' | '*' | '_')) && trimmed.len() >= 3 {
        return format!("{FAINT}{}{RESET}", "─".repeat(width().min(60)));
    }
    format!("{indent}{}", inline(trimmed))
}

/// Bold, italics, inline code and links within a line.
fn inline(text: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '`') {
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                out.push_str(&format!("{CYAN}{code}{RESET}"));
                i += end + 2;
                continue;
            }
        }
        if chars[i] == '*' && chars.get(i + 1) == Some(&'*') {
            if let Some(end) = (i + 2..chars.len().saturating_sub(1)).find(|&j| chars[j] == '*' && chars[j + 1] == '*') {
                let bold: String = chars[i + 2..end].iter().collect();
                out.push_str(&format!("{BOLD}{bold}{RESET}"));
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '[' {
            if let Some(close) = chars[i..].iter().position(|c| *c == ']').map(|p| p + i) {
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close..].iter().position(|c| *c == ')').map(|p| p + close) {
                        let label: String = chars[i + 1..close].iter().collect();
                        let url: String = chars[close + 2..end].iter().collect();
                        out.push_str(&format!("\x1b]8;;{url}\x1b\\{SAGE}\x1b[4m{label}\x1b[24m{RESET}\x1b]8;;\x1b\\"));
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ---------- syntax highlighting (VS Code Dark+ colors) ----------

const KEYWORD: &str = "\x1b[38;2;86;156;214m";
const CONTROL: &str = "\x1b[38;2;197;134;192m";
const STRING: &str = "\x1b[38;2;206;145;120m";
const NUMBER: &str = "\x1b[38;2;181;206;168m";
const COMMENT: &str = "\x1b[38;2;106;153;85m";
const FUNCTION: &str = "\x1b[38;2;220;220;170m";
const TYPE: &str = "\x1b[38;2;78;201;176m";
const VARIABLE: &str = "\x1b[38;2;156;220;254m";
const TAG: &str = "\x1b[38;2;86;156;214m";
const PUNCT: &str = "\x1b[38;2;128;128;128m";
const PLAIN: &str = "\x1b[38;2;212;212;212m";

fn language_family(lang: &str) -> &'static str {
    match lang.to_ascii_lowercase().as_str() {
        "html" | "htm" | "xml" | "svg" | "vue" | "svelte" => "markup",
        "css" | "scss" | "less" => "css",
        "py" | "python" => "python",
        "sh" | "bash" | "zsh" | "shell" | "ps1" | "powershell" | "pwsh" | "console" => "shell",
        "json" | "jsonc" => "json",
        "rs" | "rust" => "rust",
        "" | "text" | "txt" | "plaintext" => "text",
        _ => "c",
    }
}

fn keyword_color(word: &str, family: &str) -> Option<&'static str> {
    const CONTROL_WORDS: &[&str] = &["if", "else", "for", "while", "do", "return", "break", "continue", "switch", "case", "default", "try", "catch", "finally", "throw", "await", "yield", "import", "export", "from", "match", "loop", "elif", "except", "raise", "with", "as", "in", "of", "use", "mod", "pub", "then", "fi", "done", "esac"];
    const KEYWORDS: &[&str] = &["const", "let", "var", "function", "class", "new", "this", "typeof", "instanceof", "async", "static", "extends", "implements", "interface", "type", "enum", "true", "false", "null", "undefined", "void", "public", "private", "protected", "readonly", "fn", "impl", "struct", "trait", "self", "Self", "mut", "ref", "where", "def", "lambda", "None", "True", "False", "and", "or", "not", "is", "pass", "global", "int", "float", "string", "bool", "char", "double", "long", "super", "package", "func", "go", "defer", "echo", "local", "export"];
    if family == "text" || family == "json" {
        return None;
    }
    if CONTROL_WORDS.contains(&word) {
        Some(CONTROL)
    } else if KEYWORDS.contains(&word) {
        Some(KEYWORD)
    } else {
        None
    }
}

/// One line of code in Dark+ colors. Line-local: block comments spanning lines are not tracked.
pub fn highlight(line: &str, lang: &str) -> String {
    let family = language_family(lang);
    if family == "text" {
        return format!("{PLAIN}{line}{RESET}");
    }
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let push = |out: &mut String, color: &str, text: &str| {
        out.push_str(color);
        out.push_str(text);
        out.push_str(RESET);
    };
    let starts = |at: usize, pat: &str| -> bool { pat.chars().enumerate().all(|(k, c)| chars.get(at + k) == Some(&c)) };
    while i < chars.len() {
        let c = chars[i];
        // Comments
        let comment = match family {
            "python" | "shell" => c == '#',
            "markup" => starts(i, "<!--"),
            "css" => starts(i, "/*"),
            _ => starts(i, "//") || starts(i, "/*") || (family == "c" && starts(i, "#!")),
        };
        if comment {
            let rest: String = chars[i..].iter().collect();
            push(&mut out, COMMENT, &rest);
            break;
        }
        // Strings
        if matches!(c, '"' | '\'' | '`') {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            let text: String = chars[i..end].iter().collect();
            let color = if family == "json" && chars[end..].iter().find(|c| !c.is_whitespace()) == Some(&':') { VARIABLE } else { STRING };
            push(&mut out, color, &text);
            i = end;
            continue;
        }
        // Markup tags and attributes
        if family == "markup" && c == '<' {
            let mut j = i + 1;
            if chars.get(j) == Some(&'/') {
                j += 1;
            }
            let name_start = j;
            while j < chars.len() && (chars[j].is_alphanumeric() || matches!(chars[j], '-' | ':' | '!')) {
                j += 1;
            }
            push(&mut out, PUNCT, &chars[i..name_start].iter().collect::<String>());
            push(&mut out, TAG, &chars[name_start..j].iter().collect::<String>());
            i = j;
            continue;
        }
        if family == "markup" && (c == '>' || (c == '/' && chars.get(i + 1) == Some(&'>'))) {
            let end = if c == '/' { i + 2 } else { i + 1 };
            push(&mut out, PUNCT, &chars[i..end].iter().collect::<String>());
            i = end;
            continue;
        }
        // Numbers
        if c.is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric() && chars[i - 1] != '_') {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '%') {
                j += 1;
            }
            push(&mut out, NUMBER, &chars[i..j].iter().collect::<String>());
            i = j;
            continue;
        }
        // Words
        if c.is_alphabetic() || c == '_' || c == '$' || (family == "css" && c == '-') {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || matches!(chars[j], '_' | '$') || (matches!(family, "css" | "markup") && chars[j] == '-')) {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let next = chars[j..].iter().find(|c| !c.is_whitespace()).copied();
            let color = if family == "markup" {
                if next == Some('=') { VARIABLE } else { PLAIN }
            } else if family == "css" {
                if next == Some(':') && !line.contains('{') { VARIABLE } else if line.trim_start().starts_with(&word) && line.contains('{') { "\x1b[38;2;215;186;125m" } else { STRING }
            } else if let Some(color) = keyword_color(&word, family) {
                color
            } else if next == Some('(') {
                FUNCTION
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                TYPE
            } else {
                VARIABLE
            };
            push(&mut out, color, &word);
            i = j;
            continue;
        }
        out.push_str(PLAIN);
        out.push(c);
        out.push_str(RESET);
        i += 1;
    }
    out
}

pub fn language_for_path(path: &str) -> &str {
    path.rsplit('.').next().filter(|ext| ext.len() < path.len()).unwrap_or("")
}

/// A few lines of a file with line numbers, as Claude Code shows after a write.
pub fn code_preview(content: &str, path: &str, max: usize) -> String {
    let lang = language_for_path(path);
    let lines: Vec<&str> = content.lines().collect();
    let mut text = String::new();
    for (index, line) in lines.iter().take(max).enumerate() {
        text.push_str(&format!("      {FAINT}{:>4}{RESET} {}\n", index + 1, highlight(&truncate(line, width().saturating_sub(14)), lang)));
    }
    if lines.len() > max {
        text.push_str(&format!("      {DIM}… +{} lines{RESET}\n", lines.len() - max));
    }
    text.trim_end_matches('\n').to_string()
}

/// A unified diff in red and green, like Claude Code's approval view.
pub fn diff(diff: &str, max: usize) -> String {
    let mut text = String::new();
    let mut shown = 0;
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let width = width().saturating_sub(12);
    let total = diff.lines().filter(|line| (line.starts_with('+') || line.starts_with('-')) && !line.starts_with("+++") && !line.starts_with("---")).count();
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("diff ") || line.starts_with("index ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@") {
            // "@@ -a,b +c,d @@"
            let numbers: Vec<usize> = rest.split(|c: char| !c.is_ascii_digit()).filter_map(|n| n.parse().ok()).collect();
            old_line = numbers.first().copied().unwrap_or(1);
            new_line = numbers.get(2).copied().unwrap_or(1);
            if shown > 0 {
                text.push_str(&format!("   {FAINT}   ⋮{RESET}\n"));
            }
            continue;
        }
        if shown >= max {
            text.push_str(&format!("   {DIM}… more changes not shown{RESET}\n"));
            break;
        }
        let body = truncate(line.get(1..).unwrap_or(""), width);
        match line.chars().next() {
            Some('+') => {
                text.push_str(&format!("   {GREEN}{:>4} +{RESET}{DIFF_ADD_BG} {body}{}{RESET}\n", new_line, " ".repeat(width.saturating_sub(visible(&body)))));
                new_line += 1;
                shown += 1;
            }
            Some('-') => {
                text.push_str(&format!("   {RED}{:>4} -{RESET}{DIFF_DEL_BG} {body}{}{RESET}\n", old_line, " ".repeat(width.saturating_sub(visible(&body)))));
                old_line += 1;
                shown += 1;
            }
            _ => {
                text.push_str(&format!("   {FAINT}{:>4}  {RESET}{DIM} {body}{RESET}\n", new_line));
                old_line += 1;
                new_line += 1;
            }
        }
    }
    if total == 0 {
        text.push_str(&format!("   {DIM}(no line changes){RESET}\n"));
    }
    text.trim_end_matches('\n').to_string()
}

// ---------- input ----------

#[derive(Clone)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    /// Replaces the word being completed.
    pub insert: String,
}

pub enum Input {
    Line(String),
    /// Ctrl+C on an empty line (the caller decides whether that quits).
    Interrupt,
    /// Ctrl+D on an empty line.
    Eof,
    /// Shift+Tab: switch to the next permission mode. What was typed is kept for the next read.
    CycleMode,
    /// Esc twice on an empty line: step back to an earlier message.
    Rewind,
}

/// The prompt box's placeholder and the status line under it.
pub struct Prompt<'a> {
    pub hint: &'a str,
    /// Left of the status line: the permission mode, or the shortcuts hint.
    pub left: String,
    /// Right of the status line: model and context use.
    pub right: String,
}

pub struct Editor {
    pub history: Vec<String>,
    /// Text kept across a mode switch, and the cursor within it.
    draft: Option<(Vec<char>, usize)>,
}

fn is_press(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat
}

impl Editor {
    pub fn new(history: Vec<String>) -> Self {
        Self { history, draft: None }
    }

    /// Ctrl+R: type to find an earlier message, ctrl+r again for an older match, enter to use it.
    fn search_history(&self, screen: &mut Screen) -> Option<String> {
        let mut query = String::new();
        let mut skip = 0usize;
        loop {
            let lower = query.to_lowercase();
            let matches: Vec<&String> = self.history.iter().rev().filter(|line| lower.is_empty() || line.to_lowercase().contains(&lower)).collect();
            let found = matches.get(skip.min(matches.len().saturating_sub(1))).copied();
            let shown = found.map_or(format!("{DIM}no match{RESET}"), |line| line.replace('\n', " ⏎ "));
            screen.footer(&[format!(" {SAGE}history search{RESET} {BOLD}{query}{RESET}{DIM}▏{RESET}"), format!(" {CREAM}{}{RESET}", truncate(&strip(&shown), width().saturating_sub(4))), format!(" {FAINT}ctrl+r older · enter use · esc cancel{RESET}")], None);
            let Ok(Event::Key(key)) = event::read() else { continue };
            if !is_press(&key) {
                continue;
            }
            match key.code {
                KeyCode::Esc => return None,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return None,
                KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => skip = (skip + 1).min(matches.len().saturating_sub(1)),
                KeyCode::Enter | KeyCode::Tab => return found.cloned(),
                KeyCode::Backspace => {
                    query.pop();
                    skip = 0;
                }
                KeyCode::Char(c) => {
                    query.push(c);
                    skip = 0;
                }
                _ => {}
            }
        }
    }

    /// Text the next read starts with, such as a rewound message to edit and send again.
    pub fn set_draft(&mut self, text: &str) {
        let chars: Vec<char> = text.chars().collect();
        let at = chars.len();
        self.draft = Some((chars, at));
    }

    /// Reads one message in a boxed prompt. `suggest` gives completions for the text so far.
    pub fn read(&mut self, screen: &mut Screen, prompt: &Prompt, suggest: &dyn Fn(&str) -> Vec<Suggestion>) -> Input {
        let _ = terminal::enable_raw_mode();
        let (mut text, mut at) = self.draft.take().unwrap_or_default();
        let mut history_at = self.history.len();
        let mut stash = String::new();
        let mut selected = 0usize;
        let mut dismissed = String::new();
        let mut last_escape: Option<Instant> = None;
        let result = loop {
            let current: String = text.iter().collect();
            let suggestions = if dismissed == current { Vec::new() } else { suggest(&current[..text[..at].iter().collect::<String>().len()]) };
            selected = selected.min(suggestions.len().saturating_sub(1));
            draw(screen, &text, at, &suggestions, selected, prompt);
            let Ok(event) = event::read() else { break Input::Eof };
            let key = match event {
                Event::Key(key) if is_press(&key) => key,
                Event::Paste(paste) => {
                    for c in paste.replace("\r\n", "\n").replace('\r', "\n").chars() {
                        text.insert(at, c);
                        at += 1;
                    }
                    continue;
                }
                Event::Resize(_, _) => continue,
                _ => continue,
            };
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Enter => {
                    // Keys already waiting mean a paste is arriving: keep its newlines.
                    let pasting = event::poll(Duration::from_millis(0)).unwrap_or(false);
                    if pasting || key.modifiers.contains(KeyModifiers::SHIFT) || key.modifiers.contains(KeyModifiers::ALT) {
                        text.insert(at, '\n');
                        at += 1;
                        continue;
                    }
                    if at > 0 && text.get(at - 1) == Some(&'\\') {
                        text[at - 1] = '\n';
                        continue;
                    }
                    if let Some(choice) = suggestions.get(selected) {
                        // Enter on a highlighted command runs it; on a file mention it completes it.
                        let (start, _) = word_bounds(&text, at);
                        let replaced: String = text[start..at].iter().collect();
                        if replaced != choice.insert.trim_end() {
                            complete(&mut text, &mut at, &choice.insert);
                            if !choice.insert.starts_with('/') {
                                continue;
                            }
                        }
                    }
                    let line: String = text.iter().collect();
                    break Input::Line(line);
                }
                KeyCode::BackTab => {
                    self.draft = Some((text.clone(), at));
                    break Input::CycleMode;
                }
                KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    self.draft = Some((text.clone(), at));
                    break Input::CycleMode;
                }
                KeyCode::Tab => {
                    if let Some(choice) = suggestions.get(selected) {
                        complete(&mut text, &mut at, &choice.insert);
                    }
                }
                KeyCode::Char('c') if ctrl => {
                    if text.is_empty() {
                        break Input::Interrupt;
                    }
                    text.clear();
                    at = 0;
                }
                KeyCode::Char('d') if ctrl => {
                    if text.is_empty() {
                        break Input::Eof;
                    }
                    if at < text.len() {
                        text.remove(at);
                    }
                }
                KeyCode::Char('a') if ctrl => at = 0,
                KeyCode::Char('e') if ctrl => at = text.len(),
                KeyCode::Char('u') if ctrl => {
                    text.drain(..at);
                    at = 0;
                }
                KeyCode::Char('k') if ctrl => text.truncate(at),
                KeyCode::Char('w') if ctrl => {
                    let (start, _) = word_bounds(&text, at);
                    let start = if start == at { text[..at].iter().rposition(|c| !c.is_whitespace()).map_or(0, |p| text[..p].iter().rposition(|c| c.is_whitespace()).map_or(0, |q| q + 1)) } else { start };
                    text.drain(start..at);
                    at = start;
                }
                KeyCode::Char('l') if ctrl => {
                    screen.clear();
                    let _ = write!(stdout(), "\x1b[2J\x1b[H");
                }
                KeyCode::Char('r') if ctrl => {
                    if let Some(found) = self.search_history(screen) {
                        text = found.chars().collect();
                        at = text.len();
                        dismissed = found;
                    }
                }
                KeyCode::Char('j') if ctrl => {
                    text.insert(at, '\n');
                    at += 1;
                }
                KeyCode::Char(c) => {
                    text.insert(at, c);
                    at += 1;
                    selected = 0;
                }
                KeyCode::Backspace => {
                    if at > 0 {
                        at -= 1;
                        text.remove(at);
                        selected = 0;
                    }
                }
                KeyCode::Delete => {
                    if at < text.len() {
                        text.remove(at);
                    }
                }
                KeyCode::Left => at = at.saturating_sub(1),
                KeyCode::Right => at = (at + 1).min(text.len()),
                KeyCode::Home => at = 0,
                KeyCode::End => at = text.len(),
                KeyCode::Up => {
                    if !suggestions.is_empty() {
                        selected = if selected == 0 { suggestions.len() - 1 } else { selected - 1 };
                    } else if text[..at].contains(&'\n') {
                        at = line_move(&text, at, -1);
                    } else if history_at > 0 {
                        if history_at == self.history.len() {
                            stash = text.iter().collect();
                        }
                        history_at -= 1;
                        text = self.history[history_at].chars().collect();
                        at = text.len();
                        dismissed = self.history[history_at].clone();
                    }
                }
                KeyCode::Down => {
                    if !suggestions.is_empty() {
                        selected = (selected + 1) % suggestions.len();
                    } else if text[at..].contains(&'\n') {
                        at = line_move(&text, at, 1);
                    } else if history_at < self.history.len() {
                        history_at += 1;
                        text = if history_at == self.history.len() { stash.chars().collect() } else { self.history[history_at].chars().collect() };
                        at = text.len();
                        dismissed = text.iter().collect();
                    }
                }
                KeyCode::Esc => {
                    if !suggestions.is_empty() {
                        dismissed = text.iter().collect();
                    } else if !text.is_empty() {
                        text.clear();
                        at = 0;
                    } else if last_escape.is_some_and(|at| at.elapsed() < Duration::from_millis(800)) {
                        break Input::Rewind;
                    } else {
                        last_escape = Some(Instant::now());
                    }
                }
                _ => {}
            }
        };
        screen.clear();
        let _ = terminal::disable_raw_mode();
        if let Input::Line(line) = &result {
            let line = line.trim().to_string();
            if !line.is_empty() && self.history.last() != Some(&line) {
                self.history.push(line);
            }
        }
        result
    }
}

fn line_move(text: &[char], at: usize, direction: i32) -> usize {
    let start = text[..at].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1);
    let column = at - start;
    if direction < 0 {
        let prev_end = start.saturating_sub(1);
        let prev_start = text[..prev_end].iter().rposition(|c| *c == '\n').map_or(0, |p| p + 1);
        (prev_start + column).min(prev_end)
    } else {
        let Some(end) = text[at..].iter().position(|c| *c == '\n').map(|p| p + at) else { return at };
        let next_end = text[end + 1..].iter().position(|c| *c == '\n').map_or(text.len(), |p| p + end + 1);
        (end + 1 + column).min(next_end)
    }
}

/// The word under the cursor (for completion): from the last space before it.
fn word_bounds(text: &[char], at: usize) -> (usize, usize) {
    let start = text[..at].iter().rposition(|c| c.is_whitespace()).map_or(0, |p| p + 1);
    (start, at)
}

fn complete(text: &mut Vec<char>, at: &mut usize, insert: &str) {
    let (start, end) = word_bounds(text, *at);
    text.splice(start..end, insert.chars());
    *at = start + insert.chars().count();
}

/// Pink for shell commands (`!`), cyan for things to remember (`#`), quiet otherwise.
const SHELL_MODE: &str = "\x1b[38;2;214;140;176m";

const SHORTCUTS: [(&str, &str); 12] = [
    ("/", "commands"),
    ("@", "mention a file"),
    ("!", "run a shell command"),
    ("#", "remember something"),
    ("shift+tab", "change permission mode"),
    ("esc", "interrupt Neru"),
    ("esc esc", "rewind to a message"),
    ("shift+enter", "new line (or \\ enter)"),
    ("↑ ↓", "history"),
    ("ctrl+r", "search history"),
    ("ctrl+l", "clear the screen"),
    ("ctrl+c ×2", "quit"),
];

fn draw(screen: &mut Screen, text: &[char], at: usize, suggestions: &[Suggestion], selected: usize, prompt: &Prompt) {
    let width = width();
    let inner = width.saturating_sub(4).max(10);
    let (frame, mark) = match text.first() {
        Some('!') => (SHELL_MODE, format!("{SHELL_MODE}{BOLD}!{RESET}")),
        Some('#') => (CYAN, format!("{CYAN}{BOLD}#{RESET}")),
        _ => (FAINT, format!("{SAGE}❯{RESET}")),
    };
    // The mode character replaces the prompt mark, so it is not drawn twice.
    let skip = usize::from(matches!(text.first(), Some('!' | '#')));
    let mut lines = vec![format!("{frame}╭{}╮{RESET}", "─".repeat(width.saturating_sub(2)))];
    // Wrap the text into rows of `inner` columns, remembering where the cursor lands.
    let mut rows: Vec<String> = vec![String::new()];
    let mut cursor = (0usize, 0usize);
    let mut column = 0;
    for (index, c) in text.iter().enumerate().skip(skip) {
        if index == at {
            cursor = (rows.len() - 1, column);
        }
        if *c == '\n' || column >= inner - 1 {
            rows.push(String::new());
            column = 0;
            if *c == '\n' {
                continue;
            }
        }
        rows.last_mut().unwrap().push(*c);
        column += 1;
    }
    if at == text.len() || at < skip {
        cursor = if at < skip { (0, 0) } else { (rows.len() - 1, column) };
    }
    for (index, row) in rows.iter().enumerate() {
        let lead = if index == 0 { format!("{mark} ") } else { "  ".into() };
        let body = if text.is_empty() && index == 0 {
            format!("{DIM}{}{RESET}", prompt.hint)
        } else if index == 0 && skip == 1 && row.is_empty() {
            format!("{DIM}{}{RESET}", if text[0] == '!' { "Run a shell command in this project" } else { "Something Neru should remember in this project" })
        } else {
            row.clone()
        };
        let pad = inner.saturating_sub(visible(&body) + 1);
        lines.push(format!("{frame}│{RESET}{lead}{body}{}{frame}│{RESET}", " ".repeat(pad)));
    }
    lines.push(format!("{frame}╰{}╯{RESET}", "─".repeat(width.saturating_sub(2))));
    if text.len() == 1 && text[0] == '?' {
        // Claude Code's shortcut sheet: three columns under the box.
        let column = (width.saturating_sub(4) / 3).max(24);
        for chunk in SHORTCUTS.chunks(3) {
            let cells: Vec<String> = chunk.iter().map(|(key, what)| pad(&format!("{CREAM}{key}{RESET} {DIM}{what}{RESET}"), column)).collect();
            lines.push(format!("  {}", cells.join("")));
        }
    } else if suggestions.is_empty() {
        let left = match text.first() {
            Some('!') => format!("{SHELL_MODE}! shell mode{RESET} {DIM}· runs in this folder, the output goes to Neru{RESET}"),
            Some('#') => format!("{CYAN}# memory{RESET} {DIM}· saved to this project's memory{RESET}"),
            _ => prompt.left.clone(),
        };
        let room = width.saturating_sub(visible(&left) + 4);
        let right = if visible(&prompt.right) <= room { prompt.right.clone() } else { String::new() };
        let gap = width.saturating_sub(visible(&left) + visible(&right) + 3);
        lines.push(format!("  {left}{}{right}", " ".repeat(gap)));
    } else {
        let window = 8;
        let first = selected.saturating_sub(window - 1);
        for (index, item) in suggestions.iter().enumerate().skip(first).take(window) {
            let label = truncate(&item.label, 28);
            let detail = truncate(&item.detail, width.saturating_sub(34));
            if index == selected {
                lines.push(format!("  {SAGE}{BOLD}{label:<28}{RESET}  {CREAM}{detail}{RESET}"));
            } else {
                lines.push(format!("  {DIM}{label:<28}  {detail}{RESET}"));
            }
        }
        if suggestions.len() > window {
            lines.push(format!("  {FAINT}{} of {}{RESET}", selected + 1, suggestions.len()));
        }
    }
    screen.footer(&lines, Some((1 + cursor.0, 3 + cursor.1)));
}

/// Reads a secret, such as an API key, showing only dots. Esc or Ctrl+C cancels.
pub fn read_secret(screen: &mut Screen, label: &str) -> Option<String> {
    let _ = terminal::enable_raw_mode();
    let mut text = String::new();
    let result = loop {
        let dots = "•".repeat(text.chars().count().min(width().saturating_sub(12)));
        let body = if text.is_empty() { format!("{DIM}paste it here, then press enter{RESET}") } else { dots };
        screen.footer(&[format!(" {BOLD}{label}{RESET}"), format!(" {SAGE}❯{RESET} {body}"), format!(" {FAINT}stored encrypted on this computer · esc to cancel{RESET}")], Some((1, 3 + text.chars().count().min(width().saturating_sub(12)))));
        match event::read() {
            Ok(Event::Paste(paste)) => text.push_str(paste.trim()),
            Ok(Event::Key(key)) if is_press(&key) => match key.code {
                KeyCode::Enter => break Some(text.trim().to_string()),
                KeyCode::Esc => break None,
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break None,
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Char(c) => text.push(c),
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    screen.clear();
    let _ = terminal::disable_raw_mode();
    result.filter(|text| !text.is_empty())
}

// ---------- picker ----------

/// A list to choose from with arrows, typing to filter. Returns the chosen index into `items`.
pub fn pick(screen: &mut Screen, title: &str, items: &[(String, String)], current: Option<usize>) -> Option<usize> {
    if items.is_empty() {
        return None;
    }
    let _ = terminal::enable_raw_mode();
    let mut filter = String::new();
    let mut selected = current.unwrap_or(0);
    let result = loop {
        let terms: Vec<String> = filter.to_lowercase().split_whitespace().map(str::to_string).collect();
        let shown: Vec<usize> = (0..items.len()).filter(|&i| terms.iter().all(|t| items[i].0.to_lowercase().contains(t) || items[i].1.to_lowercase().contains(t))).collect();
        let position = shown.iter().position(|&i| i == selected).unwrap_or(0);
        if let Some(&first) = shown.get(position) {
            selected = first;
        }
        let window = 10;
        let start = position.saturating_sub(window - 1);
        let mut lines = vec![format!(" {BOLD}{title}{RESET}  {DIM}{}{RESET}", if filter.is_empty() { "type to filter · ↑↓ to move · enter to choose · esc to cancel".into() } else { format!("filter: {filter}") })];
        for &index in shown.iter().skip(start).take(window) {
            let (label, detail) = &items[index];
            let mark = if Some(index) == current { format!("{GREEN}✓{RESET}") } else { " ".into() };
            if index == selected {
                lines.push(format!(" {SAGE}❯{RESET} {mark} {BOLD}{}{RESET}  {DIM}{}{RESET}", truncate(label, 50), truncate(detail, 40)));
            } else {
                lines.push(format!("   {mark} {}  {FAINT}{}{RESET}", truncate(label, 50), truncate(detail, 40)));
            }
        }
        if shown.is_empty() {
            lines.push(format!("   {DIM}Nothing matches “{filter}”{RESET}"));
        } else if shown.len() > window {
            lines.push(format!("   {FAINT}{} of {}{RESET}", position + 1, shown.len()));
        }
        screen.footer(&lines, None);
        let Ok(Event::Key(key)) = event::read() else { continue };
        if !is_press(&key) {
            continue;
        }
        match key.code {
            KeyCode::Esc => break None,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break None,
            KeyCode::Enter => break shown.get(position).copied(),
            KeyCode::Up => selected = shown[(position + shown.len().max(1) - 1) % shown.len().max(1)],
            KeyCode::Down => selected = shown[(position + 1) % shown.len().max(1)],
            KeyCode::Backspace => {
                filter.pop();
            }
            KeyCode::Char(c) => {
                filter.push(c);
            }
            _ => {}
        }
    };
    screen.clear();
    let _ = terminal::disable_raw_mode();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_keep_the_text() {
        for (line, lang) in [("const x = foo(\"a\", 2); // hi", "js"), ("<a href=\"x\">link</a>", "html"), ("body { color: red; }", "css"), ("def f(x): return 'y'  # c", "py")] {
            assert_eq!(strip(&highlight(line, lang)), line);
        }
    }

    #[test]
    fn markdown_formats_lines_and_fences() {
        let mut fence = None;
        assert!(strip(&markdown_line("## Title", &mut fence)).contains("Title"));
        assert_eq!(strip(&markdown_line("- **bold** and `code`", &mut fence)), "• bold and code");
        markdown_line("```js", &mut fence);
        assert_eq!(fence.as_deref(), Some("js"));
        assert!(strip(&markdown_line("let a = 1", &mut fence)).ends_with("let a = 1"));
        markdown_line("```", &mut fence);
        assert!(fence.is_none());
    }

    #[test]
    fn diffs_count_and_color_lines() {
        let text = diff("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n keep\n-old\n+new\n", 20);
        let plain = strip(&text);
        assert!(plain.contains("- old") || plain.contains("-  old") || plain.contains("old"));
        assert!(plain.contains("new"));
    }
}

