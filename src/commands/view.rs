use crate::notebook;
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nbformat::v4::{Cell, Output};
use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

#[derive(Parser)]
pub struct ViewArgs {
    /// Path to notebook file
    pub file: String,

    /// Color scheme: dark, light, or auto (default: dark)
    #[arg(long, default_value = "dark")]
    pub theme: String,
}

struct App {
    notebook: nbformat::v4::Notebook,
    selected_cell: usize,
    scroll_offset: u16,
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
    is_dark_theme: bool,
    file_path: PathBuf,
}

impl App {
    fn new(notebook: nbformat::v4::Notebook, theme: &str, file_path: PathBuf) -> Self {
        let is_dark_theme = theme.to_lowercase() != "light";
        Self {
            notebook,
            selected_cell: 0,
            scroll_offset: 0,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
            is_dark_theme,
            file_path,
        }
    }

    fn reload(&mut self) -> Result<()> {
        // Try to reload the notebook, preserving current position
        match notebook::read_notebook(self.file_path.to_str().unwrap()) {
            Ok(new_notebook) => {
                self.notebook = new_notebook;
                // Clamp selected cell to new notebook size
                if self.selected_cell >= self.notebook.cells.len() {
                    self.selected_cell = self.notebook.cells.len().saturating_sub(1);
                }
                Ok(())
            }
            Err(e) => {
                // If reload fails, keep the old notebook
                Err(e)
            }
        }
    }

    fn next_cell(&mut self) {
        if self.selected_cell < self.notebook.cells.len().saturating_sub(1) {
            self.selected_cell += 1;
            self.scroll_offset = 0;
        }
    }

    fn previous_cell(&mut self) {
        if self.selected_cell > 0 {
            self.selected_cell -= 1;
            self.scroll_offset = 0;
        }
    }

    fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    fn jump_to_first(&mut self) {
        self.selected_cell = 0;
        self.scroll_offset = 0;
    }

    fn jump_to_last(&mut self) {
        self.selected_cell = self.notebook.cells.len().saturating_sub(1);
        self.scroll_offset = 0;
    }

    fn get_cell_type_str(&self, cell: &Cell) -> &str {
        match cell {
            Cell::Code { .. } => "Code",
            Cell::Markdown { .. } => "Markdown",
            Cell::Raw { .. } => "Raw",
        }
    }

    fn get_cell_type_symbol(&self, cell: &Cell) -> &str {
        match cell {
            Cell::Code { .. } => "⚡",
            Cell::Markdown { .. } => "▪",
            Cell::Raw { .. } => "○",
        }
    }

