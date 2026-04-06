//! Agent system — persistent identity, memory, and daily journaling for the Claude CLI agent.
//!
//! All agent files live under `~/.cokacdir/agent/`:
//!   SOUL.md, IDENTITY.md, USER.md, MEMORY.md, AGENT.md, HEARTBEAT.md
//!   workspace/   — free working directory for the agent
//!   daily/       — daily memo files (daily_memo_YYYY_MM_DD.md)
//!   LAST_SESSION.txt — last Claude session ID for resume

use std::fs;
use std::path::{Path, PathBuf};

/// Maximum MEMORY.md size (in bytes) before triggering summarization.
const MEMORY_MAX_BYTES: usize = 50_000;

/// Return the agent root directory: `~/.cokacdir/agent/`
pub fn agent_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cokacdir").join("agent"))
}

/// Return the workspace directory: `~/.cokacdir/agent/workspace/`
pub fn workspace_dir() -> Option<PathBuf> {
    agent_dir().map(|d| d.join("workspace"))
}

/// Return the daily directory: `~/.cokacdir/agent/daily/`
pub fn daily_dir() -> Option<PathBuf> {
    agent_dir().map(|d| d.join("daily"))
}

/// Check whether agent mode is initialized (the agent directory and AGENT.md exist).
pub fn is_agent_initialized() -> bool {
    agent_dir()
        .map(|d| d.join("AGENT.md").exists())
        .unwrap_or(false)
}

/// Initialize the agent file system with default files.
/// Returns Ok(agent_dir_path) on success.
pub fn init_agent() -> Result<PathBuf, String> {
    let root = agent_dir().ok_or("Cannot determine home directory")?;
    let workspace = root.join("workspace");
    let daily = root.join("daily");

    fs::create_dir_all(&workspace).map_err(|e| format!("Failed to create workspace dir: {}", e))?;
    fs::create_dir_all(&daily).map_err(|e| format!("Failed to create daily dir: {}", e))?;

    // Write each default file only if it does not already exist
    write_if_absent(&root.join("SOUL.md"), default_soul())?;
    write_if_absent(&root.join("IDENTITY.md"), default_identity())?;
    write_if_absent(&root.join("USER.md"), default_user())?;
    write_if_absent(&root.join("MEMORY.md"), default_memory())?;
    write_if_absent(&root.join("AGENT.md"), default_agent())?;
    write_if_absent(&root.join("HEARTBEAT.md"), default_heartbeat())?;

    Ok(root)
}

// ---------------------------------------------------------------------------
// System prompt assembly
// ---------------------------------------------------------------------------

/// Maximum characters to include from MEMORY.md in the system prompt.
/// Older entries beyond this limit are truncated to save tokens.
const MEMORY_PROMPT_MAX_CHARS: usize = 8_000;

/// Truncate text to max_chars, cutting from the BEGINNING (keeps most recent content).
fn truncate_keep_tail(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let skip = text.chars().count() - max_chars;
    let truncated: String = text.chars().skip(skip).collect();
    format!("[...truncated {} chars...]\n{}", skip, truncated)
}

