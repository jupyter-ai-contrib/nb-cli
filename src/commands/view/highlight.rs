use super::app::App;
use nbformat::v4::Cell;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use syntect::easy::HighlightLines;
use syntect::highlighting::FontStyle;
use syntect::util::LinesWithEndings;

impl App {
    // Convert RGB color to closest ANSI indexed color
    pub(super) fn rgb_to_ansi(&self, r: u8, g: u8, b: u8) -> Color {
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

    pub(super) fn heading_color(&self, level: u8) -> Color {
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

    pub(super) fn string_color(&self) -> Color {
        if self.is_dark_theme {
            Color::Indexed(10) // Bright Green
        } else {
            Color::Indexed(2) // Green
        }
    }

    pub(super) fn get_cell_language(&self, cell: &Cell) -> String {
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

    pub(super) fn highlight_code(&self, code: &str, language: &str) -> Vec<Line<'static>> {
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
}
