use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::Path;
use std::fs;

use tokio::sync::Mutex;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use sha2::{Sha256, Digest};

use crate::services::claude::{self, CancelToken, StreamMessage, DEFAULT_ALLOWED_TOOLS};
use crate::ui::ai_screen::{self, HistoryItem, HistoryType, SessionData};

/// Global debug log flag for Telegram API calls
static TG_DEBUG: AtomicBool = AtomicBool::new(false);

/// Log Telegram API call result to ~/.cokacdir/debug/ file
fn tg_debug<T, E: std::fmt::Display>(name: &str, result: &Result<T, E>) {
    if !TG_DEBUG.load(Ordering::Relaxed) {
        return;
    }
    let Some(debug_dir) = dirs::home_dir().map(|h| h.join(".cokacdir").join("debug")) else {
        return;
    };
    let _ = fs::create_dir_all(&debug_dir);
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let log_path = debug_dir.join(format!("{}.log", date));
    let ts = chrono::Local::now().format("%H:%M:%S%.3f");
    let status = match result {
        Ok(_) => "✓".to_string(),
        Err(e) => format!("✗ {e}"),
    };
    let line = format!("[{ts}] {name}: {status}\n");
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
}

/// Wrap a Telegram API call to log its result in debug mode
macro_rules! tg {
    ($name:expr, $fut:expr) => {{
        let r = $fut;
        tg_debug($name, &r);
        r
    }};
}

/// Per-chat session state
#[derive(Clone)]
struct ChatSession {
    session_id: Option<String>,
    current_path: Option<String>,
    history: Vec<HistoryItem>,
    /// File upload records not yet sent to Claude AI.
    /// Drained and prepended to the next user prompt so Claude knows about uploaded files.
    pending_uploads: Vec<String>,
    /// Set to true by /clear to prevent a racing polling loop from re-populating history.
    cleared: bool,
}

/// Bot-level settings persisted to disk
#[derive(Clone)]
struct BotSettings {
    allowed_tools: HashMap<String, Vec<String>>,
    /// chat_id (string) → last working directory path
    last_sessions: HashMap<String, String>,
    /// Telegram user ID of the registered owner (imprinting auth)
    owner_user_id: Option<u64>,
    /// chat_id (string) → true if group chat is public (non-owner users allowed)
    as_public_for_group_chat: HashMap<String, bool>,
    /// chat_id (string) → model name (e.g. "sonnet", "opus", "haiku")
    models: HashMap<String, String>,
    /// Debug logging toggle
    debug: bool,
}

impl Default for BotSettings {
    fn default() -> Self {
        Self {
            allowed_tools: HashMap::new(),
            last_sessions: HashMap::new(),
            owner_user_id: None,
            as_public_for_group_chat: HashMap::new(),
            models: HashMap::new(),
            debug: false,
        }
    }
}

/// Get allowed tools for a specific chat_id.
/// Returns the chat-specific list if configured, otherwise DEFAULT_ALLOWED_TOOLS.
fn get_allowed_tools(settings: &BotSettings, chat_id: ChatId) -> Vec<String> {
    let key = chat_id.0.to_string();
    settings.allowed_tools.get(&key)
        .cloned()
        .unwrap_or_else(|| DEFAULT_ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect())
}

/// Get the configured model for a specific chat_id, if any.
fn get_model(settings: &BotSettings, chat_id: ChatId) -> Option<String> {
    let key = chat_id.0.to_string();
    settings.models.get(&key).cloned()
}

/// Schedule entry persisted as JSON in ~/.cokacdir/schedule/
#[derive(Clone)]
struct ScheduleEntry {
    id: String,
    chat_id: i64,
    bot_key: String,
    current_path: String,
    prompt: String,
    schedule: String,         // original --at value (cron expression or absolute time)
    schedule_type: String,    // "absolute" | "cron"
    once: bool,
    last_run: Option<String>, // "2026-02-23 14:00:00"
    created_at: String,
    context_summary: Option<String>, // context summary text for session-isolated schedule
}

/// Directory for schedule files: ~/.cokacdir/schedule/
fn schedule_dir() -> Option<std::path::PathBuf> {
    let result = dirs::home_dir().map(|h| h.join(".cokacdir").join("schedule"));
    sched_debug(&format!("[schedule_dir] → {:?}", result));
    result
}

fn sched_debug(msg: &str) {
    crate::services::claude::debug_log_to("cron.log", msg);
}

/// Read a single schedule entry from a JSON file
fn read_schedule_entry(path: &std::path::Path) -> Option<ScheduleEntry> {
    sched_debug(&format!("[read_schedule_entry] reading: {}", path.display()));
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            sched_debug(&format!("[read_schedule_entry] read failed: {}", e));
            return None;
        }
    };
    let v: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            sched_debug(&format!("[read_schedule_entry] parse failed: {}", e));
            return None;
        }
    };
    let entry = Some(ScheduleEntry {
        id: v.get("id")?.as_str()?.to_string(),
        chat_id: v.get("chat_id")?.as_i64()?,
        bot_key: v.get("bot_key")?.as_str()?.to_string(),
        current_path: v.get("current_path")?.as_str()?.to_string(),
        prompt: v.get("prompt")?.as_str()?.to_string(),
        schedule: v.get("schedule")?.as_str()?.to_string(),
        schedule_type: v.get("schedule_type")?.as_str()?.to_string(),
        once: v.get("once").and_then(|v| v.as_bool()).unwrap_or(false),
        last_run: v.get("last_run").and_then(|v| v.as_str()).map(String::from),
        created_at: v.get("created_at")?.as_str()?.to_string(),
        context_summary: v.get("context_summary").and_then(|v| v.as_str()).map(String::from),
    });
    sched_debug(&format!("[read_schedule_entry] result: id={}, type={}, schedule={}, last_run={:?}",
        entry.as_ref().map(|e| e.id.as_str()).unwrap_or("?"),
        entry.as_ref().map(|e| e.schedule_type.as_str()).unwrap_or("?"),
        entry.as_ref().map(|e| e.schedule.as_str()).unwrap_or("?"),
        entry.as_ref().and_then(|e| e.last_run.as_deref()),
    ));
    entry
}

/// Write a schedule entry to its JSON file
fn write_schedule_entry(entry: &ScheduleEntry) -> Result<(), String> {
    sched_debug(&format!("[write_schedule_entry] id={}, type={}, schedule={}, once={}, last_run={:?}",
        entry.id, entry.schedule_type, entry.schedule, entry.once, entry.last_run));
    let dir = schedule_dir().ok_or("Cannot determine home directory")?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create schedule dir: {e}"))?;
    let json = serde_json::json!({
        "id": entry.id,
        "chat_id": entry.chat_id,
        "bot_key": entry.bot_key,
        "current_path": entry.current_path,
        "prompt": entry.prompt,
        "schedule": entry.schedule,
        "schedule_type": entry.schedule_type,
        "once": entry.once,
        "last_run": entry.last_run,
        "created_at": entry.created_at,
        "context_summary": entry.context_summary,
    });
    let path = dir.join(format!("{}.json", entry.id));
    let tmp_path = dir.join(format!("{}.json.tmp", entry.id));
    sched_debug(&format!("[write_schedule_entry] writing tmp: {}", tmp_path.display()));
    fs::write(&tmp_path, serde_json::to_string_pretty(&json).unwrap_or_default())
        .map_err(|e| format!("Failed to write schedule file: {e}"))?;
    sched_debug(&format!("[write_schedule_entry] atomic rename: {} → {}", tmp_path.display(), path.display()));
    let result = fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to finalize schedule file: {e}"));
    sched_debug(&format!("[write_schedule_entry] result: {:?}", result));
    result
}

/// List all schedule entries matching the given bot_key and optionally chat_id
fn list_schedule_entries(bot_key: &str, chat_id: Option<i64>) -> Vec<ScheduleEntry> {
    sched_debug(&format!("[list_schedule_entries] bot_key={}, chat_id={:?}", bot_key, chat_id));
    let Some(dir) = schedule_dir() else {
        sched_debug("[list_schedule_entries] no schedule dir");
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        sched_debug("[list_schedule_entries] read_dir failed");
        return Vec::new();
    };
    let mut result: Vec<ScheduleEntry> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
        .filter_map(|e| read_schedule_entry(&e.path()))
        .filter(|e| e.bot_key == bot_key)
        .filter(|e| chat_id.map_or(true, |cid| e.chat_id == cid))
        .collect();
    result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    sched_debug(&format!("[list_schedule_entries] found {} entries: [{}]",
        result.len(),
        result.iter().map(|e| format!("{}({})", e.id, e.schedule_type)).collect::<Vec<_>>().join(", ")));
    result
}

/// Delete a schedule entry by ID
fn delete_schedule_entry(id: &str) -> bool {
    sched_debug(&format!("[delete_schedule_entry] id={}", id));
    let Some(dir) = schedule_dir() else {
        sched_debug("[delete_schedule_entry] no schedule dir");
        return false;
    };
    let path = dir.join(format!("{id}.json"));
    let existed = path.exists();
    let ok = fs::remove_file(&path).is_ok();
    sched_debug(&format!("[delete_schedule_entry] path={}, existed={}, removed={}", path.display(), existed, ok));
    ok
}

/// Parse a relative time string (e.g. "4h", "30m", "1d") into a future DateTime
fn parse_relative_time(s: &str) -> Option<chrono::DateTime<chrono::Local>> {
    sched_debug(&format!("[parse_relative_time] input: {:?}", s));
    let s = s.trim();
    if s.len() < 2 {
        sched_debug("[parse_relative_time] too short → None");
        return None;
    }
    let (num_part, unit) = s.split_at(s.len() - 1);
    let num: i64 = match num_part.parse() {
        Ok(n) => n,
        Err(_) => {
            sched_debug(&format!("[parse_relative_time] invalid number: {:?} → None", num_part));
            return None;
        }
    };
    if num <= 0 {
        sched_debug("[parse_relative_time] num <= 0 → None");
        return None;
    }
    let seconds = match unit {
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => {
            sched_debug(&format!("[parse_relative_time] unknown unit: {:?} → None", unit));
            return None;
        }
    };
    let result = Some(chrono::Local::now() + chrono::Duration::seconds(seconds));
    sched_debug(&format!("[parse_relative_time] → {:?}", result.as_ref().map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())));
    result
}

/// Check if a cron expression matches the given datetime.
/// 5 fields: minute, hour, day-of-month, month, day-of-week (0=Sun)
fn cron_matches(expr: &str, dt: chrono::DateTime<chrono::Local>) -> bool {
    use chrono::Datelike;
    use chrono::Timelike;

    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        sched_debug(&format!("[cron_matches] invalid field count: {} (expected 5) for expr={:?}", fields.len(), expr));
        return false;
    }

    let values = [
        dt.minute(),
        dt.hour(),
        dt.day(),
        dt.month(),
        dt.weekday().num_days_from_sunday(),
    ];
    let field_names = ["minute", "hour", "day", "month", "dow"];

    // Range start for each field: minute(0), hour(0), day-of-month(1), month(1), day-of-week(0)
    let range_starts = [0u32, 0, 1, 1, 0];

    for (i, ((field, &val), &range_start)) in fields.iter().zip(values.iter()).zip(range_starts.iter()).enumerate() {
        let matched = cron_field_matches(field, val, range_start);
        if !matched {
            sched_debug(&format!("[cron_matches] expr={:?}, dt={}, {}({})!={} → false",
                expr, dt.format("%H:%M"), field_names[i], val, field));
            return false;
        }
    }
    sched_debug(&format!("[cron_matches] expr={:?}, dt={} → true", expr, dt.format("%H:%M")));
    true
}

/// Check if a single cron field matches a value.
/// Supports: *, single number, comma-separated list, ranges (a-b), step (*/n, a-b/n)
/// range_start: the minimum value for this field (0 for minute/hour/dow, 1 for day/month)
fn cron_field_matches(field: &str, val: u32, range_start: u32) -> bool {
    if field == "*" { return true; }

    for part in field.split(',') {
        let part = part.trim();
        // Handle step: */n or a-b/n
        if let Some((range_part, step_str)) = part.split_once('/') {
            if let Ok(step) = step_str.parse::<u32>() {
                if step == 0 { continue; }
                if range_part == "*" {
                    if (val - range_start) % step == 0 {
                        sched_debug(&format!("[cron_field_matches] field={}, val={}, */{}  → true", field, val, step));
                        return true;
                    }
                } else if let Some((start_str, end_str)) = range_part.split_once('-') {
                    if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>()) {
                        if val >= start && val <= end && (val - start) % step == 0 {
                            sched_debug(&format!("[cron_field_matches] field={}, val={}, {}-{}/{} → true", field, val, start, end, step));
                            return true;
                        }
                    }
                }
            }
        } else if let Some((start_str, end_str)) = part.split_once('-') {
            // Range: a-b
            if let (Ok(start), Ok(end)) = (start_str.parse::<u32>(), end_str.parse::<u32>()) {
                if val >= start && val <= end {
                    sched_debug(&format!("[cron_field_matches] field={}, val={}, range {}-{} → true", field, val, start, end));
                    return true;
                }
            }
        } else {
            // Single number
            if let Ok(n) = part.parse::<u32>() {
                if val == n {
                    sched_debug(&format!("[cron_field_matches] field={}, val={}, exact {} → true", field, val, n));
                    return true;
                }
            }
        }
    }
    false
}

// === Public API for CLI commands (main.rs) ===

/// Public data struct mirroring ScheduleEntry for cross-module use
#[derive(Clone)]
pub struct ScheduleEntryData {
    pub id: String,
    pub chat_id: i64,
    pub bot_key: String,
    pub current_path: String,
    pub prompt: String,
    pub schedule: String,
    pub schedule_type: String,
    pub once: bool,
    pub last_run: Option<String>,
    pub created_at: String,
    pub context_summary: Option<String>,
}

impl From<&ScheduleEntry> for ScheduleEntryData {
    fn from(e: &ScheduleEntry) -> Self {
        Self {
            id: e.id.clone(),
            chat_id: e.chat_id,
            bot_key: e.bot_key.clone(),
            current_path: e.current_path.clone(),
            prompt: e.prompt.clone(),
            schedule: e.schedule.clone(),
            schedule_type: e.schedule_type.clone(),
            once: e.once,
            last_run: e.last_run.clone(),
            created_at: e.created_at.clone(),
            context_summary: e.context_summary.clone(),
        }
    }
}

impl From<&ScheduleEntryData> for ScheduleEntry {
    fn from(d: &ScheduleEntryData) -> Self {
        Self {
            id: d.id.clone(),
            chat_id: d.chat_id,
            bot_key: d.bot_key.clone(),
            current_path: d.current_path.clone(),
            prompt: d.prompt.clone(),
            schedule: d.schedule.clone(),
            schedule_type: d.schedule_type.clone(),
            once: d.once,
            last_run: d.last_run.clone(),
            created_at: d.created_at.clone(),
            context_summary: d.context_summary.clone(),
        }
    }
}

pub fn parse_relative_time_pub(s: &str) -> Option<chrono::DateTime<chrono::Local>> {
    parse_relative_time(s)
}

pub fn write_schedule_entry_pub(data: &ScheduleEntryData) -> Result<(), String> {
    let entry = ScheduleEntry::from(data);
    write_schedule_entry(&entry)
}

pub fn list_schedule_entries_pub(bot_key: &str, chat_id: Option<i64>) -> Vec<ScheduleEntryData> {
    list_schedule_entries(bot_key, chat_id).iter().map(ScheduleEntryData::from).collect()
}

