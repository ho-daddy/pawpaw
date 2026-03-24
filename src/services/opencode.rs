//! OpenCode service — spawns `opencode run --format json` and translates its
//! JSONL event stream into the existing `StreamMessage` / `ClaudeResponse` types.
//!
//! The public API mirrors `claude.rs` / `gemini.rs` so callers can swap backends
//! with minimal code changes.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use serde_json::Value;

use crate::services::claude::{
    ClaudeResponse, StreamMessage, CancelToken,
    debug_log_to, kill_child_tree,
};

fn opencode_debug(msg: &str) {
    debug_log_to("opencode.log", msg);
}

/// Truncate a string for log previews (char-boundary safe).
fn log_preview(s: &str, max: usize) -> &str {
    if s.len() <= max { return s; }
    // Find the last char boundary at or before max
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

// ============================================================
// OpenCode availability check
// ============================================================

static OPENCODE_AVAILABLE: OnceLock<bool> = OnceLock::new();

fn check_opencode_available() -> bool {
    opencode_debug("[check_opencode_available] START");

    if let Ok(val) = std::env::var("COKAC_OPENCODE_PATH") {
        if !val.is_empty() {
            opencode_debug(&format!("[check_opencode_available] found via COKAC_OPENCODE_PATH={}", val));
            return true;
        }
    }

    #[cfg(unix)]
    {
        if let Ok(output) = Command::new("which").arg("opencode").output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !p.is_empty() {
                    opencode_debug(&format!("[check_opencode_available] found via which: {}", p));
                    return true;
                }
            }
        }
        if let Ok(output) = Command::new("bash").args(["-lc", "which opencode"]).output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !p.is_empty() {
                    opencode_debug(&format!("[check_opencode_available] found via bash -lc which: {}", p));
                    return true;
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where").arg("opencode").output() {
            if output.status.success() {
                opencode_debug("[check_opencode_available] found via where");
                return true;
            }
        }
    }

    opencode_debug("[check_opencode_available] NOT FOUND");
    false
}

pub fn is_opencode_available() -> bool {
    let result = *OPENCODE_AVAILABLE.get_or_init(check_opencode_available);
    opencode_debug(&format!("[is_opencode_available] result={}", result));
    result
}

/// Check if a model string refers to the OpenCode backend
pub fn is_opencode_model(model: Option<&str>) -> bool {
    let result = model.map(|m| m == "opencode" || m.starts_with("opencode:")).unwrap_or(false);
    opencode_debug(&format!("[is_opencode_model] model={:?} result={}", model, result));
    result
}

/// Strip "opencode:" prefix and return the actual model name.
/// Returns None if the input is just "opencode" (use default).
pub fn strip_opencode_prefix(model: &str) -> Option<&str> {
    let result = model.strip_prefix("opencode:").filter(|s| !s.is_empty());
    opencode_debug(&format!("[strip_opencode_prefix] model={:?} result={:?}", model, result));
    result
}

// ============================================================
// List available models (cached)
// ============================================================

static OPENCODE_MODELS: OnceLock<Vec<String>> = OnceLock::new();

/// Fetch available models by running `opencode models`.
/// Result is cached for the process lifetime.
pub fn list_models() -> &'static [String] {
    OPENCODE_MODELS.get_or_init(|| {
        opencode_debug("[list_models] fetching model list...");
        let bin = resolve_opencode_path().unwrap_or_else(|| "opencode".to_string());
        let output = match Command::new(&bin).args(["models"]).output() {
            Ok(o) => o,
            Err(e) => {
                opencode_debug(&format!("[list_models] FAILED to run '{}': {}", bin, e));
                return Vec::new();
            }
        };
        if !output.status.success() {
            opencode_debug(&format!("[list_models] exit code {:?}", output.status.code()));
            return Vec::new();
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let models: Vec<String> = stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('{'))
            .collect();
        opencode_debug(&format!("[list_models] found {} models: {:?}", models.len(), models));
        models
    })
}

// ============================================================
// Resolve opencode binary path
// ============================================================

fn resolve_opencode_path() -> Option<String> {
    opencode_debug("[resolve_opencode_path] START");

    if let Ok(val) = std::env::var("COKAC_OPENCODE_PATH") {
        if !val.is_empty() {
            opencode_debug(&format!("[resolve_opencode_path] COKAC_OPENCODE_PATH={}", val));
            return Some(val);
        }
    }

    #[cfg(unix)]
    {
        if let Ok(output) = Command::new("which").arg("opencode").output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !p.is_empty() {
                    opencode_debug(&format!("[resolve_opencode_path] which → {}", p));
                    return Some(p);
                }
            }
        }
        if let Ok(output) = Command::new("bash").args(["-lc", "which opencode"]).output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !p.is_empty() {
                    opencode_debug(&format!("[resolve_opencode_path] bash -lc which → {}", p));
                    return Some(p);
                }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where").arg("opencode").output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).lines().next()
                    .unwrap_or("").to_string();
                if !p.is_empty() {
                    opencode_debug(&format!("[resolve_opencode_path] where → {}", p));
                    return Some(p);
                }
            }
        }
    }

    opencode_debug("[resolve_opencode_path] NOT FOUND, will use 'opencode'");
    None
}

