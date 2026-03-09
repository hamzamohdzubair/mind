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

fn add_note_to_db(conn: &Connection, content: &str, timestamp: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO notes (content, created_at, updated_at) VALUES (?1, ?2, ?3)",
        params![content, timestamp, timestamp],
    )
    .context("Could not insert note")?;

    Ok(conn.last_insert_rowid())
}

fn add_note(content: &str) -> Result<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path)
        .context("Could not open database")?;

    init_db(&conn)?;

    let now = Utc::now().to_rfc3339();
    let note_id = add_note_to_db(&conn, content, &now)?;

    println!("Note added with ID: {}", note_id);
    Ok(())
}

fn list_notes_from_db(conn: &Connection) -> Result<Vec<(i64, String, String)>> {
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

    let mut result = Vec::new();
    for note in notes {
        result.push(note?);
    }

    Ok(result)
}

fn list_notes() -> Result<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path)
        .context("Could not open database")?;

    init_db(&conn)?;

    let notes = list_notes_from_db(&conn)?;

    if notes.is_empty() {
        println!("No notes yet. Add one with: mind add \"your note\"");
    } else {
        for (id, content, created_at) in &notes {
            // Parse and format the timestamp
            let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
                .context("Could not parse timestamp")?;
            let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S");

            println!("[{}] {} | {}", id, formatted_time, content);
        }
        println!("\nTotal: {} note(s)", notes.len());
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{NamedTempFile, TempDir};
    use std::env;
    use serial_test::serial;

    fn setup_test_db() -> Result<(Connection, NamedTempFile)> {
        let temp_file = NamedTempFile::new()?;
        let conn = Connection::open(temp_file.path())?;
        init_db(&conn)?;
        Ok((conn, temp_file))
    }

    fn setup_test_env() -> Result<TempDir> {
        let temp_dir = TempDir::new()?;
        unsafe {
            env::set_var("HOME", temp_dir.path());
        }
        Ok(temp_dir)
    }

    #[test]
    fn test_init_db_creates_table() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Check if table exists
        let table_exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes'",
            [],
            |row| row.get(0),
        )?;

        assert_eq!(table_exists, 1);
        Ok(())
    }

    #[test]
    fn test_init_db_is_idempotent() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Call init_db multiple times
        init_db(&conn)?;
        init_db(&conn)?;

        let table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes'",
            [],
            |row| row.get(0),
        )?;

        assert_eq!(table_count, 1);
        Ok(())
    }

    #[test]
    fn test_add_note_to_db() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        let note_id = add_note_to_db(&conn, "Test note", timestamp)?;

        assert_eq!(note_id, 1);

        // Verify the note was inserted
        let (id, content, created_at, updated_at): (i64, String, String, String) = conn.query_row(
            "SELECT id, content, created_at, updated_at FROM notes WHERE id = ?1",
            params![note_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        assert_eq!(id, 1);
        assert_eq!(content, "Test note");
        assert_eq!(created_at, timestamp);
        assert_eq!(updated_at, timestamp);

        Ok(())
    }

    #[test]
    fn test_add_multiple_notes() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        let id1 = add_note_to_db(&conn, "First note", timestamp)?;
        let id2 = add_note_to_db(&conn, "Second note", timestamp)?;
        let id3 = add_note_to_db(&conn, "Third note", timestamp)?;

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 3);

        Ok(())
    }

    #[test]
    fn test_list_notes_from_empty_db() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let notes = list_notes_from_db(&conn)?;

        assert_eq!(notes.len(), 0);
        Ok(())
    }

    #[test]
    fn test_list_notes_from_db() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp1 = "2026-03-09T00:00:00+00:00";
        let timestamp2 = "2026-03-09T01:00:00+00:00";
        let timestamp3 = "2026-03-09T02:00:00+00:00";

        add_note_to_db(&conn, "First note", timestamp1)?;
        add_note_to_db(&conn, "Second note", timestamp2)?;
        add_note_to_db(&conn, "Third note", timestamp3)?;

        let notes = list_notes_from_db(&conn)?;

        assert_eq!(notes.len(), 3);

        // Check reverse chronological order (newest first, by ID DESC)
        assert_eq!(notes[0].0, 3); // ID
        assert_eq!(notes[0].1, "Third note"); // Content
        assert_eq!(notes[0].2, timestamp3); // Timestamp

        assert_eq!(notes[1].0, 2);
        assert_eq!(notes[1].1, "Second note");
        assert_eq!(notes[1].2, timestamp2);

        assert_eq!(notes[2].0, 1);
        assert_eq!(notes[2].1, "First note");
        assert_eq!(notes[2].2, timestamp1);

        Ok(())
    }

    #[test]
    fn test_add_note_with_special_characters() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let special_content = "Note with 'quotes' and \"double quotes\" and unicode: 你好 🎉";
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, special_content, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, special_content);
        Ok(())
    }

    #[test]
    fn test_add_note_with_empty_content() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        let note_id = add_note_to_db(&conn, "", timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, "");
        Ok(())
    }

    #[test]
    fn test_add_note_with_long_content() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let long_content = "a".repeat(10000);
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, &long_content, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content.len(), 10000);
        assert_eq!(content, long_content);
        Ok(())
    }

    #[test]
    fn test_timestamp_format() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T12:34:56.789+00:00";
        add_note_to_db(&conn, "Test", timestamp)?;

        let created_at: String = conn.query_row(
            "SELECT created_at FROM notes WHERE id = 1",
            [],
            |row| row.get(0),
        )?;

        // Verify it can be parsed back
        let parsed = chrono::DateTime::parse_from_rfc3339(&created_at);
        assert!(parsed.is_ok());
        assert_eq!(created_at, timestamp);

        Ok(())
    }

    #[test]
    fn test_list_notes_order() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Add notes with different timestamps
        for i in 1..=5 {
            let timestamp = format!("2026-03-09T{:02}:00:00+00:00", i);
            add_note_to_db(&conn, &format!("Note {}", i), &timestamp)?;
        }

        let notes = list_notes_from_db(&conn)?;

        // Should be in descending order (5, 4, 3, 2, 1)
        for (i, note) in notes.iter().enumerate() {
            let expected_id = 5 - i as i64;
            assert_eq!(note.0, expected_id);
            assert_eq!(note.1, format!("Note {}", expected_id));
        }

        Ok(())
    }

    #[test]
    #[serial]
    fn test_get_db_path() -> Result<()> {
        let _temp_dir = setup_test_env()?;

        let db_path = get_db_path()?;

        // Should create the data directory
        assert!(db_path.parent().unwrap().exists());

        // Should end with mind.db
        assert_eq!(db_path.file_name().unwrap(), "mind.db");

        // Should be in .local/share/mind
        assert!(db_path.to_string_lossy().contains(".local/share/mind"));

        Ok(())
    }

    #[test]
    #[serial]
    fn test_get_db_path_creates_directory() -> Result<()> {
        let temp_dir = setup_test_env()?;

        let data_dir = temp_dir.path().join(".local/share/mind");
        assert!(!data_dir.exists());

        let _db_path = get_db_path()?;

        // Directory should now exist
        assert!(data_dir.exists());

        Ok(())
    }

    #[test]
    #[serial]
    fn test_add_note_creates_db() -> Result<()> {
        let temp_dir = setup_test_env()?;

        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        assert!(!db_path.exists());

        // This would normally print to stdout, but we're just testing it runs
        let result = add_note("Test note from integration test");

        // Should succeed
        assert!(result.is_ok());

        // Database file should exist now
        assert!(db_path.exists());

        // Verify the note was added
        let conn = Connection::open(&db_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[test]
    #[serial]
    fn test_list_notes_with_empty_db() -> Result<()> {
        let _temp_dir = setup_test_env()?;

        // This would normally print to stdout
        let result = list_notes();

        // Should succeed even with empty database
        assert!(result.is_ok());

        Ok(())
    }

    #[test]
    #[serial]
    fn test_list_notes_with_data() -> Result<()> {
        let temp_dir = setup_test_env()?;

        // Add some notes first
        add_note("First note")?;
        add_note("Second note")?;

        // This would normally print to stdout
        let result = list_notes();

        // Should succeed
        assert!(result.is_ok());

        // Verify notes exist in database
        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        let conn = Connection::open(&db_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 2);

        Ok(())
    }

    #[test]
    fn test_add_note_with_newlines() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let multiline_content = "Line 1\nLine 2\nLine 3";
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, multiline_content, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, multiline_content);
        Ok(())
    }

    #[test]
    fn test_database_schema() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Check column names and types
        let mut stmt = conn.prepare("PRAGMA table_info(notes)")?;
        let columns: Vec<(String, String)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?.collect::<Result<Vec<_>, _>>()?;

        // Verify schema
        assert_eq!(columns.len(), 4);
        assert_eq!(columns[0].0, "id");
        assert_eq!(columns[1].0, "content");
        assert_eq!(columns[1].1, "TEXT");
        assert_eq!(columns[2].0, "created_at");
        assert_eq!(columns[2].1, "TEXT");
        assert_eq!(columns[3].0, "updated_at");
        assert_eq!(columns[3].1, "TEXT");

        Ok(())
    }

    #[test]
    fn test_note_with_sql_injection_attempt() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Try SQL injection (should be safely handled by parameterized queries)
        let malicious_content = "'; DROP TABLE notes; --";
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, malicious_content, timestamp)?;

        // Table should still exist
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        // Content should be stored as-is
        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;
        assert_eq!(content, malicious_content);

        Ok(())
    }

    #[test]
    fn test_concurrent_inserts() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Simulate multiple sequential inserts
        let mut ids = Vec::new();
        for i in 0..10 {
            let timestamp = format!("2026-03-09T{:02}:00:00+00:00", i);
            let id = add_note_to_db(&conn, &format!("Note {}", i), &timestamp)?;
            ids.push(id);
        }

        // All IDs should be unique and sequential
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(*id, (i + 1) as i64);
        }

        // Total count should be 10
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 10);

        Ok(())
    }

    #[test]
    fn test_list_notes_formatting() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Add note with valid RFC3339 timestamp
        let timestamp = "2026-03-09T12:34:56+00:00";
        add_note_to_db(&conn, "Test note", timestamp)?;

        let notes = list_notes_from_db(&conn)?;

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].1, "Test note");

        // Verify timestamp can be parsed for formatting
        let datetime = chrono::DateTime::parse_from_rfc3339(&notes[0].2);
        assert!(datetime.is_ok());

        Ok(())
    }

    #[test]
    fn test_add_note_with_tabs_and_newlines() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let content_with_whitespace = "Note\twith\ttabs\nand\nnewlines\r\n";
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, content_with_whitespace, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, content_with_whitespace);
        Ok(())
    }

    #[test]
    #[serial]
    fn test_add_multiple_notes_integration() -> Result<()> {
        let temp_dir = setup_test_env()?;

        // Add multiple notes
        add_note("First integration note")?;
        add_note("Second integration note")?;
        add_note("Third integration note")?;

        // Verify all notes in database
        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        let conn = Connection::open(&db_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 3);

        // Verify they're in the correct order
        let notes = list_notes_from_db(&conn)?;
        assert_eq!(notes[0].1, "Third integration note");
        assert_eq!(notes[1].1, "Second integration note");
        assert_eq!(notes[2].1, "First integration note");

        Ok(())
    }

    #[test]
    fn test_empty_database_list() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let notes = list_notes_from_db(&conn)?;

        assert!(notes.is_empty());
        assert_eq!(notes.len(), 0);

        Ok(())
    }

    #[test]
    fn test_note_id_autoincrement() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";

        // Add and delete a note
        let id1 = add_note_to_db(&conn, "First", timestamp)?;
        let id2 = add_note_to_db(&conn, "Second", timestamp)?;

        // Delete the second note
        conn.execute("DELETE FROM notes WHERE id = ?1", params![id2])?;

        // Add another note - ID should continue incrementing
        let id3 = add_note_to_db(&conn, "Third", timestamp)?;

        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3); // Should be 3, not 2

        Ok(())
    }

    #[test]
    fn test_very_long_unicode_content() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Create content with various unicode characters
        let unicode_content = "🎉🎊✨🌟⭐💫🔥💯🚀".repeat(100);
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, &unicode_content, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, unicode_content);
        Ok(())
    }
}