    // Convert RGB color to closest ANSI indexed color
    fn rgb_to_ansi(&self, r: u8, g: u8, b: u8) -> Color {
        // Calculate relative brightness
        let brightness = (r as u16 + g as u16 + b as u16) / 3;

        // Check if it's grayscale
        let is_gray = (r as i16 - g as i16).abs() < 30
            && (g as i16 - b as i16).abs() < 30
            && (r as i16 - b as i16).abs() < 30;

        if is_gray {
            // Grayscale handling - different for light vs dark themes
            if self.is_dark_theme {
                // Dark theme: use brighter grays
                if brightness < 60 {
                    return Color::Indexed(0); // Black
                } else if brightness < 120 {
                    return Color::Indexed(8); // Bright Black (dark gray)
                } else if brightness < 200 {
                    return Color::Indexed(7); // White (light gray)
                } else {
                    return Color::Indexed(15); // Bright White
                }
            } else {
                // Light theme: use darker grays
                if brightness < 80 {
                    return Color::Indexed(0); // Black
                } else if brightness < 160 {
                    return Color::Indexed(8); // Bright Black (gray) - good for comments
                } else {
                    return Color::Indexed(0); // Black - default text
                }
            }
        }

        // Determine dominant color channel
        let max_channel = r.max(g).max(b);
        let min_channel = r.min(g).min(b);
        let saturation = max_channel - min_channel;

        // Low saturation - treat as gray
        if saturation < 40 {
            if self.is_dark_theme {
                return if brightness > 128 {
                    Color::Indexed(7) // White
                } else {
                    Color::Indexed(8) // Bright Black (gray)
                };
            } else {
                return if brightness > 128 {
                    Color::Indexed(8) // Gray
                } else {
                    Color::Indexed(0) // Black
                };
            }
        }

        // Determine which color variant to use based on theme
        // For dark themes: use bright colors (9-15) for better visibility
        // For light themes: use normal colors (1-6) for better contrast
        let use_bright = if self.is_dark_theme {
            // Dark theme: bright if color is vibrant enough
            max_channel > 180 || brightness > 150
        } else {
            // Light theme: never use bright (they're too light), always use normal
            false
        };

        // Red channel dominant
        if r >= g && r >= b {
            if g > b + 40 {
                // Red + Green = Yellow
                return if use_bright {
                    Color::Indexed(11) // Bright Yellow
                } else {
                    Color::Indexed(3) // Yellow
                };
            } else if b > g + 40 {
                // Red + Blue = Magenta
                return if use_bright {
                    Color::Indexed(13) // Bright Magenta
                } else {
                    Color::Indexed(5) // Magenta
                };
            } else {
                // Pure Red
                return if use_bright {
                    Color::Indexed(9) // Bright Red
                } else {
                    Color::Indexed(1) // Red
                };
            }
        }

        // Green channel dominant
        if g >= r && g >= b {
            if b > r + 40 {
                // Green + Blue = Cyan
                return if use_bright {
                    Color::Indexed(14) // Bright Cyan
                } else {
                    Color::Indexed(6) // Cyan
                };
            } else if r > b + 40 {
                // Green + Red = Yellow
                return if use_bright {
                    Color::Indexed(11) // Bright Yellow
                } else {
                    Color::Indexed(3) // Yellow
                };
            } else {
                // Pure Green
                return if use_bright {
                    Color::Indexed(10) // Bright Green
                } else {
                    Color::Indexed(2) // Green
                };
            }
        }

        // Blue channel dominant
        if b >= r && b >= g {
            if r > g + 40 {
                // Blue + Red = Magenta
                return if use_bright {
                    Color::Indexed(13) // Bright Magenta
                } else {
                    Color::Indexed(5) // Magenta
                };
            } else if g > r + 40 {
                // Blue + Green = Cyan
                return if use_bright {
                    Color::Indexed(14) // Bright Cyan
                } else {
                    Color::Indexed(6) // Cyan
                };
            } else {
                // Pure Blue
                return if use_bright {
                    Color::Indexed(12) // Bright Blue
                } else {
                    Color::Indexed(4) // Blue
                };
            }
        }

        // Fallback
        Color::Indexed(7) // White
    }

    fn heading_color(&self, level: u8) -> Color {
        if self.is_dark_theme {
            match level {
                1 => Color::Indexed(14), // Bright Cyan
                2 => Color::Indexed(12), // Bright Blue
                3 => Color::Indexed(6),  // Cyan
                _ => Color::Indexed(4),  // Blue
            }
        } else {
            match level {
                1 => Color::Indexed(4),  // Blue
                2 => Color::Indexed(6),  // Cyan
                3 => Color::Indexed(12), // Bright Blue
                _ => Color::Indexed(14), // Bright Cyan
            }
        }
    }

    // Helper methods for Python highlighter fallback (currently unused)
    #[allow(dead_code)]
    fn keyword_color(&self) -> Color {
        if self.is_dark_theme {
            Color::Indexed(13) // Bright Magenta
        } else {
            Color::Indexed(5) // Magenta
        }
    }

    #[allow(dead_code)]
    fn string_color(&self) -> Color {
        if self.is_dark_theme {
            Color::Indexed(10) // Bright Green
        } else {
            Color::Indexed(2) // Green
        }
    }

