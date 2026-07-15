use crate::notebook;
use anyhow::Result;
use nbformat::v4::Cell;
use std::path::PathBuf;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

pub struct App {
    pub(super) notebook: nbformat::v4::Notebook,
    pub(super) selected_cell: usize,
    pub(super) scroll_offset: u16,
    pub(super) syntax_set: SyntaxSet,
    pub(super) theme_set: ThemeSet,
    pub(super) is_dark_theme: bool,
    pub(super) file_path: PathBuf,
}

impl App {
    pub(super) fn new(notebook: nbformat::v4::Notebook, theme: &str, file_path: PathBuf) -> Self {
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

    pub(super) fn reload(&mut self) -> Result<()> {
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

    pub(super) fn next_cell(&mut self) {
        if self.selected_cell < self.notebook.cells.len().saturating_sub(1) {
            self.selected_cell += 1;
            self.scroll_offset = 0;
        }
    }

    pub(super) fn previous_cell(&mut self) {
        if self.selected_cell > 0 {
            self.selected_cell -= 1;
            self.scroll_offset = 0;
        }
    }

    pub(super) fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub(super) fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub(super) fn jump_to_first(&mut self) {
        self.selected_cell = 0;
        self.scroll_offset = 0;
    }

    pub(super) fn jump_to_last(&mut self) {
        self.selected_cell = self.notebook.cells.len().saturating_sub(1);
        self.scroll_offset = 0;
    }

    pub(super) fn get_cell_type_str(&self, cell: &Cell) -> &str {
        match cell {
            Cell::Code { .. } => "Code",
            Cell::Markdown { .. } => "Markdown",
            Cell::Raw { .. } => "Raw",
        }
    }

    pub(super) fn get_cell_type_symbol(&self, cell: &Cell) -> &str {
        match cell {
            Cell::Code { .. } => "⚡",
            Cell::Markdown { .. } => "▪",
            Cell::Raw { .. } => "○",
        }
    }

    pub(super) fn get_cell_source(&self, cell: &Cell) -> String {
        cell.source().join("")
    }
}
