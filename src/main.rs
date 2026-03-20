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
    /// Create a new notebook file
    Create(commands::create_notebook::CreateArgs),
    /// Read and extract notebook content
    Read(commands::read::ReadArgs),
    /// View notebook in an interactive TUI
    View(commands::view::ViewArgs),
    /// Execute cells in a notebook
    Execute(commands::execute_notebook::ExecuteNotebookArgs),
    /// Search for text and errors in notebook cells
    Search(commands::search::SearchArgs),
    /// Add, update or delete cells
    Cell {
        #[command(subcommand)]
        command: CellCommands,
    },
    /// Clear outputs
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
    /// Manage nb CLI (version, update)
    #[command(name = "self")]
    SelfCmd {
        #[command(subcommand)]
        command: commands::self_cmd::SelfCommands,
    },
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
    /// Remove all externalized output files from the temp directory
    Clean(commands::clean_output_dirs::CleanOutputDirsArgs),
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Create(args) => commands::create_notebook::execute(args),
        Commands::Read(args) => commands::read::execute(args),
        Commands::View(args) => commands::view::execute(args),
        Commands::Execute(args) => commands::execute_notebook::execute(args),
        Commands::Search(args) => commands::search::execute(args),
        Commands::Cell { command } => match command {
            CellCommands::Add(args) => commands::add_cell::execute(args),
            CellCommands::Update(args) => commands::update_cell::execute(args),
            CellCommands::Delete(args) => commands::delete_cell::execute(args),
        },
        Commands::Output { command } => match command {
            OutputCommands::Clear(args) => commands::clear_outputs::execute(args),
            OutputCommands::Clean(args) => commands::clean_output_dirs::execute(args),
        },
        Commands::Connect(args) => commands::connect::execute(args),
        Commands::Status(args) => commands::status::execute(args),
        Commands::Disconnect(args) => commands::disconnect::execute(args),
        Commands::SelfCmd { command } => commands::self_cmd::execute(command),
    };

    if let Err(e) = result {
        eprintln!("Error: {:#}", e);
        process::exit(1);
    }
}