/// Build the complete agent system prompt by reading all md files and merging
/// them into a single string suitable for `--append-system-prompt-file`.
///
/// Token optimization:
/// - MEMORY is truncated (keeps tail = most recent 8K chars)
/// - Daily memo: only yesterday's is included (for context on resuming next day)
/// - Today's memo is write-only — path is provided, content is not loaded
/// - Session resume (`--resume`) already carries conversation context
pub fn build_agent_system_prompt() -> Option<String> {
    let root = agent_dir()?;
    if !root.join("AGENT.md").exists() {
        return None;
    }

    let read = |name: &str| -> String {
        let path = root.join(name);
        fs::read_to_string(&path).unwrap_or_default()
    };

    let soul = read("SOUL.md");
    let identity = read("IDENTITY.md");
    let user = read("USER.md");
    let memory_raw = read("MEMORY.md");
    let agent = read("AGENT.md");
    let heartbeat = read("HEARTBEAT.md");

    // Truncate memory to save tokens (keep tail = most recent entries)
    let memory = truncate_keep_tail(&memory_raw, MEMORY_PROMPT_MAX_CHARS);

    // Today's daily memo path (write-only — not loaded into prompt)
    let today = chrono::Local::now();
    let today_str = today.format("%Y_%m_%d").to_string();
    let daily_path = root.join("daily").join(format!("daily_memo_{}.md", today_str));

    // Yesterday's daily memo (read-only — for context when resuming next day)
    let yesterday = today - chrono::Duration::days(1);
    let yesterday_str = yesterday.format("%Y_%m_%d").to_string();
    let yesterday_path = root.join("daily").join(format!("daily_memo_{}.md", yesterday_str));
    let yesterday_memo = if yesterday_path.exists() {
        let raw = fs::read_to_string(&yesterday_path).unwrap_or_default();
        truncate_keep_tail(&raw, 2_000)
    } else {
        String::new()
    };

    // Last session ID
    let last_session = read_last_session();

    let workspace = workspace_dir().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
    let agent_root = root.to_string_lossy().to_string();
    let daily_dir_str = root.join("daily").to_string_lossy().to_string();

    // ---------------------------------------------------------------
    // Prompt layout optimized for Claude prompt caching.
    // Cache key = longest matching PREFIX across requests.
    // Order: STATIC first (rarely change) → DYNAMIC last (change often).
    //
    //  [CACHEABLE — stable across requests within a day/session]
    //   1. SYSTEM RULES        (never changes)
    //   2. AGENT GUIDELINES     (edited rarely)
    //   3. SOUL                 (edited rarely)
    //   4. IDENTITY             (edited rarely)
    //   5. USER                 (updated occasionally)
    //   6. HEARTBEAT            (updated occasionally)
    //
    //  [DYNAMIC — changes per-request or per-day, invalidates cache tail]
    //   7. MEMORY               (grows each session)
    //   8. YESTERDAY'S MEMO     (changes daily)
    //   9. Paths & date         (changes daily)
    // ---------------------------------------------------------------
    let prompt = format!(
r#"# AGENT MODE ACTIVE

You are a persistent personal assistant agent with identity, memory, and autonomy.

## SYSTEM RULES
- All bash commands MUST be non-interactive (use -y, --no-pager, -m flags; never open editors)
- Respond in the same language as the user
- Use Markdown formatting

## AGENT GUIDELINES
{agent}

## SOUL
{soul}

## IDENTITY
{identity}

## USER
{user}

## HEARTBEAT
{heartbeat}

## MEMORY (use Read tool for full file if truncated)
{memory}

## YESTERDAY'S MEMO
{yesterday_section}

## Paths
- Agent root: {agent_root}
- Workspace: {workspace}
- Daily memos: {daily_dir_str}
- Today's daily memo (write here): {daily_path}
- Today: {today_display}
{session_section}
"#,
        agent = agent,
        soul = if soul.trim().is_empty() { "(not yet defined)".to_string() } else { soul },
        identity = if identity.trim().is_empty() { "(not yet defined)".to_string() } else { identity },
        user = if user.trim().is_empty() { "(no info yet)".to_string() } else { user },
        heartbeat = if heartbeat.trim().is_empty() { "(none)".to_string() } else { heartbeat },
        memory = if memory.trim().is_empty() { "(empty)".to_string() } else { memory },
        yesterday_section = if yesterday_memo.trim().is_empty() {
            "(no memo from yesterday)".to_string()
        } else {
            yesterday_memo
        },
        agent_root = agent_root,
        workspace = workspace,
        daily_dir_str = daily_dir_str,
        daily_path = daily_path.to_string_lossy(),
        today_display = today.format("%Y-%m-%d (%A)"),
        session_section = match &last_session {
            Some(sid) => format!("- Last session: {}", sid),
            None => String::new(),
        },
    );

    Some(prompt)
}

// ---------------------------------------------------------------------------
// Session persistence
// ---------------------------------------------------------------------------

/// Save the session ID so the next launch can resume.
pub fn save_last_session(session_id: &str) {
    if let Some(root) = agent_dir() {
        let path = root.join("LAST_SESSION.txt");
        let _ = fs::write(path, session_id);
    }
}

