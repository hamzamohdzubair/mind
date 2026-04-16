use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand};
use colored::*;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use rusqlite::{Connection, params};
use std::io::{self, stdout, Write};
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
    /// Add a new note (interactive outliner if no content provided)
    Add {
        /// The content of the note (optional - omit to use interactive editor)
        content: Option<String>,
    },
    /// List all notes
    #[command(visible_alias = "ls")]
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

#[derive(Debug, Clone)]
struct OutlineLine {
    content: String,
    indent: usize,
}

struct OutlinerEditor {
    lines: Vec<OutlineLine>,
    current_line: usize,
    cursor_col: usize,
}

impl OutlinerEditor {
    fn new() -> Self {
        Self {
            lines: vec![OutlineLine {
                content: String::new(),
                indent: 0,
            }],
            current_line: 0,
            cursor_col: 0,
        }
    }

    fn render(&self, start_row: u16) -> Result<()> {
        let mut stdout = stdout();

        // Clear from start row downward
        execute!(stdout, cursor::MoveTo(0, start_row))?;
        execute!(stdout, Clear(ClearType::FromCursorDown))?;

        for (idx, line) in self.lines.iter().enumerate() {
            execute!(stdout, cursor::MoveTo(0, start_row + idx as u16))?;

            let indent_str = "  ".repeat(line.indent);
            let bullet = if line.indent == 0 { "•" } else { "◦" };
            let content = &line.content;

            if idx == self.current_line {
                print!("{}", format!("{}{} {}", indent_str, bullet, content).on_truecolor(18, 18, 18));
            } else {
                print!("{}{} {}", indent_str, bullet, content);
            }
        }

        // Position cursor - calculate display width correctly
        let current_line = &self.lines[self.current_line];
        let indent_spaces = current_line.indent * 2; // 2 spaces per indent
        let bullet_width = 2; // "• " or "◦ " = 1 char + 1 space = 2 display positions
        let cursor_x = (indent_spaces + bullet_width + self.cursor_col) as u16;
        let cursor_y = start_row + self.current_line as u16;
        execute!(stdout, cursor::MoveTo(cursor_x, cursor_y))?;
        stdout.flush()?;

        Ok(())
    }

    fn handle_enter(&mut self) {
        let current_indent = self.lines[self.current_line].indent;

        // Auto-add colon to first line if missing and it has content
        let mut has_colon = self.lines[self.current_line].content.trim_end().ends_with(':');
        if self.current_line == 0 && !self.lines[self.current_line].content.is_empty() && !has_colon {
            self.lines[self.current_line].content.push(':');
            has_colon = true; // Update has_colon after adding it
        }

        // Determine indent for new line
        let new_indent = if has_colon {
            // Create child if current line has colon
            current_indent + 1
        } else {
            // Create sibling at same level
            current_indent
        };

        // Insert new line
        let new_line = OutlineLine {
            content: String::new(),
            indent: new_indent,
        };
        self.lines.insert(self.current_line + 1, new_line);
        self.current_line += 1;
        self.cursor_col = 0;
    }

    fn handle_tab(&mut self, shift: bool) {
        if shift {
            // Decrease indent (Shift+Tab)
            let current_indent = self.lines[self.current_line].indent;
            if current_indent > 0 && self.current_line > 0 {
                let prev_indent = self.lines[self.current_line - 1].indent;
                // Only allow if not the first child
                if current_indent > prev_indent + 1 || self.current_line > 1 {
                    self.lines[self.current_line].indent = current_indent.saturating_sub(1);
                }
            }
        } else {
            // Increase indent (Tab)
            if self.current_line > 0 {
                let prev_indent = self.lines[self.current_line - 1].indent;
                let current_indent = self.lines[self.current_line].indent;

                // Auto-add colon to previous line if making this a child
                if current_indent == prev_indent && !self.lines[self.current_line - 1].content.ends_with(':') {
                    self.lines[self.current_line - 1].content.push(':');
                }

                // Only allow indent if within one level of previous
                if current_indent <= prev_indent {
                    self.lines[self.current_line].indent = prev_indent + 1;
                }
            }
        }
    }

