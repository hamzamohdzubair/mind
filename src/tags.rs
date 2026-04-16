use anyhow::Result;
use colored::*;
use crossterm::{cursor, execute};
use rusqlite::Connection;
use std::io::{stdout, Write};

// ── Tag extraction ────────────────────────────────────────────────────────────

pub(crate) fn extract_tags_from_line(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter_map(|word| {
            let w = word.trim_end_matches(|c: char| !c.is_alphanumeric());
            if w.starts_with('#') && w.len() > 1 {
                Some(w.to_lowercase())
            } else {
                None
            }
        })
        .collect()
}

pub(crate) fn extract_tags_from_first_line(content: &str) -> Vec<String> {
    let first_line = content.lines().next().unwrap_or("");
    extract_tags_from_line(first_line)
}

#[derive(Debug, Default)]
pub(crate) struct TagRelationships {
    pub header_tags: Vec<String>,
    pub sibling_pairs: Vec<(String, String)>,  // canonical: first < second
    pub child_pairs: Vec<(String, String)>,    // (header_tag, child_tag)
}

pub(crate) fn extract_tag_relationships(content: &str) -> TagRelationships {
    let mut lines = content.lines();
    let first_line = lines.next().unwrap_or("");
    let header_tags = extract_tags_from_line(first_line);

    // Sibling pairs: all C(n,2) combos of header tags, canonically ordered
    let mut sibling_pairs: Vec<(String, String)> = Vec::new();
    for i in 0..header_tags.len() {
        for j in (i + 1)..header_tags.len() {
            let (a, b) = if header_tags[i] < header_tags[j] {
                (header_tags[i].clone(), header_tags[j].clone())
            } else {
                (header_tags[j].clone(), header_tags[i].clone())
            };
            sibling_pairs.push((a, b));
        }
    }

    // Child pairs: tags in indented lines × each header tag
    let mut child_pairs: Vec<(String, String)> = Vec::new();
    for line in lines {
        // Count leading spaces to determine indent (2 spaces = 1 level)
        let indent = line.len() - line.trim_start().len();
        if indent >= 2 {
            let child_tags = extract_tags_from_line(line);
            for child in &child_tags {
                for header in &header_tags {
                    if child != header {
                        let pair = (header.clone(), child.clone());
                        if !child_pairs.contains(&pair) {
                            child_pairs.push(pair);
                        }
                    }
                }
            }
        }
    }

    TagRelationships { header_tags, sibling_pairs, child_pairs }
}

// ── Levenshtein / fuzzy tag matching ─────────────────────────────────────────

pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (m, n) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in 0..=m { dp[i][0] = i; }
    for j in 0..=n { dp[0][j] = j; }
    for i in 1..=m {
        for j in 1..=n {
            dp[i][j] = if a[i - 1] == b[j - 1] {
                dp[i - 1][j - 1]
            } else {
                1 + dp[i - 1][j].min(dp[i][j - 1]).min(dp[i - 1][j - 1])
            };
        }
    }
    dp[m][n]
}

pub(crate) fn find_similar_tags(needle: &str, all_tags: &[String]) -> Vec<String> {
    let clean = needle.trim_start_matches('#').to_lowercase();
    let threshold = (clean.len() / 3).max(1).min(3);
    let mut scored: Vec<(usize, &String)> = all_tags
        .iter()
        .filter_map(|tag| {
            let t = tag.trim_start_matches('#');
            let d = levenshtein(&clean, t);
            if d <= threshold && d > 0 {
                Some((d, tag))
            } else {
                None
            }
        })
        .collect();
    scored.sort_by_key(|(d, _)| *d);
    scored.into_iter().map(|(_, t)| t.clone()).collect()
}

pub(crate) fn collect_all_tags(notes: &[(i64, String, String)]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut tags: Vec<String> = Vec::new();
    for (_, content, _) in notes {
        for tag in extract_tags_from_first_line(content) {
            if seen.insert(tag.clone()) {
                tags.push(tag);
            }
        }
    }
    tags.sort();
    tags
}

// ── Tag family ────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub(crate) struct TagFamily {
    pub tag: String,
    /// (parent_tag, tree_siblings, orphan_siblings)
    /// tree_siblings   = siblings that are also children of this parent
    /// orphan_siblings = siblings NOT under this parent
    pub parents: Vec<(String, Vec<String>, Vec<String>)>,
    /// Used when there are no parents
    pub all_siblings: Vec<String>,
    pub children: Vec<String>,
}