    #[allow(dead_code)]
    fn comment_color(&self) -> Color {
        if self.is_dark_theme {
            Color::Indexed(8) // Bright Black (typically gray)
        } else {
            Color::Indexed(8) // Same - comments should be subdued
        }
    }

    #[allow(dead_code)]
    fn number_color(&self) -> Color {
        if self.is_dark_theme {
            Color::Indexed(11) // Bright Yellow
        } else {
            Color::Indexed(3) // Yellow
        }
    }

    #[allow(dead_code)]
    fn punctuation_color(&self) -> Color {
        if self.is_dark_theme {
            Color::Indexed(14) // Bright Cyan
        } else {
            Color::Indexed(6) // Cyan
        }
    }

    // Simple Python syntax highlighter (kept as fallback, currently unused)
    // Syntect handles all languages including Python
    #[allow(dead_code)]
    fn highlight_python_simple(&self, code: &str) -> Vec<Line<'static>> {
        let keywords = [
            "import", "from", "as", "def", "class", "if", "elif", "else", "for", "while", "return",
            "yield", "break", "continue", "pass", "try", "except", "finally", "raise", "with",
            "async", "await", "lambda", "and", "or", "not", "in", "is", "True", "False", "None",
        ];

        let keyword_color = self.keyword_color();
        let string_color = self.string_color();
        let comment_color = self.comment_color();
        let number_color = self.number_color();
        let punctuation_color = self.punctuation_color();

