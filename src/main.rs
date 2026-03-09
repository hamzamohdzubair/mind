use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use rusqlite::{Connection, params};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mind")]
#[command(about = "I am your mind, at your command, on the line: your command line mind", long_about = None)]
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
    /// List all notes
    List,
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

fn list_notes() -> Result<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path)
        .context("Could not open database")?;

    init_db(&conn)?;

    let mut stmt = conn.prepare(
        "SELECT id, content, created_at FROM notes ORDER BY id DESC"
    )?;

    let notes = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;

    let mut count = 0;
    for note in notes {
        let (id, content, created_at) = note?;
        count += 1;

        // Parse and format the timestamp
        let datetime = chrono::DateTime::parse_from_rfc3339(&created_at)
            .context("Could not parse timestamp")?;
        let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S");

        println!("[{}] {} | {}", id, formatted_time, content);
    }

    if count == 0 {
        println!("No notes yet. Add one with: mind add \"your note\"");
    } else {
        println!("\nTotal: {} note(s)", count);
    }

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { content } => add_note(&content)?,
        Commands::List => list_notes()?,
    }

    Ok(())
}