pub fn list_all_schedule_ids_pub() -> std::collections::HashSet<String> {
    let Some(dir) = schedule_dir() else { return std::collections::HashSet::new() };
    let Ok(entries) = fs::read_dir(&dir) else { return std::collections::HashSet::new() };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().map(|ext| ext == "json").unwrap_or(false) {
                path.file_stem().map(|s| s.to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect()
}

pub fn delete_schedule_entry_pub(id: &str) -> bool {
    delete_schedule_entry(id)
}

/// Resolve the current working path for a chat from bot_settings.json
pub fn resolve_current_path_for_chat(chat_id: i64, hash_key: &str) -> Option<String> {
    let path = bot_settings_path()?;
    let content = fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let entry = json.get(hash_key)?;
    let last_sessions = entry.get("last_sessions")?.as_object()?;
    let chat_key = chat_id.to_string();
    last_sessions.get(&chat_key)?.as_str().map(String::from)
}

/// Build the system prompt for Telegram AI sessions
fn build_system_prompt(role: &str, current_path: &str, chat_id: i64, bot_key: &str, disabled_notice: &str, session_id: Option<&str>) -> String {
    let session_notice = match session_id {
        Some(sid) => format!(
            "\n\n\
             Current session ID: {sid}\n\
             When scheduling a task that CONTINUES or EXTENDS the current conversation \
             (e.g. \"finish this later\", \"do the rest tomorrow\", \"remind me to continue this\"), \
             add --session {sid} to the --cron command so the scheduled task inherits this conversation context.\n\
             Do NOT use --session for independent tasks that don't need the current conversation history \
             (e.g. \"check server status every hour\", \"send a daily report\")."
        ),
        None => String::new(),
    };
    format!(
        "{role}\n\
         Current working directory: {current_path}\n\n\
         Always keep the user informed about what you are doing. \
         Briefly explain each step as you work (e.g. \"Reading the file...\", \"Creating the script...\", \"Running tests...\"). \
         The user cannot see your tool calls, so narrate your progress so they know what is happening.\n\n\
         IMPORTANT: The user is on Telegram and CANNOT interact with any interactive prompts, dialogs, or confirmation requests. \
         All tools that require user interaction (such as AskUserQuestion, EnterPlanMode, ExitPlanMode) will NOT work. \
         Never use tools that expect user interaction. If you need clarification, just ask in plain text.\n\n\
         Response format: Use Markdown by default, but do NOT use Markdown tables.\n\n\
         ═══════════════════════════════════════\n\
         COKACDIR COMMAND REFERENCE\n\
         ═══════════════════════════════════════\n\
         All commands output JSON. Success: {{\"status\":\"ok\",...}}, Error: {{\"status\":\"error\",\"message\":\"...\"}}\n\n\
         ── FILE DELIVERY ──\n\
         Send a file to the user's Telegram chat:\n\
         cokacdir --sendfile <FILEPATH> --chat {chat_id} --key {bot_key}\n\
         • Use this whenever your work produces a file (code, reports, images, archives, etc.)\n\
         • Do NOT tell the user to use /down — always use this command instead\n\
         • Output: {{\"status\":\"ok\",\"path\":\"<absolute_path>\"}}\n\n\
         ── SERVER TIME ──\n\
         Get current server time (use before scheduling to confirm timezone):\n\
         cokacdir --currenttime\n\
         • Output: {{\"status\":\"ok\",\"time\":\"2026-02-25 14:30:00\"}}\n\n\
         ── SCHEDULE: REGISTER ──\n\
         cokacdir --cron \"<PROMPT>\" --at \"<TIME>\" --chat {chat_id} --key {bot_key} [--once] [--session <SESSION_ID>]\n\
         • --at formats:\n\
           - \"2026-02-25 18:00:00\" or \"30m\", \"4h\", \"1d\" → one-time (auto-deleted after execution)\n\
           - \"0 * * * *\" (5-field cron) → recurring\n\
         • --once: cron only — run once at the next match then auto-delete\n\
         • --session <SID>: pass ONLY when the task continues the current conversation context\n\
         • PROMPT rules:\n\
           1. Write as an imperative INSTRUCTION for another AI, not conversational text\n\
           2. ★ MUST be in the user's language (한국어 사용자 → 한국어, English user → English)\n\
         • Output: {{\"status\":\"ok\",\"id\":\"...\",\"prompt\":\"...\",\"schedule\":\"...\"}}{session_notice}\n\n\
         ── SCHEDULE: LIST ──\n\
         cokacdir --cron-list --chat {chat_id} --key {bot_key}\n\
         • Output: {{\"status\":\"ok\",\"schedules\":[{{\"id\":\"...\",\"prompt\":\"...\",\"schedule\":\"...\",\"created_at\":\"...\"}},...]}}\n\n\
         ── SCHEDULE: REMOVE ──\n\
         cokacdir --cron-remove <SCHEDULE_ID> --chat {chat_id} --key {bot_key}\n\
         • Output: {{\"status\":\"ok\",\"id\":\"...\"}}\n\n\
         ── SCHEDULE: UPDATE TIME ──\n\
         cokacdir --cron-update <SCHEDULE_ID> --at \"<NEW_TIME>\" --chat {chat_id} --key {bot_key}\n\
         • --at accepts the same formats as --cron\n\
         • Output: {{\"status\":\"ok\",\"id\":\"...\",\"schedule\":\"...\"}}\n\n\
         ═══════════════════════════════════════{disabled_notice}",
        role = role,
        current_path = current_path,
        chat_id = chat_id,
        bot_key = bot_key,
        disabled_notice = disabled_notice,
        session_notice = session_notice,
    )
}

/// Check if a newer version is available by fetching Cargo.toml from GitHub.
/// Returns a notice string if an update is available, None otherwise.
async fn check_latest_version(current: &str) -> Option<String> {
    let url = "https://raw.githubusercontent.com/kstost/cokacdir/refs/heads/main/Cargo.toml";
    let resp = reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(5))
        .send().await.ok()?;
    let text = resp.text().await.ok()?;
    let latest = text.lines()
        .find(|l| l.starts_with("version"))?
        .split('"').nth(1)?;
    if version_is_newer(latest, current) {
        Some(format!("🆕 v{} available — https://cokacdir.cokac.com/", latest))
    } else {
        None
    }
}

/// Compare two semver-like version strings. Returns true if `a` is strictly greater than `b`.
fn version_is_newer(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.').filter_map(|p| p.parse().ok()).collect()
    };
    let va = parse(a);
    let vb = parse(b);
    va > vb
}

/// Shared state: per-chat sessions + bot settings
struct SharedData {
    sessions: HashMap<ChatId, ChatSession>,
    settings: BotSettings,
    /// Per-chat cancel tokens for stopping in-progress AI requests
    cancel_tokens: HashMap<ChatId, Arc<CancelToken>>,
    /// Message ID of the "Stopping..." message sent by /stop, so the polling loop can update it
    stop_message_ids: HashMap<ChatId, teloxide::types::MessageId>,
    /// Per-chat timestamp of the last Telegram API call (for rate limiting)
    api_timestamps: HashMap<ChatId, tokio::time::Instant>,
    /// Telegram API polling interval in milliseconds (shared across all bots)
    polling_time_ms: u64,
    /// Schedule IDs currently being executed or pending, per chat
    pending_schedules: HashMap<ChatId, std::collections::HashSet<String>>,
}

type SharedState = Arc<Mutex<SharedData>>;

/// Telegram message length limit
const TELEGRAM_MSG_LIMIT: usize = 4096;

/// Compute a short hash key from the bot token (first 16 chars of SHA-256 hex)
pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8]) // 16 hex chars
}

/// Path to bot settings file: ~/.cokacdir/bot_settings.json
fn bot_settings_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".cokacdir").join("bot_settings.json"))
}

/// Load bot settings from bot_settings.json
fn load_bot_settings(token: &str) -> BotSettings {
    let Some(path) = bot_settings_path() else {
        return BotSettings::default();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return BotSettings::default();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) else {
        return BotSettings::default();
    };
    let key = token_hash(token);
    let Some(entry) = json.get(&key) else {
        return BotSettings::default();
    };
    let owner_user_id = entry.get("owner_user_id").and_then(|v| v.as_u64());
    let last_sessions: HashMap<String, String> = entry.get("last_sessions")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let allowed_tools = match entry.get("allowed_tools") {
        Some(serde_json::Value::Array(arr)) => {
            // Legacy migration: array → per-chat HashMap
            let tool_list: Vec<String> = arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if tool_list.is_empty() {
                HashMap::new()
            } else {
                let mut map = HashMap::new();
                for chat_id_str in last_sessions.keys() {
                    map.insert(chat_id_str.clone(), tool_list.clone());
                }
                map
            }
        }
        Some(serde_json::Value::Object(obj)) => {
            // New format: object with chat_id keys
            obj.iter()
                .filter_map(|(k, v)| {
                    v.as_array().map(|arr| {
                        let tools: Vec<String> = arr.iter()
                            .filter_map(|t| t.as_str().map(String::from))
                            .collect();
                        (k.clone(), tools)
                    })
                })
                .collect()
        }
        _ => HashMap::new(),
    };

    let as_public_for_group_chat: HashMap<String, bool> = entry.get("as_public_for_group_chat")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_bool().map(|b| (k.clone(), b)))
                .collect()
        })
        .unwrap_or_default();

    let models: HashMap<String, String> = entry.get("models")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    let debug = entry.get("debug").and_then(|v| v.as_bool()).unwrap_or(false);

    BotSettings { allowed_tools, last_sessions, owner_user_id, as_public_for_group_chat, models, debug }
}

/// Save bot settings to bot_settings.json
fn save_bot_settings(token: &str, settings: &BotSettings) {
    let Some(path) = bot_settings_path() else { return };
    // Ensure directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // Load existing JSON or start fresh
    let mut json: serde_json::Value = if let Ok(content) = fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let key = token_hash(token);
    let mut entry = serde_json::json!({
        "token": token,
        "allowed_tools": settings.allowed_tools,
        "last_sessions": settings.last_sessions,
        "as_public_for_group_chat": settings.as_public_for_group_chat,
        "models": settings.models,
        "debug": settings.debug,
    });
    if let Some(owner_id) = settings.owner_user_id {
        entry["owner_user_id"] = serde_json::json!(owner_id);
    }
    json[key] = entry;
    if let Ok(s) = serde_json::to_string_pretty(&json) {
        let tmp_path = path.with_extension("json.tmp");
        if fs::write(&tmp_path, &s).is_ok() {
            let _ = fs::rename(&tmp_path, &path);
        }
    }
}

/// Resolve a bot token from its hash by searching bot_settings.json
pub fn resolve_token_by_hash(hash: &str) -> Option<String> {
    let path = bot_settings_path()?;
    let content = fs::read_to_string(&path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&content).ok()?;
    let obj = json.as_object()?;
    let entry = obj.get(hash)?;
    entry.get("token").and_then(|v| v.as_str()).map(String::from)
}

/// Normalize tool name: first letter uppercase, rest lowercase
fn normalize_tool_name(name: &str) -> String {
    let lower = name.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// All available tools with (description, is_destructive)
const ALL_TOOLS: &[(&str, &str, bool)] = &[
    ("Bash",            "Execute shell commands",                          true),
    ("Read",            "Read file contents from the filesystem",          false),
    ("Edit",            "Perform find-and-replace edits in files",         true),
    ("Write",           "Create or overwrite files",                       true),
    ("Glob",            "Find files by name pattern",                      false),
    ("Grep",            "Search file contents with regex",                 false),
    ("Task",            "Launch autonomous sub-agents for complex tasks",  true),
    ("TaskOutput",      "Retrieve output from background tasks",           false),
    ("TaskStop",        "Stop a running background task",                  false),
    ("WebFetch",        "Fetch and process web page content",              true),
    ("WebSearch",       "Search the web for up-to-date information",       true),
    ("NotebookEdit",    "Edit Jupyter notebook cells",                     true),
    ("Skill",           "Invoke slash-command skills",                     false),
    ("TaskCreate",      "Create a structured task in the task list",       false),
    ("TaskGet",         "Retrieve task details by ID",                     false),
    ("TaskUpdate",      "Update task status or details",                   false),
    ("TaskList",        "List all tasks and their status",                 false),
    ("AskUserQuestion", "Ask the user a question (interactive)",           false),
    ("EnterPlanMode",   "Enter planning mode (interactive)",               false),
    ("ExitPlanMode",    "Exit planning mode (interactive)",                false),
];

/// Tool info: (description, is_destructive)
fn tool_info(name: &str) -> (&'static str, bool) {
    ALL_TOOLS.iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, desc, destr)| (*desc, *destr))
        .unwrap_or(("Custom tool", false))
}

/// Format a risk badge for display
fn risk_badge(destructive: bool) -> &'static str {
    if destructive { "!!!" } else { "" }
}

/// Entry point: start the Telegram bot with long polling
pub async fn run_bot(token: &str) {
    let bot = Bot::new(token);
    let bot_settings = load_bot_settings(token);

    // Restore debug flag from saved settings
    if bot_settings.debug {
        TG_DEBUG.store(true, Ordering::Relaxed);
        crate::services::claude::DEBUG_ENABLED.store(true, Ordering::Relaxed);
    }

    // Register bot commands for autocomplete
    let commands = vec![
        teloxide::types::BotCommand::new("help", "Show help"),
        teloxide::types::BotCommand::new("start", "Start session at directory"),
        teloxide::types::BotCommand::new("pwd", "Show current working directory"),
        teloxide::types::BotCommand::new("clear", "Clear AI conversation history"),
        teloxide::types::BotCommand::new("stop", "Stop current AI request"),
        teloxide::types::BotCommand::new("down", "Download file from server"),
        teloxide::types::BotCommand::new("public", "Toggle public access (group only)"),
        teloxide::types::BotCommand::new("availabletools", "List all available tools"),
        teloxide::types::BotCommand::new("allowedtools", "Show currently allowed tools"),
        teloxide::types::BotCommand::new("allowed", "Add/remove tool (+name / -name)"),
        teloxide::types::BotCommand::new("setpollingtime", "Set API polling interval (ms)"),
        teloxide::types::BotCommand::new("model", "Set AI model"),
        teloxide::types::BotCommand::new("debug", "Toggle debug logging"),
    ];
    if let Err(e) = tg!("set_my_commands", bot.set_my_commands(commands).await) {
        println!("  ⚠ Failed to set bot commands: {e}");
    }

    match bot_settings.owner_user_id {
        Some(owner_id) => println!("  ✓ Owner: {owner_id}"),
        None => println!("  ⚠ No owner registered — first user will be registered as owner"),
    }

    let app_settings = crate::config::Settings::load();
    let polling_time_ms = app_settings.telegram_polling_time.max(2500);

    let state: SharedState = Arc::new(Mutex::new(SharedData {
        sessions: HashMap::new(),
        settings: bot_settings,
        cancel_tokens: HashMap::new(),
        stop_message_ids: HashMap::new(),
        api_timestamps: HashMap::new(),
        polling_time_ms,
        pending_schedules: HashMap::new(),
    }));

    println!("  ✓ Bot connected — Listening for messages");
    println!("  ✓ Scheduler started (5s interval)");

    // Send startup greeting to known chats
    {
        let data = state.lock().await;
        let chat_ids: Vec<i64> = data.settings.last_sessions.keys()
            .filter_map(|k| k.parse::<i64>().ok())
            .collect();
        let version = env!("CARGO_PKG_VERSION");
        let update_notice = check_latest_version(version).await;
        for cid in chat_ids {
            let chat_id = ChatId(cid);
            let last_path = data.settings.last_sessions.get(&cid.to_string())
                .map(|p| p.as_str())
                .unwrap_or("(unknown)");
            let mut msg = format!("🟢 cokacdir started (v{})\n📂 Resuming session at {}", version, last_path);
            if let Some(ref notice) = update_notice {
                msg.push('\n');
                msg.push_str(notice);
            }
            let _ = tg!("send_message", bot.send_message(chat_id, msg).await);
        }
    }

    // Schedule workspace directories are preserved for user access via /start

    // Spawn scheduler loop
    let scheduler_bot = bot.clone();
    let scheduler_state = state.clone();
    let scheduler_token = token.to_string();
    let scheduler_handle = tokio::spawn(scheduler_loop(scheduler_bot, scheduler_state, scheduler_token));

    let shared_state = state.clone();
    let token_owned = token.to_string();
    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let state = shared_state.clone();
        let token = token_owned.clone();
        async move {
            handle_message(bot, msg, state, &token).await
        }
    })
    .await;

    scheduler_handle.abort();
}

/// Route incoming messages to appropriate handlers
async fn handle_message(
    bot: Bot,
    msg: Message,
    state: SharedState,
    token: &str,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let raw_user_name = msg.from.as_ref()
        .map(|u| u.first_name.as_str())
        .unwrap_or("unknown");
    let timestamp = chrono::Local::now().format("%H:%M:%S");
    let user_id = msg.from.as_ref().map(|u| u.id.0);

    // Auth check (imprinting)
    let Some(uid) = user_id else {
        // No user info (e.g. channel post) → reject
        return Ok(());
    };
    let is_group_chat = matches!(msg.chat.kind, teloxide::types::ChatKind::Public(_));
    let imprinted = {
        let mut data = state.lock().await;
        match data.settings.owner_user_id {
            None => {
                // Imprint: register first user as owner
                data.settings.owner_user_id = Some(uid);
                save_bot_settings(token, &data.settings);
                println!("  [{timestamp}] ★ Owner registered: {raw_user_name} (id:{uid})");
                true
            }
            Some(owner_id) => {
                if uid != owner_id {
                    // Check if this is a public group chat
                    let chat_key = chat_id.0.to_string();
                    let is_public = is_group_chat
                        && data.settings.as_public_for_group_chat.get(&chat_key).copied().unwrap_or(false);
                    if !is_public {
                        // Unregistered user → reject silently (log only)
                        println!("  [{timestamp}] ✗ Rejected: {raw_user_name} (id:{uid})");
                        return Ok(());
                    }
                    // Public group chat: allow non-owner user
                    println!("  [{timestamp}] ○ [{raw_user_name}(id:{uid})] Public group access");
                }
                false
            }
        }
    };
    if imprinted {
        // Owner registration is logged to server console only
        // No response sent to the user
    }

    let is_owner = {
        let data = state.lock().await;
        data.settings.owner_user_id == Some(uid)
    };

    let user_name = format!("{}({uid})", raw_user_name);

    // Handle file/photo uploads
    if msg.document().is_some() || msg.photo().is_some() {
        // In group chats, only process uploads whose caption starts with ';'
        if is_group_chat {
            let caption = msg.caption().unwrap_or("");
            if !caption.starts_with(';') {
                return Ok(());
            }
        }
        let file_hint = if msg.document().is_some() { "document" } else { "photo" };
        println!("  [{timestamp}] ◀ [{user_name}] Upload: {file_hint}");
        handle_file_upload(&bot, chat_id, &msg, &state).await?;
        println!("  [{timestamp}] ▶ [{user_name}] Upload complete");
        // If caption contains text after ';', send it to AI as a follow-up message
        if let Some(caption) = msg.caption() {
            let text_part = if is_group_chat {
                // Group chat: extract text after ';'
                caption.find(';').map(|pos| caption[pos + 1..].trim())
            } else {
                // DM: use entire caption as-is
                let trimmed = caption.trim();
                if trimmed.is_empty() { None } else { Some(trimmed) }
            };
            if let Some(text) = text_part {
                if !text.is_empty() {
                    // Block if an AI request is already in progress
                    let ai_busy = {
                        let data = state.lock().await;
                        data.cancel_tokens.contains_key(&chat_id)
                    };
                    if ai_busy {
                        shared_rate_limit_wait(&state, chat_id).await;
                        tg!("send_message", bot.send_message(chat_id, "AI request in progress. Use /stop to cancel.")
                            .await)?;
                    } else {
                        handle_text_message(&bot, chat_id, text, &state).await?;
                    }
                }
            }
        }
        return Ok(());
    }

    let Some(raw_text) = msg.text() else {
        return Ok(());
    };

    // Strip @botname suffix from commands (e.g. "/pwd@mybot" → "/pwd")
    let text = if raw_text.starts_with('/') {
        if let Some(space_pos) = raw_text.find(' ') {
            // "/cmd@bot args" → "/cmd args"
            let cmd_part = &raw_text[..space_pos];
            let args_part = &raw_text[space_pos..];
            if let Some(at_pos) = cmd_part.find('@') {
                format!("{}{}", &cmd_part[..at_pos], args_part)
            } else {
                raw_text.to_string()
            }
        } else {
            // "/cmd@bot" (no args) → "/cmd"
            if let Some(at_pos) = raw_text.find('@') {
                raw_text[..at_pos].to_string()
            } else {
                raw_text.to_string()
            }
        }
    } else {
        raw_text.to_string()
    };
    let preview = &text;

    // Auto-restore session from bot_settings.json if not in memory
    if !text.starts_with("/start") {
        let mut data = state.lock().await;
        if !data.sessions.contains_key(&chat_id) {
            if let Some(last_path) = data.settings.last_sessions.get(&chat_id.0.to_string()).cloned() {
                if Path::new(&last_path).is_dir() {
                    let existing = load_existing_session(&last_path);
                    let session = data.sessions.entry(chat_id).or_insert_with(|| ChatSession {
                        session_id: None,
                        current_path: None,
                        history: Vec::new(),
                        pending_uploads: Vec::new(),
                        cleared: false,
                    });
                    session.current_path = Some(last_path.clone());
                    if let Some((session_data, _)) = existing {
                        session.session_id = Some(session_data.session_id.clone());
                        session.history = session_data.history.clone();
                    }
                    let ts = chrono::Local::now().format("%H:%M:%S");
                    println!("  [{ts}] ↻ [{user_name}] Auto-restored session: {last_path}");
                }
            }
        }
    }

    // In group chats, ignore plain text (only /, !, ; prefixed messages are processed)
    if is_group_chat && !text.starts_with('/') && !text.starts_with('!') && !text.starts_with(';') {
        return Ok(());
    }

    // Block all messages except /stop while an AI request is in progress
    if !text.starts_with("/stop") {
        let data = state.lock().await;
        if data.cancel_tokens.contains_key(&chat_id) {
            drop(data);
            shared_rate_limit_wait(&state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, "AI request in progress. Use /stop to cancel.")
                .await)?;
            return Ok(());
        }
    }

    if text.starts_with("/stop") {
        println!("  [{timestamp}] ◀ [{user_name}] /stop");
        handle_stop_command(&bot, chat_id, &state).await?;
    } else if text.starts_with("/help") {
        println!("  [{timestamp}] ◀ [{user_name}] /help");
        handle_help_command(&bot, chat_id, &state).await?;
    } else if text.starts_with("/start") {
        println!("  [{timestamp}] ◀ [{user_name}] /start");
        handle_start_command(&bot, chat_id, &text, &state, token).await?;
    } else if text.starts_with("/clear") {
        println!("  [{timestamp}] ◀ [{user_name}] /clear");
        handle_clear_command(&bot, chat_id, &state).await?;
        println!("  [{timestamp}] ▶ [{user_name}] Session cleared");
    } else if text.starts_with("/pwd") {
        println!("  [{timestamp}] ◀ [{user_name}] /pwd");
        handle_pwd_command(&bot, chat_id, &state).await?;
    } else if text.starts_with("/down") {
        println!("  [{timestamp}] ◀ [{user_name}] /down {}", text.strip_prefix("/down").unwrap_or("").trim());
        handle_down_command(&bot, chat_id, &text, &state).await?;
    } else if text.starts_with("/public") {
        println!("  [{timestamp}] ◀ [{user_name}] /public {}", text.strip_prefix("/public").unwrap_or("").trim());
        handle_public_command(&bot, chat_id, &text, &state, token, is_group_chat, is_owner).await?;
    } else if text.starts_with("/availabletools") {
        println!("  [{timestamp}] ◀ [{user_name}] /availabletools");
        handle_availabletools_command(&bot, chat_id, &state).await?;
    } else if text.starts_with("/allowedtools") {
        println!("  [{timestamp}] ◀ [{user_name}] /allowedtools");
        handle_allowedtools_command(&bot, chat_id, &state).await?;
    } else if text.starts_with("/setpollingtime") {
        println!("  [{timestamp}] ◀ [{user_name}] /setpollingtime {}", text.strip_prefix("/setpollingtime").unwrap_or("").trim());
        handle_setpollingtime_command(&bot, chat_id, &text, &state).await?;
    } else if text.starts_with("/model") {
        println!("  [{timestamp}] ◀ [{user_name}] /model {}", text.strip_prefix("/model").unwrap_or("").trim());
        handle_model_command(&bot, chat_id, &text, &state, token).await?;
    } else if text.starts_with("/debug") {
        println!("  [{timestamp}] ◀ [{user_name}] /debug");
        handle_debug_command(&bot, chat_id, &state, token).await?;
    } else if text.starts_with("/allowed") {
        println!("  [{timestamp}] ◀ [{user_name}] /allowed {}", text.strip_prefix("/allowed").unwrap_or("").trim());
        handle_allowed_command(&bot, chat_id, &text, &state, token).await?;
    } else if text.starts_with('/') && is_workspace_id(text[1..].split_whitespace().next().unwrap_or("")) {
        let workspace_id = text[1..].split_whitespace().next().unwrap();
        println!("  [{timestamp}] ◀ [{user_name}] /{workspace_id}");
        handle_workspace_resume(&bot, chat_id, workspace_id, &state, token).await?;
    } else if text.starts_with('!') {
        println!("  [{timestamp}] ◀ [{user_name}] Shell: {preview}");
        handle_shell_command(&bot, chat_id, &text, &state).await?;
    } else if text.starts_with(';') {
        let stripped = text.strip_prefix(';').unwrap_or(&text).trim().to_string();
        if stripped.is_empty() {
            return Ok(());
        }
        let preview = &stripped;
        println!("  [{timestamp}] ◀ [{user_name}] {preview}");
        handle_text_message(&bot, chat_id, &stripped, &state).await?;
    } else {
        println!("  [{timestamp}] ◀ [{user_name}] {preview}");
        handle_text_message(&bot, chat_id, &text, &state).await?;
    }

    Ok(())
}

