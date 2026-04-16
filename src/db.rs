use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::PathBuf;

use crate::tags::extract_tag_relationships;

pub(crate) fn get_db_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .context("Could not find HOME directory")?;
    let data_dir = PathBuf::from(home).join(".local/share/mind");
    std::fs::create_dir_all(&data_dir)
        .context("Could not create data directory")?;
    Ok(data_dir.join("mind.db"))
}

pub(crate) fn init_db(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS notes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
        [],
    ).context("Could not create notes table")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS header_tags (
            tag  TEXT PRIMARY KEY,
            freq INTEGER NOT NULL DEFAULT 0
        )",
        [],
    ).context("Could not create header_tags table")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS tag_siblings (
            tag_a TEXT NOT NULL,
            tag_b TEXT NOT NULL,
            freq  INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (tag_a, tag_b)
        )",
        [],
    ).context("Could not create tag_siblings table")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS tag_children (
            header_tag TEXT NOT NULL,
            child_tag  TEXT NOT NULL,
            freq       INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (header_tag, child_tag)
        )",
        [],
    ).context("Could not create tag_children table")?;

    // Backfill tag stats if tables are empty but notes exist
    let header_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM header_tags", [], |row| row.get(0)
    ).unwrap_or(0);
    let notes_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM notes", [], |row| row.get(0)
    ).unwrap_or(0);
    if header_count == 0 && notes_count > 0 {
        backfill_tag_stats(conn)?;
    }

    Ok(())
}

// ── Tag stat helpers ──────────────────────────────────────────────────────────

pub(crate) fn update_tag_stats_on_add(conn: &Connection, content: &str) -> Result<()> {
    let rel = extract_tag_relationships(content);

    for tag in &rel.header_tags {
        conn.execute(
            "INSERT INTO header_tags (tag, freq) VALUES (?1, 1)
             ON CONFLICT(tag) DO UPDATE SET freq = freq + 1",
            params![tag],
        )?;
    }

    for (tag_a, tag_b) in &rel.sibling_pairs {
        conn.execute(
            "INSERT INTO tag_siblings (tag_a, tag_b, freq) VALUES (?1, ?2, 1)
             ON CONFLICT(tag_a, tag_b) DO UPDATE SET freq = freq + 1",
            params![tag_a, tag_b],
        )?;
    }

    for (header, child) in &rel.child_pairs {
        conn.execute(
            "INSERT INTO tag_children (header_tag, child_tag, freq) VALUES (?1, ?2, 1)
             ON CONFLICT(header_tag, child_tag) DO UPDATE SET freq = freq + 1",
            params![header, child],
        )?;
    }

    Ok(())
}

pub(crate) fn update_tag_stats_on_delete(conn: &Connection, content: &str) -> Result<()> {
    let rel = extract_tag_relationships(content);

    for tag in &rel.header_tags {
        conn.execute(
            "UPDATE header_tags SET freq = MAX(0, freq - 1) WHERE tag = ?1",
            params![tag],
        )?;
        conn.execute("DELETE FROM header_tags WHERE freq = 0", [])?;
    }

    for (tag_a, tag_b) in &rel.sibling_pairs {
        conn.execute(
            "UPDATE tag_siblings SET freq = MAX(0, freq - 1) WHERE tag_a = ?1 AND tag_b = ?2",
            params![tag_a, tag_b],
        )?;
        conn.execute("DELETE FROM tag_siblings WHERE freq = 0", [])?;
    }

    for (header, child) in &rel.child_pairs {
        conn.execute(
            "UPDATE tag_children SET freq = MAX(0, freq - 1) WHERE header_tag = ?1 AND child_tag = ?2",
            params![header, child],
        )?;
        conn.execute("DELETE FROM tag_children WHERE freq = 0", [])?;
    }

    Ok(())
}

