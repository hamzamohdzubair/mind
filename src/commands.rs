use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use colored::*;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use rusqlite::Connection;
use std::io::{self, Write};

use crate::db::{
    add_note_to_db, delete_notes_by_ids as db_delete_notes_by_ids, get_db_path,
    get_notes_by_ids, init_db, list_notes_from_db,
};
use crate::editor::interactive_outliner_add;
use crate::tags::{collect_all_tags, find_similar_tags};

// ── Formatting ────────────────────────────────────────────────────────────────

pub(crate) fn build_notes_output(notes: &[(i64, String, String)]) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{}", "─".repeat(100).bright_black()));

    for (index, (id, content, created_at)) in notes.iter().enumerate() {
        let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
            .context("Could not parse timestamp")?;
        let date = datetime.format("%Y-%m-%d");
        let time = datetime.format("%H:%M:%S");
        let content_lines: Vec<&str> = content.lines().collect();

        let apply_bg = |text: String| -> String {
            if index % 2 == 0 {
                format!("{}", text.on_truecolor(18, 18, 18))
            } else {
                text
            }
        };

        let left_width = 12;

        if let Some(line) = content_lines.first() {
            lines.push(apply_bg(format!("{:<width$} {}", id.to_string().bright_cyan().bold(), line, width = left_width)));
        } else {
            lines.push(apply_bg(format!("{}", id.to_string().bright_cyan().bold())));
        }
        if let Some(line) = content_lines.get(1) {
            lines.push(apply_bg(format!("{:<width$} {}", date.to_string().dimmed(), line, width = left_width)));
        } else {
            lines.push(apply_bg(format!("{:<width$}", date.to_string().dimmed(), width = left_width)));
        }
        if let Some(line) = content_lines.get(2) {
            lines.push(apply_bg(format!("{:<width$} {}", time.to_string().dimmed(), line, width = left_width)));
        } else {
            lines.push(apply_bg(format!("{:<width$}", time.to_string().dimmed(), width = left_width)));
        }
        for line in content_lines.iter().skip(3) {
            lines.push(apply_bg(format!("{:<width$} {}", "", line, width = left_width)));
        }

        if index < notes.len() - 1 {
            let next_id = notes[index + 1].0;
            if *id - next_id > 1 {
                lines.push(String::new());
            }
        }
    }

    lines.push(format!("{}", "─".repeat(100).bright_black()));
    lines.push(format!("Total: {} note(s)", notes.len()));
    Ok(lines.join("\n") + "\n")
}

// ── Filtering ─────────────────────────────────────────────────────────────────

pub(crate) fn filter_notes_by_tag<'a>(
    notes: &'a [(i64, String, String)],
    tag: &str,
) -> Vec<&'a (i64, String, String)> {
    let needle = format!("#{}", tag.trim_start_matches('#').to_lowercase());
    notes
        .iter()
        .filter(|(_, content, _)| {
            let first_line = content.lines().next().unwrap_or("").to_lowercase();
            first_line.split_whitespace().any(|word| {
                let w = word.trim_end_matches(|c: char| !c.is_alphanumeric());
                w == needle
            })
        })
        .collect()
}

// ── Filter parsing ────────────────────────────────────────────────────────────

pub(crate) fn parse_filter(filter: &str) -> Result<Vec<i64>> {
    let mut ids = Vec::new();
    if filter.contains('-') {
        let parts: Vec<&str> = filter.split('-').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid range format. Use: <start>-<end>"));
        }
        let start: i64 = parts[0].trim().parse().context("Invalid start of range")?;
        let end: i64 = parts[1].trim().parse().context("Invalid end of range")?;
        if start > end {
            return Err(anyhow!("Range start must be less than or equal to end"));
        }
        ids.extend(start..=end);
    } else if filter.contains(',') {
        for part in filter.split(',') {
            let id: i64 = part.trim().parse()
                .context(format!("Invalid ID: {}", part.trim()))?;
            ids.push(id);
        }
    } else {
        let id: i64 = filter.trim().parse().context("Invalid ID format")?;
        ids.push(id);
    }
    Ok(ids)
}

