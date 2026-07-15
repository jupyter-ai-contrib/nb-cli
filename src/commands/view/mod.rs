mod app;
mod highlight;
mod markdown;
mod ui;

use crate::notebook;
use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify::{Event as NotifyEvent, EventKind, RecursiveMode, Watcher};
use ratatui::{backend::Backend, backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Parser)]
pub struct ViewArgs {
    /// Path to notebook file
    pub file: String,

    /// Color scheme: dark, light, or auto (default: dark)
    #[arg(long, default_value = "dark")]
    pub theme: String,
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
) -> Result<(), B::Error>
where
    B::Error: From<io::Error>,
{
    loop {
        terminal.draw(|f| ui::ui(f, &mut app))?;

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
