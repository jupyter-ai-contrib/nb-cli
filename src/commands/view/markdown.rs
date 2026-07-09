use super::app::App;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

impl App {
    pub(super) fn render_markdown(&self, markdown: &str) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        let mut in_code_block = false;
        let mut code_block_content = String::new();
        let mut code_block_language = String::new();

        for line in markdown.lines() {
            // Handle code blocks
            if let Some(after_backticks) = line.strip_prefix("```") {
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
                    code_block_language = after_backticks.trim().to_string();
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
            } else if let Some(quoted) = line.strip_prefix("> ") {
                let quote_color = if self.is_dark_theme {
                    Color::Indexed(11)
                } else {
                    Color::Indexed(4)
                }; // Yellow/Blue
                lines.push(Line::from(vec![
                    Span::styled("┃ ", Style::default().fg(quote_color)),
                    Span::styled(quoted.to_string(), Style::default().fg(quote_color)),
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

    pub(super) fn parse_inline_markdown(&self, text: &str) -> Vec<Span<'static>> {
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
}