pub(crate) fn load_tag_family(conn: &Connection, partial: &str) -> Result<Option<TagFamily>> {
    let pattern = format!("{}%", partial.to_lowercase());

    // Resolve partial to the best-matching known tag
    let resolved: Option<String> = {
        // Check header_tags first
        let mut stmt = conn.prepare(
            "SELECT tag FROM header_tags WHERE tag LIKE ?1 ORDER BY freq DESC LIMIT 1"
        )?;
        let from_headers: Option<String> = stmt
            .query_map([&pattern], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .next();

        if from_headers.is_some() {
            from_headers
        } else {
            // Fall back to child tags
            let mut stmt2 = conn.prepare(
                "SELECT child_tag FROM tag_children WHERE child_tag LIKE ?1 ORDER BY freq DESC LIMIT 1"
            )?;
            stmt2
                .query_map([&pattern], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .next()
        }
    };

    let tag = match resolved {
        Some(t) => t,
        None => return Ok(None),
    };

    // Query children of this tag
    let children: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT child_tag FROM tag_children WHERE header_tag = ?1 ORDER BY freq DESC LIMIT 3"
        )?;
        stmt.query_map([&tag], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect()
    };

    // Query all siblings of this tag
    let all_siblings: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT tag_b, freq FROM tag_siblings WHERE tag_a = ?1
             UNION
             SELECT tag_a, freq FROM tag_siblings WHERE tag_b = ?1
             ORDER BY freq DESC"
        )?;
        // ?1 refers to the same parameter in both halves of the UNION
        stmt.query_map(rusqlite::params![tag], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect()
    };

    // Query parents
    let parent_tags: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT header_tag FROM tag_children WHERE child_tag = ?1 ORDER BY freq DESC LIMIT 3"
        )?;
        stmt.query_map([&tag], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect()
    };

    if parent_tags.is_empty() {
        return Ok(Some(TagFamily {
            tag,
            parents: vec![],
            all_siblings: all_siblings.into_iter().take(3).collect(),
            children,
        }));
    }

    // For each parent, split siblings into tree vs orphan
    let mut parents: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for parent in &parent_tags {
        // Get all children of this parent
        let parent_children: std::collections::HashSet<String> = {
            let mut stmt = conn.prepare(
                "SELECT child_tag FROM tag_children WHERE header_tag = ?1"
            )?;
            stmt.query_map([parent], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let mut tree_siblings: Vec<String> = Vec::new();
        let mut orphan_siblings: Vec<String> = Vec::new();
        for sib in &all_siblings {
            if parent_children.contains(sib) {
                tree_siblings.push(sib.clone());
            } else {
                orphan_siblings.push(sib.clone());
            }
        }
        // Limit each to 3
        tree_siblings.truncate(3);
        orphan_siblings.truncate(3);
        parents.push((parent.clone(), tree_siblings, orphan_siblings));
    }

    Ok(Some(TagFamily {
        tag,
        parents,
        all_siblings: vec![],
        children,
    }))
}

// ── Tag family panel rendering ────────────────────────────────────────────────

fn write_panel_line(row: u16, text: &str) -> Result<()> {
    let mut out = stdout();
    execute!(out, cursor::MoveTo(0, row))?;
    execute!(out, crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine))?;
    write!(out, "{}", text)?;
    Ok(())
}