pub(crate) fn backfill_tag_stats(conn: &Connection) -> Result<()> {
    let mut stmt = conn.prepare("SELECT content FROM notes")?;
    let contents: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();
    for content in &contents {
        update_tag_stats_on_add(conn, content)?;
    }
    Ok(())
}

// ── Note CRUD ─────────────────────────────────────────────────────────────────

pub(crate) fn add_note_to_db(conn: &Connection, content: &str, timestamp: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO notes (content, created_at, updated_at) VALUES (?1, ?2, ?3)",
        params![content, timestamp, timestamp],
    ).context("Could not insert note")?;

    let id = conn.last_insert_rowid();
    update_tag_stats_on_add(conn, content)?;
    Ok(id)
}

pub(crate) fn list_notes_from_db(conn: &Connection) -> Result<Vec<(i64, String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT id, content, created_at FROM notes ORDER BY id DESC"
    )?;
    let notes = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    })?;
    let mut result = Vec::new();
    for note in notes {
        result.push(note?);
    }
    Ok(result)
}

pub(crate) fn get_notes_by_ids(conn: &Connection, ids: &[i64]) -> Result<Vec<(i64, String, String)>> {
    let mut notes = Vec::new();
    for &id in ids {
        let result = conn.query_row(
            "SELECT id, content, created_at FROM notes WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
        );
        if let Ok(note) = result {
            notes.push(note);
        }
    }
    Ok(notes)
}

