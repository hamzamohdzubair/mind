use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::{Connection, params};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mind")]
#[command(about = "A zettelkasten-inspired note-taking and task management CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new note
    Add {
        /// The content of the note
        content: String,
    },
}

fn get_db_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .context("Could not find HOME directory")?;
    let data_dir = PathBuf::from(home).join(".local/share/mind");
    std::fs::create_dir_all(&data_dir)
        .context("Could not create data directory")?;
    Ok(data_dir.join("mind.db"))
}

fn init_db(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    )
    .context("Could not create notes table")?;
    Ok(())
}

fn add_note(content: &str) -> Result<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path)
        .context("Could not open database")?;

    init_db(&conn)?;

    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO notes (content, created_at, updated_at) VALUES (?1, ?2, ?3)",
        params![content, &now, &now],
    )
    .context("Could not insert note")?;

    let note_id = conn.last_insert_rowid();

    println!("Note added with ID: {}", note_id);
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { content } => add_note(&content)?,
    }

    Ok(())
}