/// Renders the tag family tree panel starting at `start_row`.
/// Returns the number of terminal rows written.
pub(crate) fn render_tag_family_panel(
    family: &TagFamily,
    start_row: u16,
) -> Result<usize> {
    let mut row = start_row;

    if family.parents.is_empty() {
        // No parents — show tag at root level with inline orphan siblings
        write_panel_line(row, &build_tag_line_no_parent(family))?;
        row += 1;
        row += render_children_at(&family.children, row, "  ")? as u16;
    } else {
        let last_parent_idx = family.parents.len() - 1;
        for (pidx, (parent, tree_sibs, orphan_sibs)) in family.parents.iter().enumerate() {
            // Parent folder line
            write_panel_line(row, &format!("  {}", format!("{}/", parent).dimmed().cyan()))?;
            row += 1;

            // Tree siblings as peer branches (├──)
            for sib in tree_sibs {
                write_panel_line(row, &format!("  {}", format!("├── {}", sib).dimmed()))?;
                row += 1;
            }

            // The tag itself (└──), with orphan siblings inline
            let tag_display = if orphan_sibs.is_empty() {
                format!("  {} {}", "└──".dimmed(), format!("{} ◄", family.tag).bold().white())
            } else {
                let orphans = orphan_sibs
                    .iter()
                    .map(|s| s.dimmed().to_string())
                    .collect::<Vec<_>>()
                    .join(&format!(" {} ", "·".dimmed()));
                format!(
                    "  {} {} {} {} {}",
                    "└──".dimmed(),
                    format!("{}", family.tag).bold().white(),
                    "·".dimmed(),
                    orphans,
                    "◄".bold().white(),
                )
            };
            write_panel_line(row, &tag_display)?;
            row += 1;

            // Children nested under the tag
            row += render_children_at(&family.children, row, "      ")? as u16;

            // Blank line between parent blocks (not after last)
            if pidx < last_parent_idx {
                write_panel_line(row, "")?;
                row += 1;
            }
        }
    }

    stdout().flush()?;
    Ok((row - start_row) as usize)
}

fn build_tag_line_no_parent(family: &TagFamily) -> String {
    if family.all_siblings.is_empty() {
        format!("  {} {}", family.tag.bold().white(), "◄".bold().white())
    } else {
        let sibs = family.all_siblings
            .iter()
            .map(|s| s.dimmed().to_string())
            .collect::<Vec<_>>()
            .join(&format!(" {} ", "·".dimmed()));
        format!(
            "  {} {} {} {}",
            family.tag.bold().white(),
            "·".dimmed(),
            sibs,
            "◄".bold().white(),
        )
    }
}

