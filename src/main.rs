mod commands;
mod config;
mod execution;
mod notebook;

use clap::{Parser, Subcommand};
use std::process;

#[derive(Parser)]
#[command(name = "nb")]
#[command(about = "CLI tool for working with Jupyter notebooks", version)]
#[command(allow_negative_numbers = true)]
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
    /// Connect to a Jupyter server
    Connect(commands::connect::ConnectArgs),
    /// Show connection status
    Status(commands::status::StatusArgs),
    /// Disconnect from Jupyter server
    Disconnect(commands::disconnect::DisconnectArgs),
}

#[derive(Subcommand)]
enum NotebookCommands {
    /// Create a new notebook file
    Create(commands::create_notebook::CreateArgs),
    /// Read and extract notebook content
    Read(commands::read::ReadArgs),
    /// Execute all cells in a notebook
    Execute(commands::execute_notebook::ExecuteNotebookArgs),
    /// Search for patterns in notebook cells
    Search(commands::search::SearchArgs),
}

#[derive(Subcommand)]
enum CellCommands {
    /// Add a new cell to a notebook
    Add(commands::add_cell::AddCellArgs),
    /// Update an existing cell
    Update(commands::update_cell::UpdateCellArgs),
    /// Delete cells from a notebook
    Delete(commands::delete_cell::DeleteCellArgs),
    /// Execute a single cell
    Execute(commands::execute_cell::ExecuteCellArgs),
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
            NotebookCommands::Execute(args) => commands::execute_notebook::execute(args),
            NotebookCommands::Search(args) => commands::search::execute(args),
        },
        Commands::Cell { command } => match command {
            CellCommands::Add(args) => commands::add_cell::execute(args),
            CellCommands::Update(args) => commands::update_cell::execute(args),
            CellCommands::Delete(args) => commands::delete_cell::execute(args),
            CellCommands::Execute(args) => commands::execute_cell::execute(args),
        },
        Commands::Output { command } => match command {
            OutputCommands::Clear(args) => commands::clear_outputs::execute(args),
        },
        Commands::Connect(args) => commands::connect::execute(args),
        Commands::Status(args) => commands::status::execute(args),
        Commands::Disconnect(args) => commands::disconnect::execute(args),
    };

    if let Err(e) = result {
        eprintln!("Error: {:#}", e);
        process::exit(1);
    }
}