    fn handle_char(&mut self, c: char) {
        let current_line = &mut self.lines[self.current_line];
        current_line.content.insert(self.cursor_col, c);
        self.cursor_col += 1;
    }

    fn handle_backspace(&mut self) {
        if self.cursor_col > 0 {
            let current_line = &mut self.lines[self.current_line];
            self.cursor_col -= 1;
            current_line.content.remove(self.cursor_col);
        } else if self.current_line > 0 {
            // Delete current line if empty and not first
            if self.lines[self.current_line].content.is_empty() {
                self.lines.remove(self.current_line);
                self.current_line -= 1;
                self.cursor_col = self.lines[self.current_line].content.len();
            }
        }
    }

    fn to_note_content(&self) -> String {
        self.lines
            .iter()
            .filter(|line| !line.content.is_empty())
            .map(|line| {
                let indent = "  ".repeat(line.indent);
                let bullet = if line.indent == 0 { "•" } else { "◦" };
                format!("{}{} {}", indent, bullet, line.content)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn interactive_outliner_add() -> Result<String> {
    // Ensure we have space at bottom - print 15 newlines to scroll up if needed
    for _ in 0..15 {
        println!();
    }

    // Move cursor back up
    let (_, current_row) = cursor::position()?;
    let start_row = current_row.saturating_sub(15);
    execute!(stdout(), cursor::MoveTo(0, start_row))?;

    // Print header without clearing screen
    println!("{}", "─".repeat(80).bright_black());
    println!("{}", "  Interactive outliner - Esc to save, Tab/Shift+Tab to indent, Enter for new line".dimmed());
    println!("{}", "─".repeat(80).bright_black());

    // Get current cursor position (after header)
    let (_, editor_start_row) = cursor::position()?;

    enable_raw_mode()?;
    let mut editor = OutlinerEditor::new();

    editor.render(editor_start_row)?;

    loop {
        if let Event::Key(KeyEvent { code, modifiers, .. }) = event::read()? {
            match code {
                KeyCode::Esc => {
                    disable_raw_mode()?;
                    // Move cursor to end of content
                    let final_row = editor_start_row + editor.lines.len() as u16;
                    execute!(stdout(), cursor::MoveTo(0, final_row))?;
                    println!(); // Add blank line after
                    return Ok(editor.to_note_content());
                }
                KeyCode::Enter => {
                    editor.handle_enter();
                }
                KeyCode::Tab => {
                    editor.handle_tab(modifiers.contains(KeyModifiers::SHIFT));
                }
                KeyCode::BackTab => {
                    // Shift+Tab is sent as BackTab on most terminals
                    editor.handle_tab(true);
                }
                KeyCode::Backspace => {
                    editor.handle_backspace();
                }
                KeyCode::Char(c) => {
                    editor.handle_char(c);
                }
                KeyCode::Up => {
                    if editor.current_line > 0 {
                        editor.current_line -= 1;
                        editor.cursor_col = editor.cursor_col.min(editor.lines[editor.current_line].content.len());
                    }
                }
                KeyCode::Down => {
                    if editor.current_line < editor.lines.len() - 1 {
                        editor.current_line += 1;
                        editor.cursor_col = editor.cursor_col.min(editor.lines[editor.current_line].content.len());
                    }
                }
                KeyCode::Left => {
                    if editor.cursor_col > 0 {
                        editor.cursor_col -= 1;
                    }
                }
                KeyCode::Right => {
                    let line_len = editor.lines[editor.current_line].content.len();
                    if editor.cursor_col < line_len {
                        editor.cursor_col += 1;
                    }
                }
                _ => {}
            }

            editor.render(editor_start_row)?;
        }
    }
}

fn add_note_to_db(conn: &Connection, content: &str, timestamp: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO notes (content, created_at, updated_at) VALUES (?1, ?2, ?3)",
        params![content, timestamp, timestamp],
    )
    .context("Could not insert note")?;

    Ok(conn.last_insert_rowid())
}

fn add_note(content: Option<&str>) -> Result<()> {
    let final_content = match content {
        Some(c) => c.to_string(),
        None => {
            // Enter interactive outliner mode
            let content = interactive_outliner_add()?;
            if content.is_empty() {
                println!("No content entered. Note not saved.");
                return Ok(());
            }
            content
        }
    };

    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path)
        .context("Could not open database")?;

    init_db(&conn)?;

    let now = Utc::now().to_rfc3339();
    let note_id = add_note_to_db(&conn, &final_content, &now)?;

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
        // Get terminal height
        let (_, terminal_height) = crossterm::terminal::size()
            .unwrap_or((80, 24)); // Default to 80x24 if can't detect

        // Reserve lines for: header (1) + footer (2) + buffer (2) = 5 lines
        let max_content_lines = (terminal_height as usize).saturating_sub(5);

        println!("{}", "─".repeat(100).bright_black());

        let mut lines_printed = 0;
        let mut notes_displayed = 0;
        let mut truncated = false;

        // Print notes with two-column layout: metadata on left, content on right
        for (index, (id, content, created_at)) in notes.iter().enumerate() {
            let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
                .context("Could not parse timestamp")?;
            let date = datetime.format("%Y-%m-%d");
            let time = datetime.format("%H:%M:%S");

            // Split content into lines
            let content_lines: Vec<&str> = content.lines().collect();

            // Calculate how many lines this note will take
            let note_lines = content_lines.len().max(3);
            let mut extra_lines = 0;

            // Check if there's a gap line after this note
            if index < notes.len() - 1 {
                let next_id = notes[index + 1].0;
                if *id - next_id > 1 {
                    extra_lines = 1;
                }
            }

            // Check if we have room for this note
            if lines_printed + note_lines + extra_lines > max_content_lines {
                truncated = true;
                break;
            }

            // Apply zebra striping
            let apply_bg = |text: String| -> String {
                if index % 2 == 0 {
                    format!("{}", text.on_truecolor(18, 18, 18))
                } else {
                    text
                }
            };

            // Left column width for metadata
            let left_width = 12;

            // Print first three lines with metadata
            if let Some(line) = content_lines.get(0) {
                println!("{}", apply_bg(format!("{:<width$} {}", id.to_string().bright_cyan().bold(), line, width = left_width)));
            } else {
                println!("{}", apply_bg(format!("{}", id.to_string().bright_cyan().bold())));
            }

            if let Some(line) = content_lines.get(1) {
                println!("{}", apply_bg(format!("{:<width$} {}", date.to_string().dimmed(), line, width = left_width)));
            } else {
                println!("{}", apply_bg(format!("{:<width$}", date.to_string().dimmed(), width = left_width)));
            }

            if let Some(line) = content_lines.get(2) {
                println!("{}", apply_bg(format!("{:<width$} {}", time.to_string().dimmed(), line, width = left_width)));
            } else {
                println!("{}", apply_bg(format!("{:<width$}", time.to_string().dimmed(), width = left_width)));
            }

            // Print remaining content lines with empty left column
            for line in content_lines.iter().skip(3) {
                println!("{}", apply_bg(format!("{:<width$} {}", "", line, width = left_width)));
            }

            lines_printed += note_lines;
            notes_displayed += 1;

            // Add line break if next ID is not consecutive (notes are in DESC order)
            if extra_lines > 0 {
                println!();
                lines_printed += 1;
            }
        }

        println!("{}", "─".repeat(100).bright_black());
        if truncated {
            println!("Showing {} of {} note(s) {}",
                notes_displayed,
                notes.len(),
                "(use delete or view commands to manage)".dimmed()
            );
        } else {
            println!("Total: {} note(s)", notes.len());
        }
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
                    break Ok(true);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    break Ok(false);
                }
                _ => continue,
            }
        }
    };
    disable_raw_mode()?;

    // Print the response after disabling raw mode
    match result {
        Ok(true) => println!("y"),
        Ok(false) => println!("n"),
        Err(_) => {},
    }

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
        Commands::Add { content } => add_note(content.as_deref())?,
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
        let result = add_note(Some("Test note from integration test"));

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
        add_note(Some("First note"))?;
        add_note(Some("Second note"))?;

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
        add_note(Some("First integration note"))?;
        add_note(Some("Second integration note"))?;
        add_note(Some("Third integration note"))?;

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
