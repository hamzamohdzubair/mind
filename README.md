# Mind

A zettelkasten-inspired note-taking and task management CLI tool, influenced by Taskwarrior and Logseq.

## Status

🚧 **Alpha** - Currently in early development. This is v0.1.0-alpha.1 with basic functionality.

## Installation

```bash
cargo install mind
```

Or build from source:

```bash
git clone https://github.com/yourusername/mind
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

## Roadmap

See [DESIGN.md](DESIGN.md) for the full vision and planned features.

**Upcoming features:**
- List and view notes
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
