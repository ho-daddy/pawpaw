//! Context Trigger — 5-layer memory context loading based on user message keywords.
//!
//! Phase 1: Keyword matching from frontmatter `triggers` fields (cached index, partial file read).
//! Phase 2: Semantic search via local-agent — runs in background thread, results cached for next turn.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use crate::utils::format::safe_prefix;

/// Maximum characters to include from a single triggered file.
const TRIGGER_FILE_MAX_CHARS: usize = 3_000;

/// Maximum number of files to include per request.
const TRIGGER_MAX_FILES: usize = 3;

/// Cache TTL for the trigger index (seconds).
const INDEX_CACHE_TTL_SECS: u64 = 60;

/// A single trigger entry: keywords mapped to a file path.
#[derive(Debug, Clone)]
struct TriggerEntry {
    keywords: Vec<String>,
    file_path: PathBuf,
    layer: &'static str,
    priority: u8, // lower = higher priority
}

/// Cached trigger index with timestamp.
struct IndexCache {
    entries: Vec<TriggerEntry>,
    built_at: Instant,
    agent_root: PathBuf,
}

static INDEX_CACHE: Mutex<Option<IndexCache>> = Mutex::new(None);

/// Cached semantic search result from a background query.
struct SemanticCache {
    /// The user message that triggered this search.
    query: String,
    /// Results: (relative_file_path, category).
    results: Vec<(String, String)>,
    cached_at: Instant,
}

/// TTL for semantic cache (seconds). Results stay valid for one conversation turn cycle.
const SEMANTIC_CACHE_TTL_SECS: u64 = 300;

static SEMANTIC_CACHE: Mutex<Option<SemanticCache>> = Mutex::new(None);

/// Check if two messages are similar enough to reuse cached semantic results.
/// Uses simple word-overlap heuristic: if ≥50% of words in the query overlap, reuse.
fn messages_similar(cached_query: &str, new_query: &str) -> bool {
    let cached_words: std::collections::HashSet<&str> = cached_query.split_whitespace().collect();
    let new_words: Vec<&str> = new_query.split_whitespace().collect();
    if new_words.is_empty() || cached_words.is_empty() {
        return false;
    }
    let overlap = new_words.iter().filter(|w| cached_words.contains(*w)).count();
    // At least 50% of the new query's words must appear in the cached query
    overlap * 2 >= new_words.len()
}

/// Get the trigger index, using cache if still fresh.
fn get_trigger_index(agent_root: &Path) -> Vec<TriggerEntry> {
    let mut cache = INDEX_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref c) = *cache {
        if c.agent_root == agent_root && c.built_at.elapsed().as_secs() < INDEX_CACHE_TTL_SECS {
            return c.entries.clone();
        }
    }
    let entries = build_trigger_index(agent_root);
    *cache = Some(IndexCache {
        entries: entries.clone(),
        built_at: Instant::now(),
        agent_root: agent_root.to_path_buf(),
    });
    entries
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
/// Only reads the frontmatter portion of the file using BufReader, not the entire file.
/// Returns lowercased keywords.
fn parse_frontmatter_triggers(path: &Path) -> Vec<String> {
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    // First line must be "---"
    match lines.next() {
        Some(Ok(line)) if line.trim() == "---" => {}
        _ => return Vec::new(),
    }

    // Read frontmatter lines until closing "---"
    for line in lines {
        let line = match line {
            Ok(l) => l,
            Err(_) => return Vec::new(),
        };
        let trimmed = line.trim();
        if trimmed == "---" {
            // End of frontmatter, no triggers found
            return Vec::new();
        }
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
/// If keywords match: returns immediately with matched content.
/// If no keyword match: checks cached semantic results from a previous background search,
/// then spawns a new background semantic search for the next turn.
/// Returns a formatted string to be injected into the system prompt, or empty string if no matches.
pub fn detect_and_format_context(agent_root: &Path, user_message: &str) -> String {
    let index = get_trigger_index(agent_root);

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
        // Phase 2: check semantic cache from previous background search
        let cached_hit = {
            let cache = SEMANTIC_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref c) = *cache {
                if c.cached_at.elapsed().as_secs() < SEMANTIC_CACHE_TTL_SECS
                    && messages_similar(&c.query, &lower_message)
                {
                    Some(c.results.clone())
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(results) = cached_hit {
            // Use cached semantic results from previous turn's background search
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

        // Always fire background semantic search for the next turn
        // (regardless of cache hit — refreshes results for the current query)
        let bg_root = agent_root.to_path_buf();
        let bg_message = user_message.to_string();
        let bg_lower = lower_message.clone();
        std::thread::spawn(move || {
            if let Some(results) = trigger_search_fallback(&bg_root, &bg_message) {
                let mut cache = SEMANTIC_CACHE.lock().unwrap_or_else(|e| e.into_inner());
                *cache = Some(SemanticCache {
                    query: bg_lower,
                    results,
                    cached_at: Instant::now(),
                });
            }
        });
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
        format!("{}...\n[truncated]", safe_prefix(&content, TRIGGER_FILE_MAX_CHARS))
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

    // Wait with timeout — runs in background thread so generous timeout is fine
    let timeout = std::time::Duration::from_secs(15);
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
