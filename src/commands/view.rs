use crate::notebook;
use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use nbformat::v4::{Cell, Output};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

#[derive(Parser)]
pub struct ViewArgs {
    /// Path to notebook file
    pub file: String,
}

struct App {
    notebook: nbformat::v4::Notebook,
    selected_cell: usize,
    scroll_offset: u16,
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl App {
    fn new(notebook: nbformat::v4::Notebook) -> Self {
        Self {
            notebook,
            selected_cell: 0,
            scroll_offset: 0,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
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

    fn get_cell_language(&self, cell: &Cell) -> String {
        match cell {
            Cell::Code { .. } => {
                // Get language from notebook metadata
                self.notebook
                    .metadata
                    .language_info
                    .as_ref()
                    .map(|li| li.name.clone())
                    .unwrap_or_else(|| "python".to_string())
            }
            _ => "markdown".to_string(),
        }
    }

    fn highlight_code(&self, code: &str, language: &str) -> Vec<Line<'static>> {
        let theme = &self.theme_set.themes["base16-ocean.dark"];
        let syntax = self
            .syntax_set
            .find_syntax_by_name(language)
            .or_else(|| self.syntax_set.find_syntax_by_extension(language))
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

                spans.push(Span::styled(
                    text.to_string(),
                    Style::default()
                        .fg(Color::Rgb(fg.r, fg.g, fg.b))
                        .add_modifier(modifier),
                ));
            }
            lines.push(Line::from(spans));
        }

        lines
    }

    fn render_markdown(&self, markdown: &str) -> Vec<Line<'static>> {
        // Simple markdown rendering - could be enhanced with termimad
        let mut lines = Vec::new();

        for line in markdown.lines() {
            if line.starts_with("# ") {
                lines.push(Line::from(vec![
                    Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else if line.starts_with("## ") {
                lines.push(Line::from(vec![
                    Span::styled(
                        line.to_string(),
                        Style::default()
                            .fg(Color::Blue)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else if line.starts_with("**") && line.ends_with("**") {
                lines.push(Line::from(vec![
                    Span::styled(
                        line.to_string(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]));
            } else if line.starts_with("- ") || line.starts_with("* ") {
                lines.push(Line::from(vec![
                    Span::styled("  • ", Style::default().fg(Color::Yellow)),
                    Span::raw(line[2..].to_string()),
                ]));
            } else {
                lines.push(Line::from(line.to_string()));
            }
        }

        lines
    }

    fn get_cell_source(&self, cell: &Cell) -> String {
        cell.source().join("")
    }

    fn format_output(&self, output: &Output) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        match output {
            Output::Stream { name, text } => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("[{}] ", name),
                        Style::default().fg(Color::Green),
                    ),
                ]));
                for line in text.0.lines() {
                    lines.push(Line::from(line.to_string()));
                }
            }
            Output::ExecuteResult(result) => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("[Out {}] ", result.execution_count),
                        Style::default().fg(Color::Blue),
                    ),
                ]));
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
                    Span::styled("[Display] ", Style::default().fg(Color::Magenta)),
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
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("Error: {}", error.ename),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("  {}", error.evalue),
                        Style::default().fg(Color::Red),
                    ),
                ]));
            }
        }

        lines
    }
}

pub fn execute(args: ViewArgs) -> Result<()> {
    let notebook = notebook::read_notebook(&args.file)?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and run
    let app = App::new(notebook);
    let res = run_app(&mut terminal, app);

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

fn run_app<B: Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

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
        Span::styled("Notebook Viewer", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(" | Kernel: {} | ", kernel)),
        Span::styled(
            format!("{} cells", app.notebook.cells.len()),
            Style::default().fg(Color::Cyan),
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
            let cell_type = app.get_cell_type_str(cell);
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
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let content = format!("{:2} [{}]{} {}", i, cell_type, exec_marker, preview);
            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Cells"))
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
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
        if let Cell::Code { outputs, execution_count, .. } = cell {
            if !outputs.is_empty() {
                content_lines.push(Line::from(""));
                content_lines.push(Line::from(vec![
                    Span::styled(
                        "─".repeat(40),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
                content_lines.push(Line::from(vec![
                    Span::styled(
                        format!("Outputs (exec: {:?})", execution_count),
                        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                    ),
                ]));
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
        Span::styled("j/↓", Style::default().fg(Color::Yellow)),
        Span::raw(" Next | "),
        Span::styled("k/↑", Style::default().fg(Color::Yellow)),
        Span::raw(" Prev | "),
        Span::styled("d/u", Style::default().fg(Color::Yellow)),
        Span::raw(" Scroll | "),
        Span::styled("g/G", Style::default().fg(Color::Yellow)),
        Span::raw(" First/Last | "),
        Span::styled("q/Esc", Style::default().fg(Color::Red)),
        Span::raw(" Quit"),
    ];

    let footer = Paragraph::new(Line::from(help_text))
        .block(Block::default().borders(Borders::ALL).title("Help"));

    f.render_widget(footer, area);
}
