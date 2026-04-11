//! Context Trigger — 5-layer memory context loading based on user message keywords.
//!
//! Phase 1: Keyword matching from frontmatter `triggers` fields.
//! Phase 2 (future): Semantic search via gemma4 + bge-m3.

use std::fs;
use std::path::{Path, PathBuf};

/// Maximum characters to include from a single triggered file.
const TRIGGER_FILE_MAX_CHARS: usize = 3_000;

/// Maximum number of files to include per request.
const TRIGGER_MAX_FILES: usize = 3;

/// A single trigger entry: keywords mapped to a file path.
#[derive(Debug)]
struct TriggerEntry {
    keywords: Vec<String>,
    file_path: PathBuf,
    layer: &'static str,
    priority: u8, // lower = higher priority
}

/// Build the keyword trigger index by scanning projects/*/wiki.md and knowledge/wiki/*.md
/// for `triggers:` fields in YAML frontmatter.
fn build_trigger_index(agent_root: &Path) -> Vec<TriggerEntry> {
    let mut entries = Vec::new();

    // Scan projects/*/wiki.md
    let projects_dir = agent_root.join("projects");
    if let Ok(dirs) = fs::read_dir(&projects_dir) {
        for entry in dirs.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('_') || name == "." {
                continue;
            }
            // Follow symlinks: check wiki.md in the resolved path
            let wiki = path.join("wiki.md");
            if wiki.exists() {
                let triggers = parse_frontmatter_triggers(&wiki);
                if !triggers.is_empty() {
                    entries.push(TriggerEntry {
                        keywords: triggers,
                        file_path: wiki,
                        layer: "PROJECT",
                        priority: 1,
                    });
                }
            }
        }
    }

    // Scan knowledge/wiki/*.md
    let knowledge_dir = agent_root.join("knowledge").join("wiki");
    if let Ok(files) = fs::read_dir(&knowledge_dir) {
        for entry in files.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false) {
                let triggers = parse_frontmatter_triggers(&path);
                if !triggers.is_empty() {
                    entries.push(TriggerEntry {
                        keywords: triggers,
                        file_path: path,
                        layer: "KNOWLEDGE",
                        priority: 2,
                    });
                }
            }
        }
    }

    entries
}

/// Parse `triggers: [kw1, kw2, ...]` from YAML frontmatter.
/// Returns lowercased keywords.
fn parse_frontmatter_triggers(path: &Path) -> Vec<String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Check for frontmatter delimiters
    if !content.starts_with("---") {
        return Vec::new();
    }

    // Find the closing ---
    let rest = &content[3..];
    let end = match rest.find("\n---") {
        Some(pos) => pos,
        None => return Vec::new(),
    };

    let frontmatter = &rest[..end];

    // Find triggers line
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("triggers:") {
            let value = value.trim();
            // Parse [kw1, kw2, kw3] format
            let value = value.trim_start_matches('[').trim_end_matches(']');
            return value
                .split(',')
                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }

    Vec::new()
}

/// Detect which files should be loaded based on keyword matching against the user message.
/// Falls back to semantic search (Phase 2) if no keyword matches found.
/// Returns a formatted string to be injected into the system prompt, or empty string if no matches.
pub fn detect_and_format_context(agent_root: &Path, user_message: &str) -> String {
    let index = build_trigger_index(agent_root);

    let lower_message = user_message.to_lowercase();

    // Phase 1: keyword matching
    let mut matches: Vec<&TriggerEntry> = if !index.is_empty() {
        index
            .iter()
            .filter(|entry| entry.keywords.iter().any(|kw| lower_message.contains(kw)))
            .collect()
    } else {
        Vec::new()
    };

    // Sort by priority (project first) and dedup
    matches.sort_by_key(|e| e.priority);
    matches.truncate(TRIGGER_MAX_FILES);

    let mut sections = Vec::new();

    if !matches.is_empty() {
        // Phase 1 hit — load matched files
        for entry in &matches {
            if let Some(section) = format_file_section(&entry.file_path, entry.layer) {
                sections.push(section);
            }
        }
    } else {
        // Phase 2: semantic search fallback via local-agent
        if let Some(results) = trigger_search_fallback(agent_root, user_message) {
            for (file_rel, category) in results.iter().take(TRIGGER_MAX_FILES) {
                let file_path = agent_root.join(file_rel);
                let layer = match category.as_str() {
                    "project" => "PROJECT",
                    "knowledge" => "KNOWLEDGE",
                    "episodic" => "EPISODIC",
                    _ => "CONTEXT",
                };
                if let Some(section) = format_file_section(&file_path, layer) {
                    sections.push(section);
                }
            }
        }
    }

    if sections.is_empty() {
        return String::new();
    }

    format!(
        "\n## TRIGGERED CONTEXT\nThe following context was auto-loaded based on the user's message.\n\n{}",
        sections.join("\n\n")
    )
}

/// Format a single file as a context section.
fn format_file_section(file_path: &Path, layer: &str) -> Option<String> {
    let content = fs::read_to_string(file_path).ok()?;
    let truncated = if content.len() > TRIGGER_FILE_MAX_CHARS {
        format!("{}...\n[truncated]", &content[..TRIGGER_FILE_MAX_CHARS])
    } else {
        content
    };
    let filename = file_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    Some(format!("### {} — {}\n{}", layer, filename, truncated))
}

/// Phase 2 fallback: call local-agent trigger-search.
/// Returns Vec<(relative_file_path, category)> or None on failure.
/// Uses a 5-second timeout to prevent blocking the async message handler.
fn trigger_search_fallback(agent_root: &Path, user_message: &str) -> Option<Vec<(String, String)>> {
    let local_agent_dir = dirs::home_dir()?.join(".cokacdir").join("local-agent");
    if !local_agent_dir.join("agent.py").exists() {
        return None;
    }

    // Truncate message to avoid command-line length issues
    let msg: String = user_message.chars().take(200).collect();

    let mut child = std::process::Command::new("python3")
        .args(&["agent.py", "trigger-search", &msg])
        .current_dir(&local_agent_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    // Wait with timeout to prevent indefinite blocking
    let timeout = std::time::Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                break;
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }

    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).ok()?;

    if parsed.get("status")?.as_str()? != "ok" {
        return None;
    }

    let files = parsed.get("files")?.as_array()?;
    let mut results = Vec::new();
    for f in files {
        let file = f.get("file")?.as_str()?.to_string();
        let category = f.get("category").and_then(|c| c.as_str()).unwrap_or("").to_string();
        results.push((file, category));
    }

    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_frontmatter_triggers() {
        let dir = std::env::temp_dir().join("test_triggers");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("test.md");
        fs::write(
            &file,
            "---\nname: Test\ntriggers: [hello, world, 테스트]\nstatus: active\n---\n\n# Content",
        )
        .unwrap();

        let triggers = parse_frontmatter_triggers(&file);
        assert_eq!(triggers, vec!["hello", "world", "테스트"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let dir = std::env::temp_dir().join("test_triggers2");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("test2.md");
        fs::write(&file, "# Just a heading\nNo frontmatter here.").unwrap();

        let triggers = parse_frontmatter_triggers(&file);
        assert!(triggers.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }
}