/// Handle /help command
async fn handle_help_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let help = "\
<b>cokacdir Telegram Bot</b>
Manage server files &amp; chat with Claude AI.

<b>Session</b>
<code>/start &lt;path&gt;</code> — Start session at directory
<code>/start</code> — Start with auto-generated workspace
<code>/pwd</code> — Show current working directory
<code>/clear</code> — Clear AI conversation history
<code>/stop</code> — Stop current AI request

<b>File Transfer</b>
<code>/down &lt;file&gt;</code> — Download file from server
Send a file/photo — Upload to session directory

<b>Shell</b>
<code>!&lt;command&gt;</code> — Run shell command directly
  e.g. <code>!ls -la</code>, <code>!git status</code>

<b>AI Chat</b>
Any other message is sent to Claude AI.
AI can read, edit, and run commands in your session.

<b>Tool Management</b>
<code>/availabletools</code> — List all available tools
<code>/allowedtools</code> — Show currently allowed tools
<code>/allowed +name</code> — Add tool (e.g. <code>/allowed +Bash</code>)
<code>/allowed -name</code> — Remove tool
<code>/allowed +a -b +c</code> — Multiple at once

<b>Group Chat</b>
<code>;</code><i>message</i> — Send message to AI
<code>;</code><i>caption</i> — Upload file with AI prompt
<code>/public on</code> — Allow all members to use bot
<code>/public off</code> — Owner only (default)

<b>Schedule</b>
Ask in natural language to manage schedules.

<b>Settings</b>
<code>/model</code> — Show current AI model
<code>/model &lt;name&gt;</code> — Set model (sonnet/opus/haiku/sonnet[1m]/opus[1m]/haiku[1m])
<code>/setpollingtime &lt;ms&gt;</code> — Set API polling interval
  Too low may cause Telegram API rate limits.
  Minimum 2500ms, recommended 3000ms+.
<code>/debug</code> — Toggle debug logging

<code>/help</code> — Show this help";

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, help)
        .parse_mode(ParseMode::Html)
        .await)?;

    Ok(())
}

/// Handle /start <path> command
async fn handle_start_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
    token: &str,
) -> ResponseResult<()> {
    // Extract path from "/start <path>"
    let path_str = text.strip_prefix("/start").unwrap_or("").trim();

    let canonical_path = if path_str.is_empty() {
        // Create random workspace directory
        let Some(home) = dirs::home_dir() else {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, "Error: cannot determine home directory.")
                .await)?;
            return Ok(());
        };
        let workspace_dir = home.join(".cokacdir").join("workspace");
        use rand::Rng;
        let random_name: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(8)
            .map(|b| (b as char).to_ascii_lowercase())
            .collect();
        let new_dir = workspace_dir.join(&random_name);
        if let Err(e) = fs::create_dir_all(&new_dir) {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, format!("Error: failed to create workspace: {}", e))
                .await)?;
            return Ok(());
        }
        new_dir.display().to_string()
    } else {
        // Expand ~ to home directory
        let expanded = if path_str.starts_with("~/") || path_str == "~" {
            if let Some(home) = dirs::home_dir() {
                home.join(path_str.strip_prefix("~/").unwrap_or("")).display().to_string()
            } else {
                path_str.to_string()
            }
        } else {
            path_str.to_string()
        };
        // Validate path exists
        let path = Path::new(&expanded);
        if !path.exists() || !path.is_dir() {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, format!("Error: '{}' is not a valid directory.", expanded))
                .await)?;
            return Ok(());
        }
        path.canonicalize()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| expanded)
    };

    // Try to load existing session for this path
    let existing = load_existing_session(&canonical_path);

    let mut response_lines = Vec::new();

    {
        let mut data = state.lock().await;
        let session = data.sessions.entry(chat_id).or_insert_with(|| ChatSession {
            session_id: None,
            current_path: None,
            history: Vec::new(),
            pending_uploads: Vec::new(),
            cleared: false,
        });

        if let Some((session_data, _)) = &existing {
            session.session_id = Some(session_data.session_id.clone());
            session.current_path = Some(canonical_path.clone());
            session.history = session_data.history.clone();

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ▶ Session restored: {canonical_path}");
            response_lines.push(format!("Session restored at `{}`.", canonical_path));
            if let Some(folder_name) = std::path::Path::new(&canonical_path).file_name().and_then(|n| n.to_str()) {
                if is_workspace_id(folder_name) {
                    response_lines.push(format!("Use /{} to resume this session.", folder_name));
                }
            }
            let header_len: usize = response_lines.iter().map(|l| l.len() + 1).sum();
            let remaining = TELEGRAM_MSG_LIMIT.saturating_sub(header_len + 2);
            let preview = build_history_preview(&session_data.history, remaining);
            if !preview.is_empty() {
                response_lines.push(String::new());
                response_lines.push(preview);
            }
        } else {
            session.session_id = None;
            session.current_path = Some(canonical_path.clone());
            session.history.clear();

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ▶ Session started: {canonical_path}");
            response_lines.push(format!("Session started at `{}`.", canonical_path));
            // Show workspace ID shortcut if this is a workspace directory
            if let Some(folder_name) = std::path::Path::new(&canonical_path).file_name().and_then(|n| n.to_str()) {
                if is_workspace_id(folder_name) {
                    response_lines.push(format!("Use /{} to resume this session.", folder_name));
                }
            }
        }
    }

    // Persist chat_id → path mapping for auto-restore after restart
    {
        let mut data = state.lock().await;
        data.settings.last_sessions.insert(chat_id.0.to_string(), canonical_path);
        save_bot_settings(token, &data.settings);
    }

    let response_text = response_lines.join("\n");
    let html = markdown_to_telegram_html(&response_text);
    send_long_message(bot, chat_id, &html, Some(ParseMode::Html), state).await?;

    Ok(())
}

/// Build a history preview code block that fits within the given byte budget.
/// Items are shown oldest-first (most recent at bottom), filling from the bottom up.
fn build_history_preview(history: &[HistoryItem], budget: usize) -> String {
    if history.is_empty() {
        return String::new();
    }
    let code_block_overhead = "```\n".len() + "\n```".len(); // 8 bytes
    if budget <= code_block_overhead + 10 {
        return String::new();
    }
    let content_budget = budget - code_block_overhead;

    // Build lines from newest to oldest, stop when budget exhausted
    let mut collected: Vec<String> = Vec::new();
    let mut used = 0;
    for item in history.iter().rev() {
        let prefix = match item.item_type {
            HistoryType::User => "👤",
            HistoryType::Assistant => "🤖",
            HistoryType::Error => "❌",
            HistoryType::System => "⚙️",
            HistoryType::ToolUse => "🔧",
            HistoryType::ToolResult => "📋",
        };
        let line = format!("{} {}", prefix, item.content);
        let line_len = line.len() + 1; // +1 for newline
        if used + line_len > content_budget {
            break;
        }
        collected.push(line);
        used += line_len;
    }
    if collected.is_empty() {
        return String::new();
    }
    collected.reverse();
    format!("```\n{}\n```", collected.join("\n"))
}

/// Check if a string is a valid 8-character workspace ID (e.g. "B4E9451D" or "k3m9x2ab")
fn is_workspace_id(s: &str) -> bool {
    s.len() == 8 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Handle /WORKSPACE_ID command - resume a workspace session by its ID
async fn handle_workspace_resume(
    bot: &Bot,
    chat_id: ChatId,
    workspace_id: &str,
    state: &SharedState,
    token: &str,
) -> ResponseResult<()> {
    let Some(home) = dirs::home_dir() else {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Error: cannot determine home directory.")
            .await)?;
        return Ok(());
    };

    let workspace_path = home.join(".cokacdir").join("workspace").join(workspace_id);
    if !workspace_path.exists() || !workspace_path.is_dir() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, format!("Error: no workspace found for '{}'.", workspace_id))
            .await)?;
        return Ok(());
    }

    let canonical_path = workspace_path.canonicalize()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| workspace_path.display().to_string());

    let existing = load_existing_session(&canonical_path);

    let mut response_lines = Vec::new();

    {
        let mut data = state.lock().await;
        let session = data.sessions.entry(chat_id).or_insert_with(|| ChatSession {
            session_id: None,
            current_path: None,
            history: Vec::new(),
            pending_uploads: Vec::new(),
            cleared: false,
        });

        if let Some((session_data, _)) = &existing {
            session.session_id = Some(session_data.session_id.clone());
            session.current_path = Some(canonical_path.clone());
            session.history = session_data.history.clone();

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ▶ Workspace session restored: {workspace_id} → {canonical_path}");
            response_lines.push(format!("Session restored at `{}`.", canonical_path));

            let header_len: usize = response_lines.iter().map(|l| l.len() + 1).sum();
            let remaining = TELEGRAM_MSG_LIMIT.saturating_sub(header_len + 2);
            let preview = build_history_preview(&session_data.history, remaining);
            if !preview.is_empty() {
                response_lines.push(String::new());
                response_lines.push(preview);
            }
        } else {
            // Workspace exists but no session — start a new session there
            session.session_id = None;
            session.current_path = Some(canonical_path.clone());
            session.history.clear();

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ▶ Workspace session started: {workspace_id} → {canonical_path}");
            response_lines.push(format!("Session started at `{}`.", canonical_path));
        }
    }

    // Persist chat_id → path mapping for auto-restore after restart
    {
        let mut data = state.lock().await;
        data.settings.last_sessions.insert(chat_id.0.to_string(), canonical_path);
        save_bot_settings(token, &data.settings);
    }

    let response_text = response_lines.join("\n");
    let html = markdown_to_telegram_html(&response_text);
    send_long_message(bot, chat_id, &html, Some(ParseMode::Html), state).await?;

    Ok(())
}

/// Handle /clear command
async fn handle_clear_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    // Cancel in-progress AI request if any
    let cancel_token = {
        let data = state.lock().await;
        data.cancel_tokens.get(&chat_id).cloned()
    };
    if let Some(token) = cancel_token {
        token.cancelled.store(true, Ordering::Relaxed);
        if let Ok(guard) = token.child_pid.lock() {
            if let Some(pid) = *guard {
                #[cfg(unix)]
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM); }
            }
        }
    }

    let current_path = {
        let mut data = state.lock().await;
        let path = data.sessions.get(&chat_id).and_then(|s| s.current_path.clone());
        if let Some(session) = data.sessions.get_mut(&chat_id) {
            session.session_id = None;
            session.history.clear();
            session.pending_uploads.clear();
            session.cleared = true;
        }
        data.cancel_tokens.remove(&chat_id);
        data.stop_message_ids.remove(&chat_id);
        path
    };

    // Delete session file from disk so session_id is completely forgotten
    if let Some(ref path) = current_path {
        if let Some(sessions_dir) = crate::ui::ai_screen::ai_sessions_dir() {
            if let Ok(entries) = fs::read_dir(&sessions_dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let file_path = entry.path();
                    if file_path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(content) = fs::read_to_string(&file_path) {
                            if let Ok(session_data) = serde_json::from_str::<SessionData>(&content) {
                                if session_data.current_path == *path {
                                    let _ = fs::remove_file(&file_path);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let msg = match current_path {
        Some(ref path) => format!("Session cleared.\n`{}`", path),
        None => "Session cleared.".to_string(),
    };

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, msg)
        .await)?;

    Ok(())
}

/// Handle /pwd command - show current session path
async fn handle_pwd_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let current_path = {
        let data = state.lock().await;
        data.sessions.get(&chat_id).and_then(|s| s.current_path.clone())
    };

    shared_rate_limit_wait(state, chat_id).await;
    match current_path {
        Some(path) => {
            let mut msg = format!("`{}`", path);
            if let Some(folder_name) = std::path::Path::new(&path).file_name().and_then(|n| n.to_str()) {
                if is_workspace_id(folder_name) {
                    msg.push_str(&format!("\nUse /{} to switch back to this session.", folder_name));
                }
            }
            tg!("send_message", bot.send_message(chat_id, msg).await)?
        }
        None => tg!("send_message", bot.send_message(chat_id, "No active session. Use /start <path> first.").await)?,
    };

    Ok(())
}

/// Handle /stop command - cancel in-progress AI request
async fn handle_stop_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let token = {
        let data = state.lock().await;
        data.cancel_tokens.get(&chat_id).cloned()
    };

    match token {
        Some(token) => {
            // Ignore duplicate /stop if already cancelled
            if token.cancelled.load(Ordering::Relaxed) {
                return Ok(());
            }

            // Send immediate feedback to user
            shared_rate_limit_wait(state, chat_id).await;
            let stop_msg = tg!("send_message", bot.send_message(chat_id, "Stopping...").await)?;

            // Store the stop message ID so the polling loop can update it later
            {
                let mut data = state.lock().await;
                data.stop_message_ids.insert(chat_id, stop_msg.id);
            }

            // Set cancellation flag
            token.cancelled.store(true, Ordering::Relaxed);

            // Kill child process directly to unblock reader.lines()
            // When the child dies, its stdout pipe closes → reader returns EOF → blocking thread exits
            if let Ok(guard) = token.child_pid.lock() {
                if let Some(pid) = *guard {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGTERM);
                    }
                }
            }

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ■ Cancel signal sent");
        }
        None => {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, "No active request to stop.")
                .await)?;
        }
    }

    Ok(())
}

