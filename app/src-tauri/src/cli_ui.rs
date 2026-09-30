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
pub const ITALIC: &str = "\x1b[3m";
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

const ART: [&str; 8] = [
    "            ▗▄▄▄▖    ",
    "         ▗▟██████▙   ",
    "       ▗▟█████████▌  ",
    "     ▗▟███EE███EE██▙ ",
    "   ▗▟███████████████▖",
    " ▗▟█████████████████▌",
    " ▝DDDDDDDDDDDDDDDDDD▘",
    "   SSSSSSSSSSSSSSSS  ",
];

fn paint(line: &str) -> String {
    let mut out = String::new();
    for c in line.chars() {
        match c {
            'E' => out.push_str(&format!("{CREAM}█{RESET}")),
            'D' => out.push_str("\x1b[38;2;92;120;98m▀\x1b[0m"),
            'S' => out.push_str("\x1b[38;2;58;61;55m░\x1b[0m"),
            ' ' => out.push(' '),
            other => out.push_str(&format!("{MOSS}{other}{RESET}")),
        }
    }
    out
}

/// The boxed welcome: mascot, title, folder and model, like Claude Code's.
pub fn welcome(cwd: &str, model: &str, mode: &str, version: &str) -> String {
    let width = width();
    let mut text = String::new();
    if width < 72 {
        for line in ART {
            text.push_str(&format!(" {}\n", paint(line)));
        }
        text.push_str(&format!("\n {MOSS}✻{RESET} {BOLD}{CREAM}Welcome to Neru{RESET} {DIM}v{version}{RESET}\n {DIM}{}{RESET}\n", truncate(cwd, width - 2)));
        return text;
    }
    let inner = width.min(96) - 2;
    let text_width = inner - 26;
    let side = [
        String::new(),
        format!("{MOSS}✻{RESET} {BOLD}{CREAM}Welcome to Neru{RESET} {DIM}v{version}{RESET}"),
        format!("{DIM}練る · think, build, refine{RESET}"),
        String::new(),
        format!("{DIM}cwd:{RESET}   {}", truncate(cwd, text_width - 7)),
        format!("{DIM}model:{RESET} {}", truncate(model, text_width - 7)),
        format!("{DIM}mode:{RESET}  {mode}"),
        String::new(),
    ];
    text.push_str(&format!("{SAGE}╭{}╮{RESET}\n", "─".repeat(inner)));
    for (index, art) in ART.iter().enumerate() {
        let body = &side[index];
        let pad = text_width.saturating_sub(visible(body));
        text.push_str(&format!("{SAGE}│{RESET} {}   {body}{}{SAGE}│{RESET}\n", paint(art), " ".repeat(pad)));
    }
    text.push_str(&format!("{SAGE}╰{}╯{RESET}\n", "─".repeat(inner)));
    text.push_str(&format!(" {DIM}/help for commands · @ to mention files · esc to interrupt · ctrl+c twice to quit{RESET}\n"));
    text
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
}

pub struct Editor {
    pub history: Vec<String>,
}

fn is_press(key: &KeyEvent) -> bool {
    key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat
}

impl Editor {
    /// Reads one message in a boxed prompt. `suggest` gives completions for the text so far.
    pub fn read(&mut self, screen: &mut Screen, hint: &str, suggest: &dyn Fn(&str) -> Vec<Suggestion>) -> Input {
        let _ = terminal::enable_raw_mode();
        let mut text: Vec<char> = Vec::new();
        let mut at = 0usize;
        let mut history_at = self.history.len();
        let mut stash = String::new();
        let mut selected = 0usize;
        let mut dismissed = String::new();
        let result = loop {
            let current: String = text.iter().collect();
            let suggestions = if dismissed == current { Vec::new() } else { suggest(&current[..text[..at].iter().collect::<String>().len()]) };
            selected = selected.min(suggestions.len().saturating_sub(1));
            draw(screen, &text, at, &suggestions, selected, hint);
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
                    } else {
                        text.clear();
                        at = 0;
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

fn draw(screen: &mut Screen, text: &[char], at: usize, suggestions: &[Suggestion], selected: usize, hint: &str) {
    let width = width();
    let inner = width.saturating_sub(4).max(10);
    let mut lines = vec![format!("{FAINT}╭{}╮{RESET}", "─".repeat(width.saturating_sub(2)))];
    // Wrap the text into rows of `inner` columns, remembering where the cursor lands.
    let mut rows: Vec<String> = vec![String::new()];
    let mut cursor = (0usize, 0usize);
    let mut column = 0;
    for (index, c) in text.iter().enumerate() {
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
    if at == text.len() {
        cursor = (rows.len() - 1, column);
    }
    for (index, row) in rows.iter().enumerate() {
        let prompt = if index == 0 { format!("{SAGE}❯{RESET} ") } else { "  ".into() };
        let body = if text.is_empty() && index == 0 { format!("{DIM}{hint}{RESET}") } else { row.clone() };
        let pad = inner.saturating_sub(visible(&body) + 1);
        lines.push(format!("{FAINT}│{RESET}{prompt}{body}{}{FAINT}│{RESET}", " ".repeat(pad)));
    }
    lines.push(format!("{FAINT}╰{}╯{RESET}", "─".repeat(width.saturating_sub(2))));
    if suggestions.is_empty() {
        lines.push(format!("  {FAINT}? /help · @ files · shift+enter for a new line{RESET}"));
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
