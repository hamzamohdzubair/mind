use anyhow::Result;
use colored::*;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType},
};
use rusqlite::Connection;
use std::io::{stdout, Write};

use crate::db::get_db_path;
use crate::db::init_db;
use crate::tags::{load_tag_family, render_tag_family_panel, TagFamily};

#[derive(Debug, Clone)]
pub(crate) struct OutlineLine {
    pub content: String,
    pub indent: usize,
}

pub(crate) struct OutlinerEditor {
    pub lines: Vec<OutlineLine>,
    pub current_line: usize,
    pub cursor_col: usize,
    /// Number of rows the family panel occupied on the last render (for clearing)
    panel_rows: usize,
}

impl OutlinerEditor {
    pub fn new() -> Self {
        Self {
            lines: vec![OutlineLine { content: String::new(), indent: 0 }],
            current_line: 0,
            cursor_col: 0,
            panel_rows: 0,
        }
    }

    /// Returns the `#tag` word currently being typed at the cursor, or None.
    pub fn current_tag_at_cursor(&self) -> Option<String> {
        let content = &self.lines[self.current_line].content;
        let before_cursor = &content[..self.cursor_col.min(content.len())];
        // If the character immediately before the cursor is whitespace, no word is being typed
        if before_cursor.ends_with(|c: char| c.is_whitespace()) {
            return None;
        }
        let last_word = before_cursor.split_whitespace().last()?;
        if last_word.starts_with('#') {
            Some(last_word.to_string())
        } else {
            None
        }
    }

    pub fn render(&mut self, start_row: u16, family: Option<&TagFamily>) -> Result<()> {
        let mut stdout = stdout();

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

        // Render family panel below editor content
        let panel_start = start_row + self.lines.len() as u16 + 1;

        // Clear previous panel area
        for i in 0..self.panel_rows {
            execute!(stdout, cursor::MoveTo(0, panel_start + i as u16))?;
            execute!(stdout, Clear(ClearType::CurrentLine))?;
        }

        let new_panel_rows = if let Some(fam) = family {
            render_tag_family_panel(fam, panel_start)?
        } else {
            0
        };
        self.panel_rows = new_panel_rows;

        // Restore editing cursor
        let current_line = &self.lines[self.current_line];
        let indent_spaces = current_line.indent * 2;
        let bullet_width = 2;
        let cursor_x = (indent_spaces + bullet_width + self.cursor_col) as u16;
        let cursor_y = start_row + self.current_line as u16;
        execute!(stdout, cursor::MoveTo(cursor_x, cursor_y))?;
        stdout.flush()?;

        Ok(())
    }

    pub fn handle_enter(&mut self) {
        let current_indent = self.lines[self.current_line].indent;
        let mut has_colon = self.lines[self.current_line].content.trim_end().ends_with(':');
        if self.current_line == 0 && !self.lines[self.current_line].content.is_empty() && !has_colon {
            self.lines[self.current_line].content.push(':');
            has_colon = true;
        }
        let new_indent = if has_colon { current_indent + 1 } else { current_indent };
        self.lines.insert(self.current_line + 1, OutlineLine { content: String::new(), indent: new_indent });
        self.current_line += 1;
        self.cursor_col = 0;
    }

    pub fn handle_tab(&mut self, shift: bool) {
        if shift {
            let current_indent = self.lines[self.current_line].indent;
            if current_indent > 0 && self.current_line > 0 {
                let prev_indent = self.lines[self.current_line - 1].indent;
                if current_indent > prev_indent + 1 || self.current_line > 1 {
                    self.lines[self.current_line].indent = current_indent.saturating_sub(1);
                }
            }
        } else {
            if self.current_line > 0 {
                let prev_indent = self.lines[self.current_line - 1].indent;
                let current_indent = self.lines[self.current_line].indent;
                if current_indent == prev_indent && !self.lines[self.current_line - 1].content.ends_with(':') {
                    self.lines[self.current_line - 1].content.push(':');
                }
                if current_indent <= prev_indent {
                    self.lines[self.current_line].indent = prev_indent + 1;
                }
            }
        }
    }

    pub fn handle_char(&mut self, c: char) {
        let current_line = &mut self.lines[self.current_line];
        current_line.content.insert(self.cursor_col, c);
        self.cursor_col += 1;
    }

    pub fn handle_backspace(&mut self) {
        if self.cursor_col > 0 {
            let current_line = &mut self.lines[self.current_line];
            self.cursor_col -= 1;
            current_line.content.remove(self.cursor_col);
        } else if self.current_line > 0 {
            if self.lines[self.current_line].content.is_empty() {
                self.lines.remove(self.current_line);
                self.current_line -= 1;
                self.cursor_col = self.lines[self.current_line].content.len();
            }
        }
    }