// ============================================================
// Inject system prompt into AGENTS.md with automatic restore
// ============================================================
//
// OpenCode reads project instructions from AGENTS.md (preferred) or CLAUDE.md.
// If AGENTS.md exists, CLAUDE.md is ignored entirely.
//
// Safety requirements:
//   - The user's original AGENTS.md must NEVER be lost or corrupted.
//   - Recovery must work after SIGKILL, crash, or power loss.
//   - Concurrent execution on the same directory must not corrupt files.
//
// Strategy:
//   1. Acquire a PID-based lock file to prevent concurrent access.
//   2. Recover from any previous crash (leftover backup/sentinel).
//   3. Back up original AGENTS.md atomically (write tmp → rename).
//      If no original exists, write a sentinel file instead.
//   4. Only AFTER backup is confirmed on disk, write the modified AGENTS.md.
//   5. On Drop: restore from backup (or delete if sentinel), then release lock.
//   6. On next call: detect leftover backup/sentinel and auto-recover.

const AGENTS_MD: &str = "AGENTS.md";
const BACKUP_FILE: &str = ".AGENTS.md.cokacdir-backup";
const NO_ORIGINAL_SENTINEL: &str = ".AGENTS.md.cokacdir-no-original";
const LOCK_FILE: &str = ".AGENTS.md.cokacdir-lock";

/// Check if a process with the given PID is still alive.
/// Note: on Unix, uses `kill -0` which requires same-user ownership.
/// A process owned by a different user returns false (EPERM → exit code 1).
/// This is acceptable because cokacdir instances on the same directory
/// are always run by the same user.
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use std::process::Command;
        Command::new("kill").args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        // Conservative: assume alive to avoid stealing lock.
        let _ = pid;
        true
    }
}

/// Try to acquire a PID-based lock file. Returns false if another live
/// process holds the lock (concurrent execution on same directory).
/// Uses O_EXCL (create_new) for atomic creation to prevent TOCTOU races.
fn try_acquire_lock(dir: &std::path::Path) -> bool {
    use std::io::Write;
    let lock_path = dir.join(LOCK_FILE);
    let my_pid = std::process::id();

    // Attempt 1: atomic create (O_EXCL) — fails if file already exists
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock_path) {
        Ok(mut f) => {
            let _ = f.write_all(my_pid.to_string().as_bytes());
            opencode_debug(&format!("[lock] acquired (PID={})", my_pid));
            return true;
        }
        Err(_) => {
            // File exists — check if the holder is still alive
        }
    }

    // Read existing lock to check liveness
    let content = match std::fs::read_to_string(&lock_path) {
        Ok(c) => c,
        Err(_) => {
            opencode_debug("[lock] cannot read existing lock file, skipping");
            return false;
        }
    };
    let holder_pid = match content.trim().parse::<u32>() {
        Ok(p) => p,
        Err(_) => {
            opencode_debug("[lock] lock file has invalid content, treating as stale");
            0 // treat as dead
        }
    };

    if holder_pid != my_pid && holder_pid != 0 && is_pid_alive(holder_pid) {
        opencode_debug(&format!("[lock] another process (PID={}) holds the lock, skipping injection", holder_pid));
        return false;
    }

    // Stale lock from dead process — remove and retry with O_EXCL
    opencode_debug(&format!("[lock] stale lock from dead PID={}, taking over", holder_pid));
    let _ = std::fs::remove_file(&lock_path);

    // Attempt 2: another process might have grabbed it between remove and create
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock_path) {
        Ok(mut f) => {
            let _ = f.write_all(my_pid.to_string().as_bytes());
            opencode_debug(&format!("[lock] acquired on retry (PID={})", my_pid));
            true
        }
        Err(_) => {
            // Another process won the race — that's fine, we skip injection
            opencode_debug("[lock] lost race on retry, skipping injection");
            false
        }
    }
}

fn release_lock(dir: &std::path::Path) {
    let lock_path = dir.join(LOCK_FILE);
    let _ = std::fs::remove_file(&lock_path);
    opencode_debug("[lock] released");
}