fn render_children_at(children: &[String], start_row: u16, indent: &str) -> Result<usize> {
    for (i, child) in children.iter().enumerate() {
        let connector = if i == children.len() - 1 { "└──" } else { "├──" };
        write_panel_line(
            start_row + i as u16,
            &format!("{}{} {}", indent, connector, child).dimmed().to_string(),
        )?;
    }
    Ok(children.len())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_tags_from_line_basic() {
        let tags = extract_tags_from_line("• Sprint planning #work #urgent");
        assert_eq!(tags, vec!["#work", "#urgent"]);
    }

    #[test]
    fn test_extract_tags_from_line_no_tags() {
        let tags = extract_tags_from_line("• Plain note");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_extract_tags_from_line_with_punctuation() {
        let tags = extract_tags_from_line("Note with #work, and #home.");
        assert_eq!(tags, vec!["#work", "#home"]);
    }

    #[test]
    fn test_extract_tags_from_first_line_only_first() {
        let content = "• First line #work\n  ◦ Second line #other";
        let tags = extract_tags_from_first_line(content);
        assert_eq!(tags, vec!["#work"]);
    }

    #[test]
    fn test_extract_tag_relationships_single_header() {
        let content = "• Note #work\n  ◦ detail #task";
        let rel = extract_tag_relationships(content);
        assert_eq!(rel.header_tags, vec!["#work"]);
        assert!(rel.sibling_pairs.is_empty());
        assert!(rel.child_pairs.contains(&("#work".to_string(), "#task".to_string())));
    }

    #[test]
    fn test_extract_tag_relationships_siblings() {
        let content = "• Note #work #office";
        let rel = extract_tag_relationships(content);
        assert_eq!(rel.header_tags.len(), 2);
        assert_eq!(rel.sibling_pairs.len(), 1);
        // canonical ordering: #office < #work
        assert_eq!(rel.sibling_pairs[0], ("#office".to_string(), "#work".to_string()));
    }

    #[test]
    fn test_extract_tag_relationships_three_siblings() {
        let content = "• Note #a #b #c";
        let rel = extract_tag_relationships(content);
        // C(3,2) = 3 pairs
        assert_eq!(rel.sibling_pairs.len(), 3);
    }

    #[test]
    fn test_extract_tag_relationships_no_child_without_header() {
        // Sub-bullet tag without any header tag → no child pairs
        let content = "• Plain note\n  ◦ sub with #task";
        let rel = extract_tag_relationships(content);
        assert!(rel.child_pairs.is_empty());
    }

    #[test]
    fn test_extract_tag_relationships_deduplicates_children() {
        let content = "• Note #work\n  ◦ #task detail\n  ◦ #task again";
        let rel = extract_tag_relationships(content);
        let task_pairs: Vec<_> = rel.child_pairs.iter()
            .filter(|(_, c)| c == "#task")
            .collect();
        assert_eq!(task_pairs.len(), 1);
    }

    #[test]
    fn test_extract_tag_relationships_sibling_canonical_order() {
        let content = "• Note #zebra #apple";
        let rel = extract_tag_relationships(content);
        assert_eq!(rel.sibling_pairs[0].0, "#apple");
        assert_eq!(rel.sibling_pairs[0].1, "#zebra");
    }

    #[test]
    fn test_extract_tag_relationships_no_indent_no_children() {
        // Second line with indent=0 (no indent) → not a child
        let content = "• Line1 #work\n• Line2 #task";
        let rel = extract_tag_relationships(content);
        assert!(rel.child_pairs.is_empty());
    }

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein("work", "work"), 0);
    }

    #[test]
    fn test_levenshtein_one_edit() {
        assert_eq!(levenshtein("work", "word"), 1);
        assert_eq!(levenshtein("work", "wor"), 1);
        assert_eq!(levenshtein("work", "works"), 1);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn test_find_similar_tags_close_match() {
        let tags = vec!["#work".to_string(), "#shopping".to_string()];
        let similar = find_similar_tags("worx", &tags);
        assert!(similar.contains(&"#work".to_string()));
    }

    #[test]
    fn test_find_similar_tags_no_match() {
        let tags = vec!["#work".to_string(), "#shopping".to_string()];
        let similar = find_similar_tags("xyz", &tags);
        assert!(similar.is_empty());
    }

    #[test]
    fn test_find_similar_tags_excludes_exact() {
        let tags = vec!["#work".to_string(), "#wok".to_string()];
        let similar = find_similar_tags("work", &tags);
        assert!(!similar.contains(&"#work".to_string()));
    }

    #[test]
    fn test_collect_all_tags_deduplicates() {
        let notes = vec![
            (1, "• Task #work #urgent".to_string(), "ts".to_string()),
            (2, "• Meeting #work #standup".to_string(), "ts".to_string()),
        ];
        let tags = collect_all_tags(&notes);
        assert_eq!(tags.iter().filter(|t| t.as_str() == "#work").count(), 1);
        assert!(tags.contains(&"#urgent".to_string()));
        assert!(tags.contains(&"#standup".to_string()));
    }

    // ── load_tag_family tests ────────────────────────────────────────────────

    fn setup_tag_db() -> Result<(rusqlite::Connection, tempfile::NamedTempFile)> {
        use tempfile::NamedTempFile;
        use crate::db::{init_db, add_note_to_db};
        let f = NamedTempFile::new()?;
        let conn = rusqlite::Connection::open(f.path())?;
        init_db(&conn)?;
        let ts = "2026-03-09T00:00:00+00:00";
        // #work header with #project as child; #office co-appears with #work
        add_note_to_db(&conn, "• Sprint #work #office\n  ◦ discuss #project", ts)?;
        // another note with #work and #project as sibling (not child)
        add_note_to_db(&conn, "• Meeting #project #planning", ts)?;
        Ok((conn, f))
    }

    #[test]
    fn test_load_tag_family_no_match() -> Result<()> {
        let (conn, _f) = setup_tag_db()?;
        let result = load_tag_family(&conn, "#zzz")?;
        assert!(result.is_none());
        Ok(())
    }

    #[test]
    fn test_load_tag_family_exact_header_match() -> Result<()> {
        let (conn, _f) = setup_tag_db()?;
        let family = load_tag_family(&conn, "#work")?.unwrap();
        assert_eq!(family.tag, "#work");
        // #work has #project as child
        assert!(family.children.contains(&"#project".to_string()));
        Ok(())
    }

    #[test]
    fn test_load_tag_family_partial_match() -> Result<()> {
        let (conn, _f) = setup_tag_db()?;
        // "#wor" should resolve to "#work"
        let family = load_tag_family(&conn, "#wor")?.unwrap();
        assert_eq!(family.tag, "#work");
        Ok(())
    }

    #[test]
    fn test_load_tag_family_has_parents() -> Result<()> {
        let (conn, _f) = setup_tag_db()?;
        // #project appears as child of #work, so #work should be a parent
        let family = load_tag_family(&conn, "#project")?.unwrap();
        let parent_names: Vec<&str> = family.parents.iter().map(|(p, _, _)| p.as_str()).collect();
        assert!(parent_names.contains(&"#work"));
        Ok(())
    }

    #[test]
    fn test_load_tag_family_sibling_classification() -> Result<()> {
        use crate::db::{init_db, add_note_to_db};
        use tempfile::NamedTempFile;
        let f = NamedTempFile::new()?;
        let conn = rusqlite::Connection::open(f.path())?;
        init_db(&conn)?;
        let ts = "2026-03-09T00:00:00+00:00";
        // #work is parent of both #project and #office
        add_note_to_db(&conn, "• Note #work\n  ◦ sub #project", ts)?;
        add_note_to_db(&conn, "• Note #work\n  ◦ sub #office", ts)?;
        // #project and #office co-appear → they are siblings
        add_note_to_db(&conn, "• Meeting #project #office", ts)?;
        // #health co-appears with #project but is NOT under #work
        add_note_to_db(&conn, "• Personal #project #health", ts)?;

        let family = load_tag_family(&conn, "#project")?.unwrap();
        let work_parent = family.parents.iter().find(|(p, _, _)| p == "#work");
        assert!(work_parent.is_some(), "#work should be a parent of #project");
        let (_, tree_sibs, orphan_sibs) = work_parent.unwrap();
        // #office IS also a child of #work → tree sibling (shown as peer branch)
        assert!(tree_sibs.contains(&"#office".to_string()), "#office should be tree sibling");
        // #health is NOT a child of #work → orphan sibling (shown inline with ·)
        assert!(orphan_sibs.contains(&"#health".to_string()), "#health should be orphan sibling");
        Ok(())
    }

    #[test]
    fn test_load_tag_family_no_parents_uses_all_siblings() -> Result<()> {
        use crate::db::{init_db, add_note_to_db};
        use tempfile::NamedTempFile;
        let f = NamedTempFile::new()?;
        let conn = rusqlite::Connection::open(f.path())?;
        init_db(&conn)?;
        let ts = "2026-03-09T00:00:00+00:00";
        // #project and #standup as siblings, no parent relationship
        add_note_to_db(&conn, "• Meeting #project #standup", ts)?;

        let family = load_tag_family(&conn, "#project")?.unwrap();
        assert!(family.parents.is_empty());
        assert!(family.all_siblings.contains(&"#standup".to_string()));
        Ok(())
    }

    #[test]
    fn test_build_tag_line_no_parent_no_siblings() {
        let family = TagFamily {
            tag: "#work".to_string(),
            parents: vec![],
            all_siblings: vec![],
            children: vec![],
        };
        let line = build_tag_line_no_parent(&family);
        assert!(line.contains("#work"));
        assert!(line.contains("◄"));
    }

    #[test]
    fn test_build_tag_line_no_parent_with_siblings() {
        let family = TagFamily {
            tag: "#work".to_string(),
            parents: vec![],
            all_siblings: vec!["#office".to_string(), "#home".to_string()],
            children: vec![],
        };
        let line = build_tag_line_no_parent(&family);
        assert!(line.contains("#work"));
        assert!(line.contains("#office"));
        assert!(line.contains("#home"));
        assert!(line.contains("◄"));
    }

    #[test]
    fn test_load_tag_family_child_tag_resolution() -> Result<()> {
        use crate::db::{init_db, add_note_to_db};
        use tempfile::NamedTempFile;
        let f = NamedTempFile::new()?;
        let conn = rusqlite::Connection::open(f.path())?;
        init_db(&conn)?;
        let ts = "2026-03-09T00:00:00+00:00";
        // #task is a child but not a header tag
        add_note_to_db(&conn, "• Work #work\n  ◦ fix #task", ts)?;

        // Should resolve via child_tag lookup
        let family = load_tag_family(&conn, "#task")?.unwrap();
        assert_eq!(family.tag, "#task");
        Ok(())
    }
}
