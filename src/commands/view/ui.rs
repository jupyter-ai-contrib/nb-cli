use super::app::App;
use nbformat::v4::{Cell, Output};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

impl App {
    pub(super) fn format_output(&self, output: &Output) -> Vec<Line<'static>> {
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

pub(super) fn ui(f: &mut Frame, app: &mut App) {
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
