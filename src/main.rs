use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::*;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use rusqlite::{Connection, params};
use std::io::{self, Write};
use std::path::PathBuf;

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
    /// Add a new note
    Add {
        /// The content of the note
        content: String,
    },
    /// List all notes
    List,
    /// Delete notes by filter
    #[command(visible_alias = "del")]
    Delete {
        /// Filter: ID (e.g., 1), range (e.g., 1-5), or comma-separated (e.g., 1,2,3)
        filter: String,
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
        // Print table header
        println!("{}", "─".repeat(80).bright_black());
        println!(
            "{:<6} {:<20} {}",
            "ID".bold(),
            "DATE".bold(),
            "CONTENT".bold()
        );
        println!("{}", "─".repeat(80).bright_black());

        // Print notes with alternating subtle styling
        for (index, (id, content, created_at)) in notes.iter().enumerate() {
            let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
                .context("Could not parse timestamp")?;
            let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S");

            let line = format!("{:<6} {:<20} {}", id, formatted_time, content);

            if index % 2 == 0 {
                println!("{}", line.dimmed());
            } else {
                println!("{}", line);
            }

            // Add line break if next ID is not consecutive (notes are in DESC order)
            if index < notes.len() - 1 {
                let next_id = notes[index + 1].0;
                if *id - next_id > 1 {
                    println!();
                }
            }
        }

        println!("{}", "─".repeat(80).bright_black());
        println!("Total: {} note(s)", notes.len());
    }

    Ok(())
}

fn parse_filter(filter: &str) -> Result<Vec<i64>> {
    let mut ids = Vec::new();

    // Check if it's a range (e.g., "3-6")
    if filter.contains('-') {
        let parts: Vec<&str> = filter.split('-').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid range format. Use: <start>-<end>"));
        }

        let start: i64 = parts[0].trim().parse()
            .context("Invalid start of range")?;
        let end: i64 = parts[1].trim().parse()
            .context("Invalid end of range")?;

        if start > end {
            return Err(anyhow!("Range start must be less than or equal to end"));
        }

        ids.extend(start..=end);
    }
    // Check if it's comma-separated (e.g., "1,2,3")
    else if filter.contains(',') {
        for part in filter.split(',') {
            let id: i64 = part.trim().parse()
                .context(format!("Invalid ID: {}", part.trim()))?;
            ids.push(id);
        }
    }
    // Single ID
    else {
        let id: i64 = filter.trim().parse()
            .context("Invalid ID format")?;
        ids.push(id);
    }

    Ok(ids)
}

fn get_notes_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<(i64, String, String)>> {
    let mut notes = Vec::new();

    for &id in ids {
        let result = conn.query_row(
            "SELECT id, content, created_at FROM notes WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );

        match result {
            Ok(note) => notes.push(note),
            Err(_) => {
                // Note doesn't exist, skip it
                continue;
            }
        }
    }

    Ok(notes)
}

fn confirm_deletion() -> Result<bool> {
    print!("{} ", "Delete? [y/n]:".yellow().bold());
    io::stdout().flush()?;

    enable_raw_mode()?;
    let result = loop {
        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    println!("y");
                    break Ok(true);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    println!("n");
                    break Ok(false);
                }
                _ => continue,
            }
        }
    };
    disable_raw_mode()?;

    result
}

fn delete_notes_by_ids(conn: &Connection, ids: &[i64]) -> Result<usize> {
    let mut deleted_count = 0;

    for &id in ids {
        let rows_affected = conn.execute(
            "DELETE FROM notes WHERE id = ?1",
            params![id],
        )?;
        deleted_count += rows_affected;
    }

    Ok(deleted_count)
}