// ── Confirmation ──────────────────────────────────────────────────────────────

pub(crate) fn confirm_deletion() -> Result<bool> {
    print!("{} ", "Delete? [y/n]:".yellow().bold());
    io::stdout().flush()?;
    enable_raw_mode()?;
    let result = loop {
        if let Event::Key(KeyEvent { code, .. }) = event::read()? {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => break Ok(true),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => break Ok(false),
                _ => continue,
            }
        }
    };
    disable_raw_mode()?;
    match result {
        Ok(true) => println!("y"),
        Ok(false) => println!("n"),
        Err(_) => {}
    }
    result
}

// ── Commands ──────────────────────────────────────────────────────────────────

pub(crate) fn add_note(content: Option<&str>) -> Result<()> {
    let final_content = match content {
        Some(c) => c.to_string(),
        None => {
            let content = interactive_outliner_add()?;
            if content.is_empty() {
                println!("No content entered. Note not saved.");
                return Ok(());
            }
            content
        }
    };

    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path).context("Could not open database")?;
    init_db(&conn)?;

    let now = Utc::now().to_rfc3339();
    let note_id = add_note_to_db(&conn, &final_content, &now)?;
    println!("Note added with ID: {}", note_id);
    Ok(())
}

pub(crate) fn list_notes(tag: Option<&str>) -> Result<()> {
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path).context("Could not open database")?;
    init_db(&conn)?;

    let all_notes = list_notes_from_db(&conn)?;

    let notes_owned: Vec<(i64, String, String)>;
    let notes: &[(i64, String, String)] = if let Some(t) = tag {
        let filtered = filter_notes_by_tag(&all_notes, t);
        notes_owned = filtered.into_iter().cloned().collect();
        &notes_owned
    } else {
        &all_notes
    };

    if notes.is_empty() {
        if let Some(t) = tag {
            println!("No notes found with tag {}.", format!("#{}", t.trim_start_matches('#')).bright_cyan());
            let all_tags = collect_all_tags(&all_notes);
            let similar = find_similar_tags(t, &all_tags);
            if !similar.is_empty() {
                println!("Are you looking for:");
                for suggestion in similar {
                    println!("  {}", suggestion.bright_cyan());
                }
            }
        } else {
            println!("No notes yet. Add one with: mind add \"your note\"");
        }
        return Ok(());
    }

    colored::control::set_override(true);
    let output = build_notes_output(notes)?;
    colored::control::unset_override();

    let (_, terminal_height) = crossterm::terminal::size().unwrap_or((80, 24));
    let line_count = output.lines().count();

    if line_count > terminal_height as usize {
        let pager = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
        let mut cmd = if pager == "less" || pager.ends_with("/less") {
            let mut c = std::process::Command::new(&pager);
            c.arg("-R");
            c
        } else {
            std::process::Command::new(&pager)
        };
        let mut child = cmd.stdin(std::process::Stdio::piped()).spawn()
            .context("Could not spawn pager")?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(output.as_bytes())?;
        }
        child.wait()?;
    } else {
        print!("{}", output);
    }

    Ok(())
}

