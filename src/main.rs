mod notebook;
mod commands;

use clap::{Parser, Subcommand};
use std::process;

#[derive(Parser)]
#[command(name = "jupyter-cli")]
#[command(about = "CLI tool for working with Jupyter notebooks", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Notebook operations
    Notebook {
        #[command(subcommand)]
        command: NotebookCommands,
    },
    /// Cell operations
    Cell {
        #[command(subcommand)]
        command: CellCommands,
    },
    /// Output operations
    Output {
        #[command(subcommand)]
        command: OutputCommands,
    },
}

#[derive(Subcommand)]
enum NotebookCommands {
    /// Create a new notebook file
    Create(commands::create_notebook::CreateArgs),
    /// Read and extract notebook content
    Read(commands::read::ReadArgs),
}

#[derive(Subcommand)]
enum CellCommands {
    /// Add a new cell to a notebook
    Add(commands::add_cell::AddCellArgs),
    /// Update an existing cell
    Update(commands::update_cell::UpdateCellArgs),
    /// Delete cells from a notebook
    Delete(commands::delete_cell::DeleteCellArgs),
}

#[derive(Subcommand)]
enum OutputCommands {
    /// Clear outputs from code cells
    Clear(commands::clear_outputs::ClearOutputsArgs),
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Notebook { command } => match command {
            NotebookCommands::Create(args) => commands::create_notebook::execute(args),
            NotebookCommands::Read(args) => commands::read::execute(args),
        },
        Commands::Cell { command } => match command {
            CellCommands::Add(args) => commands::add_cell::execute(args),
            CellCommands::Update(args) => commands::update_cell::execute(args),
            CellCommands::Delete(args) => commands::delete_cell::execute(args),
        },
        Commands::Output { command } => match command {
            OutputCommands::Clear(args) => commands::clear_outputs::execute(args),
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {:#}", e);
        process::exit(1);
    }
}