        let mut lines = Vec::new();
        for line_text in code.lines() {
            let mut spans = Vec::new();
            let mut current = String::new();
            let mut in_string = false;
            let mut string_char = ' ';
            let mut in_comment = false;

            let chars: Vec<char> = line_text.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                let ch = chars[i];

                // Handle comments
                if ch == '#' && !in_string {
                    if !current.is_empty() {
                        spans.push(Span::raw(current.clone()));
                        current.clear();
                    }
                    in_comment = true;
                    current.push(ch);
                    i += 1;
                    continue;
                }

                if in_comment {
                    current.push(ch);
                    i += 1;
                    continue;
                }

                // Handle strings
                if (ch == '"' || ch == '\'') && !in_string {
                    // Check for triple quotes
                    if i + 2 < chars.len() && chars[i + 1] == ch && chars[i + 2] == ch {
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        let mut string_content = String::from("\"\"\"");
                        i += 3;
                        // Find end of triple quote string (simplified)
                        while i < chars.len() {
                            string_content.push(chars[i]);
                            if i >= 2 && chars[i] == ch && chars[i - 1] == ch && chars[i - 2] == ch
                            {
                                break;
                            }
                            i += 1;
                        }
                        spans.push(Span::styled(
                            string_content,
                            Style::default().fg(string_color),
                        ));
                        i += 1;
                        continue;
                    }

                    if !current.is_empty() {
                        spans.push(Span::raw(current.clone()));
                        current.clear();
                    }
                    in_string = true;
                    string_char = ch;
                    current.push(ch);
                } else if in_string && ch == string_char {
                    current.push(ch);
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(string_color),
                    ));
                    current.clear();
                    in_string = false;
                    string_char = ' ';
                } else if in_string {
                    current.push(ch);
                } else if ch.is_whitespace() {
                    if !current.is_empty() {
                        // Check if current is a keyword
                        if keywords.contains(&current.as_str()) {
                            spans.push(Span::styled(
                                current.clone(),
                                Style::default()
                                    .fg(keyword_color)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        } else if current
                            .chars()
                            .all(|c| c.is_numeric() || c == '.' || c == '_')
                        {
                            // Numbers
                            spans.push(Span::styled(
                                current.clone(),
                                Style::default().fg(number_color),
                            ));
                        } else {
                            spans.push(Span::raw(current.clone()));
                        }
                        current.clear();
                    }
                    spans.push(Span::raw(ch.to_string()));
                } else if "()[]{}:,;.".contains(ch) {
                    if !current.is_empty() {
                        if keywords.contains(&current.as_str()) {
                            spans.push(Span::styled(
                                current.clone(),
                                Style::default()
                                    .fg(keyword_color)
                                    .add_modifier(Modifier::BOLD),
                            ));
                        } else {
                            spans.push(Span::raw(current.clone()));
                        }
                        current.clear();
                    }
                    spans.push(Span::styled(
                        ch.to_string(),
                        Style::default().fg(punctuation_color),
                    ));
                } else {
                    current.push(ch);
                }

                i += 1;
            }

            // Handle remaining content
            if !current.is_empty() {
                if in_comment {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(comment_color),
                    ));
                } else if in_string {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default().fg(string_color),
                    ));
                } else if keywords.contains(&current.as_str()) {
                    spans.push(Span::styled(
                        current.clone(),
                        Style::default()
                            .fg(keyword_color)
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    spans.push(Span::raw(current.clone()));
                }
            }

            if spans.is_empty() {
                spans.push(Span::raw(""));
            }

            lines.push(Line::from(spans));
        }

        lines
    }

    fn get_cell_language(&self, cell: &Cell) -> String {
        match cell {
            Cell::Code { .. } => {
                // Get language from notebook metadata
                let lang = self
                    .notebook
                    .metadata
                    .language_info
                    .as_ref()
                    .map(|li| li.name.clone())
                    .unwrap_or_else(|| "python".to_string());

                // Capitalize first letter for syntect (e.g., "python" -> "Python")
                if let Some(first) = lang.chars().next() {
                    first.to_uppercase().collect::<String>() + &lang[1..]
                } else {
                    "Python".to_string()
                }
            }
            _ => "Markdown".to_string(),
        }
    }

    fn highlight_code(&self, code: &str, language: &str) -> Vec<Line<'static>> {
        // Try multiple themes in order of preference - use vibrant dark themes
        let theme = self
            .theme_set
            .themes
            .get("base16-eighties.dark")
            .or_else(|| self.theme_set.themes.get("base16-mocha.dark"))
            .or_else(|| self.theme_set.themes.get("base16-ocean.dark"))
            .unwrap_or_else(|| self.theme_set.themes.values().next().unwrap());

        // Try to find syntax by name first, then by common aliases
        let syntax = self
            .syntax_set
            .find_syntax_by_name(language)
            .or_else(|| {
                // Try lowercase version
                self.syntax_set
                    .find_syntax_by_name(&language.to_lowercase())
            })
            .or_else(|| {
                // Try as extension
                self.syntax_set
                    .find_syntax_by_extension(&language.to_lowercase())
            })
            .or_else(|| {
                // Common language mappings
                match language.to_lowercase().as_str() {
                    "python" | "py" => self.syntax_set.find_syntax_by_extension("py"),
                    "javascript" | "js" => self.syntax_set.find_syntax_by_extension("js"),
                    "typescript" | "ts" => self.syntax_set.find_syntax_by_extension("ts"),
                    "rust" | "rs" => self.syntax_set.find_syntax_by_extension("rs"),
                    "java" => self.syntax_set.find_syntax_by_extension("java"),
                    "cpp" | "c++" => self.syntax_set.find_syntax_by_extension("cpp"),
                    "c" => self.syntax_set.find_syntax_by_extension("c"),
                    "ruby" | "rb" => self.syntax_set.find_syntax_by_extension("rb"),
                    "go" => self.syntax_set.find_syntax_by_extension("go"),
                    _ => None,
                }
            })
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut lines = Vec::new();

        for line in LinesWithEndings::from(code) {
            let ranges = highlighter
                .highlight_line(line, &self.syntax_set)
                .unwrap_or_default();

            let mut spans = Vec::new();
            for (style, text) in ranges {
                let fg = style.foreground;
                let mut modifier = Modifier::empty();
                if style.font_style.contains(FontStyle::BOLD) {
                    modifier |= Modifier::BOLD;
                }
                if style.font_style.contains(FontStyle::ITALIC) {
                    modifier |= Modifier::ITALIC;
                }
                if style.font_style.contains(FontStyle::UNDERLINE) {
                    modifier |= Modifier::UNDERLINED;
                }

                // Convert RGB to closest ANSI color
                let color = self.rgb_to_ansi(fg.r, fg.g, fg.b);

                spans.push(Span::styled(
                    text.to_string(),
                    Style::default().fg(color).add_modifier(modifier),
                ));
            }
            lines.push(Line::from(spans));
        }

        lines
    }

    fn render_markdown(&self, markdown: &str) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut in_code_block = false;
        let mut code_block_content = String::new();
        let mut code_block_language = String::new();

        for line in markdown.lines() {
            // Handle code blocks
            if line.starts_with("```") {
                if in_code_block {
                    // End of code block - highlight and add it
                    if !code_block_content.is_empty() {
                        let highlighted =
                            self.highlight_code(&code_block_content, &code_block_language);
                        lines.extend(highlighted);
                    }
                    code_block_content.clear();
                    code_block_language.clear();
                    in_code_block = false;
                } else {
                    // Start of code block
                    in_code_block = true;
                    code_block_language = line[3..].trim().to_string();
                    if code_block_language.is_empty() {
                        code_block_language = "python".to_string();
                    }
                }
                continue;
            }

            if in_code_block {
                code_block_content.push_str(line);
                code_block_content.push('\n');
                continue;
            }

            // Headings
            if line.starts_with("# ") {
                lines.push(Line::from(vec![Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(self.heading_color(1))
                        .add_modifier(Modifier::BOLD),
                )]));
            } else if line.starts_with("## ") {
                lines.push(Line::from(vec![Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(self.heading_color(2))
                        .add_modifier(Modifier::BOLD),
                )]));
            } else if line.starts_with("### ") {
                lines.push(Line::from(vec![Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(self.heading_color(3))
                        .add_modifier(Modifier::BOLD),
                )]));
            } else if line.starts_with("#### ") {
                lines.push(Line::from(vec![Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(self.heading_color(4))
                        .add_modifier(Modifier::BOLD),
                )]));
            // Horizontal rule
            } else if line.trim() == "---" || line.trim() == "***" || line.trim() == "___" {
                lines.push(Line::from(vec![Span::styled(
                    "─".repeat(60),
                    Style::default().fg(Color::Indexed(8)), // Bright Black (gray)
                )]));
            // Block quote
            } else if line.starts_with("> ") {
                let quote_color = if self.is_dark_theme {
                    Color::Indexed(11)
                } else {
                    Color::Indexed(4)
                }; // Yellow/Blue
                lines.push(Line::from(vec![
                    Span::styled("┃ ", Style::default().fg(quote_color)),
                    Span::styled(line[2..].to_string(), Style::default().fg(quote_color)),
                ]));
            // Lists
            } else if line.starts_with("- ") || line.starts_with("* ") {
                let content = self.parse_inline_markdown(&line[2..]);
                let list_color = if self.is_dark_theme {
                    Color::Indexed(11)
                } else {
                    Color::Indexed(4)
                }; // Yellow/Blue
                let mut spans = vec![Span::styled("  • ", Style::default().fg(list_color))];
                spans.extend(content);
                lines.push(Line::from(spans));
            } else if line
                .trim_start()
                .chars()
                .next()
                .is_some_and(|c| c.is_numeric())
                && line.contains(". ")
            {
                // Numbered list
                if let Some(pos) = line.find(". ") {
                    let number = &line[..pos].trim_start();
                    let content = self.parse_inline_markdown(&line[pos + 2..]);
                    let list_color = if self.is_dark_theme {
                        Color::Indexed(11)
                    } else {
                        Color::Indexed(4)
                    }; // Yellow/Blue
                    let mut spans = vec![Span::styled(
                        format!(" {} ", number),
                        Style::default().fg(list_color),
                    )];
                    spans.extend(content);
                    lines.push(Line::from(spans));
                } else {
                    lines.push(Line::from(self.parse_inline_markdown(line)));
                }
            } else {
                // Regular text with inline formatting
                lines.push(Line::from(self.parse_inline_markdown(line)));
            }
        }

        // Handle unclosed code block
        if in_code_block && !code_block_content.is_empty() {
            let highlighted = self.highlight_code(&code_block_content, &code_block_language);
            lines.extend(highlighted);
        }

        lines
    }

    fn parse_inline_markdown(&self, text: &str) -> Vec<Span<'static>> {
        let mut spans = Vec::new();
        let mut current = String::new();
        let mut chars = text.chars().peekable();

        while let Some(ch) = chars.next() {
            match ch {
                '`' => {
                    // Inline code
                    if !current.is_empty() {
                        spans.push(Span::raw(current.clone()));
                        current.clear();
                    }
                    let mut code = String::new();
                    while let Some(&next_ch) = chars.peek() {
                        if next_ch == '`' {
                            chars.next();
                            break;
                        }
                        code.push(chars.next().unwrap());
                    }
                    spans.push(Span::styled(
                        code,
                        Style::default()
                            .fg(self.string_color())
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                '*' => {
                    if chars.peek() == Some(&'*') {
                        // Bold **text**
                        chars.next();
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        let mut bold = String::new();
                        while let Some(ch) = chars.next() {
                            if ch == '*' && chars.peek() == Some(&'*') {
                                chars.next();
                                break;
                            }
                            bold.push(ch);
                        }
                        spans.push(Span::styled(
                            bold,
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        // Italic *text*
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        let mut italic = String::new();
                        for ch in chars.by_ref() {
                            if ch == '*' {
                                break;
                            }
                            italic.push(ch);
                        }
                        spans.push(Span::styled(
                            italic,
                            Style::default().add_modifier(Modifier::ITALIC),
                        ));
                    }
                }
                '_' => {
                    if chars.peek() == Some(&'_') {
                        // Bold __text__
                        chars.next();
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        let mut bold = String::new();
                        while let Some(ch) = chars.next() {
                            if ch == '_' && chars.peek() == Some(&'_') {
                                chars.next();
                                break;
                            }
                            bold.push(ch);
                        }
                        spans.push(Span::styled(
                            bold,
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        // Italic _text_
                        if !current.is_empty() {
                            spans.push(Span::raw(current.clone()));
                            current.clear();
                        }
                        let mut italic = String::new();
                        for ch in chars.by_ref() {
                            if ch == '_' {
                                break;
                            }
                            italic.push(ch);
                        }
                        spans.push(Span::styled(
                            italic,
                            Style::default().add_modifier(Modifier::ITALIC),
                        ));
                    }
                }
                _ => current.push(ch),
            }
        }

        if !current.is_empty() {
            spans.push(Span::raw(current));
        }

        if spans.is_empty() {
            spans.push(Span::raw(text.to_string()));
        }

        spans
    }

    fn get_cell_source(&self, cell: &Cell) -> String {
        cell.source().join("")
    }

    fn format_output(&self, output: &Output) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        match output {
            Output::Stream { name, text } => {
                lines.push(Line::from(vec![Span::styled(
                    format!("[{}] ", name),
                    Style::default().fg(Color::Indexed(10)), // Bright Green
                )]));
                for line in text.0.lines() {
                    lines.push(Line::from(line.to_string()));
                }
            }
            Output::ExecuteResult(result) => {
                lines.push(Line::from(vec![Span::styled(
                    format!("[Out {}] ", result.execution_count),
                    Style::default().fg(Color::Indexed(12)), // Bright Blue
                )]));
                // Simple text output for now
                if let Ok(json_val) = serde_json::to_value(&result.data) {
                    if let Some(obj) = json_val.as_object() {
                        if let Some(text) = obj.get("text/plain") {
                            if let Some(text_str) = text.as_str() {
                                for line in text_str.lines() {
                                    lines.push(Line::from(line.to_string()));
                                }
                            }
                        }
                    }
                }
            }
            Output::DisplayData(data) => {
                lines.push(Line::from(vec![
                    Span::styled("[Display] ", Style::default().fg(Color::Indexed(13))), // Bright Magenta
                ]));
                if let Ok(json_val) = serde_json::to_value(&data.data) {
                    if let Some(obj) = json_val.as_object() {
                        if let Some(text) = obj.get("text/plain") {
                            if let Some(text_str) = text.as_str() {
                                for line in text_str.lines() {
                                    lines.push(Line::from(line.to_string()));
                                }
                            }
                        }
                    }
                }
            }
            Output::Error(error) => {
                lines.push(Line::from(vec![Span::styled(
                    format!("Error: {}", error.ename),
                    Style::default()
                        .fg(Color::Indexed(9))
                        .add_modifier(Modifier::BOLD), // Bright Red
                )]));
                lines.push(Line::from(vec![Span::styled(
                    format!("  {}", error.evalue),
                    Style::default().fg(Color::Indexed(9)), // Bright Red
                )]));
            }
        }

        lines
    }
}

pub fn execute(args: ViewArgs) -> Result<()> {
    use crate::commands::common;
    let normalized_path = common::normalize_notebook_path(&args.file);
    let file_path = PathBuf::from(&normalized_path);
    let notebook = notebook::read_notebook(&normalized_path)?;

    // Setup file watcher
    let (tx, rx) = mpsc::channel();
    let mut watcher =
        notify::recommended_watcher(move |res: Result<NotifyEvent, notify::Error>| {
            if let Ok(event) = res {
                // Only notify on modify events
                if matches!(event.kind, EventKind::Modify(_)) {
                    let _ = tx.send(());
                }
            }
        })?;

    // Watch the parent directory (watching the file directly can miss some editors)
    if let Some(parent) = file_path.parent() {
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run
    let app = App::new(notebook, &args.theme, file_path);
    let res = run_app(&mut terminal, app, rx);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    file_change_rx: mpsc::Receiver<()>,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        // Check for file changes (non-blocking)
        if file_change_rx.try_recv().is_ok() {
            // File changed, reload the notebook
            let _ = app.reload();
        }

        // Check for keyboard events with a timeout so we can check file changes periodically
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                        KeyCode::Char('j') | KeyCode::Down => app.next_cell(),
                        KeyCode::Char('k') | KeyCode::Up => app.previous_cell(),
                        KeyCode::Char('d') => app.scroll_down(),
                        KeyCode::Char('u') => app.scroll_up(),
                        KeyCode::Char('g') => app.jump_to_first(),
                        KeyCode::Char('G') => app.jump_to_last(),
                        KeyCode::Char('r') => {
                            // Manual reload with 'r' key
                            let _ = app.reload();
                        }
                        KeyCode::PageDown => {
                            for _ in 0..5 {
                                app.next_cell();
                            }
                        }
                        KeyCode::PageUp => {
                            for _ in 0..5 {
                                app.previous_cell();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Header
    render_header(f, app, chunks[0]);

    // Main content area - split into cell list and cell detail
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(chunks[1]);

    // Cell list
    render_cell_list(f, app, main_chunks[0]);

    // Cell detail
    render_cell_detail(f, app, main_chunks[1]);

    // Footer
    render_footer(f, chunks[2]);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let kernel = app
        .notebook
        .metadata
        .kernelspec
        .as_ref()
        .map(|ks| ks.display_name.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    let title = vec![
        Span::styled(
            "Notebook Viewer",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" | Kernel: {} | ", kernel)),
        Span::styled(
            format!("{} cells", app.notebook.cells.len()),
            Style::default().fg(Color::Indexed(14)), // Bright Cyan
        ),
    ];

    let header = Paragraph::new(Line::from(title))
        .block(Block::default().borders(Borders::ALL).title("Info"));

    f.render_widget(header, area);
}

fn render_cell_list(f: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .notebook
        .cells
        .iter()
        .enumerate()
        .map(|(i, cell)| {
            let cell_symbol = app.get_cell_type_symbol(cell);
            let exec_marker = match cell {
                Cell::Code {
                    execution_count, ..
                } => {
                    if execution_count.is_some() {
                        " ✓"
                    } else {
                        " ○"
                    }
                }
                _ => "",
            };

            let source = app.get_cell_source(cell);
            let preview = source
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(20)
                .collect::<String>();

            let style = if i == app.selected_cell {
                Style::default()
                    .fg(Color::Indexed(11)) // Bright Yellow
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let content = format!("{:2} {}{} {}", i, cell_symbol, exec_marker, preview);
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Cells"))
        .highlight_style(
            Style::default()
                .bg(Color::Indexed(8)) // Bright Black (gray background)
                .add_modifier(Modifier::BOLD),
        );

    f.render_widget(list, area);
}

fn render_cell_detail(f: &mut Frame, app: &mut App, area: Rect) {
    if let Some(cell) = app.notebook.cells.get(app.selected_cell) {
        let cell_type = app.get_cell_type_str(cell);
        let title = format!("Cell {} [{}]", app.selected_cell, cell_type);

        let source = app.get_cell_source(cell);

        let mut content_lines = match cell {
            Cell::Code { .. } => {
                let language = app.get_cell_language(cell);
                app.highlight_code(&source, &language)
            }
            Cell::Markdown { .. } => app.render_markdown(&source),
            Cell::Raw { .. } => source.lines().map(|l| Line::from(l.to_string())).collect(),
        };

        // Add outputs for code cells
        if let Cell::Code {
            outputs,
            execution_count,
            ..
        } = cell
        {
            if !outputs.is_empty() {
                content_lines.push(Line::from(""));
                content_lines.push(Line::from(vec![Span::styled(
                    "─".repeat(40),
                    Style::default().fg(Color::Indexed(8)), // Bright Black (gray)
                )]));
                let output_header = match execution_count {
                    Some(n) => format!("Outputs (exec: {})", n),
                    None => "Outputs".to_string(),
                };
                content_lines.push(Line::from(vec![Span::styled(
                    output_header,
                    Style::default()
                        .fg(Color::Indexed(10))
                        .add_modifier(Modifier::BOLD), // Bright Green
                )]));
                content_lines.push(Line::from(""));

                for output in outputs {
                    let output_lines = app.format_output(output);
                    content_lines.extend(output_lines);
                    content_lines.push(Line::from(""));
                }
            }
        }

        let text = Text::from(content_lines);

        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false })
            .scroll((app.scroll_offset, 0));

        f.render_widget(paragraph, area);
    }
}

fn render_footer(f: &mut Frame, area: Rect) {
    let help_text = vec![
        Span::styled("j/↓", Style::default().fg(Color::Indexed(11))), // Bright Yellow
        Span::raw(" Next | "),
        Span::styled("k/↑", Style::default().fg(Color::Indexed(11))), // Bright Yellow
        Span::raw(" Prev | "),
        Span::styled("d/u", Style::default().fg(Color::Indexed(11))), // Bright Yellow
        Span::raw(" Scroll | "),
        Span::styled("g/G", Style::default().fg(Color::Indexed(11))), // Bright Yellow
        Span::raw(" First/Last | "),
        Span::styled("r", Style::default().fg(Color::Indexed(10))), // Bright Green
        Span::raw(" Reload | "),
        Span::styled("q/Esc", Style::default().fg(Color::Indexed(9))), // Bright Red
        Span::raw(" Quit"),
    ];

    let footer = Paragraph::new(Line::from(help_text))
        .block(Block::default().borders(Borders::ALL).title("Help"));

    f.render_widget(footer, area);
}