/// Handle /down <filepath> - send file to user
async fn handle_down_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
) -> ResponseResult<()> {
    let file_path = text.strip_prefix("/down").unwrap_or("").trim();

    if file_path.is_empty() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Usage: /down <filepath>\nExample: /down /home/kst/file.txt")
            .await)?;
        return Ok(());
    }

    // Resolve relative path using current session path
    let resolved_path = if Path::new(file_path).is_absolute() {
        file_path.to_string()
    } else {
        let current_path = {
            let data = state.lock().await;
            data.sessions.get(&chat_id).and_then(|s| s.current_path.clone())
        };
        match current_path {
            Some(base) => format!("{}/{}", base.trim_end_matches('/'), file_path),
            None => {
                shared_rate_limit_wait(state, chat_id).await;
                tg!("send_message", bot.send_message(chat_id, "No active session. Use absolute path or /start <path> first.")
                    .await)?;
                return Ok(());
            }
        }
    };

    let path = Path::new(&resolved_path);
    if !path.exists() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, &format!("File not found: {}", resolved_path)).await)?;
        return Ok(());
    }
    if !path.is_file() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, &format!("Not a file: {}", resolved_path)).await)?;
        return Ok(());
    }

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_document", bot.send_document(chat_id, teloxide::types::InputFile::file(path))
        .await)?;

    Ok(())
}

/// Handle file/photo upload - save to current session path
async fn handle_file_upload(
    bot: &Bot,
    chat_id: ChatId,
    msg: &Message,
    state: &SharedState,
) -> ResponseResult<()> {
    // Get current session path
    let current_path = {
        let data = state.lock().await;
        data.sessions.get(&chat_id).and_then(|s| s.current_path.clone())
    };

    let Some(save_dir) = current_path else {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "No active session. Use /start <path> first.")
            .await)?;
        return Ok(());
    };

    // Get file_id and file_name
    let (file_id, file_name) = if let Some(doc) = msg.document() {
        let name = doc.file_name.clone().unwrap_or_else(|| "uploaded_file".to_string());
        (doc.file.id.clone(), name)
    } else if let Some(photos) = msg.photo() {
        // Get the largest photo
        if let Some(photo) = photos.last() {
            let name = format!("photo_{}.jpg", photo.file.unique_id);
            (photo.file.id.clone(), name)
        } else {
            return Ok(());
        }
    } else {
        return Ok(());
    };

    // Download file from Telegram via HTTP
    shared_rate_limit_wait(state, chat_id).await;
    let file = tg!("get_file", bot.get_file(&file_id).await)?;
    let url = format!("https://api.telegram.org/file/bot{}/{}", bot.token(), file.path);
    let buf = match reqwest::get(&url).await {
        Ok(resp) => match resp.bytes().await {
            Ok(bytes) => bytes,
            Err(e) => {
                shared_rate_limit_wait(state, chat_id).await;
                tg!("send_message", bot.send_message(chat_id, &format!("Download failed: {}", e)).await)?;
                return Ok(());
            }
        },
        Err(e) => {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, &format!("Download failed: {}", e)).await)?;
            return Ok(());
        }
    };

    // Save to session path (sanitize file_name to prevent path traversal)
    let safe_name = Path::new(&file_name)
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("uploaded_file"));
    let dest = Path::new(&save_dir).join(safe_name);
    let file_size = buf.len();
    match fs::write(&dest, &buf) {
        Ok(_) => {
            let msg_text = format!("Saved: {}\n({} bytes)", dest.display(), file_size);
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, &msg_text).await)?;
        }
        Err(e) => {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, &format!("Failed to save file: {}", e)).await)?;
            return Ok(());
        }
    }

    // Record upload in session history and pending queue for Claude
    let upload_record = format!(
        "[File uploaded] {} → {} ({} bytes)",
        file_name, dest.display(), file_size
    );
    {
        let mut data = state.lock().await;
        if let Some(session) = data.sessions.get_mut(&chat_id) {
            session.history.push(HistoryItem {
                item_type: HistoryType::User,
                content: upload_record.clone(),
            });
            session.pending_uploads.push(upload_record);
            save_session_to_file(session, &save_dir);
        }
    }

    Ok(())
}

/// Shell command output message type
enum ShellOutput {
    Line(String),
    Done { exit_code: i32 },
    Error(String),
}

/// Handle !command - execute shell command directly with lock/stop/streaming support
async fn handle_shell_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
) -> ResponseResult<()> {
    let cmd_str = text.strip_prefix('!').unwrap_or("").trim();

    if cmd_str.is_empty() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Usage: !<command>\nExample: !mkdir /home/kst/testcode")
            .await)?;
        return Ok(());
    }

    // Get current_path for working directory (default to home directory)
    let working_dir = {
        let data = state.lock().await;
        data.sessions.get(&chat_id)
            .and_then(|s| s.current_path.clone())
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .map(|h| h.display().to_string())
                    .unwrap_or_else(|| "/".to_string())
            })
    };

    // Send placeholder message
    let cmd_display = cmd_str.to_string();
    shared_rate_limit_wait(state, chat_id).await;
    let placeholder = tg!("send_message", bot.send_message(chat_id, format!("Processing <code>{}</code>", html_escape(&cmd_display)))
        .parse_mode(ParseMode::Html).await)?;
    let placeholder_msg_id = placeholder.id;

    // Register cancel token (lock) — must be AFTER placeholder send succeeds,
    // otherwise a failed send leaves the chat permanently locked.
    let cancel_token = Arc::new(CancelToken::new());
    {
        let mut data = state.lock().await;
        data.cancel_tokens.insert(chat_id, cancel_token.clone());
    }

    // Create channel
    let (tx, rx) = mpsc::channel();

    let cmd_owned = cmd_str.to_string();
    let working_dir_clone = working_dir.clone();
    let cancel_token_clone = cancel_token.clone();

    // Spawn blocking thread for shell command execution
    tokio::task::spawn_blocking(move || {
        let child = std::process::Command::new("bash")
            .args(["-c", &cmd_owned])
            .current_dir(&working_dir_clone)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(ShellOutput::Error(format!("Failed to execute: {}", e)));
                return;
            }
        };

        // Store PID for /stop kill
        if let Ok(mut guard) = cancel_token_clone.child_pid.lock() {
            *guard = Some(child.id());
        }

        // Read stderr in a separate thread
        let stderr_handle = child.stderr.take();
        let stderr_thread = std::thread::spawn(move || {
            let mut buf = String::new();
            if let Some(se) = stderr_handle {
                use std::io::BufRead;
                for line in std::io::BufReader::new(se).lines().flatten() {
                    buf.push_str(&line);
                    buf.push('\n');
                }
            }
            buf
        });

        // Read stdout line by line with cancel checks
        if let Some(stdout) = child.stdout.take() {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines().flatten() {
                if cancel_token_clone.cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                let _ = tx.send(ShellOutput::Line(line));
            }
        }

        let stderr_output = stderr_thread.join().unwrap_or_default();
        if !stderr_output.is_empty() {
            let _ = tx.send(ShellOutput::Line(format!("[stderr]\n{}", stderr_output.trim_end())));
        }

        let status = child.wait();
        let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        let _ = tx.send(ShellOutput::Done { exit_code });
    });

    // Spawn polling loop (same pattern as AI streaming)
    let bot_owned = bot.clone();
    let state_owned = state.clone();
    let cmd_display_owned = cmd_display.clone();
    tokio::spawn(async move {
        const SPINNER: &[&str] = &[
            "🕐 P",           "🕑 Pr",          "🕒 Pro",
            "🕓 Proc",        "🕔 Proce",       "🕕 Proces",
            "🕖 Process",     "🕗 Processi",    "🕘 Processin",
            "🕙 Processing",  "🕚 Processing.", "🕛 Processing..",
        ];
        let mut full_output = String::new();
        let mut last_edit_text = String::new();
        let mut done = false;
        let mut cancelled = false;
        let mut spin_idx: usize = 0;
        let mut exit_code: i32 = -1;
        let mut spawn_error: Option<String> = None;

        let polling_time_ms = {
            let data = state_owned.lock().await;
            data.polling_time_ms
        };
        let mut queue_done = false;
        let mut response_rendered = false;
        while !done || !queue_done {
            // Check cancel
            if cancel_token.cancelled.load(Ordering::Relaxed) {
                if !done { cancelled = true; }
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(polling_time_ms)).await;

            if cancel_token.cancelled.load(Ordering::Relaxed) {
                if !done { cancelled = true; }
                break;
            }

            // Drain channel
            if !done {
                loop {
                    match rx.try_recv() {
                        Ok(msg) => match msg {
                            ShellOutput::Line(line) => {
                                if !full_output.is_empty() {
                                    full_output.push('\n');
                                }
                                full_output.push_str(&line);
                            }
                            ShellOutput::Done { exit_code: code } => {
                                exit_code = code;
                                done = true;
                            }
                            ShellOutput::Error(e) => {
                                spawn_error = Some(e);
                                done = true;
                            }
                        },
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            done = true;
                            break;
                        }
                    }
                }

                // Update placeholder with spinner
                if !done {
                    let indicator = SPINNER[spin_idx % SPINNER.len()];
                    spin_idx += 1;

                    let display_text = format!("Processing <code>{}</code>\n\n{}", html_escape(&cmd_display_owned), indicator);

                    if display_text != last_edit_text {
                        shared_rate_limit_wait(&state_owned, chat_id).await;
                        let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &display_text)
                            .parse_mode(ParseMode::Html).await);
                        last_edit_text = display_text;
                    } else {
                        shared_rate_limit_wait(&state_owned, chat_id).await;
                        let _ = tg!("send_chat_action", bot_owned.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await);
                    }
                }
            }

            // Render final result once
            if done && !response_rendered {
                response_rendered = true;

                if let Some(err) = &spawn_error {
                    // Spawn error - just show error message
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, err).await);
                } else {
                    if !full_output.trim().is_empty() {
                        let file_content = format!("$ {}\n\n{}", cmd_display_owned, full_output);
                        let content_bytes = file_content.len();

                        if content_bytes <= 4000 {
                            // Short output: update placeholder with completion + result in one call
                            let combined = format!("Done <code>{}</code> (exit code: {})\n\n<pre>$ {}\n\n{}</pre>",
                                html_escape(&cmd_display_owned), exit_code,
                                html_escape(&cmd_display_owned), html_escape(full_output.trim()));
                            shared_rate_limit_wait(&state_owned, chat_id).await;
                            if let Err(_) = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &combined)
                                .parse_mode(ParseMode::Html)
                                .await)
                            {
                                let fallback = format!("Done {} (exit code: {})\n\n$ {}\n\n{}",
                                    cmd_display_owned, exit_code, cmd_display_owned, full_output.trim());
                                shared_rate_limit_wait(&state_owned, chat_id).await;
                                let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &fallback).await);
                            }
                        } else {
                            // Long output: update placeholder + send as .txt file
                            let final_msg = format!("Done <code>{}</code> (exit code: {})", html_escape(&cmd_display_owned), exit_code);
                            shared_rate_limit_wait(&state_owned, chat_id).await;
                            let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &final_msg)
                                .parse_mode(ParseMode::Html).await);

                            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                            let tmp_path = format!("/tmp/cokacdir_shell_{}_{}.txt", chat_id.0, timestamp);
                            if std::fs::write(&tmp_path, &file_content).is_ok() {
                                shared_rate_limit_wait(&state_owned, chat_id).await;
                                let _ = tg!("send_document", bot_owned.send_document(
                                    chat_id,
                                    teloxide::types::InputFile::file(std::path::Path::new(&tmp_path)),
                                ).await);
                                let _ = std::fs::remove_file(&tmp_path);
                            }
                        }
                    } else {
                        // No output
                        let final_msg = format!("Done <code>{}</code> (exit code: {})", html_escape(&cmd_display_owned), exit_code);
                        shared_rate_limit_wait(&state_owned, chat_id).await;
                        let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &final_msg)
                            .parse_mode(ParseMode::Html).await);
                    }
                }

                let ts = chrono::Local::now().format("%H:%M:%S");
                println!("  [{ts}] ▶ Shell command completed: !{}", cmd_display_owned);
            }

            // Queue processing
            let queued = process_upload_queue(&bot_owned, chat_id, &state_owned).await;
            if done {
                queue_done = !queued;
            }
        }

        // Post-loop: cancel handling
        if cancelled {
            if let Ok(guard) = cancel_token.child_pid.lock() {
                if let Some(pid) = *guard {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGTERM);
                    }
                }
            }

            shared_rate_limit_wait(&state_owned, chat_id).await;
            let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, "[Stopped]").await);

            let stop_msg_id = {
                let data = state_owned.lock().await;
                data.stop_message_ids.get(&chat_id).cloned()
            };
            if let Some(msg_id) = stop_msg_id {
                shared_rate_limit_wait(&state_owned, chat_id).await;
                let _ = tg!("delete_message", bot_owned.delete_message(chat_id, msg_id).await);
            }

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ■ Shell command stopped: !{}", cmd_display_owned);

            let mut data = state_owned.lock().await;
            data.cancel_tokens.remove(&chat_id);
            data.stop_message_ids.remove(&chat_id);
            return;
        }

        // Clean up stop message if /stop raced with completion
        {
            let mut data = state_owned.lock().await;
            if let Some(msg_id) = data.stop_message_ids.remove(&chat_id) {
                drop(data);
                shared_rate_limit_wait(&state_owned, chat_id).await;
                let _ = tg!("delete_message", bot_owned.delete_message(chat_id, msg_id).await);
            }
        }

        // Release lock
        {
            let mut data = state_owned.lock().await;
            data.cancel_tokens.remove(&chat_id);
        }
    });

    Ok(())
}

/// Handle /availabletools command - show all available tools
async fn handle_availabletools_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let mut msg = String::from("<b>Available Tools</b>\n\n");

    for &(name, desc, destructive) in ALL_TOOLS {
        let badge = risk_badge(destructive);
        if badge.is_empty() {
            msg.push_str(&format!("<code>{}</code> — {}\n", html_escape(name), html_escape(desc)));
        } else {
            msg.push_str(&format!("<code>{}</code> {} — {}\n", html_escape(name), badge, html_escape(desc)));
        }
    }
    msg.push_str(&format!("\n{} = destructive\nTotal: {}", risk_badge(true), ALL_TOOLS.len()));

    send_long_message(bot, chat_id, &msg, Some(ParseMode::Html), state).await?;

    Ok(())
}

/// Handle /allowedtools command - show current allowed tools list
async fn handle_allowedtools_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
) -> ResponseResult<()> {
    let tools = {
        let data = state.lock().await;
        get_allowed_tools(&data.settings, chat_id)
    };

    let mut msg = String::from("<b>Allowed Tools</b>\n\n");
    for tool in &tools {
        let (desc, destructive) = tool_info(tool);
        let badge = risk_badge(destructive);
        if badge.is_empty() {
            msg.push_str(&format!("<code>{}</code> — {}\n", html_escape(tool), html_escape(desc)));
        } else {
            msg.push_str(&format!("<code>{}</code> {} — {}\n", html_escape(tool), badge, html_escape(desc)));
        }
    }
    msg.push_str(&format!("\n{} = destructive\nTotal: {}", risk_badge(true), tools.len()));

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, &msg)
        .parse_mode(ParseMode::Html)
        .await)?;

    Ok(())
}

/// Handle /setpollingtime command - set Telegram API polling interval
async fn handle_setpollingtime_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
) -> ResponseResult<()> {
    let arg = text.strip_prefix("/setpollingtime").unwrap_or("").trim();

    if arg.is_empty() {
        let current = {
            let data = state.lock().await;
            data.polling_time_ms
        };
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, format!("Current polling time: {}ms\nUsage: /setpollingtime <ms>\nMinimum: 2500ms", current))
            .await)?;
        return Ok(());
    }

    let value: u64 = match arg.parse() {
        Ok(v) => v,
        Err(_) => {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, "Invalid number. Usage: /setpollingtime <ms>\nExample: /setpollingtime 3000")
                .await)?;
            return Ok(());
        }
    };

    if value < 2500 {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Minimum polling time is 2500ms.")
            .await)?;
        return Ok(());
    }

    // Update in-memory state
    {
        let mut data = state.lock().await;
        data.polling_time_ms = value;
    }

    // Save to settings.json
    if let Ok(mut app_settings) = crate::config::Settings::load_with_error() {
        app_settings.telegram_polling_time = value;
        let _ = app_settings.save();
    }

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, format!("✅ Polling time set to {}ms", value))
        .await)?;

    Ok(())
}

/// Handle /debug command - toggle all debug logging (Telegram API, Claude, cron)
async fn handle_debug_command(
    bot: &Bot,
    chat_id: ChatId,
    state: &SharedState,
    token: &str,
) -> ResponseResult<()> {
    let prev = TG_DEBUG.load(Ordering::Relaxed);
    let next = !prev;
    TG_DEBUG.store(next, Ordering::Relaxed);
    crate::services::claude::DEBUG_ENABLED.store(next, Ordering::Relaxed);
    {
        let mut data = state.lock().await;
        data.settings.debug = next;
        save_bot_settings(token, &data.settings);
    }
    let status = if next { "ON" } else { "OFF" };
    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, format!("🔍 Debug logging: {status}"))
        .await)?;
    Ok(())
}


/// Handle /allowed command - add/remove tools
/// Usage: /allowed +toolname  (add)
///        /allowed -toolname  (remove)
///        /allowed +tool1 -tool2 +tool3  (multiple)
async fn handle_allowed_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
    token: &str,
) -> ResponseResult<()> {
    let arg = text.strip_prefix("/allowed").unwrap_or("").trim();

    if arg.is_empty() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Usage:\n/allowed +toolname — Add a tool\n/allowed -toolname — Remove a tool\n/allowed +tool1 -tool2 — Multiple at once\n/allowedtools — Show current list")
            .await)?;
        return Ok(());
    }

    // Skip if argument starts with "tools" (that's /allowedtools handled separately)
    if arg.starts_with("tools") {
        // This shouldn't happen due to routing order, but just in case
        return handle_allowedtools_command(bot, chat_id, state).await;
    }

    // Parse multiple +name / -name tokens
    let mut operations: Vec<(char, String)> = Vec::new();
    for token_str in arg.split_whitespace() {
        if let Some(name) = token_str.strip_prefix('+') {
            let name = name.trim();
            if !name.is_empty() {
                operations.push(('+', normalize_tool_name(name)));
            }
        } else if let Some(name) = token_str.strip_prefix('-') {
            let name = name.trim();
            if !name.is_empty() {
                operations.push(('-', normalize_tool_name(name)));
            }
        }
    }

    if operations.is_empty() {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Use +toolname to add or -toolname to remove.\nExample: /allowed +Bash -Edit")
            .await)?;
        return Ok(());
    }

    let response_msg = {
        let mut data = state.lock().await;
        let chat_key = chat_id.0.to_string();
        // Ensure this chat has its own tool list (initialize from defaults if missing)
        if !data.settings.allowed_tools.contains_key(&chat_key) {
            let defaults: Vec<String> = DEFAULT_ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect();
            data.settings.allowed_tools.insert(chat_key.clone(), defaults);
        }
        let tools = data.settings.allowed_tools.get_mut(&chat_key).unwrap();
        let mut results: Vec<String> = Vec::new();
        let mut changed = false;
        for (op, tool_name) in &operations {
            match op {
                '+' => {
                    if tools.iter().any(|t| t == tool_name) {
                        results.push(format!("<code>{}</code> already in list", html_escape(tool_name)));
                    } else {
                        tools.push(tool_name.clone());
                        changed = true;
                        results.push(format!("✅ <code>{}</code>", html_escape(tool_name)));
                    }
                }
                '-' => {
                    let before_len = tools.len();
                    tools.retain(|t| t != tool_name);
                    if tools.len() < before_len {
                        changed = true;
                        results.push(format!("❌ <code>{}</code>", html_escape(tool_name)));
                    } else {
                        results.push(format!("<code>{}</code> not in list", html_escape(tool_name)));
                    }
                }
                _ => unreachable!(),
            }
        }
        if changed {
            save_bot_settings(token, &data.settings);
        }
        results.join("\n")
    };

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, &response_msg)
        .parse_mode(ParseMode::Html)
        .await)?;

    Ok(())
}

