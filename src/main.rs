use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod db;
mod editor;
mod tags;

#[derive(Parser)]
#[command(name = "mind")]
#[command(version)]
#[command(about = "I am your mind, at your command, on the line: your command line mind", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new note (interactive outliner if no content provided)
    Add {
        /// The content of the note (optional - omit to use interactive editor)
        content: Option<String>,
    },
    /// List all notes (optionally filtered by tag in first line)
    #[command(visible_alias = "ls")]
    List {
        /// Filter notes by tag present in the first line (e.g., #work)
        tag: Option<String>,
    },
    /// Delete notes by filter
    #[command(visible_alias = "del")]
    Delete {
        /// Filter: ID (e.g., 1), range (e.g., 1-5), or comma-separated (e.g., 1,2,3)
        filter: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Add { content } => commands::add_note(content.as_deref())?,
        Commands::List { tag } => commands::list_notes(tag.as_deref())?,
        Commands::Delete { filter } => commands::delete_notes(&filter)?,
    }
    Ok(())
}