/// Read the last saved session ID, if any.
/// Returns None if the file is missing, empty, or older than 7 days.
pub fn read_last_session() -> Option<String> {
    let root = agent_dir()?;
    let path = root.join("LAST_SESSION.txt");

    // Check TTL: invalidate sessions older than 7 days
    if let Ok(meta) = path.metadata() {
        if let Ok(modified) = meta.modified() {
            let age = modified.elapsed().unwrap_or_default();
            if age > std::time::Duration::from_secs(7 * 24 * 3600) {
                let _ = fs::remove_file(&path);
                return None;
            }
        }
    }

    fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Daily memo helpers
// ---------------------------------------------------------------------------

/// Return the path for today's daily memo.
pub fn today_daily_memo_path() -> Option<PathBuf> {
    let daily = daily_dir()?;
    let today = chrono::Local::now().format("%Y_%m_%d").to_string();
    Some(daily.join(format!("daily_memo_{}.md", today)))
}

/// Ensure today's daily memo file exists (creates an empty template if not).
pub fn ensure_daily_memo() -> Result<PathBuf, String> {
    let path = today_daily_memo_path().ok_or("Cannot determine daily memo path")?;
    if !path.exists() {
        let daily = daily_dir().ok_or("Cannot determine daily dir")?;
        fs::create_dir_all(&daily).map_err(|e| format!("Failed to create daily dir: {}", e))?;
        let today = chrono::Local::now().format("%Y-%m-%d (%A)").to_string();
        let template = format!(
            "# Daily Memo — {}\n\n## Work Log\n\n## Key Conversations\n\n## Notes\n",
            today
        );
        fs::write(&path, template).map_err(|e| format!("Failed to create daily memo: {}", e))?;
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Memory management
// ---------------------------------------------------------------------------

/// Check if MEMORY.md exceeds the size threshold.
pub fn memory_needs_summarization() -> bool {
    agent_dir()
        .map(|root| {
            let path = root.join("MEMORY.md");
            path.metadata().map(|m| m.len() as usize > MEMORY_MAX_BYTES).unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Return the current MEMORY.md size in bytes.
pub fn memory_size() -> usize {
    agent_dir()
        .and_then(|root| {
            let path = root.join("MEMORY.md");
            path.metadata().ok().map(|m| m.len() as usize)
        })
        .unwrap_or(0)
}

/// Build a prompt asking the agent to summarize its own memory.
/// The caller should send this to Claude CLI and write the response back to MEMORY.md.
pub fn build_memory_summarization_prompt() -> Option<String> {
    let root = agent_dir()?;
    let memory_path = root.join("MEMORY.md");
    let memory = fs::read_to_string(&memory_path).ok()?;
    if memory.trim().is_empty() {
        return None;
    }

    Some(format!(
r#"You are performing maintenance on your own long-term memory file.
The MEMORY.md file has grown too large ({} bytes, threshold: {} bytes).

Your task:
1. Read the current memory content below.
2. Summarize it — keep ALL important facts, decisions, user preferences, and key learnings.
3. Remove redundant, outdated, or trivial entries.
4. Output ONLY the new summarized MEMORY.md content (no explanations, no extra text).
5. Maintain the same markdown format.

Current MEMORY.md:
---
{}
---

Output the summarized MEMORY.md content now:"#,
        memory.len(),
        MEMORY_MAX_BYTES,
        memory,
    ))
}

// ---------------------------------------------------------------------------
// Heartbeat
// ---------------------------------------------------------------------------

/// Read and parse HEARTBEAT.md to extract task definitions.
/// Returns a list of (task_description, cron_expression) tuples.
pub fn read_heartbeat_tasks() -> Vec<(String, String)> {
    let root = match agent_dir() {
        Some(r) => r,
        None => return Vec::new(),
    };
    let path = root.join("HEARTBEAT.md");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut tasks = Vec::new();
    let mut current_task: Option<String> = None;
    let mut current_cron: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        // Parse format: `- [cron: */30 * * * *] Task description`
        if let Some(rest) = trimmed.strip_prefix("- [cron:") {
            if let Some(bracket_end) = rest.find(']') {
                let cron = rest[..bracket_end].trim().to_string();
                let desc = rest[bracket_end + 1..].trim().to_string();
                if !cron.is_empty() && !desc.is_empty() {
                    current_cron = Some(cron);
                    current_task = Some(desc);
                }
            }
        }

        if let (Some(task), Some(cron)) = (current_task.take(), current_cron.take()) {
            tasks.push((task, cron));
        }
    }

    tasks
}

/// Return heartbeat tasks whose cron expression matches the given time.
pub fn due_heartbeat_tasks(dt: chrono::DateTime<chrono::Local>) -> Vec<String> {
    read_heartbeat_tasks()
        .into_iter()
        .filter(|(_desc, cron)| super::telegram::cron_matches(cron, dt))
        .map(|(desc, _cron)| desc)
        .collect()
}

/// File that records the last heartbeat check time (ISO 8601).
/// Used to prevent duplicate execution within the same minute.
fn heartbeat_last_check_path() -> Option<PathBuf> {
    agent_dir().map(|d| d.join(".heartbeat_last_check"))
}

/// Check whether a heartbeat tick is due (at most once per minute).
/// Returns the list of task descriptions whose cron matches the current time,
/// or an empty vec if already checked this minute or no tasks are due.
pub fn tick_heartbeat() -> Vec<String> {
    let now = chrono::Local::now();
    let now_minute = now.format("%Y-%m-%d %H:%M").to_string();

    // Dedup: only check once per minute
    if let Some(path) = heartbeat_last_check_path() {
        if let Ok(last) = fs::read_to_string(&path) {
            if last.trim() == now_minute {
                return Vec::new();
            }
        }
        let _ = fs::write(&path, &now_minute);
    }

    due_heartbeat_tasks(now)
}

// ---------------------------------------------------------------------------
// Default file contents
// ---------------------------------------------------------------------------

fn default_soul() -> &'static str {
r#"# Soul

Define the agent's core personality and values here.

## Personality Traits
- (e.g., Thoughtful, proactive, honest)

## Communication Style
- (e.g., Warm but concise, uses clear language)

## Values
- (e.g., Respects user's time, maintains transparency)
"#
}

fn default_identity() -> &'static str {
r#"# Identity

## Name
(Give your agent a name)

## Role
Personal AI assistant with persistent memory and autonomous capabilities.

## Capabilities
- File management and system operations
- Code writing, review, and debugging
- Research and information gathering
- Task scheduling and automation
- Long-term context retention across sessions
"#
}

fn default_user() -> &'static str {
r#"# User Profile

Record information about the user here as you learn it.

## Basic Info
- Name:
- Role:

## Preferences
- Language:
- Communication style:

## Work Context
- Primary projects:
- Tech stack:

## Notes
"#
}

fn default_memory() -> &'static str {
r#"# Long-Term Memory

Record important facts, decisions, and learnings here.
This file is read at every session start to maintain continuity.

## Key Facts

## Decisions Made

## Learnings
"#
}

fn default_agent() -> &'static str {
r#"# Agent Behavioral Guidelines

## Core Files (read at every session start)
- **SOUL.md**: Your personality, values, and tone. Always embody them.
- **IDENTITY.md**: Your name, role, and capabilities.
- **USER.md**: Everything you know about the user. Update when you learn new info.
- **MEMORY.md**: Long-term memory. Append important facts, decisions, and learnings.
- **HEARTBEAT.md**: Periodic tasks to execute automatically.
- **AGENT.md**: This file — your behavioral guidelines.

## Rules
1. **Memory**: During conversations, record important information to MEMORY.md — facts, preferences, decisions, anything that should persist across sessions.
2. **Daily Memo**: At the start of each work day, create (or append to) the daily memo file (`daily/daily_memo_YYYY_MM_DD.md`). Log:
   - Tasks performed
   - Key conversations and decisions
   - Items to follow up on
3. **User Profile**: Proactively update USER.md when you discover new information about the user — their name, preferences, projects, working style.
4. **Workspace**: Use the `workspace/` directory freely for drafts, temp files, scripts, and any working materials.
5. **Continuity**: Always reference past memories and daily memos when relevant. You are a persistent entity — act like one.
6. **Heartbeat**: Execute HEARTBEAT tasks when their schedule conditions are met.
7. **Autonomy**: Act decisively. You have full local system access. Perform routine operations without asking for permission.
8. **Transparency**: For significant or irreversible actions, briefly explain what you're doing and why.

## File Format for HEARTBEAT.md
Define periodic tasks using this format:
```
- [cron: */30 * * * *] Check for new files in workspace and summarize
- [cron: 0 9 * * *] Write morning daily memo and review yesterday's notes
```
"#
}

fn default_heartbeat() -> &'static str {
r#"# Heartbeat — Periodic Tasks

Define tasks that should run on a schedule.
Format: `- [cron: <cron_expression>] <task description>`

## Examples
```
- [cron: 0 9 * * *] Create today's daily memo and review yesterday's work
- [cron: 0 18 * * *] Summarize today's work in the daily memo
- [cron: 0 0 * * 0] Weekly review — summarize the week and update MEMORY.md
```

## Active Tasks
(Add your tasks below)
"#
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

fn write_if_absent(path: &Path, content: &str) -> Result<(), String> {
    if !path.exists() {
        fs::write(path, content).map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    }
    Ok(())
}