/// Handle /public command - toggle public access for group chats
async fn handle_public_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
    token: &str,
    is_group_chat: bool,
    is_owner: bool,
) -> ResponseResult<()> {
    if !is_group_chat {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "This command is only available in group chats.")
            .await)?;
        return Ok(());
    }

    if !is_owner {
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Only the bot owner can change public access settings.")
            .await)?;
        return Ok(());
    }

    let arg = text.strip_prefix("/public").unwrap_or("").trim().to_lowercase();
    let chat_key = chat_id.0.to_string();

    let response_msg = match arg.as_str() {
        "on" => {
            let mut data = state.lock().await;
            data.settings.as_public_for_group_chat.insert(chat_key, true);
            save_bot_settings(token, &data.settings);
            "✅ Public access <b>enabled</b> for this group.\nAll members can now use the bot.".to_string()
        }
        "off" => {
            let mut data = state.lock().await;
            data.settings.as_public_for_group_chat.remove(&chat_key);
            save_bot_settings(token, &data.settings);
            "❌ Public access <b>disabled</b> for this group.\nOnly the owner can use the bot.".to_string()
        }
        "" => {
            let data = state.lock().await;
            let is_public = data.settings.as_public_for_group_chat.get(&chat_key).copied().unwrap_or(false);
            let status = if is_public { "enabled" } else { "disabled" };
            format!(
                "Public access is currently <b>{}</b> for this group.\n\n\
                 <code>/public on</code> — Allow all members\n\
                 <code>/public off</code> — Owner only",
                status
            )
        }
        _ => {
            "Usage:\n<code>/public on</code> — Allow all group members\n<code>/public off</code> — Owner only".to_string()
        }
    };

    shared_rate_limit_wait(state, chat_id).await;
    tg!("send_message", bot.send_message(chat_id, &response_msg)
        .parse_mode(ParseMode::Html)
        .await)?;

    Ok(())
}

/// Resolve a model alias to pass through to Claude CLI.
/// Only exact matches from the allowed list are accepted.
fn resolve_model_name(name: &str) -> Option<String> {
    match name {
        "sonnet" | "opus" | "haiku" |
        "sonnet[1m]" | "opus[1m]" | "haiku[1m]" => Some(name.to_string()),
        _ => None,
    }
}

/// Format a model name for display.
fn display_model_name(model: &str) -> String {
    model.to_string()
}

/// Handle /model command
async fn handle_model_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    state: &SharedState,
    token: &str,
) -> ResponseResult<()> {
    let arg = text.strip_prefix("/model").unwrap_or("").trim();

    if arg.is_empty() {
        // Show current model
        let current = {
            let data = state.lock().await;
            get_model(&data.settings, chat_id)
        };
        let msg = match current {
            Some(m) => format!("Current model: <b>{}</b>", display_model_name(&m)),
            None => "Current model: <b>default</b>".to_string(),
        };
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, msg)
            .parse_mode(ParseMode::Html)
            .await)?;
        return Ok(());
    }

    // Reset to default
    if arg.eq_ignore_ascii_case("default") || arg.eq_ignore_ascii_case("reset") {
        {
            let mut data = state.lock().await;
            data.settings.models.remove(&chat_id.0.to_string());
            save_bot_settings(token, &data.settings);
        }
        shared_rate_limit_wait(state, chat_id).await;
        tg!("send_message", bot.send_message(chat_id, "Model reset to <b>default</b>.")
            .parse_mode(ParseMode::Html)
            .await)?;
        return Ok(());
    }

    // Set model
    match resolve_model_name(arg) {
        Some(model_id) => {
            let display = display_model_name(&model_id);
            {
                let mut data = state.lock().await;
                data.settings.models.insert(chat_id.0.to_string(), model_id);
                save_bot_settings(token, &data.settings);
            }
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, format!("Model set to <b>{display}</b>."))
                .parse_mode(ParseMode::Html)
                .await)?;
        }
        None => {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id,
                "Unknown model. Available:\n\
                 <code>/model sonnet</code>\n\
                 <code>/model opus</code>\n\
                 <code>/model haiku</code>\n\
                 <code>/model sonnet[1m]</code>\n\
                 <code>/model opus[1m]</code>\n\
                 <code>/model haiku[1m]</code>\n\
                 <code>/model default</code> — reset to default")
                .parse_mode(ParseMode::Html)
                .await)?;
        }
    }

    Ok(())
}

/// Handle regular text messages - send to Claude AI
async fn handle_text_message(
    bot: &Bot,
    chat_id: ChatId,
    user_text: &str,
    state: &SharedState,
) -> ResponseResult<()> {
    // Get session info, allowed tools, model, and pending uploads (drop lock before any await)
    let (session_info, allowed_tools, pending_uploads, model) = {
        let mut data = state.lock().await;
        let info = data.sessions.get(&chat_id).and_then(|session| {
            session.current_path.as_ref().map(|_| {
                (session.session_id.clone(), session.current_path.clone().unwrap_or_default())
            })
        });
        let tools = get_allowed_tools(&data.settings, chat_id);
        let mdl = get_model(&data.settings, chat_id);
        // Drain pending uploads so they are sent to Claude exactly once
        let uploads = data.sessions.get_mut(&chat_id)
            .map(|s| {
                s.cleared = false; // Reset cleared flag on new message
                std::mem::take(&mut s.pending_uploads)
            })
            .unwrap_or_default();
        (info, tools, uploads, mdl)
    };

    let (session_id, current_path) = match session_info {
        Some(info) => info,
        None => {
            shared_rate_limit_wait(state, chat_id).await;
            tg!("send_message", bot.send_message(chat_id, "No active session. Use /start <path> first.")
                .await)?;
            return Ok(());
        }
    };

    // Note: user message is NOT added to history here.
    // It will be added together with the assistant response in the spawned task,
    // only on successful completion. On cancel, nothing is recorded.

    // Send placeholder message (update shared timestamp so spawned task knows)
    shared_rate_limit_wait(state, chat_id).await;
    let placeholder = tg!("send_message", bot.send_message(chat_id, "...").await)?;
    let placeholder_msg_id = placeholder.id;

    // Sanitize input
    let sanitized_input = ai_screen::sanitize_user_input(user_text);

    // Prepend pending file upload records so Claude knows about recently uploaded files
    let context_prompt = if pending_uploads.is_empty() {
        sanitized_input
    } else {
        let upload_context = pending_uploads.join("\n");
        format!("{}\n\n{}", upload_context, sanitized_input)
    };

    // Build disabled tools notice
    let default_tools: std::collections::HashSet<&str> = DEFAULT_ALLOWED_TOOLS.iter().copied().collect();
    let allowed_set: std::collections::HashSet<&str> = allowed_tools.iter().map(|s| s.as_str()).collect();
    let disabled: Vec<&&str> = default_tools.iter().filter(|t| !allowed_set.contains(**t)).collect();
    let disabled_notice = if disabled.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = disabled.iter().map(|t| **t).collect();
        format!(
            "\n\nDISABLED TOOLS: The following tools have been disabled by the user: {}.\n\
             You MUST NOT attempt to use these tools. \
             If a user's request requires a disabled tool, do NOT proceed with the task. \
             Instead, clearly inform the user which tool is needed and that it is currently disabled. \
             Suggest they re-enable it with: /allowed +ToolName",
            names.join(", ")
        )
    };

    // Build system prompt with sendfile and schedule instructions
    let bot_key_for_prompt = token_hash(bot.token());
    let system_prompt_owned = build_system_prompt(
        "You are chatting with a user through Telegram.",
        &current_path, chat_id.0, &bot_key_for_prompt, &disabled_notice,
        session_id.as_deref(),
    );

    // Create cancel token for this request
    let cancel_token = Arc::new(CancelToken::new());
    {
        let mut data = state.lock().await;
        data.cancel_tokens.insert(chat_id, cancel_token.clone());
    }

    // Create channel for streaming
    let (tx, rx) = mpsc::channel();

    let session_id_clone = session_id.clone();
    let current_path_clone = current_path.clone();
    let cancel_token_clone = cancel_token.clone();

    // Run Claude in a blocking thread
    tokio::task::spawn_blocking(move || {
        let result = claude::execute_command_streaming(
            &context_prompt,
            session_id_clone.as_deref(),
            &current_path_clone,
            tx.clone(),
            Some(&system_prompt_owned),
            Some(&allowed_tools),
            Some(cancel_token_clone),
            model.as_deref(),
            false,
        );

        if let Err(e) = result {
            let _ = tx.send(StreamMessage::Error { message: e, stdout: String::new(), stderr: String::new(), exit_code: None });
        }
    });

    // Spawn the polling loop as a separate task so the handler returns immediately.
    // This allows teloxide's per-chat worker to process subsequent messages (e.g. /stop).
    let bot_owned = bot.clone();
    let state_owned = state.clone();
    let user_text_owned = user_text.to_string();
    tokio::spawn(async move {
        const SPINNER: &[&str] = &[
            "🕐 P",           "🕑 Pr",          "🕒 Pro",
            "🕓 Proc",        "🕔 Proce",       "🕕 Proces",
            "🕖 Process",     "🕗 Processi",    "🕘 Processin",
            "🕙 Processing",  "🕚 Processing.", "🕛 Processing..",
        ];
        let mut full_response = String::new();
        let mut last_edit_text = String::new();
        let mut done = false;
        let mut cancelled = false;
        let mut new_session_id: Option<String> = None;
        let mut spin_idx: usize = 0;
        let mut pending_cokacdir_cmd: Option<String> = None;
        let mut last_tool_name: String = String::new();


        let polling_time_ms = {
            let data = state_owned.lock().await;
            data.polling_time_ms
        };
        let mut queue_done = false;
        let mut response_rendered = false;
        while !done || !queue_done {
            // Check cancel token
            if cancel_token.cancelled.load(Ordering::Relaxed) {
                if !done { cancelled = true; }
                break;
            }

            // Sleep as polling interval (without reserving a rate limit slot)
            tokio::time::sleep(tokio::time::Duration::from_millis(polling_time_ms)).await;

            // Check cancel token again after sleep
            if cancel_token.cancelled.load(Ordering::Relaxed) {
                if !done { cancelled = true; }
                break;
            }

            // === Phase 1: AI streaming (while !done) ===
            if !done {
                // Drain all available messages
                loop {
                    match rx.try_recv() {
                        Ok(msg) => {
                            match msg {
                                StreamMessage::Init { session_id: sid } => {
                                    new_session_id = Some(sid);
                                }
                                StreamMessage::Text { content } => {
                                    full_response.push_str(&content);
                                }
                                StreamMessage::ToolUse { name, input } => {
                                    pending_cokacdir_cmd = detect_cokacdir_command(&name, &input);
                                    last_tool_name = name.clone();
                                    let summary = format_tool_input(&name, &input);
                                    let ts = chrono::Local::now().format("%H:%M:%S");
                                    println!("  [{ts}]   ⚙ {name}: {summary}");
                                    if pending_cokacdir_cmd.is_none() {
                                        if name == "Bash" {
                                            full_response.push_str(&format!("\n\n```\n{}\n```\n", format_bash_command(&input)));
                                        } else {
                                            full_response.push_str(&format!("\n\n⚙️ {}\n", summary));
                                        }
                                    }
                                }
                                StreamMessage::ToolResult { content, is_error } => {
                                    if let Some(cmd) = pending_cokacdir_cmd.take() {
                                        let ts = chrono::Local::now().format("%H:%M:%S");
                                        println!("  [{ts}]   ↩ cokacdir --{cmd}: {content}");
                                        let formatted = format_cokacdir_result(&cmd, &content);
                                        if !formatted.is_empty() {
                                            full_response.push_str(&format!("\n{}\n", formatted));
                                        }
                                    } else if is_error {
                                        let ts = chrono::Local::now().format("%H:%M:%S");
                                        println!("  [{ts}]   ✗ Error: {content}");
                                        let truncated = truncate_str(&content, 500);
                                        if truncated.contains('\n') {
                                            full_response.push_str(&format!("\n❌\n```\n{}\n```\n", truncated));
                                        } else {
                                            full_response.push_str(&format!("\n❌ `{}`\n\n", truncated));
                                        }
                                    } else if last_tool_name == "Read" {
                                        full_response.push_str(&format!("\n✅ `{} bytes`\n\n", content.len()));
                                    } else if !content.is_empty() {
                                        let truncated = truncate_str(&content, 300);
                                        if truncated.contains('\n') {
                                            full_response.push_str(&format!("\n```\n{}\n```\n", truncated));
                                        } else {
                                            full_response.push_str(&format!("\n✅ `{}`\n\n", truncated));
                                        }
                                    }
                                }
                                StreamMessage::TaskNotification { summary, .. } => {
                                    if !summary.is_empty() {
                                        full_response.push_str(&format!("\n[Task: {}]\n", summary));
                                    }
                                }
                                StreamMessage::Done { result, session_id: sid } => {
                                    if !result.is_empty() && full_response.is_empty() {
                                        full_response = result;
                                    }
                                    if let Some(s) = sid {
                                        new_session_id = Some(s);
                                    }
                                    done = true;
                                }
                                StreamMessage::Error { message, stdout, stderr, exit_code } => {
                                    let stdout_display = if stdout.is_empty() { "(empty)".to_string() } else { stdout };
                                    let stderr_display = if stderr.is_empty() { "(empty)".to_string() } else { stderr };
                                    let code_display = match exit_code {
                                        Some(c) => c.to_string(),
                                        None => "(unknown)".to_string(),
                                    };
                                    full_response = format!(
                                        "Error: {}\n```\nexit code: {}\n\n[stdout]\n{}\n\n[stderr]\n{}\n```",
                                        message, code_display, stdout_display, stderr_display
                                    );
                                    done = true;
                                }
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            done = true;
                            break;
                        }
                    }
                }

                // Build display text with spinning clock+text indicator appended
                let indicator = SPINNER[spin_idx % SPINNER.len()];
                spin_idx += 1;

                let display_text = if full_response.is_empty() {
                    indicator.to_string()
                } else {
                    let normalized = normalize_empty_lines(&full_response);
                    let truncated = truncate_str(&normalized, TELEGRAM_MSG_LIMIT - 20);
                    format!("{}\n\n{}", truncated, indicator)
                };

                if display_text != last_edit_text && !done {
                    // Rate limit: reserve slot right before the actual API call
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let html_text = markdown_to_telegram_html(&display_text);
                    if let Err(e) = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &html_text)
                        .parse_mode(ParseMode::Html)
                        .await)
                    {
                        let ts = chrono::Local::now().format("%H:%M:%S");
                        println!("  [{ts}]   ⚠ edit_message failed (streaming): {e}");
                    }
                    last_edit_text = display_text;
                } else if !done {
                    // No new content to display, send typing indicator
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let _ = tg!("send_chat_action", bot_owned.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await);
                }
            }

            // === Render final response once when AI completes ===
            if done && !response_rendered {
                response_rendered = true;

                let stop_msg_id = {
                    let data = state_owned.lock().await;
                    data.stop_message_ids.get(&chat_id).cloned()
                };

                // Rate limit before final API call
                shared_rate_limit_wait(&state_owned, chat_id).await;

                // Final response
                if full_response.is_empty() {
                    full_response = "(No response)".to_string();
                }

                let final_response = normalize_empty_lines(&full_response);
                let html_response = markdown_to_telegram_html(&final_response);

                if html_response.len() <= TELEGRAM_MSG_LIMIT {
                    if let Err(e) = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &html_response)
                        .parse_mode(ParseMode::Html)
                        .await)
                    {
                        let ts = chrono::Local::now().format("%H:%M:%S");
                        println!("  [{ts}]   ⚠ edit_message failed (HTML): {e}");
                        shared_rate_limit_wait(&state_owned, chat_id).await;
                        let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &final_response)
                            .await);
                    }
                } else {
                    let send_result = send_long_message(&bot_owned, chat_id, &html_response, Some(ParseMode::Html), &state_owned).await;
                    match send_result {
                        Ok(_) => {
                            shared_rate_limit_wait(&state_owned, chat_id).await;
                            let _ = tg!("delete_message", bot_owned.delete_message(chat_id, placeholder_msg_id).await);
                        }
                        Err(e) => {
                            let ts = chrono::Local::now().format("%H:%M:%S");
                            println!("  [{ts}]   ⚠ send_long_message failed (HTML): {e}");
                            let fallback_result = send_long_message(&bot_owned, chat_id, &final_response, None, &state_owned).await;
                            match fallback_result {
                                Ok(_) => {
                                    shared_rate_limit_wait(&state_owned, chat_id).await;
                                    let _ = tg!("delete_message", bot_owned.delete_message(chat_id, placeholder_msg_id).await);
                                }
                                Err(e2) => {
                                    println!("  [{ts}]   ⚠ send_long_message failed (plain): {e2}");
                                    shared_rate_limit_wait(&state_owned, chat_id).await;
                                    let truncated = truncate_str(&final_response, TELEGRAM_MSG_LIMIT);
                                    let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &truncated)
                                        .await);
                                }
                            }
                        }
                    }
                }

                // Clean up leftover "Stopping..." message if /stop raced with normal completion
                if let Some(msg_id) = stop_msg_id {
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let _ = tg!("delete_message", bot_owned.delete_message(chat_id, msg_id).await);
                }

                // Update session state
                {
                    let mut data = state_owned.lock().await;
                    if let Some(session) = data.sessions.get_mut(&chat_id) {
                        if !session.cleared {
                            if let Some(sid) = new_session_id.take() {
                                session.session_id = Some(sid);
                            }
                            session.history.push(HistoryItem {
                                item_type: HistoryType::User,
                                content: user_text_owned.clone(),
                            });
                            session.history.push(HistoryItem {
                                item_type: HistoryType::Assistant,
                                content: final_response,
                            });
                            save_session_to_file(session, &current_path);
                        }
                    }
                }

                let ts = chrono::Local::now().format("%H:%M:%S");
                println!("  [{ts}] ▶ Response sent");
            }

            // === Queue processing (both during streaming and after done) ===
            let queued = process_upload_queue(&bot_owned, chat_id, &state_owned).await;
            if done {
                queue_done = !queued;
            }
        }

        // === Post-loop: cancelled handling or lock release ===
        if cancelled {
            if let Ok(guard) = cancel_token.child_pid.lock() {
                if let Some(pid) = *guard {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGTERM);
                    }
                }
            }

            let stopped_response = if full_response.trim().is_empty() {
                "[Stopped]".to_string()
            } else {
                let normalized = normalize_empty_lines(&full_response);
                format!("{}\n\n[Stopped]", normalized)
            };

            shared_rate_limit_wait(&state_owned, chat_id).await;

            let html_stopped = markdown_to_telegram_html(&stopped_response);
            if html_stopped.len() <= TELEGRAM_MSG_LIMIT {
                if let Err(e) = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &html_stopped)
                    .parse_mode(ParseMode::Html)
                    .await)
                {
                    let ts_err = chrono::Local::now().format("%H:%M:%S");
                    println!("  [{ts_err}]   ⚠ edit_message failed (stopped/HTML): {e}");
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &stopped_response)
                        .await);
                }
            } else {
                let send_result = send_long_message(&bot_owned, chat_id, &html_stopped, Some(ParseMode::Html), &state_owned).await;
                match send_result {
                    Ok(_) => {
                        shared_rate_limit_wait(&state_owned, chat_id).await;
                        let _ = tg!("delete_message", bot_owned.delete_message(chat_id, placeholder_msg_id).await);
                    }
                    Err(e) => {
                        let ts_err = chrono::Local::now().format("%H:%M:%S");
                        println!("  [{ts_err}]   ⚠ send_long_message failed (stopped/HTML): {e}");
                        let fallback = send_long_message(&bot_owned, chat_id, &stopped_response, None, &state_owned).await;
                        match fallback {
                            Ok(_) => {
                                shared_rate_limit_wait(&state_owned, chat_id).await;
                                let _ = tg!("delete_message", bot_owned.delete_message(chat_id, placeholder_msg_id).await);
                            }
                            Err(_) => {
                                shared_rate_limit_wait(&state_owned, chat_id).await;
                                let truncated = truncate_str(&stopped_response, TELEGRAM_MSG_LIMIT);
                                let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &truncated)
                                    .await);
                            }
                        }
                    }
                }
            }

            let stop_msg_id = {
                let data = state_owned.lock().await;
                data.stop_message_ids.get(&chat_id).cloned()
            };
            if let Some(msg_id) = stop_msg_id {
                shared_rate_limit_wait(&state_owned, chat_id).await;
                let _ = tg!("delete_message", bot_owned.delete_message(chat_id, msg_id).await);
            }

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ■ Stopped");

            let mut data = state_owned.lock().await;
            if let Some(session) = data.sessions.get_mut(&chat_id) {
                if !session.cleared {
                    if let Some(sid) = new_session_id {
                        session.session_id = Some(sid);
                    }
                    session.history.push(HistoryItem {
                        item_type: HistoryType::User,
                        content: user_text_owned,
                    });
                    session.history.push(HistoryItem {
                        item_type: HistoryType::Assistant,
                        content: stopped_response,
                    });
                    save_session_to_file(session, &current_path);
                }
            }
            data.cancel_tokens.remove(&chat_id);
            data.stop_message_ids.remove(&chat_id);
            return;
        }

        // Clean up "Stopping..." message if /stop was sent during queue drain
        {
            let mut data = state_owned.lock().await;
            if let Some(msg_id) = data.stop_message_ids.remove(&chat_id) {
                drop(data);
                shared_rate_limit_wait(&state_owned, chat_id).await;
                let _ = tg!("delete_message", bot_owned.delete_message(chat_id, msg_id).await);
            }
        }

        // Release lock: allow new messages for this chat
        {
            let mut data = state_owned.lock().await;
            data.cancel_tokens.remove(&chat_id);
        }
    });

    Ok(())
}