    pub fn to_note_content(&self) -> String {
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

pub(crate) fn interactive_outliner_add() -> Result<String> {
    // Open DB for tag family lookups
    let db_path = get_db_path()?;
    let conn = Connection::open(&db_path).ok();
    if let Some(ref c) = conn {
        let _ = init_db(c);
    }

    for _ in 0..15 { println!(); }
    let (_, current_row) = cursor::position()?;
    let start_row = current_row.saturating_sub(15);
    execute!(stdout(), cursor::MoveTo(0, start_row))?;

    println!("{}", "─".repeat(80).bright_black());
    println!("{}", "  Interactive outliner - Esc to save, Tab/Shift+Tab to indent, Enter for new line".dimmed());
    println!("{}", "─".repeat(80).bright_black());

    let (_, editor_start_row) = cursor::position()?;

    enable_raw_mode()?;
    let mut editor = OutlinerEditor::new();
    editor.render(editor_start_row, None)?;

    loop {
        if let Event::Key(KeyEvent { code, modifiers, .. }) = event::read()? {
            match code {
                KeyCode::Esc => {
                    disable_raw_mode()?;
                    let final_row = editor_start_row + editor.lines.len() as u16 + editor.panel_rows as u16 + 1;
                    execute!(stdout(), cursor::MoveTo(0, final_row))?;
                    println!();
                    return Ok(editor.to_note_content());
                }
                KeyCode::Enter => { editor.handle_enter(); }
                KeyCode::Tab => { editor.handle_tab(modifiers.contains(KeyModifiers::SHIFT)); }
                KeyCode::BackTab => { editor.handle_tab(true); }
                KeyCode::Backspace => { editor.handle_backspace(); }
                KeyCode::Char(c) => { editor.handle_char(c); }
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
                    if editor.cursor_col > 0 { editor.cursor_col -= 1; }
                }
                KeyCode::Right => {
                    let line_len = editor.lines[editor.current_line].content.len();
                    if editor.cursor_col < line_len { editor.cursor_col += 1; }
                }
                _ => {}
            }

            // Load tag family for the word at cursor
            let family: Option<TagFamily> = conn.as_ref().and_then(|c| {
                editor.current_tag_at_cursor()
                    .and_then(|t| load_tag_family(c, &t).ok().flatten())
            });

            editor.render(editor_start_row, family.as_ref())?;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_editor_with_content(content: &str, cursor_col: usize) -> OutlinerEditor {
        let mut e = OutlinerEditor::new();
        e.lines[0].content = content.to_string();
        e.cursor_col = cursor_col;
        e
    }

    #[test]
    fn test_current_tag_at_cursor_mid_hash_word() {
        let e = make_editor_with_content("meeting #pro", 12);
        assert_eq!(e.current_tag_at_cursor(), Some("#pro".to_string()));
    }

    #[test]
    fn test_current_tag_at_cursor_full_tag() {
        let e = make_editor_with_content("meeting #project", 16);
        assert_eq!(e.current_tag_at_cursor(), Some("#project".to_string()));
    }

    #[test]
    fn test_current_tag_at_cursor_no_hash() {
        let e = make_editor_with_content("meeting notes", 13);
        assert_eq!(e.current_tag_at_cursor(), None);
    }

    #[test]
    fn test_current_tag_at_cursor_trailing_space() {
        // After completing tag and pressing space, cursor is past it
        let e = make_editor_with_content("#project ", 9);
        assert_eq!(e.current_tag_at_cursor(), None);
    }

    #[test]
    fn test_current_tag_at_cursor_empty_line() {
        let e = make_editor_with_content("", 0);
        assert_eq!(e.current_tag_at_cursor(), None);
    }

    #[test]
    fn test_current_tag_at_cursor_bare_hash() {
        let e = make_editor_with_content("note #", 6);
        assert_eq!(e.current_tag_at_cursor(), Some("#".to_string()));
    }

    #[test]
    fn test_to_note_content_basic() {
        let mut e = OutlinerEditor::new();
        e.lines[0].content = "Task".to_string();
        assert_eq!(e.to_note_content(), "• Task");
    }

    #[test]
    fn test_to_note_content_skips_empty_lines() {
        let mut e = OutlinerEditor::new();
        e.lines[0].content = "Task".to_string();
        e.lines.push(OutlineLine { content: String::new(), indent: 0 });
        e.lines.push(OutlineLine { content: "Sub".to_string(), indent: 1 });
        let content = e.to_note_content();
        assert!(content.contains("• Task"));
        assert!(content.contains("◦ Sub"));
        assert!(!content.contains("• \n"));
    }

    #[test]
    fn test_handle_char_inserts() {
        let mut e = OutlinerEditor::new();
        e.handle_char('h');
        e.handle_char('i');
        assert_eq!(e.lines[0].content, "hi");
        assert_eq!(e.cursor_col, 2);
    }

    #[test]
    fn test_handle_backspace_removes_char() {
        let mut e = OutlinerEditor::new();
        e.handle_char('h');
        e.handle_char('i');
        e.handle_backspace();
        assert_eq!(e.lines[0].content, "h");
        assert_eq!(e.cursor_col, 1);
    }

    #[test]
    fn test_handle_enter_creates_sibling() {
        let mut e = OutlinerEditor::new();
        e.lines[0].content = "Parent".to_string();
        e.cursor_col = 6;
        e.handle_enter(); // first line has no colon yet but it's line 0 so colon is added
        // Line 0 gets colon, new line is child (indent 1)
        assert_eq!(e.lines[0].content, "Parent:");
        assert_eq!(e.lines[1].indent, 1);
    }
}