/// Write content to a file atomically: write to a temp file in the same
/// directory, then rename. This prevents partial writes from corrupting
/// the target file.
fn atomic_write(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("cokacdir-tmp");
    if let Err(e) = std::fs::write(&tmp, content) {
        // tmp write failed — clean up partial tmp and return error
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        // rename failed — clean up tmp, do NOT attempt non-atomic fallback
        opencode_debug(&format!("[atomic_write] rename failed: {}", e));
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Recover from a previous crash: if a backup/sentinel file exists, restore.
/// Called at the start of every inject to guarantee AGENTS.md is clean.
/// Also cleans up any leftover tmp files from interrupted atomic_write.
fn recover_agents_md_if_needed(dir: &std::path::Path) {
    // Clean up leftover tmp file from interrupted atomic_write
    let tmp_path = dir.join(BACKUP_FILE).with_extension("cokacdir-tmp");
    if tmp_path.exists() {
        opencode_debug("[recover] removing leftover tmp file");
        let _ = std::fs::remove_file(&tmp_path);
    }

    let agents_path = dir.join(AGENTS_MD);
    let backup_path = dir.join(BACKUP_FILE);
    let sentinel_path = dir.join(NO_ORIGINAL_SENTINEL);

    if sentinel_path.exists() {
        // Original file did not exist → remove injected AGENTS.md and sentinel
        opencode_debug("[recover] sentinel found → removing injected AGENTS.md");
        let _ = std::fs::remove_file(&agents_path);
        if !agents_path.exists() {
            // AGENTS.md is gone — safe to remove sentinel
            let _ = std::fs::remove_file(&sentinel_path);
            opencode_debug("[recover] cleaned up sentinel");
        } else {
            // Cannot delete AGENTS.md — keep sentinel for next recovery attempt
            opencode_debug("[recover] cannot delete AGENTS.md, sentinel preserved for next recovery");
        }
        // Also clean up any stale lock
        release_lock(dir);
    } else if backup_path.exists() {
        // Original file existed → restore from backup
        opencode_debug("[recover] backup found → restoring original AGENTS.md");
        match std::fs::rename(&backup_path, &agents_path) {
            Ok(()) => opencode_debug("[recover] restored OK (rename)"),
            Err(e) => {
                opencode_debug(&format!("[recover] rename failed ({}), trying read+write", e));
                match std::fs::read_to_string(&backup_path) {
                    Ok(content) => {
                        match std::fs::write(&agents_path, &content) {
                            Ok(()) => {
                                // Only delete backup AFTER successful restore
                                let _ = std::fs::remove_file(&backup_path);
                                opencode_debug("[recover] restored OK (copy+delete)");
                            }
                            Err(e2) => {
                                // Write failed — do NOT delete backup
                                opencode_debug(&format!("[recover] CRITICAL: restore write FAILED: {}, backup preserved", e2));
                            }
                        }
                    }
                    Err(e2) => {
                        opencode_debug(&format!("[recover] CRITICAL: cannot read backup: {}", e2));
                        // Leave backup in place — don't delete what we can't read.
                    }
                }
            }
        }
        release_lock(dir);
    }
}

/// RAII guard that restores AGENTS.md to its original state when dropped.
struct AgentsMdGuard {
    dir: std::path::PathBuf,
    /// true = original file existed and was backed up to BACKUP_FILE.
    /// false = original file did not exist; sentinel was written.
    had_original: bool,
}

impl Drop for AgentsMdGuard {
    fn drop(&mut self) {
        let agents_path = self.dir.join(AGENTS_MD);
        let backup_path = self.dir.join(BACKUP_FILE);
        let sentinel_path = self.dir.join(NO_ORIGINAL_SENTINEL);

        if self.had_original {
            opencode_debug("[AgentsMdGuard] restoring original AGENTS.md from backup");
            match std::fs::rename(&backup_path, &agents_path) {
                Ok(()) => opencode_debug("[AgentsMdGuard] restored OK (rename)"),
                Err(e) => {
                    opencode_debug(&format!("[AgentsMdGuard] rename failed ({}), trying read+write", e));
                    match std::fs::read_to_string(&backup_path) {
                        Ok(content) => {
                            match std::fs::write(&agents_path, &content) {
                                Ok(()) => {
                                    opencode_debug("[AgentsMdGuard] restored OK (copy)");
                                    // Only delete backup AFTER successful restore
                                    let _ = std::fs::remove_file(&backup_path);
                                }
                                Err(e2) => {
                                    // Write failed — do NOT delete backup, it's the only copy of the original
                                    opencode_debug(&format!("[AgentsMdGuard] CRITICAL: restore write FAILED: {}, backup preserved for recovery", e2));
                                }
                            }
                        }
                        Err(e2) => {
                            // Backup unreadable — do NOT delete it. Leave it for manual recovery.
                            opencode_debug(&format!("[AgentsMdGuard] CRITICAL: backup unreadable ({}), leaving for manual recovery", e2));
                        }
                    }
                }
            }
        } else {
            opencode_debug("[AgentsMdGuard] removing injected AGENTS.md (no original)");
            let _ = std::fs::remove_file(&agents_path);
            if !agents_path.exists() {
                // AGENTS.md is gone — safe to remove sentinel
                let _ = std::fs::remove_file(&sentinel_path);
            } else {
                // Cannot delete AGENTS.md — keep sentinel for crash recovery
                opencode_debug("[AgentsMdGuard] cannot delete AGENTS.md, sentinel preserved for recovery");
            }
        }

        release_lock(&self.dir);
    }
}

/// Inject system prompt into AGENTS.md, prepended before any existing content.
/// Returns `Some(guard)` on success (guard restores on drop), or `None` if
/// injection was skipped (lock held by another process, or write failures).
fn inject_system_prompt_into_agents_md(working_dir: &str, system_prompt: &str) -> Option<AgentsMdGuard> {
    let dir = std::path::Path::new(working_dir);
    let agents_path = dir.join(AGENTS_MD);
    let backup_path = dir.join(BACKUP_FILE);
    let sentinel_path = dir.join(NO_ORIGINAL_SENTINEL);

    opencode_debug(&format!("[inject_agents_md] dir={} system_prompt_len={}", working_dir, system_prompt.len()));

    // Step 0: recover from any previous crash
    recover_agents_md_if_needed(dir);

    // Step 1: acquire lock (prevents concurrent corruption)
    if !try_acquire_lock(dir) {
        opencode_debug("[inject_agents_md] SKIPPED: could not acquire lock");
        return None;
    }

    // Step 2: back up original to disk (atomically)
    let had_original = agents_path.exists();
    if had_original {
        let original = match std::fs::read_to_string(&agents_path) {
            Ok(c) => c,
            Err(e) => {
                opencode_debug(&format!("[inject_agents_md] ABORT: cannot read original AGENTS.md: {}", e));
                release_lock(dir);
                return None;
            }
        };
        opencode_debug(&format!("[inject_agents_md] backing up original ({} bytes)", original.len()));

        // Atomic write: tmp file → rename to backup
        if let Err(e) = atomic_write(&backup_path, &original) {
            opencode_debug(&format!("[inject_agents_md] ABORT: backup write FAILED: {}", e));
            release_lock(dir);
            return None; // Do NOT touch AGENTS.md without a confirmed backup
        }
        opencode_debug("[inject_agents_md] backup confirmed on disk");

        // Step 3: NOW safe to write combined content
        let combined = format!("{}\n\n{}\n", system_prompt, original.trim());
        if let Err(e) = std::fs::write(&agents_path, &combined) {
            opencode_debug(&format!("[inject_agents_md] combined write FAILED: {}, restoring from backup", e));
            // Immediately restore — backup is confirmed intact (atomic_write succeeded).
            // If rename also fails, leave backup in place for crash recovery.
            match std::fs::rename(&backup_path, &agents_path) {
                Ok(()) => opencode_debug("[inject_agents_md] restored from backup OK"),
                Err(e2) => opencode_debug(&format!(
                    "[inject_agents_md] restore rename also FAILED: {} — backup preserved at {:?} for recovery",
                    e2, backup_path)),
            }
            release_lock(dir);
            return None;
        }
        opencode_debug(&format!("[inject_agents_md] injected OK ({} bytes)", combined.len()));
    } else {
        // No original → write sentinel FIRST (so crash recovery knows to delete)
        opencode_debug("[inject_agents_md] no original AGENTS.md");
        if let Err(e) = std::fs::write(&sentinel_path, "") {
            opencode_debug(&format!("[inject_agents_md] ABORT: sentinel write FAILED: {}", e));
            release_lock(dir);
            return None; // Do NOT create AGENTS.md without a sentinel
        }
        opencode_debug("[inject_agents_md] sentinel written");

        // NOW safe to write AGENTS.md
        let content = format!("{}\n", system_prompt);
        if let Err(e) = std::fs::write(&agents_path, &content) {
            opencode_debug(&format!("[inject_agents_md] write FAILED: {}", e));
            // Partial write may have left a corrupted AGENTS.md — try to delete it.
            match std::fs::remove_file(&agents_path) {
                Ok(()) => {
                    // AGENTS.md cleaned up — safe to remove sentinel
                    let _ = std::fs::remove_file(&sentinel_path);
                    opencode_debug("[inject_agents_md] cleaned up partial AGENTS.md + sentinel");
                }
                Err(_) => {
                    // Cannot delete corrupted AGENTS.md — keep sentinel for crash recovery
                    opencode_debug("[inject_agents_md] cannot delete partial AGENTS.md, sentinel preserved for recovery");
                }
            }
            release_lock(dir);
            return None;
        }
        opencode_debug(&format!("[inject_agents_md] created AGENTS.md ({} bytes)", content.len()));
    }

    Some(AgentsMdGuard { dir: dir.to_path_buf(), had_original })
}

// ============================================================
// Build the `opencode run` command
// ============================================================

fn build_opencode_command(
    session_id: Option<&str>,
    working_dir: &str,
    system_prompt_file: Option<&str>,
    model: Option<&str>,
) -> (Command, Option<std::path::PathBuf>) {
    let opencode_bin = resolve_opencode_path().unwrap_or_else(|| "opencode".to_string());
    opencode_debug(&format!("[build_cmd] bin={} working_dir={} session_id={:?} model={:?}",
        opencode_bin, working_dir, session_id, model));

    let mut args: Vec<String> = vec![
        "run".into(),
        "--format".into(), "json".into(),
    ];

    // Working directory
    args.push("--dir".into());
    args.push(working_dir.into());

    // Model
    if let Some(m) = model {
        args.push("--model".into());
        args.push(m.to_string());
    }

    // Session resume
    if let Some(sid) = session_id {
        args.push("--session".into());
        args.push(sid.to_string());
        args.push("--continue".into());
    }

    // System prompt is written to AGENTS.md in working_dir by the caller
    // (opencode reads AGENTS.md as project instructions automatically)
    let sp_path: Option<std::path::PathBuf> = None;
    let _ = system_prompt_file;

    opencode_debug(&format!("[build_cmd] full args: {} {}", opencode_bin, args.join(" ")));

    let mut cmd = Command::new(&opencode_bin);
    cmd.args(&args)
        .current_dir(working_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    (cmd, sp_path)
}

// ============================================================
// Parse opencode JSONL events → StreamMessage
// ============================================================

/// Extract text content from an opencode `text` event
fn parse_text_event(json: &Value) -> Option<String> {
    json.get("part")
        .and_then(|p| p.get("text"))
        .and_then(|t| t.as_str())
        .map(String::from)
}

/// Normalize opencode's lowercase tool names to PascalCase (system standard).
fn normalize_tool_name(name: &str) -> String {
    match name {
        "bash" => "Bash",
        "read" => "Read",
        "write" => "Write",
        "edit" => "Edit",
        "glob" => "Glob",
        "grep" => "Grep",
        "webfetch" => "WebFetch",
        "websearch" => "WebSearch",
        "notebookedit" => "NotebookEdit",
        "list" => "Glob",
        "task" => "Task",
        "taskoutput" => "TaskOutput",
        "taskstop" => "TaskStop",
        "taskcreate" => "TaskCreate",
        "taskupdate" => "TaskUpdate",
        "taskget" => "TaskGet",
        "tasklist" => "TaskList",
        "skill" => "Skill",
        "todowrite" => "TodoWrite",
        "todoread" => "TodoRead",
        "askuserquestion" => "AskUserQuestion",
        "enterplanmode" => "EnterPlanMode",
        "exitplanmode" => "ExitPlanMode",
        "codesearch" => "Grep",
        _ => name,
    }.to_string()
}

/// Extract tool use info from an opencode `tool_use` event
fn parse_tool_use_event(json: &Value) -> Option<(String, String, String, String, bool)> {
    let part = json.get("part")?;
    let raw_name = part.get("tool").and_then(|v| v.as_str()).unwrap_or("unknown");
    let tool_name = normalize_tool_name(raw_name);
    let call_id = part.get("callID").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let state = part.get("state")?;
    let status = state.get("status").and_then(|v| v.as_str()).unwrap_or("");

    let input = state.get("input")
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
        .unwrap_or_default();

    let (output, is_error) = match status {
        "completed" => {
            let out = state.get("output").and_then(|v| v.as_str()).unwrap_or("").to_string();
            (out, false)
        }
        "error" => {
            let err = state.get("error").and_then(|v| v.as_str()).unwrap_or("Tool error").to_string();
            (err, true)
        }
        _ => (String::new(), false),
    };

    opencode_debug(&format!("[parse_tool_use] tool={} call_id={} status={} input_len={} output_len={} is_error={}",
        tool_name, call_id, status, input.len(), output.len(), is_error));
    Some((tool_name, call_id, input, output, is_error))
}

/// Extract session ID from any event
fn extract_session_id(json: &Value) -> Option<String> {
    json.get("sessionID").and_then(|v| v.as_str()).map(String::from)
}

/// Extract tokens/cost from step_finish event
fn extract_step_finish(json: &Value) -> (Option<String>, bool) {
    let part = match json.get("part") {
        Some(p) => p,
        None => {
            opencode_debug("[extract_step_finish] no 'part' field");
            return (None, false);
        }
    };
    let reason = part.get("reason").and_then(|v| v.as_str()).unwrap_or("");
    let is_final = reason == "stop";
    let cost = part.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // Extract token details
    let tokens = part.get("tokens");
    let total_tokens = tokens.and_then(|t| t.get("total")).and_then(|v| v.as_u64()).unwrap_or(0);
    let input_tokens = tokens.and_then(|t| t.get("input")).and_then(|v| v.as_u64()).unwrap_or(0);
    let output_tokens = tokens.and_then(|t| t.get("output")).and_then(|v| v.as_u64()).unwrap_or(0);
    let reasoning_tokens = tokens.and_then(|t| t.get("reasoning")).and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_read = tokens.and_then(|t| t.get("cache")).and_then(|c| c.get("read")).and_then(|v| v.as_u64()).unwrap_or(0);

    opencode_debug(&format!("[extract_step_finish] reason={} is_final={} cost={:.6} tokens(total={} in={} out={} reasoning={} cache_read={})",
        reason, is_final, cost, total_tokens, input_tokens, output_tokens, reasoning_tokens, cache_read));

    (Some(reason.to_string()), is_final)
}

// ============================================================
// execute_command — non-streaming
// ============================================================

pub fn execute_command(
    prompt: &str,
    session_id: Option<&str>,
    working_dir: &str,
    _allowed_tools: Option<&[String]>,
    model: Option<&str>,
) -> ClaudeResponse {
    opencode_debug(&format!("[execute_command] START prompt_len={} session_id={:?} working_dir={} model={:?}",
        prompt.len(), session_id, working_dir, model));
    opencode_debug(&format!("[execute_command] prompt_preview={:?}", log_preview(prompt, 200)));

    let (mut cmd, _sp_path) = build_opencode_command(
        session_id, working_dir, None, model,
    );

    // When --model is specified, opencode ignores stdin → must use positional arg.
    // When no --model, stdin works and avoids shell arg size limits.
    let use_positional = model.is_some();
    if use_positional {
        opencode_debug(&format!("[execute_command] using positional arg (--model set), prompt_len={}", prompt.len()));
        cmd.arg("--");
        cmd.arg(prompt);
    }

    opencode_debug("[execute_command] spawning process...");
    let mut child = match cmd.spawn() {
        Ok(c) => {
            opencode_debug(&format!("[execute_command] spawned PID={}", c.id()));
            c
        }
        Err(e) => {
            opencode_debug(&format!("[execute_command] spawn FAILED: {}", e));
            return ClaudeResponse {
                success: false, response: None, session_id: None,
                error: Some(format!("Failed to start opencode: {}", e)),
            };
        }
    };

    // Write prompt to stdin (only when not using positional arg)
    if use_positional {
        drop(child.stdin.take());
        opencode_debug("[execute_command] stdin closed (positional mode)");
    } else if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(prompt.as_bytes()) {
            Ok(()) => opencode_debug(&format!("[execute_command] stdin: wrote {} bytes", prompt.len())),
            Err(e) => opencode_debug(&format!("[execute_command] stdin write FAILED: {}", e)),
        }
        drop(stdin);
        opencode_debug("[execute_command] stdin closed");
    } else {
        opencode_debug("[execute_command] WARN: no stdin handle");
    }

    opencode_debug("[execute_command] waiting for output...");
    match child.wait_with_output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            opencode_debug(&format!("[execute_command] exit={:?} stdout_len={} stderr_len={}",
                output.status.code(), stdout.len(), stderr.len()));
            if !stderr.is_empty() {
                opencode_debug(&format!("[execute_command] STDERR: {}", log_preview(&stderr, 500)));
            }

            let mut sid: Option<String> = None;
            let mut response_text = String::new();
            let mut line_count = 0u32;

            for line in stdout.trim().lines() {
                line_count += 1;
                opencode_debug(&format!("[execute_command] line {}: {}", line_count, log_preview(line, 300)));

                if let Ok(json) = serde_json::from_str::<Value>(line) {
                    // Capture session ID from any event
                    if let Some(s) = extract_session_id(&json) {
                        opencode_debug(&format!("[execute_command] session_id extracted: {}", s));
                        sid = Some(s);
                    }

                    let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    opencode_debug(&format!("[execute_command] event_type={}", event_type));

                    match event_type {
                        "text" => {
                            if let Some(text) = parse_text_event(&json) {
                                opencode_debug(&format!("[execute_command] TEXT: {} chars, preview={:?}",
                                    text.len(), log_preview(&text, 100)));
                                response_text.push_str(&text);
                            } else {
                                opencode_debug(&format!("[execute_command] TEXT parse FAILED: {}", log_preview(line, 300)));
                            }
                        }
                        "step_start" => {
                            opencode_debug("[execute_command] STEP_START");
                        }
                        "step_finish" => {
                            let (reason, is_final) = extract_step_finish(&json);
                            opencode_debug(&format!("[execute_command] STEP_FINISH: reason={:?} is_final={}", reason, is_final));
                        }
                        "tool_use" => {
                            opencode_debug(&format!("[execute_command] TOOL_USE event (non-streaming, skipped)"));
                        }
                        "reasoning" => {
                            opencode_debug("[execute_command] REASONING event (skipped)");
                        }
                        "error" => {
                            let err_msg = json.get("error")
                                .and_then(|v| v.get("message").or(Some(v)))
                                .and_then(|v| v.as_str())
                                .unwrap_or("Unknown error");
                            opencode_debug(&format!("[execute_command] ERROR event: {}", err_msg));
                            return ClaudeResponse {
                                success: false,
                                response: None,
                                session_id: sid,
                                error: Some(err_msg.to_string()),
                            };
                        }
                        _ => {
                            opencode_debug(&format!("[execute_command] unknown event_type={}", event_type));
                        }
                    }
                } else {
                    opencode_debug(&format!("[execute_command] JSON parse failed for line {}", line_count));
                }
            }

            opencode_debug(&format!("[execute_command] DONE: lines={} response_len={} session_id={:?} success={}",
                line_count, response_text.len(), sid, output.status.success()));
            if response_text.is_empty() {
                opencode_debug("[execute_command] WARN: empty response text");
            }

            ClaudeResponse {
                success: output.status.success(),
                response: Some(response_text.trim().to_string()),
                session_id: sid,
                error: if output.status.success() { None } else {
                    Some(stderr)
                },
            }
        }
        Err(e) => {
            opencode_debug(&format!("[execute_command] wait_with_output FAILED: {}", e));
            ClaudeResponse {
                success: false, response: None, session_id: None,
                error: Some(format!("Failed to read output: {}", e)),
            }
        }
    }
}