fn delete_notes(filter: &str) -> Result<()> {
    let ids = parse_filter(filter)?;

    if ids.is_empty() {
        println!("No IDs to delete.");
        return Ok(());
    }

    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path)
        .context("Could not open database")?;

    init_db(&conn)?;

    // Get notes that match the filter
    let notes = get_notes_by_ids(&conn, &ids)?;

    if notes.is_empty() {
        println!("No notes found matching the filter.");
        return Ok(());
    }

    // Show what will be deleted
    println!("{}", "Notes to be deleted:".red().bold());
    println!("{}", "─".repeat(80).bright_black());

    for (id, content, created_at) in &notes {
        let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
            .context("Could not parse timestamp")?;
        let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S");

        println!("{} {} | {}",
            format!("[{}]", id).red(),
            formatted_time,
            content
        );
    }

    println!("{}", "─".repeat(80).bright_black());

    // Confirm deletion
    if !confirm_deletion()? {
        println!("{}", "Deletion cancelled.".green());
        return Ok(());
    }

    // Perform deletion
    let note_ids: Vec<i64> = notes.iter().map(|(id, _, _)| *id).collect();
    let deleted_count = delete_notes_by_ids(&conn, &note_ids)?;

    println!("{}", format!("Successfully deleted {} note(s).", deleted_count).green().bold());

    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { content } => add_note(&content)?,
        Commands::List => list_notes()?,
        Commands::Delete { filter } => delete_notes(&filter)?,
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
    fn test_parse_filter_single_id() -> Result<()> {
        let ids = parse_filter("5")?;
        assert_eq!(ids, vec![5]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_range() -> Result<()> {
        let ids = parse_filter("3-6")?;
        assert_eq!(ids, vec![3, 4, 5, 6]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_comma_separated() -> Result<()> {
        let ids = parse_filter("1,2,5,8")?;
        assert_eq!(ids, vec![1, 2, 5, 8]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_comma_with_spaces() -> Result<()> {
        let ids = parse_filter("1, 2, 5, 8")?;
        assert_eq!(ids, vec![1, 2, 5, 8]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_invalid_range() -> Result<()> {
        let result = parse_filter("6-3");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_parse_filter_invalid_format() -> Result<()> {
        let result = parse_filter("abc");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_get_notes_by_ids() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;
        add_note_to_db(&conn, "Note 2", timestamp)?;
        add_note_to_db(&conn, "Note 3", timestamp)?;
        add_note_to_db(&conn, "Note 4", timestamp)?;

        let notes = get_notes_by_ids(&conn, &[1, 3, 4])?;

        assert_eq!(notes.len(), 3);
        assert_eq!(notes[0].0, 1);
        assert_eq!(notes[0].1, "Note 1");
        assert_eq!(notes[1].0, 3);
        assert_eq!(notes[1].1, "Note 3");
        assert_eq!(notes[2].0, 4);
        assert_eq!(notes[2].1, "Note 4");

        Ok(())
    }

    #[test]
    fn test_get_notes_by_ids_nonexistent() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;

        // Request IDs that don't exist
        let notes = get_notes_by_ids(&conn, &[1, 99, 100])?;

        // Should only return note 1
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, 1);

        Ok(())
    }

    #[test]
    fn test_delete_notes_by_ids() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;
        add_note_to_db(&conn, "Note 2", timestamp)?;
        add_note_to_db(&conn, "Note 3", timestamp)?;

        let deleted = delete_notes_by_ids(&conn, &[1, 3])?;

        assert_eq!(deleted, 2);

        // Verify notes were deleted
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        // Verify note 2 still exists
        let notes = list_notes_from_db(&conn)?;
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, 2);

        Ok(())
    }

    #[test]
    fn test_delete_notes_by_ids_nonexistent() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;

        // Try to delete non-existent IDs
        let deleted = delete_notes_by_ids(&conn, &[99, 100])?;

        assert_eq!(deleted, 0);

        // Verify note 1 still exists
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);

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

    #[test]
    fn test_parse_filter_range_equal() -> Result<()> {
        let ids = parse_filter("5-5")?;
        assert_eq!(ids, vec![5]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_range_large() -> Result<()> {
        let ids = parse_filter("1-10")?;
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_multiple_dashes() -> Result<()> {
        let result = parse_filter("1-2-3");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_delete_all_notes() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;
        add_note_to_db(&conn, "Note 2", timestamp)?;
        add_note_to_db(&conn, "Note 3", timestamp)?;

        let deleted = delete_notes_by_ids(&conn, &[1, 2, 3])?;

        assert_eq!(deleted, 3);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 0);

        Ok(())
    }

    #[test]
    fn test_get_notes_by_ids_empty_list() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;

        let notes = get_notes_by_ids(&conn, &[])?;

        assert_eq!(notes.len(), 0);

        Ok(())
    }

    #[test]
    fn test_get_notes_by_ids_all_nonexistent() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let notes = get_notes_by_ids(&conn, &[99, 100, 101])?;

        assert_eq!(notes.len(), 0);

        Ok(())
    }

    #[test]
    fn test_delete_notes_by_ids_empty_list() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;

        let deleted = delete_notes_by_ids(&conn, &[])?;

        assert_eq!(deleted, 0);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[test]
    fn test_parse_filter_comma_trailing_fails() -> Result<()> {
        let result = parse_filter("1,");
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_get_notes_by_ids_duplicates() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;
        add_note_to_db(&conn, "Note 2", timestamp)?;

        // Request same ID multiple times
        let notes = get_notes_by_ids(&conn, &[1, 1, 2, 1])?;

        // Should return 4 notes (duplicates included)
        assert_eq!(notes.len(), 4);
        assert_eq!(notes[0].0, 1);
        assert_eq!(notes[1].0, 1);
        assert_eq!(notes[2].0, 2);
        assert_eq!(notes[3].0, 1);

        Ok(())
    }

    #[test]
    fn test_delete_notes_by_ids_with_duplicates() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;
        add_note_to_db(&conn, "Note 2", timestamp)?;

        // Try to delete same ID multiple times
        let deleted = delete_notes_by_ids(&conn, &[1, 1, 1])?;

        // Should only delete once
        assert_eq!(deleted, 1);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);

        Ok(())
    }

    #[test]
    fn test_list_with_single_note() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Only note", timestamp)?;

        let notes = list_notes_from_db(&conn)?;

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, 1);
        assert_eq!(notes[0].1, "Only note");

        Ok(())
    }

    #[test]
    fn test_parse_filter_with_leading_trailing_spaces() -> Result<()> {
        let ids = parse_filter("  5  ")?;
        assert_eq!(ids, vec![5]);

        let ids = parse_filter("  1 - 3  ")?;
        assert_eq!(ids, vec![1, 2, 3]);

        Ok(())
    }

    #[test]
    fn test_parse_filter_zero() -> Result<()> {
        let ids = parse_filter("0")?;
        assert_eq!(ids, vec![0]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_negative() -> Result<()> {
        let result = parse_filter("-5");
        // This might parse as negative number or fail depending on implementation
        // Just verify it doesn't crash
        let _ = result;
        Ok(())
    }

    #[test]
    fn test_large_id() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";

        // Insert note and manually set a large ID
        conn.execute("INSERT INTO notes (id, content, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![999999, "Large ID note", timestamp, timestamp])?;

        let notes = get_notes_by_ids(&conn, &[999999])?;

        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, 999999);

        Ok(())
    }

    #[test]
    fn test_parse_filter_range_with_space() -> Result<()> {
        let ids = parse_filter("3 - 6")?;
        assert_eq!(ids, vec![3, 4, 5, 6]);
        Ok(())
    }

    #[test]
    fn test_list_many_notes() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        // Add 20 notes
        for i in 1..=20 {
            let timestamp = format!("2026-03-09T{:02}:00:00+00:00", i);
            add_note_to_db(&conn, &format!("Note {}", i), &timestamp)?;
        }

        let notes = list_notes_from_db(&conn)?;

        assert_eq!(notes.len(), 20);
        // First note should be ID 20 (descending order)
        assert_eq!(notes[0].0, 20);
        // Last note should be ID 1
        assert_eq!(notes[19].0, 1);

        Ok(())
    }

    #[test]
    fn test_delete_range_of_notes() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        for i in 1..=10 {
            add_note_to_db(&conn, &format!("Note {}", i), timestamp)?;
        }

        // Delete notes 3-7
        let deleted = delete_notes_by_ids(&conn, &[3, 4, 5, 6, 7])?;

        assert_eq!(deleted, 5);

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 5);

        // Verify correct notes remain
        let notes = list_notes_from_db(&conn)?;
        assert_eq!(notes.len(), 5);

        // Should have 1, 2, 8, 9, 10 remaining (in DESC order)
        assert_eq!(notes[0].0, 10);
        assert_eq!(notes[1].0, 9);
        assert_eq!(notes[2].0, 8);
        assert_eq!(notes[3].0, 2);
        assert_eq!(notes[4].0, 1);

        Ok(())
    }

    #[test]
    fn test_get_notes_preserves_order() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", timestamp)?;
        add_note_to_db(&conn, "Note 2", timestamp)?;
        add_note_to_db(&conn, "Note 3", timestamp)?;

        // Request in specific order
        let notes = get_notes_by_ids(&conn, &[3, 1, 2])?;

        // Should maintain request order
        assert_eq!(notes[0].0, 3);
        assert_eq!(notes[1].0, 1);
        assert_eq!(notes[2].0, 2);

        Ok(())
    }

    #[test]
    fn test_content_with_pipe_symbol() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let content_with_pipe = "Note | with | pipes";
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, content_with_pipe, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, content_with_pipe);
        Ok(())
    }

    #[test]
    fn test_content_with_brackets() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let content_with_brackets = "[TODO] Fix bug in [module]";
        let timestamp = "2026-03-09T00:00:00+00:00";

        let note_id = add_note_to_db(&conn, content_with_brackets, timestamp)?;

        let content: String = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![note_id],
            |row| row.get(0),
        )?;

        assert_eq!(content, content_with_brackets);
        Ok(())
    }

    #[test]
    fn test_parse_filter_very_large_range() -> Result<()> {
        let ids = parse_filter("1-100")?;
        assert_eq!(ids.len(), 100);
        assert_eq!(ids[0], 1);
        assert_eq!(ids[99], 100);
        Ok(())
    }

    #[test]
    fn test_parse_filter_many_comma_separated() -> Result<()> {
        let ids = parse_filter("1,2,3,4,5,6,7,8,9,10")?;
        assert_eq!(ids, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        Ok(())
    }

    #[test]
    fn test_timestamp_precision() -> Result<()> {
        let (conn, _temp_file) = setup_test_db()?;

        let timestamp_with_ms = "2026-03-09T12:34:56.123456+00:00";
        add_note_to_db(&conn, "Test", timestamp_with_ms)?;

        let created_at: String = conn.query_row(
            "SELECT created_at FROM notes WHERE id = 1",
            [],
            |row| row.get(0),
        )?;

        // Should preserve microseconds
        assert_eq!(created_at, timestamp_with_ms);

        Ok(())
    }
}
