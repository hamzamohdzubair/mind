# Mind - Zettelkasten-inspired Outliner & Task Manager

## Vision
A CLI/TUI note-taking and task management tool combining:
- **Zettelkasten**: Bidirectional links, unique IDs, knowledge graph
- **Taskwarrior**: Powerful task management, filtering, states, priorities
- **Logseq**: Block-based outliner, daily notes, markdown syntax

## Tech Stack
- **Language**: Rust
- **TUI Framework**: ratatui
- **Database**: SQLite (rusqlite)
- **CLI**: clap
- **Terminal**: crossterm

## Future Architecture

### Database Schema (Full Vision)
- **blocks**: id, content, parent_id, created_at, modified_at
- **tasks**: block_id, status, priority, due_date, scheduled_date
- **tags**: id, name
- **block_tags**: block_id, tag_id
- **links**: from_block_id, to_block_id
- **pages**: id, title, created_at

### Features Roadmap
- [ ] **Phase 1 (MVP)**: Simple note storage with `mind add`
- [ ] **Phase 2**: List and view notes
- [ ] **Phase 3**: Task management (status, priority, due dates)
- [ ] **Phase 4**: Links between notes
- [ ] **Phase 5**: Tags
- [ ] **Phase 6**: Full TUI interface
- [ ] **Phase 7**: Block-based outliner
- [ ] **Phase 8**: Search and filtering
- [ ] **Phase 9**: Daily notes
- [ ] **Phase 10**: Backlinks and graph view

## MVP (Phase 1)
**Goal**: Store a simple one-line note with `mind add 'note content'`

**Database**: Single `notes` table with:
- id (INTEGER PRIMARY KEY)
- content (TEXT)
- created_at (TIMESTAMP)
- updated_at (TIMESTAMP)

**Commands**:
- `mind add <content>` - Add a new note
