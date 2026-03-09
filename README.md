# Mind

> _I am your mind, at your command, on the line: your command line mind_

A zettelkasten-inspired note-taking and task management CLI tool, influenced by Taskwarrior and Logseq.

## Status

🚧 **Alpha** - Currently in early development. This is v0.1.0-alpha.3 with basic functionality and comprehensive test coverage (>95%).

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

```bash
mind add "this is my first note"
```

This stores the note in an SQLite database at `~/.local/share/mind/mind.db` with:
- Unique ID (auto-incrementing)
- Content
- Creation timestamp (ISO 8601/RFC3339 format)
- Last updated timestamp

### List all notes

```bash
mind list
```

Displays all your notes in reverse chronological order (newest first) with ID, timestamp, and content.

## Roadmap

See [DESIGN.md](DESIGN.md) for the full vision and planned features.

**Upcoming features:**
- ✅ List notes
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