pub(crate) fn delete_notes(filter: &str) -> Result<()> {
    let ids = parse_filter(filter)?;
    if ids.is_empty() {
        println!("No IDs to delete.");
        return Ok(());
    }

    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path).context("Could not open database")?;
    init_db(&conn)?;

    let notes = get_notes_by_ids(&conn, &ids)?;
    if notes.is_empty() {
        println!("No notes found matching the filter.");
        return Ok(());
    }

    println!("{}", "Notes to be deleted:".red().bold());
    println!("{}", "─".repeat(80).bright_black());
    for (id, content, created_at) in &notes {
        let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
            .context("Could not parse timestamp")?;
        let formatted_time = datetime.format("%Y-%m-%d %H:%M:%S");
        println!("{} {} | {}", format!("[{}]", id).red(), formatted_time, content);
    }
    println!("{}", "─".repeat(80).bright_black());

    if !confirm_deletion()? {
        println!("{}", "Deletion cancelled.".green());
        return Ok(());
    }

    let note_ids: Vec<i64> = notes.iter().map(|(id, _, _)| *id).collect();
    let deleted_count = db_delete_notes_by_ids(&conn, &note_ids)?;
    println!("{}", format!("Successfully deleted {} note(s).", deleted_count).green().bold());
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn test_parse_filter_single_id() -> Result<()> {
        assert_eq!(parse_filter("5")?, vec![5]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_range() -> Result<()> {
        assert_eq!(parse_filter("3-6")?, vec![3, 4, 5, 6]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_comma_separated() -> Result<()> {
        assert_eq!(parse_filter("1,2,5,8")?, vec![1, 2, 5, 8]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_comma_with_spaces() -> Result<()> {
        assert_eq!(parse_filter("1, 2, 5, 8")?, vec![1, 2, 5, 8]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_invalid_range() -> Result<()> {
        assert!(parse_filter("6-3").is_err());
        Ok(())
    }

    #[test]
    fn test_parse_filter_invalid_format() -> Result<()> {
        assert!(parse_filter("abc").is_err());
        Ok(())
    }

    #[test]
    fn test_parse_filter_range_equal() -> Result<()> {
        assert_eq!(parse_filter("5-5")?, vec![5]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_multiple_dashes() -> Result<()> {
        assert!(parse_filter("1-2-3").is_err());
        Ok(())
    }

    #[test]
    fn test_parse_filter_trailing_comma() -> Result<()> {
        assert!(parse_filter("1,").is_err());
        Ok(())
    }

    #[test]
    fn test_parse_filter_spaces() -> Result<()> {
        assert_eq!(parse_filter("  5  ")?, vec![5]);
        assert_eq!(parse_filter("  1 - 3  ")?, vec![1, 2, 3]);
        Ok(())
    }

    #[test]
    fn test_parse_filter_zero() -> Result<()> {
        assert_eq!(parse_filter("0")?, vec![0]);
        Ok(())
    }

    #[test]
    fn test_filter_notes_by_tag_matches() {
        let notes = vec![
            (1, "• Groceries #shopping".to_string(), "ts".to_string()),
            (2, "• Standup #work".to_string(), "ts".to_string()),
        ];
        let filtered = filter_notes_by_tag(&notes, "work");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, 2);
    }

    #[test]
    fn test_filter_notes_by_tag_with_hash_prefix() {
        let notes = vec![
            (1, "• Task #work".to_string(), "ts".to_string()),
        ];
        let filtered = filter_notes_by_tag(&notes, "#work");
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_notes_by_tag_case_insensitive() {
        let notes = vec![
            (1, "• Meeting #Work".to_string(), "ts".to_string()),
        ];
        let filtered = filter_notes_by_tag(&notes, "work");
        assert_eq!(filtered.len(), 1);
    }

    #[test]
    fn test_filter_notes_by_tag_only_first_line() {
        let notes = vec![
            (1, "• Update\n  ◦ details #work".to_string(), "ts".to_string()),
            (2, "• Fix bug #work".to_string(), "ts".to_string()),
        ];
        let filtered = filter_notes_by_tag(&notes, "work");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, 2);
    }

    #[test]
    fn test_filter_notes_by_tag_no_partial_match() {
        let notes = vec![
            (1, "• Status #working".to_string(), "ts".to_string()),
        ];
        let filtered = filter_notes_by_tag(&notes, "work");
        assert_eq!(filtered.len(), 0);
    }

    #[test]
    fn test_build_notes_output() -> Result<()> {
        let notes = vec![
            (1_i64, "Test note".to_string(), "2026-03-09T12:34:56+00:00".to_string()),
        ];
        let output = build_notes_output(&notes)?;
        assert!(output.contains("Test note"));
        assert!(output.contains("Total: 1 note(s)"));
        Ok(())
    }

    #[test]
    fn test_build_notes_output_multiline() -> Result<()> {
        let notes = vec![
            (1_i64, "Line1\nLine2\nLine3\nLine4".to_string(), "2026-03-09T12:34:56+00:00".to_string()),
        ];
        let output = build_notes_output(&notes)?;
        assert!(output.contains("Line1"));
        assert!(output.contains("Line2"));
        assert!(output.contains("Line3"));
        assert!(output.contains("Line4"));
        Ok(())
    }

    #[test]
    fn test_build_notes_output_gap_between_nonconsecutive() -> Result<()> {
        let notes = vec![
            (5_i64, "Note 5".to_string(), "2026-03-09T12:34:56+00:00".to_string()),
            (1_i64, "Note 1".to_string(), "2026-03-09T10:00:00+00:00".to_string()),
        ];
        let output = build_notes_output(&notes)?;
        // Gap (blank line) should be added between notes with non-consecutive IDs
        assert!(output.contains("Total: 2 note(s)"));
        Ok(())
    }

    // ── Integration tests ────────────────────────────────────────────────────

    fn setup_test_env() -> Result<tempfile::TempDir> {
        let temp_dir = tempfile::TempDir::new()?;
        unsafe { std::env::set_var("HOME", temp_dir.path()); }
        Ok(temp_dir)
    }

    #[test]
    #[serial_test::serial]
    fn test_add_note_creates_db() -> Result<()> {
        let temp_dir = setup_test_env()?;
        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        assert!(!db_path.exists());
        add_note(Some("Test note from integration test"))?;
        assert!(db_path.exists());
        let conn = rusqlite::Connection::open(&db_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 1);
        // Also verify header tags are NOT populated (no tags in this note)
        let tag_count: i64 = conn.query_row("SELECT COUNT(*) FROM header_tags", [], |row| row.get(0))?;
        assert_eq!(tag_count, 0);
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_add_note_with_tag_updates_header_tags() -> Result<()> {
        let temp_dir = setup_test_env()?;
        add_note(Some("• Sprint #work"))?;
        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        let conn = rusqlite::Connection::open(&db_path)?;
        let freq: i64 = conn.query_row(
            "SELECT freq FROM header_tags WHERE tag = '#work'", [], |row| row.get(0)
        )?;
        assert_eq!(freq, 1);
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_list_notes_with_empty_db() -> Result<()> {
        let _temp = setup_test_env()?;
        assert!(list_notes(None).is_ok());
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_list_notes_with_data() -> Result<()> {
        let temp_dir = setup_test_env()?;
        add_note(Some("First note"))?;
        add_note(Some("Second note"))?;
        assert!(list_notes(None).is_ok());
        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        let conn = rusqlite::Connection::open(&db_path)?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM notes", [], |row| row.get(0))?;
        assert_eq!(count, 2);
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_list_notes_by_tag_no_match() -> Result<()> {
        let _temp = setup_test_env()?;
        add_note(Some("• Task #work"))?;
        // Listing by nonexistent tag prints suggestion message, not an error
        assert!(list_notes(Some("personal")).is_ok());
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_list_notes_by_tag_match() -> Result<()> {
        let _temp = setup_test_env()?;
        add_note(Some("• Task #work"))?;
        add_note(Some("• Gym #health"))?;
        assert!(list_notes(Some("work")).is_ok());
        Ok(())
    }

    #[test]
    #[serial_test::serial]
    fn test_add_multiple_notes_integration() -> Result<()> {
        let temp_dir = setup_test_env()?;
        add_note(Some("First"))?;
        add_note(Some("Second"))?;
        add_note(Some("Third"))?;
        let db_path = temp_dir.path().join(".local/share/mind/mind.db");
        let conn = rusqlite::Connection::open(&db_path)?;
        let notes = crate::db::list_notes_from_db(&conn)?;
        assert_eq!(notes[0].1, "Third");
        assert_eq!(notes[1].1, "Second");
        assert_eq!(notes[2].1, "First");
        Ok(())
    }
}