pub(crate) fn delete_notes_by_ids(conn: &Connection, ids: &[i64]) -> Result<usize> {
    let mut deleted_count = 0;
    for &id in ids {
        // Fetch content before deleting so we can update tag stats
        let content: Option<String> = conn.query_row(
            "SELECT content FROM notes WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).ok();

        let rows_affected = conn.execute("DELETE FROM notes WHERE id = ?1", params![id])?;
        if rows_affected > 0 {
            if let Some(c) = content {
                update_tag_stats_on_delete(conn, &c)?;
            }
            deleted_count += rows_affected;
        }
    }
    Ok(deleted_count)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;
    use std::env;
    use serial_test::serial;
    use tempfile::TempDir;

    fn setup_test_db() -> Result<(Connection, NamedTempFile)> {
        let temp_file = NamedTempFile::new()?;
        let conn = Connection::open(temp_file.path())?;
        init_db(&conn)?;
        Ok((conn, temp_file))
    }

    fn setup_test_env() -> Result<TempDir> {
        let temp_dir = TempDir::new()?;
        unsafe { env::set_var("HOME", temp_dir.path()); }
        Ok(temp_dir)
    }

    #[test]
    fn test_init_db_creates_all_tables() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        for table in &["notes", "header_tags", "tag_siblings", "tag_children"] {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table], |row| row.get(0),
            )?;
            assert_eq!(count, 1, "table {} should exist", table);
        }
        Ok(())
    }

    #[test]
    fn test_init_db_is_idempotent() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        init_db(&conn)?;
        init_db(&conn)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='notes'",
            [], |row| row.get(0),
        )?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn test_add_note_to_db_basic() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let id = add_note_to_db(&conn, "Test note", "2026-03-09T00:00:00+00:00")?;
        assert_eq!(id, 1);
        Ok(())
    }

    #[test]
    fn test_add_note_updates_header_tags() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        add_note_to_db(&conn, "• Task #work #urgent", "2026-03-09T00:00:00+00:00")?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(freq, 1);
        let freq2: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#urgent'", [], |row| row.get(0)
        )?;
        assert_eq!(freq2, 1);
        Ok(())
    }

    #[test]
    fn test_add_note_updates_siblings() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        add_note_to_db(&conn, "• Task #work #urgent", "2026-03-09T00:00:00+00:00")?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM tag_siblings", [], |row| row.get(0)
        )?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    fn test_add_note_updates_children() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        add_note_to_db(&conn, "• Note #work\n  ◦ sub #task", "2026-03-09T00:00:00+00:00")?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM tag_children WHERE header_tag = '#work' AND child_tag = '#task'",
            [], |row| row.get(0)
        )?;
        assert_eq!(freq, 1);
        Ok(())
    }

    #[test]
    fn test_update_tag_stats_increments() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let ts = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "• Task #work", ts)?;
        add_note_to_db(&conn, "• Meeting #work", ts)?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(freq, 2);
        Ok(())
    }

    #[test]
    fn test_delete_notes_decrements_tag_stats() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let ts = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "• Task #work", ts)?;
        add_note_to_db(&conn, "• Meeting #work", ts)?;
        delete_notes_by_ids(&conn, &[1])?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(freq, 1);
        Ok(())
    }

    #[test]
    fn test_delete_notes_removes_zero_freq_tags() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let ts = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "• Task #work", ts)?;
        delete_notes_by_ids(&conn, &[1])?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(count, 0);
        Ok(())
    }

    #[test]
    fn test_backfill_tag_stats() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let conn = Connection::open(temp_file.path())?;
        // Create tables without backfill (simulate pre-feature DB)
        conn.execute("CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)", [])?;
        conn.execute("CREATE TABLE IF NOT EXISTS header_tags (tag TEXT PRIMARY KEY, freq INTEGER NOT NULL DEFAULT 0)", [])?;
        conn.execute("CREATE TABLE IF NOT EXISTS tag_siblings (tag_a TEXT NOT NULL, tag_b TEXT NOT NULL, freq INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (tag_a, tag_b))", [])?;
        conn.execute("CREATE TABLE IF NOT EXISTS tag_children (header_tag TEXT NOT NULL, child_tag TEXT NOT NULL, freq INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (header_tag, child_tag))", [])?;
        // Insert note directly (bypassing tag stat update)
        conn.execute("INSERT INTO notes (content, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params!["• Task #work", "2026-03-09T00:00:00+00:00"])?;
        // Now backfill
        backfill_tag_stats(&conn)?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(freq, 1);
        Ok(())
    }

    #[test]
    fn test_init_db_backfills_existing_notes() -> Result<()> {
        let temp_file = NamedTempFile::new()?;
        let conn = Connection::open(temp_file.path())?;
        // Simulate old DB: only notes table, then add a note manually
        conn.execute("CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY AUTOINCREMENT, content TEXT NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)", [])?;
        conn.execute("INSERT INTO notes (content, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params!["• Task #work", "2026-03-09T00:00:00+00:00"])?;
        // init_db should create tables AND backfill
        init_db(&conn)?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(freq, 1);
        Ok(())
    }

    #[test]
    fn test_list_notes_from_db() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let ts = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "First", ts)?;
        add_note_to_db(&conn, "Second", ts)?;
        let notes = list_notes_from_db(&conn)?;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].1, "Second"); // DESC order
        Ok(())
    }

    #[test]
    fn test_get_notes_by_ids() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let ts = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", ts)?;
        add_note_to_db(&conn, "Note 2", ts)?;
        add_note_to_db(&conn, "Note 3", ts)?;
        let notes = get_notes_by_ids(&conn, &[1, 3])?;
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].1, "Note 1");
        assert_eq!(notes[1].1, "Note 3");
        Ok(())
    }

    #[test]
    fn test_delete_notes_by_ids() -> Result<()> {
        let (conn, _f) = setup_test_db()?;
        let ts = "2026-03-09T00:00:00+00:00";
        add_note_to_db(&conn, "Note 1", ts)?;
        add_note_to_db(&conn, "Note 2", ts)?;
        let deleted = delete_notes_by_ids(&conn, &[1])?;
        assert_eq!(deleted, 1);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[test]
    #[serial]
    fn test_get_db_path() -> Result<()> {
        let _temp = setup_test_env()?;
        let path = get_db_path()?;
        assert!(path.to_string_lossy().contains(".local/share/mind"));
        assert_eq!(path.file_name().unwrap(), "mind.db");
        Ok(())
    }
}
