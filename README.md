# Mind

> _I am your mind, at your command, on the line: your command line mind_

A zettelkasten-inspired note-taking and task management CLI tool, influenced by Taskwarrior and Logseq.

## Status

🚧 **Alpha** - Currently in early development. This is v0.1.0-alpha.11 with interactive outliner and comprehensive test coverage (>90%).

## Installation

```bash
cargo install mind
```

Or build from source:

```bash
git clone https://github.com/hamzamohdzubair/mind
cd mind
cargo install --path .
```

## Usage

### Add a note

**Quick add:**
```bash
mind add "this is my first note"
```

**Interactive outliner mode:**
```bash
mind add
```

This opens an interactive editing space with:
- **Enter**: Create new bullet point (child if parent has `:`, sibling otherwise)
- **Tab**: Indent (make child, auto-adds `:` to parent)
- **Shift+Tab**: Un-indent (make sibling)
- **Arrow keys**: Navigate
- **Escape**: Save and exit
- Auto-adds `:` to first line and parent lines when creating children
- First child cannot be un-indented (visually clear)

Notes are stored in SQLite at `~/.local/share/mind/mind.db` with:
- Unique ID (auto-incrementing)
- Content (formatted with bullets and indentation)
- Creation timestamp (ISO 8601/RFC3339 format)
- Last updated timestamp

### List all notes

```bash
mind list
```

Displays all your notes in a styled table format with:
- Alternating row colors for easy reading
- Column headers (ID, DATE, CONTENT)
- Reverse chronological order (newest first)

### Delete notes

```bash
# Delete a single note
mind delete 5
mind del 5        # shorthand alias

# Delete multiple notes (comma-separated)
mind delete 1,2,5

# Delete a range of notes
mind delete 3-6

# Future: Delete by tag
mind delete #work  # coming soon
```

The delete command supports multiple filter formats and always asks for confirmation with a single-key press (y/n).

## Roadmap

See [DESIGN.md](DESIGN.md) for the full vision and planned features.

**Completed features:**
- ✅ Add notes
- ✅ List notes (styled table view)
- ✅ Delete notes (by ID, range, or comma-separated)

**Upcoming features:**
- View individual notes
- Task management (status, priority, due dates)
- Links between notes ([[wiki-links]])
- Tags
- Full TUI interface
- Block-based outliner
- Search and filtering
- Daily notes
- Backlinks and graph view

## Database Location

- **Linux/macOS**: `~/.local/share/mind/mind.db`
- **Windows**: `%LOCALAPPDATA%\mind\mind.db` (coming soon)

## Contributing

This project is in early alpha. Contributions, ideas, and feedback are welcome!

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