// ============================================================
// execute_command_streaming — stream JSONL events
// ============================================================

/// Same signature as `claude::execute_command_streaming` / `gemini::execute_command_streaming`.
pub fn execute_command_streaming(
    prompt: &str,
    session_id: Option<&str>,
    working_dir: &str,
    sender: Sender<StreamMessage>,
    system_prompt: Option<&str>,
    _allowed_tools: Option<&[String]>,
    cancel_token: Option<std::sync::Arc<CancelToken>>,
    model: Option<&str>,
    _no_session_persistence: bool,
) -> Result<(), String> {
    opencode_debug("=== opencode execute_command_streaming START ===");
    opencode_debug(&format!("[stream] prompt_len={} session_id={:?} working_dir={} model={:?}",
        prompt.len(), session_id, working_dir, model));
    opencode_debug(&format!("[stream] system_prompt_len={} cancel_token={}",
        system_prompt.map_or(0, |s| s.len()), cancel_token.is_some()));
    opencode_debug(&format!("[stream] prompt_preview={:?}", log_preview(prompt, 200)));

    // Inject system prompt into AGENTS.md so opencode reads it as project
    // instructions. The guard restores the original file when dropped (on
    // function return, including early returns and panics).
    let _agents_md_guard: Option<AgentsMdGuard> = match system_prompt {
        Some(sp) if !sp.is_empty() => {
            opencode_debug(&format!("[stream] injecting system prompt into AGENTS.md ({} bytes)", sp.len()));
            inject_system_prompt_into_agents_md(working_dir, sp)
        }
        _ => {
            opencode_debug("[stream] no system prompt, skipping AGENTS.md injection");
            None
        }
    };

    let (mut cmd, _sp_path) = build_opencode_command(
        session_id, working_dir, None, model,
    );

    // When --model is specified, opencode ignores stdin → must use positional arg.
    // When no --model, stdin works and avoids shell arg size limits.
    let use_positional = model.is_some();
    if use_positional {
        opencode_debug(&format!("[stream] using positional arg (--model set), prompt_len={}", prompt.len()));
        cmd.arg("--");
        cmd.arg(prompt);
    }
    opencode_debug(&format!("[stream] effective_prompt_len={} delivery={}", prompt.len(),
        if use_positional { "positional" } else { "stdin" }));

    opencode_debug("[stream] spawning process...");
    let mut child = cmd.spawn().map_err(|e| {
        opencode_debug(&format!("[stream] spawn FAILED: {}", e));
        format!("Failed to start opencode: {}", e)
    })?;
    opencode_debug(&format!("[stream] spawned PID={}", child.id()));

    // Store PID for cancel
    if let Some(ref token) = cancel_token {
        *token.child_pid.lock().unwrap() = Some(child.id());
        if token.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            opencode_debug("[stream] cancelled before stdin write, killing");
            kill_child_tree(&mut child);
            let _ = child.wait();
            return Ok(());
        }
    }

    // Write prompt to stdin (only when not using positional arg)
    if use_positional {
        drop(child.stdin.take());
        opencode_debug("[stream] stdin closed (positional mode)");
    } else if let Some(mut stdin) = child.stdin.take() {
        match stdin.write_all(prompt.as_bytes()) {
            Ok(()) => opencode_debug(&format!("[stream] stdin: wrote {} bytes", prompt.len())),
            Err(e) => opencode_debug(&format!("[stream] stdin write FAILED: {}", e)),
        }
        drop(stdin);
        opencode_debug("[stream] stdin closed");
    } else {
        opencode_debug("[stream] WARN: no stdin handle");
    }

    // Read stdout line by line
    let stdout = child.stdout.take().ok_or_else(|| {
        opencode_debug("[stream] FAILED to capture stdout");
        "Failed to capture stdout".to_string()
    })?;
    let reader = BufReader::new(stdout);
    opencode_debug("[stream] stdout reader ready, entering event loop");

    let mut last_session_id: Option<String> = None;
    let mut final_result = String::new();
    let mut got_done = false;
    let mut stdout_error: Option<(String, String)> = None;
    let mut init_sent = false;
    let mut event_count = 0u32;
    let mut text_event_count = 0u32;
    let mut tool_event_count = 0u32;

    for line in reader.lines() {
        // Check cancel
        if let Some(ref token) = cancel_token {
            if token.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                opencode_debug("[stream] cancelled during event loop, killing");
                kill_child_tree(&mut child);
                let _ = child.wait();
                return Ok(());
            }
        }

        let line = match line {
            Ok(l) => l,
            Err(e) => {
                opencode_debug(&format!("[stream] stdout read error: {}", e));
                let _ = sender.send(StreamMessage::Error {
                    message: format!("Failed to read output: {}", e),
                    stdout: String::new(), stderr: String::new(), exit_code: None,
                });
                break;
            }
        };

        if line.trim().is_empty() { continue; }

        event_count += 1;

        // Log raw event (truncated) for debugging
        opencode_debug(&format!("[stream] RAW[{}]: {}", event_count, log_preview(&line, 500)));

        let json: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                opencode_debug(&format!("[stream] JSON parse error on event {}: {}", event_count, e));
                continue;
            }
        };

        // Extract session ID from every event
        if let Some(sid) = extract_session_id(&json) {
            if last_session_id.as_deref() != Some(&sid) {
                opencode_debug(&format!("[stream] session_id updated: {:?} → {}", last_session_id, sid));
            }
            last_session_id = Some(sid);
        }

        let event_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match event_type {
            "step_start" => {
                opencode_debug(&format!("[stream] STEP_START (event {}), init_sent={}", event_count, init_sent));
                // Send Init on first step_start
                if !init_sent {
                    let sid = last_session_id.clone().unwrap_or_default();
                    opencode_debug(&format!("[stream] sending Init with session_id={}", sid));
                    if sender.send(StreamMessage::Init { session_id: sid }).is_err() {
                        opencode_debug("[stream] Init send failed (receiver dropped)");
                        break;
                    }
                    init_sent = true;
                }
            }

            "text" => {
                text_event_count += 1;
                if let Some(text) = parse_text_event(&json) {
                    opencode_debug(&format!("[stream] TEXT[{}]: {} chars, preview={:?}, cumulative_result_len={}",
                        text_event_count, text.len(), log_preview(&text, 100), final_result.len() + text.len()));
                    final_result.push_str(&text);
                    if sender.send(StreamMessage::Text { content: text }).is_err() {
                        opencode_debug("[stream] Text send failed (receiver dropped)");
                        break;
                    }
                } else {
                    opencode_debug(&format!("[stream] TEXT[{}] parse FAILED: {}", text_event_count, log_preview(&line, 300)));
                }
            }

            "tool_use" => {
                tool_event_count += 1;
                opencode_debug(&format!("[stream] TOOL_USE[{}] (event {})", tool_event_count, event_count));
                if let Some((tool_name, call_id, input, output, is_error)) = parse_tool_use_event(&json) {
                    let state = json.get("part")
                        .and_then(|p| p.get("state"))
                        .and_then(|s| s.get("status"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    opencode_debug(&format!("[stream] TOOL_USE: name={} call_id={} state={} input_len={} output_len={} is_error={}",
                        tool_name, call_id, state, input.len(), output.len(), is_error));

                    // Send ToolUse
                    if sender.send(StreamMessage::ToolUse {
                        name: tool_name.clone(),
                        input: input.clone(),
                    }).is_err() {
                        opencode_debug("[stream] ToolUse send failed (receiver dropped)");
                        break;
                    }

                    // Send ToolResult if completed or error
                    if state == "completed" || state == "error" {
                        opencode_debug(&format!("[stream] sending ToolResult: tool={} is_error={} output_preview={:?}",
                            tool_name, is_error, log_preview(&output, 200)));
                        if sender.send(StreamMessage::ToolResult {
                            content: output,
                            is_error,
                        }).is_err() {
                            opencode_debug("[stream] ToolResult send failed (receiver dropped)");
                            break;
                        }
                    }
                } else {
                    opencode_debug(&format!("[stream] TOOL_USE parse FAILED: {}", log_preview(&line, 300)));
                }
            }

            "reasoning" => {
                let reasoning_text = json.get("part")
                    .and_then(|p| p.get("text"))
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                opencode_debug(&format!("[stream] REASONING (event {}): {} chars", event_count, reasoning_text.len()));
            }

            "step_finish" => {
                let (reason, is_final) = extract_step_finish(&json);
                opencode_debug(&format!("[stream] STEP_FINISH (event {}): reason={:?} is_final={} result_len={}",
                    event_count, reason, is_final, final_result.len()));
                if is_final {
                    got_done = true;
                    opencode_debug(&format!("[stream] sending Done: result_len={} session_id={:?}",
                        final_result.len(), last_session_id));
                    let _ = sender.send(StreamMessage::Done {
                        result: final_result.clone(),
                        session_id: last_session_id.clone(),
                    });
                }
            }

            "error" => {
                let err_msg = json.get("error")
                    .and_then(|v| {
                        v.get("message").and_then(|m| m.as_str())
                            .or_else(|| v.as_str())
                    })
                    .unwrap_or("Unknown error")
                    .to_string();
                opencode_debug(&format!("[stream] ERROR event (event {}): {}", event_count, err_msg));
                stdout_error = Some((err_msg.clone(), line.clone()));
            }

            _ => {
                opencode_debug(&format!("[stream] UNKNOWN event_type={} (event {}): {}",
                    event_type, event_count, log_preview(&line, 200)));
            }
        }
    }

    opencode_debug(&format!("[stream] event loop ended: events={} text_events={} tool_events={} got_done={} result_len={}",
        event_count, text_event_count, tool_event_count, got_done, final_result.len()));

    // Check cancel before waiting
    if let Some(ref token) = cancel_token {
        if token.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            opencode_debug("[stream] cancelled after event loop, killing");
            kill_child_tree(&mut child);
            let _ = child.wait();
            return Ok(());
        }
    }

    opencode_debug("[stream] waiting for process exit...");
    let status = child.wait().map_err(|e| {
        opencode_debug(&format!("[stream] wait FAILED: {}", e));
        format!("Process error: {}", e)
    })?;

    // Always capture stderr for diagnostics
    let stderr_msg = child.stderr.take()
        .and_then(|s| std::io::read_to_string(s).ok())
        .unwrap_or_default();
    if !stderr_msg.is_empty() {
        opencode_debug(&format!("[stream] STDERR: {}", log_preview(&stderr_msg, 500)));
    }
    opencode_debug(&format!("[stream] exit_code={:?} success={} got_done={} result_len={} stderr_len={}",
        status.code(), status.success(), got_done, final_result.len(), stderr_msg.len()));

    // Handle errors
    if stdout_error.is_some() || !status.success() {
        let (message, stdout_raw) = if let Some((msg, raw)) = stdout_error {
            opencode_debug(&format!("[stream] reporting error: {}", msg));
            (msg, raw)
        } else {
            let msg = format!("Process exited with code {:?}", status.code());
            opencode_debug(&format!("[stream] reporting exit error: {}", msg));
            (msg, String::new())
        };
        let _ = sender.send(StreamMessage::Error {
            message, stdout: stdout_raw, stderr: stderr_msg, exit_code: status.code(),
        });
        return Ok(());
    }

    // Send Done if not already sent
    if !got_done {
        // If we got no text at all, report as error with clear reason
        if final_result.is_empty() {
            let model_name = model.unwrap_or("default");
            let reason = format!(
                "[OpenCode] model='{}' returned empty response (events={}, text_events={}, exit_code={:?}, stderr_len={})",
                model_name, event_count, text_event_count, status.code(), stderr_msg.len()
            );
            opencode_debug(&format!("[stream] empty response → {}", reason));
            let _ = sender.send(StreamMessage::Done {
                result: reason,
                session_id: last_session_id,
            });
        } else {
            opencode_debug(&format!("[stream] sending fallback Done (no step_finish): result_len={} session_id={:?}",
                final_result.len(), last_session_id));
            let _ = sender.send(StreamMessage::Done {
                result: final_result,
                session_id: last_session_id,
            });
        }
    }

    opencode_debug("=== opencode execute_command_streaming END ===");
    Ok(())
}