/// Load existing session from ai_sessions directory matching the given path
fn load_existing_session(current_path: &str) -> Option<(SessionData, std::time::SystemTime)> {
    let sessions_dir = ai_screen::ai_sessions_dir()?;

    if !sessions_dir.exists() {
        return None;
    }

    let mut matching_session: Option<(SessionData, std::time::SystemTime)> = None;

    if let Ok(entries) = fs::read_dir(&sessions_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(session_data) = serde_json::from_str::<SessionData>(&content) {
                        if session_data.current_path == current_path {
                            if let Ok(metadata) = path.metadata() {
                                if let Ok(modified) = metadata.modified() {
                                    match &matching_session {
                                        None => matching_session = Some((session_data, modified)),
                                        Some((_, latest_time)) if modified > *latest_time => {
                                            matching_session = Some((session_data, modified));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    matching_session
}

/// Save session to file in the ai_sessions directory
fn save_session_to_file(session: &ChatSession, current_path: &str) {
    let Some(ref session_id) = session.session_id else {
        return;
    };

    if session.history.is_empty() {
        return;
    }

    let Some(sessions_dir) = ai_screen::ai_sessions_dir() else {
        return;
    };

    if fs::create_dir_all(&sessions_dir).is_err() {
        return;
    }

    // Filter out system messages
    let saveable_history: Vec<HistoryItem> = session.history.iter()
        .filter(|item| !matches!(item.item_type, HistoryType::System))
        .cloned()
        .collect();

    if saveable_history.is_empty() {
        return;
    }

    let session_data = SessionData {
        session_id: session_id.clone(),
        history: saveable_history,
        current_path: current_path.to_string(),
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    // Security: whitelist session_id to alphanumeric, hyphens, underscores only
    if !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return;
    }

    let file_path = sessions_dir.join(format!("{}.json", session_id));

    if let Ok(json) = serde_json::to_string_pretty(&session_data) {
        let _ = fs::write(file_path, json);
    }
}

/// Find the largest byte index <= `index` that is a valid UTF-8 char boundary
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        s.len()
    } else {
        let mut i = index;
        while !s.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

/// Process one pending upload queue file for the given chat.
/// Scans ~/.cokacdir/upload_queue/ for .queue files matching the current bot and chat_id,
/// sends the oldest one, and deletes the queue file on success.
/// Returns true if a file was processed (rate limit slot consumed).
async fn process_upload_queue(bot: &Bot, chat_id: ChatId, state: &SharedState) -> bool {
    let queue_dir = match dirs::home_dir() {
        Some(h) => h.join(".cokacdir").join("upload_queue"),
        None => return false,
    };
    if !queue_dir.is_dir() {
        return false;
    }

    let current_key = token_hash(bot.token());

    // Collect and sort queue files by name (timestamp-based, so alphabetical = chronological)
    let mut entries: Vec<std::path::PathBuf> = match fs::read_dir(&queue_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("queue"))
            .collect(),
        Err(_) => return false,
    };
    entries.sort();

    // Find the first entry matching this bot and chat_id
    for entry_path in entries {
        let content = match fs::read_to_string(&entry_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let json: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let file_chat_id = json.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
        let file_key = json.get("key").and_then(|v| v.as_str()).unwrap_or("");
        let file_path = json.get("path").and_then(|v| v.as_str()).unwrap_or("");

        if file_chat_id != chat_id.0 || file_key != current_key || file_path.is_empty() {
            continue;
        }

        let path = std::path::PathBuf::from(file_path);
        if !path.exists() {
            // File no longer exists, remove queue entry
            let _ = fs::remove_file(&entry_path);
            return false;
        }

        // Remove queue file before sending (regardless of send result)
        let _ = fs::remove_file(&entry_path);

        // Rate limit and send
        shared_rate_limit_wait(state, chat_id).await;
        match tg!("send_document", bot.send_document(
            chat_id,
            teloxide::types::InputFile::file(&path),
        ).await) {
            Ok(_) => {
                let ts = chrono::Local::now().format("%H:%M:%S");
                println!("  [{ts}]   📤 Upload sent: {}", file_path);
            }
            Err(e) => {
                let ts = chrono::Local::now().format("%H:%M:%S");
                println!("  [{ts}]   ⚠ Upload failed: {e}");
            }
        }
        return true;
    }

    false
}

/// Acquires the lock briefly to calculate and reserve the next API call slot,
/// then releases the lock and sleeps until the reserved time.
/// This ensures that even concurrent tasks for the same chat maintain 3s gaps.
async fn shared_rate_limit_wait(state: &SharedState, chat_id: ChatId) {
    let sleep_until = {
        let mut data = state.lock().await;
        let min_gap = tokio::time::Duration::from_millis(data.polling_time_ms);
        let last = data.api_timestamps.entry(chat_id).or_insert_with(||
            tokio::time::Instant::now() - tokio::time::Duration::from_secs(10)
        );
        let earliest_next = *last + min_gap;
        let now = tokio::time::Instant::now();
        let target = if earliest_next > now { earliest_next } else { now };
        *last = target; // Reserve this slot
        target
    }; // Mutex released here
    tokio::time::sleep_until(sleep_until).await;
}

/// Send a message that may exceed Telegram's 4096 character limit
/// by splitting it into multiple messages, handling UTF-8 boundaries
/// and unclosed HTML tags (e.g. <pre>) across split points
async fn send_long_message(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    parse_mode: Option<ParseMode>,
    state: &SharedState,
) -> ResponseResult<()> {
    if text.len() <= TELEGRAM_MSG_LIMIT {
        shared_rate_limit_wait(state, chat_id).await;
        let mut req = bot.send_message(chat_id, text);
        if let Some(mode) = parse_mode {
            req = req.parse_mode(mode);
        }
        tg!("send_message", req.await)?;
        return Ok(());
    }

    let is_html = parse_mode.is_some();
    let mut remaining = text;
    let mut in_pre = false;

    while !remaining.is_empty() {
        // Reserve space for tags we may need to add (<pre> + </pre> = 11 bytes)
        let tag_overhead = if is_html && in_pre { 11 } else { 0 };
        let effective_limit = TELEGRAM_MSG_LIMIT.saturating_sub(tag_overhead);

        if remaining.len() <= effective_limit {
            let mut chunk = String::new();
            if is_html && in_pre {
                chunk.push_str("<pre>");
            }
            chunk.push_str(remaining);

            shared_rate_limit_wait(state, chat_id).await;
            let mut req = bot.send_message(chat_id, &chunk);
            if let Some(mode) = parse_mode {
                req = req.parse_mode(mode);
            }
            tg!("send_message", req.await)?;
            break;
        }

        // Find a safe UTF-8 char boundary, then find a newline before it
        let safe_end = floor_char_boundary(remaining, effective_limit);
        let split_at = remaining[..safe_end]
            .rfind('\n')
            .unwrap_or(safe_end);

        let (raw_chunk, rest) = remaining.split_at(split_at);

        let mut chunk = String::new();
        if is_html && in_pre {
            chunk.push_str("<pre>");
        }
        chunk.push_str(raw_chunk);

        // Track unclosed <pre> tags to close/reopen across chunks
        if is_html {
            let last_open = raw_chunk.rfind("<pre>");
            let last_close = raw_chunk.rfind("</pre>");
            in_pre = match (last_open, last_close) {
                (Some(o), Some(c)) => o > c,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => in_pre,
            };
            if in_pre {
                chunk.push_str("</pre>");
            }
        }

        shared_rate_limit_wait(state, chat_id).await;
        let mut req = bot.send_message(chat_id, &chunk);
        if let Some(mode) = parse_mode {
            req = req.parse_mode(mode);
        }
        tg!("send_message", req.await)?;

        // Skip the newline character at the split point
        remaining = rest.strip_prefix('\n').unwrap_or(rest);
    }

    Ok(())
}

/// Normalize consecutive empty lines to maximum of one
fn normalize_empty_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_empty = false;

    for line in s.lines() {
        let is_empty = line.is_empty();
        if is_empty {
            if !prev_was_empty {
                result.push('\n');
            }
            prev_was_empty = true;
        } else {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
            prev_was_empty = false;
        }
    }

    result
}

/// Escape special HTML characters for Telegram HTML parse mode
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncate a string to max_len bytes, cutting at a safe UTF-8 char and line boundary
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }

    let safe_end = floor_char_boundary(s, max_len);
    let truncated = &s[..safe_end];
    if let Some(pos) = truncated.rfind('\n') {
        truncated[..pos].to_string()
    } else {
        truncated.to_string()
    }
}

/// Convert standard markdown to Telegram-compatible HTML
fn markdown_to_telegram_html(md: &str) -> String {
    let lines: Vec<&str> = md.lines().collect();
    let mut result = String::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim_start();

        // Fenced code block
        if trimmed.starts_with("```") {
            let mut code_lines = Vec::new();
            i += 1; // skip opening ```
            while i < lines.len() {
                if lines[i].trim_start().starts_with("```") {
                    break;
                }
                code_lines.push(lines[i]);
                i += 1;
            }
            let code = code_lines.join("\n");
            if !code.is_empty() {
                result.push_str(&format!("<pre>{}</pre>", html_escape(code.trim_end())));
            }
            result.push('\n');
            i += 1; // skip closing ```
            continue;
        }

        // Heading (# ~ ######)
        if let Some(rest) = strip_heading(trimmed) {
            result.push_str(&format!("<b>{}</b>", convert_inline(&html_escape(rest))));
            result.push('\n');
            i += 1;
            continue;
        }

        // Unordered list (- or *)
        if trimmed.starts_with("- ") {
            result.push_str(&format!("• {}", convert_inline(&html_escape(&trimmed[2..]))));
            result.push('\n');
            i += 1;
            continue;
        }
        if trimmed.starts_with("* ") && !trimmed.starts_with("**") {
            result.push_str(&format!("• {}", convert_inline(&html_escape(&trimmed[2..]))));
            result.push('\n');
            i += 1;
            continue;
        }

        // Regular line
        result.push_str(&convert_inline(&html_escape(lines[i])));
        result.push('\n');
        i += 1;
    }

    result.trim_end().to_string()
}

/// Strip markdown heading prefix (# ~ ######), return remaining text
fn strip_heading(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches('#');
    // Must have consumed at least one # and be followed by a space
    if trimmed.len() < line.len() && trimmed.starts_with(' ') {
        let hashes = line.len() - trimmed.len();
        if hashes <= 6 {
            return Some(trimmed.trim_start());
        }
    }
    None
}

/// Convert inline markdown elements (bold, italic, code) in already HTML-escaped text
fn convert_inline(text: &str) -> String {
    // Process inline code first to protect content from further conversion
    let mut result = String::new();
    let mut remaining = text;

    // Split by inline code spans: `...`
    loop {
        if let Some(start) = remaining.find('`') {
            let after_start = &remaining[start + 1..];
            if let Some(end) = after_start.find('`') {
                // Found a complete inline code span
                let before = &remaining[..start];
                let code_content = &after_start[..end];
                result.push_str(&convert_bold_italic(before));
                result.push_str(&format!("<code>{}</code>", code_content));
                remaining = &after_start[end + 1..];
                continue;
            }
        }
        // No more inline code spans
        result.push_str(&convert_bold_italic(remaining));
        break;
    }

    result
}

/// Convert bold (**...**) and italic (*...*) in text
fn convert_bold_italic(text: &str) -> String {
    let mut result = String::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Bold: **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing_marker(&chars, i + 2, &['*', '*']) {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str(&format!("<b>{}</b>", inner));
                i = end + 2;
                continue;
            }
        }
        // Italic: *...*
        if chars[i] == '*' {
            if let Some(end) = find_closing_single(&chars, i + 1, '*') {
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&format!("<i>{}</i>", inner));
                i = end + 1;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Find closing double marker (e.g., **) starting from pos
fn find_closing_marker(chars: &[char], start: usize, marker: &[char; 2]) -> Option<usize> {
    let len = chars.len();
    let mut i = start;
    while i + 1 < len {
        if chars[i] == marker[0] && chars[i + 1] == marker[1] {
            // Don't match empty content
            if i > start {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Find closing single marker (e.g., *) starting from pos
fn find_closing_single(chars: &[char], start: usize, marker: char) -> Option<usize> {
    let len = chars.len();
    let mut i = start;
    while i < len {
        if chars[i] == marker {
            // Don't match empty or double marker
            if i > start && (i + 1 >= len || chars[i + 1] != marker) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

/// Check if a Bash tool call is an internal cokacdir command.
/// Returns the subcommand name (e.g. "cron", "cron-list", "currenttime", "sendfile") or None.
fn detect_cokacdir_command(name: &str, input: &str) -> Option<String> {
    if name != "Bash" { return None; }
    let v: serde_json::Value = serde_json::from_str(input).ok()?;
    let cmd = v.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let trimmed = cmd.trim_start();
    if !trimmed.starts_with("cokacdir") { return None; }
    // Extract the first --xxx flag after "cokacdir"
    for token in trimmed.split_whitespace().skip(1) {
        if let Some(flag) = token.strip_prefix("--") {
            return Some(flag.to_string());
        }
    }
    Some("unknown".to_string())
}

/// Read the most recent .result file from schedule dir and delete it
fn read_latest_cron_result() -> Option<String> {
    let dir = schedule_dir()?;
    let mut results: Vec<_> = fs::read_dir(&dir).ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "result").unwrap_or(false))
        .collect();
    results.sort_by_key(|e| std::cmp::Reverse(e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::SystemTime::UNIX_EPOCH)));
    let entry = results.first()?;
    let content = fs::read_to_string(entry.path()).ok()?;
    let _ = fs::remove_file(entry.path());
    Some(content)
}

/// Format a cokacdir command's JSON result into a human-readable message
fn format_cokacdir_result(cmd: &str, content: &str) -> String {
    // Try to parse as JSON; if empty and cmd is "cron", try reading from .result file
    let effective_content = if content.trim().is_empty() && cmd == "cron" {
        read_latest_cron_result().unwrap_or_default()
    } else {
        content.to_string()
    };
    let v: serde_json::Value = match serde_json::from_str(effective_content.trim()) {
        Ok(v) => v,
        Err(_) => return effective_content.to_string(),
    };

    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");

    if status == "error" {
        let msg = v.get("message").and_then(|s| s.as_str()).unwrap_or("unknown error");
        return format!("❌ {}", msg);
    }

    match cmd {
        "currenttime" => {
            let time = v.get("time").and_then(|s| s.as_str()).unwrap_or("?");
            format!("🕐 {}", time)
        }
        "cron" => {
            let id = v.get("id").and_then(|s| s.as_str()).unwrap_or("?");
            let prompt = v.get("prompt").and_then(|s| s.as_str()).unwrap_or("");
            let schedule = v.get("schedule").and_then(|s| s.as_str()).unwrap_or("");
            let kind = if schedule.split_whitespace().count() == 5 { "cron" } else { "once" };
            format!("✅ Scheduled [{}]\n🔖 {}\n📝 {}\n🕐 `{}`", kind, id, prompt, schedule)
        }
        "cron-list" => {
            let schedules = v.get("schedules").and_then(|a| a.as_array());
            match schedules {
                Some(arr) if arr.is_empty() => "📋 No schedules found.".to_string(),
                Some(arr) => {
                    let mut lines = vec![format!("📋 {} schedule(s)", arr.len())];
                    for (i, s) in arr.iter().enumerate() {
                        let id = s.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        let schedule = s.get("schedule").and_then(|v| v.as_str()).unwrap_or("");
                        let prompt = s.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
                        let prompt_preview = if prompt.chars().count() > 40 {
                            format!("{}...", prompt.chars().take(40).collect::<String>())
                        } else {
                            prompt.to_string()
                        };
                        lines.push(format!("\n{}. {}\n   🕐 `{}`\n   🔖 {}", i + 1, prompt_preview, schedule, id));
                    }
                    lines.join("\n")
                }
                None => content.to_string(),
            }
        }
        "cron-remove" => {
            let id = v.get("id").and_then(|s| s.as_str()).unwrap_or("?");
            format!("✅ Removed\n🔖 {}", id)
        }
        "cron-update" => {
            let id = v.get("id").and_then(|s| s.as_str()).unwrap_or("?");
            let schedule = v.get("schedule").and_then(|s| s.as_str()).unwrap_or("");
            format!("✅ Updated\n🕐 `{}`\n🔖 {}", schedule, id)
        }
        "sendfile" => {
            let path = v.get("path").and_then(|s| s.as_str()).unwrap_or("?");
            format!("📎 {}", path)
        }
        _ => content.to_string(),
    }
}

/// Extract the command string (with optional description) from a Bash tool input JSON
fn format_bash_command(input: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(input) else {
        return input.to_string();
    };
    let desc = v.get("description").and_then(|v| v.as_str()).unwrap_or("");
    let cmd = v.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if !desc.is_empty() {
        format!("{}\n{}", desc, cmd)
    } else {
        cmd.to_string()
    }
}

/// Format tool input JSON into a human-readable summary
fn format_tool_input(name: &str, input: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(input) else {
        return format!("{} {}", name, input);
    };

    match name {
        "Bash" => {
            let desc = v.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let cmd = v.get("command").and_then(|v| v.as_str()).unwrap_or("");
            if !desc.is_empty() {
                format!("{}: `{}`", desc, cmd)
            } else {
                format!("`{}`", cmd)
            }
        }
        "Read" => {
            let fp = v.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            format!("Read {}", fp)
        }
        "Write" => {
            let fp = v.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let content = v.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let lines = content.lines().count();
            if lines > 0 {
                format!("Write {} ({} lines)", fp, lines)
            } else {
                format!("Write {}", fp)
            }
        }
        "Edit" => {
            let fp = v.get("file_path").and_then(|v| v.as_str()).unwrap_or("");
            let replace_all = v.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);
            if replace_all {
                format!("Edit {} (replace all)", fp)
            } else {
                format!("Edit {}", fp)
            }
        }
        "Glob" => {
            let pattern = v.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = v.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if !path.is_empty() {
                format!("Glob {} in {}", pattern, path)
            } else {
                format!("Glob {}", pattern)
            }
        }
        "Grep" => {
            let pattern = v.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
            let path = v.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let output_mode = v.get("output_mode").and_then(|v| v.as_str()).unwrap_or("");
            if !path.is_empty() {
                if !output_mode.is_empty() {
                    format!("Grep \"{}\" in {} ({})", pattern, path, output_mode)
                } else {
                    format!("Grep \"{}\" in {}", pattern, path)
                }
            } else {
                format!("Grep \"{}\"", pattern)
            }
        }
        "NotebookEdit" => {
            let nb_path = v.get("notebook_path").and_then(|v| v.as_str()).unwrap_or("");
            let cell_id = v.get("cell_id").and_then(|v| v.as_str()).unwrap_or("");
            if !cell_id.is_empty() {
                format!("Notebook {} ({})", nb_path, cell_id)
            } else {
                format!("Notebook {}", nb_path)
            }
        }
        "WebSearch" => {
            let query = v.get("query").and_then(|v| v.as_str()).unwrap_or("");
            format!("Search: {}", query)
        }
        "WebFetch" => {
            let url = v.get("url").and_then(|v| v.as_str()).unwrap_or("");
            format!("Fetch {}", url)
        }
        "Task" => {
            let desc = v.get("description").and_then(|v| v.as_str()).unwrap_or("");
            let subagent_type = v.get("subagent_type").and_then(|v| v.as_str()).unwrap_or("");
            if !subagent_type.is_empty() {
                format!("Task [{}]: {}", subagent_type, desc)
            } else {
                format!("Task: {}", desc)
            }
        }
        "TaskOutput" => {
            let task_id = v.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            format!("Get task output: {}", task_id)
        }
        "TaskStop" => {
            let task_id = v.get("task_id").and_then(|v| v.as_str()).unwrap_or("");
            format!("Stop task: {}", task_id)
        }
        "TodoWrite" => {
            if let Some(todos) = v.get("todos").and_then(|v| v.as_array()) {
                let pending = todos.iter().filter(|t| {
                    t.get("status").and_then(|s| s.as_str()) == Some("pending")
                }).count();
                let in_progress = todos.iter().filter(|t| {
                    t.get("status").and_then(|s| s.as_str()) == Some("in_progress")
                }).count();
                let completed = todos.iter().filter(|t| {
                    t.get("status").and_then(|s| s.as_str()) == Some("completed")
                }).count();
                format!("Todo: {} pending, {} in progress, {} completed", pending, in_progress, completed)
            } else {
                "Update todos".to_string()
            }
        }
        "Skill" => {
            let skill = v.get("skill").and_then(|v| v.as_str()).unwrap_or("");
            format!("Skill: {}", skill)
        }
        "AskUserQuestion" => {
            if let Some(questions) = v.get("questions").and_then(|v| v.as_array()) {
                if let Some(q) = questions.first() {
                    let question = q.get("question").and_then(|v| v.as_str()).unwrap_or("");
                    question.to_string()
                } else {
                    "Ask user question".to_string()
                }
            } else {
                "Ask user question".to_string()
            }
        }
        "ExitPlanMode" => {
            "Exit plan mode".to_string()
        }
        "EnterPlanMode" => {
            "Enter plan mode".to_string()
        }
        "TaskCreate" => {
            let subject = v.get("subject").and_then(|v| v.as_str()).unwrap_or("");
            format!("Create task: {}", subject)
        }
        "TaskUpdate" => {
            let task_id = v.get("taskId").and_then(|v| v.as_str()).unwrap_or("");
            let status = v.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if !status.is_empty() {
                format!("Update task {}: {}", task_id, status)
            } else {
                format!("Update task {}", task_id)
            }
        }
        "TaskGet" => {
            let task_id = v.get("taskId").and_then(|v| v.as_str()).unwrap_or("");
            format!("Get task: {}", task_id)
        }
        "TaskList" => {
            "List tasks".to_string()
        }
        _ => {
            format!("{} {}", name, input)
        }
    }
}

// === Scheduler ===

/// Check if a schedule entry should trigger now
fn should_trigger(entry: &ScheduleEntry) -> bool {
    let now = chrono::Local::now();
    sched_debug(&format!("[should_trigger] id={}, type={}, schedule={}, now={}, last_run={:?}",
        entry.id, entry.schedule_type, entry.schedule, now.format("%Y-%m-%d %H:%M:%S"), entry.last_run));
    match entry.schedule_type.as_str() {
        "absolute" => {
            let Ok(schedule_time) = chrono::NaiveDateTime::parse_from_str(&entry.schedule, "%Y-%m-%d %H:%M:%S") else {
                sched_debug(&format!("[should_trigger] id={}, parse failed → false", entry.id));
                return false;
            };
            let schedule_dt = schedule_time.and_local_timezone(chrono::Local).single();
            let Some(schedule_dt) = schedule_dt else {
                sched_debug(&format!("[should_trigger] id={}, timezone conversion failed → false", entry.id));
                return false;
            };
            if now < schedule_dt {
                sched_debug(&format!("[should_trigger] id={}, not yet (now < schedule_dt) → false", entry.id));
                return false;
            }
            // Already ran?
            if let Some(ref last) = entry.last_run {
                if let Ok(last_dt) = chrono::NaiveDateTime::parse_from_str(last, "%Y-%m-%d %H:%M:%S") {
                    if let Some(last_local) = last_dt.and_local_timezone(chrono::Local).single() {
                        if last_local >= schedule_dt {
                            sched_debug(&format!("[should_trigger] id={}, already ran (last={} >= sched={}) → false",
                                entry.id, last_local.format("%H:%M:%S"), schedule_dt.format("%H:%M:%S")));
                            return false;
                        }
                    }
                }
            }
            sched_debug(&format!("[should_trigger] id={}, absolute ready → true", entry.id));
            true
        }
        "cron" => {
            if !cron_matches(&entry.schedule, now) {
                sched_debug(&format!("[should_trigger] id={}, cron not matching → false", entry.id));
                return false;
            }
            // Check last_run to avoid duplicate triggers within the same minute
            if let Some(ref last) = entry.last_run {
                if let Ok(last_dt) = chrono::NaiveDateTime::parse_from_str(last, "%Y-%m-%d %H:%M:%S") {
                    if let Some(last_local) = last_dt.and_local_timezone(chrono::Local).single() {
                        let now_min = now.format("%Y-%m-%d %H:%M").to_string();
                        let last_min = last_local.format("%Y-%m-%d %H:%M").to_string();
                        if now_min == last_min {
                            sched_debug(&format!("[should_trigger] id={}, already ran this minute ({}) → false", entry.id, now_min));
                            return false;
                        }
                    }
                }
            }
            sched_debug(&format!("[should_trigger] id={}, cron matched → true", entry.id));
            true
        }
        _ => {
            sched_debug(&format!("[should_trigger] id={}, unknown type={} → false", entry.id, entry.schedule_type));
            false
        }
    }
}

/// Update schedule entry after a run: set last_run, delete if once
fn update_schedule_after_run(entry: &ScheduleEntry, new_context_summary: Option<String>) {
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    sched_debug(&format!("[update_schedule_after_run] id={}, type={}, once={}, now={}, has_new_context={}",
        entry.id, entry.schedule_type, entry.once, now, new_context_summary.is_some()));

    // 실행 중 사용자가 삭제한 경우 부활 방지
    let dir = match schedule_dir() {
        Some(d) => d,
        None => {
            sched_debug(&format!("[update_schedule_after_run] id={}, no schedule dir → skip", entry.id));
            return;
        }
    };
    let path = dir.join(format!("{}.json", entry.id));
    if !path.exists() {
        sched_debug(&format!("[update_schedule_after_run] id={}, file already deleted → skip (no resurrection)", entry.id));
        return; // 이미 삭제됨 - write하지 않음
    }

    if entry.once || entry.schedule_type == "absolute" {
        sched_debug(&format!("[update_schedule_after_run] id={}, once/absolute → write last_run then delete", entry.id));
        // Set last_run first to prevent re-trigger if delete fails
        let mut updated = entry.clone();
        updated.last_run = Some(now.clone());
        if let Err(e) = write_schedule_entry(&updated) {
            sched_debug(&format!("[update_schedule_after_run] id={}, write before delete failed: {}", entry.id, e));
            eprintln!("[Schedule] Failed to write entry {} before delete: {}", entry.id, e);
        }
        let deleted = delete_schedule_entry(&entry.id);
        sched_debug(&format!("[update_schedule_after_run] id={}, delete result={}", entry.id, deleted));
    } else {
        sched_debug(&format!("[update_schedule_after_run] id={}, cron recurring → update last_run", entry.id));
        let mut updated = entry.clone();
        updated.last_run = Some(now);
        if new_context_summary.is_some() {
            updated.context_summary = new_context_summary;
        }
        if let Err(e) = write_schedule_entry(&updated) {
            sched_debug(&format!("[update_schedule_after_run] id={}, write failed: {}", entry.id, e));
            eprintln!("[Schedule] Failed to update entry {}: {}", entry.id, e);
        } else {
            sched_debug(&format!("[update_schedule_after_run] id={}, updated successfully", entry.id));
        }
    }
}

/// Execute a scheduled task — similar pattern to handle_text_message
async fn execute_schedule(
    bot: &Bot,
    chat_id: ChatId,
    entry: &ScheduleEntry,
    state: &SharedState,
    token: &str,
    prev_session: Option<ChatSession>,
) {
    sched_debug(&format!("[execute_schedule] START id={}, chat_id={}, prompt={:?}, has_context={}, has_prev_session={}",
        entry.id, chat_id, truncate_str(&entry.prompt, 60), entry.context_summary.is_some(), prev_session.is_some()));
    // Build prompt with context summary if available
    let user_prompt = entry.prompt.clone();
    let prompt = if let Some(ref summary) = entry.context_summary {
        sched_debug(&format!("[execute_schedule] id={}, injecting context summary ({} chars)", entry.id, summary.len()));
        format!(
            "[이전 작업 맥락]\n{}\n\n[작업 지시]\n{}",
            summary, user_prompt
        )
    } else {
        user_prompt.clone()
    };
    let project_path = entry.current_path.clone();
    let schedule_id = entry.id.clone();
    let ts = chrono::Local::now().format("%H:%M:%S");
    println!("  [{ts}] ⏰ Schedule Starting: {user_prompt}");

    // Create persistent workspace directory for this schedule execution
    let workspace_dir = dirs::home_dir()
        .map(|h| h.join(".cokacdir").join("workspace").join(&schedule_id))
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp").join("cokacdir-workspace").join(&schedule_id));
    sched_debug(&format!("[execute_schedule] id={}, creating workspace: {}", schedule_id, workspace_dir.display()));
    if let Err(e) = fs::create_dir_all(&workspace_dir) {
        let ts = chrono::Local::now().format("%H:%M:%S");
        println!("  [{ts}] ⚠ [Schedule] Failed to create workspace: {e}");
        sched_debug(&format!("[execute_schedule] id={}, workspace creation failed: {}, restoring session", schedule_id, e));
        let mut data = state.lock().await;
        if let Some(set) = data.pending_schedules.get_mut(&chat_id) {
            set.remove(&schedule_id);
        }
        data.cancel_tokens.remove(&chat_id);
        if let Some(prev) = prev_session {
            data.sessions.insert(chat_id, prev);
        } else {
            data.sessions.remove(&chat_id);
        }
        return;
    }
    let workspace_path = workspace_dir.display().to_string();

    // Get allowed tools and model for this chat
    let (allowed_tools, model) = {
        let data = state.lock().await;
        (get_allowed_tools(&data.settings, chat_id), get_model(&data.settings, chat_id))
    };

    // Send placeholder (show only the user's original prompt, not the context summary)
    shared_rate_limit_wait(state, chat_id).await;
    let placeholder = match tg!("send_message", bot.send_message(chat_id, format!("⏰ {user_prompt}")).await) {
        Ok(msg) => msg,
        Err(e) => {
            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ⚠ [Schedule] Failed to send placeholder: {e}");
            // Clean up pending + cancel_token, restore session (workspace preserved)
            let mut data = state.lock().await;
            if let Some(set) = data.pending_schedules.get_mut(&chat_id) {
                set.remove(&schedule_id);
            }
            data.cancel_tokens.remove(&chat_id);
            if let Some(prev) = prev_session {
                data.sessions.insert(chat_id, prev);
            } else {
                data.sessions.remove(&chat_id);
            }
            return;
        }
    };
    let placeholder_msg_id = placeholder.id;

    // Build disabled tools notice
    let default_tools: std::collections::HashSet<&str> = DEFAULT_ALLOWED_TOOLS.iter().copied().collect();
    let allowed_set: std::collections::HashSet<&str> = allowed_tools.iter().map(|s| s.as_str()).collect();
    let disabled: Vec<&&str> = default_tools.iter().filter(|t| !allowed_set.contains(**t)).collect();
    let disabled_notice = if disabled.is_empty() {
        String::new()
    } else {
        let names: Vec<&str> = disabled.iter().map(|t| **t).collect();
        format!(
            "\n\nDISABLED TOOLS: The following tools have been disabled by the user: {}.\n\
             You MUST NOT attempt to use these tools. \
             If a user's request requires a disabled tool, do NOT proceed with the task. \
             Instead, clearly inform the user which tool is needed and that it is currently disabled. \
             Suggest they re-enable it with: /allowed +ToolName",
            names.join(", ")
        )
    };

    let bot_key = token_hash(token);
    let system_prompt_owned = build_system_prompt(
        &format!(
            "You are executing a scheduled task through Telegram.\n\
             Project directory: {project_path}\n\
             Your current working directory is a dedicated workspace for this schedule.\n\
             This workspace will be preserved after execution. The user can continue work here via /start.\n\
             To work with project files, use absolute paths to the project directory.\n\
             Any files you want to deliver must be sent via the cokacdir --sendfile command before the task ends."
        ),
        &workspace_path, chat_id.0, &bot_key, &disabled_notice,
        None, // scheduled tasks don't need to register further schedules with session context
    );

    // Retrieve pre-inserted cancel token (from scheduler_loop), or create a new one
    let cancel_token = {
        let mut data = state.lock().await;
        if let Some(existing) = data.cancel_tokens.get(&chat_id) {
            existing.clone()
        } else {
            let token = Arc::new(CancelToken::new());
            data.cancel_tokens.insert(chat_id, token.clone());
            token
        }
    };

    // Create channel for streaming
    let (tx, rx) = mpsc::channel();
    let cancel_token_clone = cancel_token.clone();
    let model_for_summary = model.clone();

    // Run Claude in a blocking thread (always new session — context is in the prompt)
    // Session persistence must be kept so users can resume via /SCHEDULE_ID
    let workspace_path_for_claude = workspace_path.clone();
    tokio::task::spawn_blocking(move || {
        let result = claude::execute_command_streaming(
            &prompt,
            None,
            &workspace_path_for_claude,
            tx.clone(),
            Some(&system_prompt_owned),
            Some(&allowed_tools),
            Some(cancel_token_clone),
            model.as_deref(),
            false,
        );
        if let Err(e) = result {
            let _ = tx.send(StreamMessage::Error { message: e, stdout: String::new(), stderr: String::new(), exit_code: None });
        }
    });

    // Polling loop
    let bot_owned = bot.clone();
    let state_owned = state.clone();
    let entry_clone = entry.clone();
    let workspace_path_owned = workspace_path.clone();
    tokio::spawn(async move {
        const SPINNER: &[&str] = &[
            "🕐 P",           "🕑 Pr",          "🕒 Pro",
            "🕓 Proc",        "🕔 Proce",       "🕕 Proces",
            "🕖 Process",     "🕗 Processi",    "🕘 Processin",
            "🕙 Processing",  "🕚 Processing.", "🕛 Processing..",
        ];
        let mut full_response = String::new();
        let mut last_edit_text = String::new();
        let mut done = false;
        let mut cancelled = false;
        let mut had_error = false;
        let mut spin_idx: usize = 0;
        let mut pending_cokacdir_cmd: Option<String> = None;
        let mut last_tool_name: String = String::new();
        let mut exec_session_id: Option<String> = None;

        let polling_time_ms = {
            let data = state_owned.lock().await;
            data.polling_time_ms
        };

        let mut queue_done = false;
        while !done || !queue_done {
            if cancel_token.cancelled.load(Ordering::Relaxed) {
                if !done { cancelled = true; }
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(polling_time_ms)).await;

            if cancel_token.cancelled.load(Ordering::Relaxed) {
                if !done { cancelled = true; }
                break;
            }

            // Drain messages
            loop {
                match rx.try_recv() {
                    Ok(msg) => {
                        match msg {
                            StreamMessage::Init { session_id } => {
                                exec_session_id = Some(session_id);
                            }
                            StreamMessage::Text { content } => {
                                full_response.push_str(&content);
                            }
                            StreamMessage::ToolUse { name, input } => {
                                pending_cokacdir_cmd = detect_cokacdir_command(&name, &input);
                                last_tool_name = name.clone();
                                let summary = format_tool_input(&name, &input);
                                let ts = chrono::Local::now().format("%H:%M:%S");
                                println!("  [{ts}]   ⚙ [Schedule] {name}: {summary}");
                                if pending_cokacdir_cmd.is_none() {
                                    if name == "Bash" {
                                        full_response.push_str(&format!("\n\n```\n{}\n```\n", format_bash_command(&input)));
                                    } else {
                                        full_response.push_str(&format!("\n\n⚙️ {}\n", summary));
                                    }
                                }
                            }
                            StreamMessage::ToolResult { content, is_error } => {
                                if let Some(cmd) = pending_cokacdir_cmd.take() {
                                    let ts = chrono::Local::now().format("%H:%M:%S");
                                    println!("  [{ts}]   ↩ [Schedule] cokacdir --{cmd}: {content}");
                                    let formatted = format_cokacdir_result(&cmd, &content);
                                    if !formatted.is_empty() {
                                        full_response.push_str(&format!("\n{}\n", formatted));
                                    }
                                } else if is_error {
                                    let truncated = truncate_str(&content, 500);
                                    if truncated.contains('\n') {
                                        full_response.push_str(&format!("\n❌\n```\n{}\n```\n", truncated));
                                    } else {
                                        full_response.push_str(&format!("\n❌ `{}`\n\n", truncated));
                                    }
                                } else if last_tool_name == "Read" {
                                    full_response.push_str(&format!("\n✅ `{} bytes`\n\n", content.len()));
                                } else if !content.is_empty() {
                                    let truncated = truncate_str(&content, 300);
                                    if truncated.contains('\n') {
                                        full_response.push_str(&format!("\n```\n{}\n```\n", truncated));
                                    } else {
                                        full_response.push_str(&format!("\n✅ `{}`\n\n", truncated));
                                    }
                                }
                            }
                            StreamMessage::TaskNotification { summary, .. } => {
                                if !summary.is_empty() {
                                    full_response.push_str(&format!("\n[Task: {}]\n", summary));
                                }
                            }
                            StreamMessage::Done { result, session_id } => {
                                if !result.is_empty() && full_response.is_empty() {
                                    full_response = result;
                                }
                                if let Some(sid) = session_id {
                                    exec_session_id = Some(sid);
                                }
                                done = true;
                            }
                            StreamMessage::Error { message, stdout, stderr, exit_code } => {
                                let stdout_display = if stdout.is_empty() { "(empty)".to_string() } else { stdout };
                                let stderr_display = if stderr.is_empty() { "(empty)".to_string() } else { stderr };
                                let code_display = match exit_code {
                                    Some(c) => c.to_string(),
                                    None => "(unknown)".to_string(),
                                };
                                // Check if this is a result-type error (from parse_stream_message)
                                // vs a process-level error. Both mean execution didn't complete normally.
                                full_response = format!(
                                    "Error: {}\n```\nexit code: {}\n\n[stdout]\n{}\n\n[stderr]\n{}\n```",
                                    message, code_display, stdout_display, stderr_display
                                );
                                had_error = true;
                                done = true;
                            }
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if !done { had_error = true; }
                        done = true;
                        break;
                    }
                }
            }

            // Update placeholder with progress
            if !done {
                let indicator = SPINNER[spin_idx % SPINNER.len()];
                spin_idx += 1;

                let display_text = if full_response.is_empty() {
                    format!("⏰ {}\n\n{}", entry_clone.prompt, indicator)
                } else {
                    let normalized = normalize_empty_lines(&full_response);
                    let truncated = truncate_str(&normalized, TELEGRAM_MSG_LIMIT - 40);
                    format!("⏰ {}\n\n{}\n\n{}", entry_clone.prompt, truncated, indicator)
                };

                if display_text != last_edit_text {
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let html_text = markdown_to_telegram_html(&display_text);
                    let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &html_text)
                        .parse_mode(ParseMode::Html)
                        .await);
                    last_edit_text = display_text;
                } else {
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let _ = tg!("send_chat_action", bot_owned.send_chat_action(chat_id, teloxide::types::ChatAction::Typing).await);
                }
            }

            // Queue processing
            let queued = process_upload_queue(&bot_owned, chat_id, &state_owned).await;
            if done {
                queue_done = !queued;
            }
        }

        // Final response
        sched_debug(&format!("[execute_schedule] id={}, polling done: cancelled={}, had_error={}, response_len={}",
            schedule_id, cancelled, had_error, full_response.len()));
        if cancelled {
            sched_debug(&format!("[execute_schedule] id={}, cancelled — killing child process", schedule_id));
            if let Ok(guard) = cancel_token.child_pid.lock() {
                if let Some(pid) = *guard {
                    #[cfg(unix)]
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGTERM);
                    }
                }
            }

            shared_rate_limit_wait(&state_owned, chat_id).await;
            let stopped_text = format!("⏰ {}\n\n⛔ Stopped\n\nUse /{} to continue this schedule session.", entry_clone.prompt, schedule_id);
            let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, stopped_text).await);

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ■ [Schedule] Stopped");
        } else {
            if full_response.is_empty() {
                full_response = "(No response)".to_string();
            }

            let final_text = format!("⏰ {}\n\n{}\n\nUse /{} to continue this schedule session.", entry_clone.prompt, normalize_empty_lines(&full_response), schedule_id);
            let html_response = markdown_to_telegram_html(&final_text);

            shared_rate_limit_wait(&state_owned, chat_id).await;
            if html_response.len() <= TELEGRAM_MSG_LIMIT {
                if let Err(_) = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &html_response)
                    .parse_mode(ParseMode::Html)
                    .await)
                {
                    shared_rate_limit_wait(&state_owned, chat_id).await;
                    let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &final_text).await);
                }
            } else {
                let send_result = send_long_message(&bot_owned, chat_id, &html_response, Some(ParseMode::Html), &state_owned).await;
                match send_result {
                    Ok(_) => {
                        shared_rate_limit_wait(&state_owned, chat_id).await;
                        let _ = tg!("delete_message", bot_owned.delete_message(chat_id, placeholder_msg_id).await);
                    }
                    Err(_) => {
                        let fallback = send_long_message(&bot_owned, chat_id, &final_text, None, &state_owned).await;
                        match fallback {
                            Ok(_) => {
                                shared_rate_limit_wait(&state_owned, chat_id).await;
                                let _ = tg!("delete_message", bot_owned.delete_message(chat_id, placeholder_msg_id).await);
                            }
                            Err(_) => {
                                shared_rate_limit_wait(&state_owned, chat_id).await;
                                let truncated = truncate_str(&final_text, TELEGRAM_MSG_LIMIT);
                                let _ = tg!("edit_message", bot_owned.edit_message_text(chat_id, placeholder_msg_id, &truncated).await);
                            }
                        }
                    }
                }
            }

            let ts = chrono::Local::now().format("%H:%M:%S");
            println!("  [{ts}] ✓ [Schedule] Done");
        }

        // For cron entries with context_summary, extract result summary for next run
        // Skip if execution was cancelled or encountered an error
        sched_debug(&format!("[execute_schedule] id={}, checking context summary: cancelled={}, had_error={}, type={}, once={}, has_context={}",
            schedule_id, cancelled, had_error, entry_clone.schedule_type, entry_clone.once, entry_clone.context_summary.is_some()));
        let new_context_summary = if !cancelled && !had_error && entry_clone.schedule_type == "cron" && !entry_clone.once && entry_clone.context_summary.is_some() {
            sched_debug(&format!("[execute_schedule] id={}, extracting result summary", schedule_id));
            if let Some(ref sid) = exec_session_id {
                let sid = sid.clone();
                let path = workspace_path_owned.clone();
                let model = model_for_summary.clone();
                let summary_result = tokio::task::spawn_blocking(move || {
                    claude::extract_result_summary(
                        &sid,
                        &path,
                        model.as_deref(),
                    )
                }).await;
                match summary_result {
                    Ok(Ok(ref summary)) => {
                        sched_debug(&format!("[execute_schedule] id={}, new context summary: {} chars", schedule_id, summary.len()));
                        Some(summary.clone())
                    }
                    _ => {
                        sched_debug(&format!("[execute_schedule] id={}, summary extraction failed", schedule_id));
                        None
                    }
                }
            } else {
                sched_debug(&format!("[execute_schedule] id={}, no session_id for summary", schedule_id));
                None
            }
        } else {
            None
        };

        // Save schedule session to file so user can resume via /start [workspace_path]
        if let Some(ref sid) = exec_session_id {
            let mut sched_session = ChatSession {
                session_id: Some(sid.clone()),
                current_path: Some(workspace_path_owned.clone()),
                history: Vec::new(),
                pending_uploads: Vec::new(),
                cleared: false,
            };
            // Add user prompt and AI response to history for session continuity
            sched_session.history.push(HistoryItem {
                item_type: HistoryType::User,
                content: entry_clone.prompt.clone(),
            });
            if !full_response.is_empty() {
                sched_session.history.push(HistoryItem {
                    item_type: HistoryType::Assistant,
                    content: full_response.clone(),
                });
            }
            save_session_to_file(&sched_session, &workspace_path_owned);
        }

        // Update schedule file (last_run / delete if once)
        sched_debug(&format!("[execute_schedule] id={}, calling update_schedule_after_run", schedule_id));
        update_schedule_after_run(&entry_clone, new_context_summary);

        // Workspace directory is preserved for user to continue work via /start

        // Clean up + restore previous session
        sched_debug(&format!("[execute_schedule] id={}, cleaning up: removing cancel_token, pending, restoring session (has_prev={})",
            schedule_id, prev_session.is_some()));
        {
            let mut data = state_owned.lock().await;
            data.cancel_tokens.remove(&chat_id);
            if let Some(set) = data.pending_schedules.get_mut(&chat_id) {
                set.remove(&schedule_id);
            }
            if let Some(prev) = prev_session {
                data.sessions.insert(chat_id, prev);
            } else {
                // No prior session existed — remove the schedule's temporary session
                data.sessions.remove(&chat_id);
            }
        }
        sched_debug(&format!("[execute_schedule] id={}, END", schedule_id));

        // Clean up leftover stop message
        let stop_msg_id = {
            let mut data = state_owned.lock().await;
            data.stop_message_ids.remove(&chat_id)
        };
        if let Some(msg_id) = stop_msg_id {
            shared_rate_limit_wait(&state_owned, chat_id).await;
            let _ = tg!("delete_message", bot_owned.delete_message(chat_id, msg_id).await);
        }
    });
}

/// Scheduler loop: runs every 60 seconds, checks for due schedules
async fn scheduler_loop(bot: Bot, state: SharedState, token: String) {
    let bot_key = token_hash(&token);
    sched_debug("[scheduler_loop] started");

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        // Scan schedule directory
        let entries = list_schedule_entries(&bot_key, None);
        if entries.is_empty() { continue; }

        sched_debug(&format!("[scheduler_loop] cycle: {} entries found", entries.len()));

        for entry in &entries {
            let chat_id = ChatId(entry.chat_id);

            // Verify current_path exists (before acquiring lock — involves filesystem I/O)
            if !Path::new(&entry.current_path).is_dir() {
                let ts = chrono::Local::now().format("%H:%M:%S");
                println!("  [{ts}] ⚠ [Scheduler] Path not found: {} (schedule: {})", entry.current_path, entry.id);
                sched_debug(&format!("[scheduler_loop] id={}, path not found: {} → skip", entry.id, entry.current_path));
                shared_rate_limit_wait(&state, chat_id).await;
                let msg = format!("⏰ {}\n\n⚠️ Skipped — path no longer exists\n📂 <code>{}</code>",
                    html_escape(&truncate_str(&entry.prompt, 40)), html_escape(&entry.current_path));
                let _ = tg!("send_message", bot.send_message(chat_id, msg).parse_mode(ParseMode::Html).await);
                continue;
            }

            // Single atomic lock: pending check + trigger check + busy check + session backup
            // All checks in one lock to prevent race between pending cleanup and re-trigger
            enum SchedAction {
                Skip,
                DiscardExpired,
                Execute(Option<ChatSession>),
            }

            let action = {
                let mut data = state.lock().await;
                let is_already_pending = data.pending_schedules.get(&chat_id)
                    .map_or(false, |set| set.contains(&entry.id));

                sched_debug(&format!("[scheduler_loop] id={}, is_already_pending={}", entry.id, is_already_pending));

                // If not pending and not due to trigger, skip
                if !is_already_pending && !should_trigger(entry) {
                    // Check if expired absolute schedule should be discarded
                    if entry.schedule_type == "absolute" {
                        if let Ok(schedule_time) = chrono::NaiveDateTime::parse_from_str(&entry.schedule, "%Y-%m-%d %H:%M:%S") {
                            if let Some(schedule_dt) = schedule_time.and_local_timezone(chrono::Local).single() {
                                if chrono::Local::now() > schedule_dt {
                                    sched_debug(&format!("[scheduler_loop] id={}, expired absolute → discard", entry.id));
                                    SchedAction::DiscardExpired
                                } else {
                                    sched_debug(&format!("[scheduler_loop] id={}, not yet due → skip", entry.id));
                                    SchedAction::Skip
                                }
                            } else {
                                SchedAction::Skip
                            }
                        } else {
                            SchedAction::Skip
                        }
                    } else {
                        SchedAction::Skip
                    }
                } else {
                    // Entry should execute — check if chat is busy
                    let is_busy = data.cancel_tokens.contains_key(&chat_id);
                    sched_debug(&format!("[scheduler_loop] id={}, should execute, is_busy={}", entry.id, is_busy));

                    if is_busy {
                        // Chat is busy — mark as pending if not already, retry next cycle
                        // Do NOT touch sessions — leave them as-is
                        if !is_already_pending {
                            data.pending_schedules.entry(chat_id).or_default().insert(entry.id.clone());
                            let ts = chrono::Local::now().format("%H:%M:%S");
                            println!("  [{ts}] ⏰ [Scheduler] Chat busy, pending: {}", entry.id);
                            sched_debug(&format!("[scheduler_loop] id={}, chat busy → marked pending", entry.id));
                        } else {
                            sched_debug(&format!("[scheduler_loop] id={}, chat busy, already pending → skip", entry.id));
                        }
                        SchedAction::Skip
                    } else {
                        // Not busy — backup session, replace with schedule-specific session, and execute
                        let prev = data.sessions.get(&chat_id).cloned();
                        sched_debug(&format!("[scheduler_loop] id={}, not busy → execute (has_prev_session={})", entry.id, prev.is_some()));
                        data.sessions.insert(chat_id, ChatSession {
                            session_id: None,
                            current_path: Some(entry.current_path.clone()),
                            history: Vec::new(),
                            pending_uploads: Vec::new(),
                            cleared: false,
                        });
                        data.pending_schedules.entry(chat_id).or_default().insert(entry.id.clone());
                        // Pre-insert cancel_token to prevent race with incoming user messages
                        let cancel_token = Arc::new(CancelToken::new());
                        data.cancel_tokens.insert(chat_id, cancel_token);
                        SchedAction::Execute(prev)
                    }
                }
            };

            match action {
                SchedAction::Skip => continue,
                SchedAction::DiscardExpired => {
                    delete_schedule_entry(&entry.id);
                    let ts = chrono::Local::now().format("%H:%M:%S");
                    println!("  [{ts}] ⏰ [Scheduler] Discarded expired once-schedule: {}", entry.id);
                    sched_debug(&format!("[scheduler_loop] id={}, discarded expired", entry.id));
                    continue;
                }
                SchedAction::Execute(prev_session) => {
                    sched_debug(&format!("[scheduler_loop] id={}, calling execute_schedule", entry.id));
                    execute_schedule(&bot, chat_id, entry, &state, &token, prev_session).await;
                }
            }
        }
    }
}
