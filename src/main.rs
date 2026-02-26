mod ui;
mod services;
mod utils;
mod config;
mod keybindings;
mod enc;

use std::io;
use std::env;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::time::Duration;
use ratatui::{
    backend::CrosstermBackend,
    Terminal,
};

use crate::ui::app::{App, Screen};
use crate::services::claude;
use crate::utils::markdown::{render_markdown, MarkdownTheme, is_line_empty};
use crate::keybindings::PanelAction;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help() {
    println!("cokacdir {} - Multi-panel terminal file manager", VERSION);
    println!();
    println!("USAGE:");
    println!("    cokacdir [OPTIONS] [PATH...]");
    println!();
    println!("ARGS:");
    println!("    [PATH...]               Open panels at given paths (max 10)");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help              Print help information");
    println!("    -v, --version           Print version information");
    println!("    --prompt <TEXT>         Send prompt to AI and print rendered response");
    println!("    --design                Enable theme hot-reload (for theme development)");
    println!("    --base64 <TEXT>         Decode base64 and print (internal use)");
    println!("    --ccserver <TOKEN>...   Start Telegram bot server(s)");
    println!("    --sendfile <PATH> --chat <ID> --key <HASH>");
    println!("                            Send file via Telegram bot (internal use, HASH = token hash)");
    println!("    --currenttime            Print current server time");
    println!("    --cron <PROMPT> --at <TIME> --chat <ID> --key <HASH> [--once] [--session <SID>]");
    println!("                            Register a scheduled task");
    println!("    --cron-list --chat <ID> --key <HASH>");
    println!("                            List registered schedules");
    println!("    --cron-remove <SID> --chat <ID> --key <HASH>");
    println!("                            Remove a schedule");
    println!("    --cron-update <SID> --at <TIME> --chat <ID> --key <HASH>");
    println!("                            Update schedule time");
    println!();
    println!("HOMEPAGE: https://cokacdir.cokac.com");
}

fn handle_base64(encoded: &str) {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    match BASE64.decode(encoded) {
        Ok(decoded) => {
            if let Ok(text) = String::from_utf8(decoded) {
                print!("{}", text);
            } else {
                std::process::exit(1);
            }
        }
        Err(_) => {
            std::process::exit(1);
        }
    }
}

fn handle_sendfile(path: &str, chat_id: i64, hash_key: &str) {
    use md5::{Md5, Digest};

    let file_path = std::path::Path::new(path);
    if !file_path.exists() {
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("file not found: {}", path)}));
        std::process::exit(1);
    }

    let abs_path = match file_path.canonicalize() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status":"error","message":format!("failed to resolve path: {}", e)}));
            std::process::exit(1);
        }
    };

    // Create upload queue directory
    let queue_dir = match dirs::home_dir() {
        Some(h) => h.join(".cokacdir").join("upload_queue"),
        None => {
            eprintln!("{}", serde_json::json!({"status":"error","message":"cannot determine home directory"}));
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("failed to create queue directory: {}", e)}));
        std::process::exit(1);
    }

    // Generate queue filename: YYYY-MM-DD-hh-mm-ii-ss-mmm.{MD5}.queue
    let now = chrono::Local::now();
    let timestamp = now.format("%Y-%m-%d-%H-%M-%S").to_string();
    let millis = now.format("%3f").to_string();
    let md5_hash = format!("{:x}", Md5::digest(abs_path.as_bytes()));
    let filename = format!("{}-{}.{}.queue", timestamp, millis, md5_hash);

    // Write queue file
    let queue_content = serde_json::json!({
        "path": abs_path,
        "chat_id": chat_id,
        "key": hash_key,
    });
    let queue_path = queue_dir.join(&filename);
    match std::fs::write(&queue_path, queue_content.to_string()) {
        Ok(_) => println!("{}", serde_json::json!({"status":"ok","path":abs_path})),
        Err(e) => {
            eprintln!("{}", serde_json::json!({"status":"error","message":format!("failed to write queue file: {}", e)}));
            std::process::exit(1);
        }
    }
}

fn cron_debug(msg: &str) {
    claude::debug_log_to("cron.log", msg);
}

fn handle_cron_register(prompt: &str, at_value: &str, chat_id: i64, hash_key: &str, once: bool, session_id: Option<&str>) {
    use services::telegram;
    use services::claude;

    cron_debug("========================================");
    cron_debug("=== handle_cron_register START ===");
    cron_debug("========================================");
    cron_debug(&format!("  prompt: {}", prompt));
    cron_debug(&format!("  at_value: {}", at_value));
    cron_debug(&format!("  chat_id: {}", chat_id));
    cron_debug(&format!("  hash_key: {}", hash_key));
    cron_debug(&format!("  once: {}", once));
    cron_debug(&format!("  session_id: {:?}", session_id));

    let now = chrono::Local::now();
    cron_debug(&format!("  now: {}", now.format("%Y-%m-%d %H:%M:%S%.3f")));

    // Determine schedule_type and schedule value
    cron_debug("  Parsing --at value...");
    let (schedule_type, schedule_value) = if let Some(dt) = telegram::parse_relative_time_pub(at_value) {
        // Relative time → convert to absolute
        cron_debug(&format!("  Parsed as relative time → absolute: {}", dt.format("%Y-%m-%d %H:%M:%S")));
        ("absolute".to_string(), dt.format("%Y-%m-%d %H:%M:%S").to_string())
    } else if at_value.split_whitespace().count() == 5 {
        // Cron expression (5 fields)
        cron_debug(&format!("  Parsed as cron expression: {}", at_value));
        ("cron".to_string(), at_value.to_string())
    } else {
        // Try absolute time: "YYYY-MM-DD HH:MM:SS"
        if chrono::NaiveDateTime::parse_from_str(at_value, "%Y-%m-%d %H:%M:%S").is_ok() {
            cron_debug(&format!("  Parsed as absolute time: {}", at_value));
            ("absolute".to_string(), at_value.to_string())
        } else {
            cron_debug(&format!("  ERROR: invalid --at value: {}", at_value));
            eprintln!("{}", serde_json::json!({"status":"error","message":format!("invalid --at value: {}", at_value)}));
            std::process::exit(1);
        }
    };
    cron_debug(&format!("  schedule_type={}, schedule_value={}", schedule_type, schedule_value));

    // Generate 8-char uppercase hex ID (0-9, A-F), unique among existing schedule files
    cron_debug("  Generating unique ID...");
    let id = {
        use std::collections::HashSet;
        let existing: HashSet<String> = telegram::list_all_schedule_ids_pub();
        cron_debug(&format!("  Existing schedule IDs: {:?}", existing));
        loop {
            let candidate = format!("{:08X}", rand::random::<u32>());
            if !existing.contains(&candidate) {
                cron_debug(&format!("  Generated ID: {}", candidate));
                break candidate;
            }
            cron_debug(&format!("  ID collision: {}, retrying...", candidate));
        }
    };

    // Resolve current_path from bot_settings using chat_id + hash_key
    cron_debug("  Resolving current_path...");
    let current_path = telegram::resolve_current_path_for_chat(chat_id, hash_key)
        .unwrap_or_else(|| {
            let fallback = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "/".to_string());
            cron_debug(&format!("  current_path fallback: {}", fallback));
            fallback
        });
    cron_debug(&format!("  current_path: {}", current_path));

    // Step 1: Register schedule immediately (without context_summary) and output result
    cron_debug("  Writing schedule entry (without context_summary)...");
    telegram::write_schedule_entry_pub(&telegram::ScheduleEntryData {
        id: id.clone(),
        chat_id,
        bot_key: hash_key.to_string(),
        current_path: current_path.clone(),
        prompt: prompt.to_string(),
        schedule: schedule_value.clone(),
        schedule_type: schedule_type.clone(),
        once,
        last_run: None,
        created_at: now.format("%Y-%m-%d %H:%M:%S").to_string(),
        context_summary: None,
    }).unwrap_or_else(|e| {
        cron_debug(&format!("  ERROR: write_schedule_entry failed: {}", e));
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("{}", e)}));
        std::process::exit(1);
    });
    cron_debug("  Schedule entry written successfully");

    let output = serde_json::json!({
        "status": "ok",
        "id": id,
        "prompt": prompt,
        "schedule": schedule_value,
    });
    cron_debug(&format!("  Output: {}", output));
    // Write result to temp file so the bot can read it even if Bash tool misses stdout
    if let Some(home) = dirs::home_dir() {
        let result_path = home.join(".cokacdir").join("schedule").join(format!("{}.result", id));
        let _ = std::fs::write(&result_path, output.to_string());
        cron_debug(&format!("  Result file written: {}", result_path.display()));
    }
    println!("{}", output);
    // Flush stdout immediately so the Bash tool captures the output
    use std::io::Write;
    let _ = std::io::stdout().flush();

    // Step 2: Spawn a detached child process to extract context summary and update the schedule
    if let Some(sid) = session_id {
        cron_debug(&format!("  Spawning background process for context summary extraction: session={}", sid));
        let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("cokacdir"));
        let child = std::process::Command::new(exe)
            .arg("--cron-context")
            .arg(&id)
            .arg(sid)
            .arg(prompt)
            .arg(&current_path)
            .arg(chat_id.to_string())
            .arg(hash_key)
            .arg(&schedule_value)
            .arg(&schedule_type)
            .arg(if once { "1" } else { "0" })
            .arg(now.format("%Y-%m-%d %H:%M:%S").to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(c) => cron_debug(&format!("  Background process spawned: pid={:?}", c.id())),
            Err(e) => cron_debug(&format!("  WARNING: Failed to spawn background process: {}", e)),
        }
    } else {
        cron_debug("  No session_id provided, skipping context summary");
    }

    cron_debug("=== handle_cron_register END ===");
}

struct CronContextArgs {
    id: String,
    session_id: String,
    prompt: String,
    current_path: String,
    chat_id: i64,
    hash_key: String,
    schedule: String,
    schedule_type: String,
    once: bool,
    created_at: String,
}

impl CronContextArgs {
    fn from_args(args: &[String]) -> Option<Self> {
        if args.len() < 10 {
            return None;
        }
        Some(Self {
            id: args[0].clone(),
            session_id: args[1].clone(),
            prompt: args[2].clone(),
            current_path: args[3].clone(),
            chat_id: args[4].parse().unwrap_or(0),
            hash_key: args[5].clone(),
            schedule: args[6].clone(),
            schedule_type: args[7].clone(),
            once: args[8] == "1",
            created_at: args[9].clone(),
        })
    }
}

/// Background process: extract context summary and update a schedule entry.
/// Called as: cokacdir --cron-context <id> <session_id> <prompt> <current_path> <chat_id> <hash_key> <schedule> <schedule_type> <once> <created_at>
fn handle_cron_context(args: &[String]) {
    use services::telegram;
    use services::claude;

    cron_debug("=== handle_cron_context START ===");
    let ctx = match CronContextArgs::from_args(args) {
        Some(c) => c,
        None => {
            cron_debug(&format!("  ERROR: insufficient args: {:?}", args));
            return;
        }
    };

    cron_debug(&format!("  id={}, session_id={}, prompt_len={}", ctx.id, ctx.session_id, ctx.prompt.len()));

    let extract_start = std::time::Instant::now();
    match claude::extract_context_summary(&ctx.session_id, &ctx.prompt, &ctx.current_path) {
        Ok(summary) => {
            cron_debug(&format!("  Context summary extracted in {:?}, len={}", extract_start.elapsed(), summary.len()));

            // 실행 중 삭제된 스케줄 부활 방지: 파일이 아직 존재하는지 확인
            if let Some(home) = dirs::home_dir() {
                let path = home.join(".cokacdir").join("schedule").join(format!("{}.json", ctx.id));
                if !path.exists() {
                    cron_debug(&format!("  Schedule {} already deleted, skipping context_summary write", ctx.id));
                    cron_debug("=== handle_cron_context END ===");
                    return;
                }
            }

            telegram::write_schedule_entry_pub(&telegram::ScheduleEntryData {
                id: ctx.id.clone(),
                chat_id: ctx.chat_id,
                bot_key: ctx.hash_key.clone(),
                current_path: ctx.current_path.clone(),
                prompt: ctx.prompt.clone(),
                schedule: ctx.schedule.clone(),
                schedule_type: ctx.schedule_type.clone(),
                once: ctx.once,
                last_run: None,
                created_at: ctx.created_at.clone(),
                context_summary: Some(summary),
            }).unwrap_or_else(|e| {
                cron_debug(&format!("  ERROR: write_schedule_entry failed: {}", e));
            });
            cron_debug("  Schedule entry updated with context_summary");
        }
        Err(e) => {
            cron_debug(&format!("  WARNING: extract_context_summary failed in {:?}: {}", extract_start.elapsed(), e));
        }
    }
    cron_debug("=== handle_cron_context END ===");
}

fn handle_cron_list(chat_id: i64, hash_key: &str) {
    use services::telegram;

    cron_debug(&format!("[handle_cron_list] chat_id={}, hash_key={}", chat_id, hash_key));
    let entries = telegram::list_schedule_entries_pub(hash_key, Some(chat_id));
    cron_debug(&format!("[handle_cron_list] found {} entries", entries.len()));
    let items: Vec<serde_json::Value> = entries.iter().map(|e| {
        serde_json::json!({
            "id": e.id,
            "prompt": e.prompt,
            "schedule": e.schedule,
            "created_at": e.created_at
        })
    }).collect();
    println!("{}", serde_json::json!({"status":"ok","schedules":items}));
}

fn handle_cron_remove(id: &str, chat_id: i64, hash_key: &str) {
    use services::telegram;

    cron_debug(&format!("[handle_cron_remove] id={}, chat_id={}, hash_key={}", id, chat_id, hash_key));
    // Verify ownership
    let entries = telegram::list_schedule_entries_pub(hash_key, Some(chat_id));
    if !entries.iter().any(|e| e.id == id) {
        cron_debug(&format!("[handle_cron_remove] id={}, not found or access denied", id));
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("schedule not found or access denied: {}", id)}));
        std::process::exit(1);
    }

    if telegram::delete_schedule_entry_pub(id) {
        cron_debug(&format!("[handle_cron_remove] id={}, deleted successfully", id));
        println!("{}", serde_json::json!({"status":"ok","id":id}));
    } else {
        cron_debug(&format!("[handle_cron_remove] id={}, delete failed", id));
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("failed to remove schedule: {}", id)}));
        std::process::exit(1);
    }
}

fn handle_cron_update(id: &str, at_value: &str, chat_id: i64, hash_key: &str) {
    use services::telegram;

    cron_debug(&format!("[handle_cron_update] id={}, at_value={:?}, chat_id={}, hash_key={}", id, at_value, chat_id, hash_key));
    // Find the entry
    let entries = telegram::list_schedule_entries_pub(hash_key, Some(chat_id));
    let entry = entries.iter().find(|e| e.id == id);
    let Some(entry) = entry else {
        cron_debug(&format!("[handle_cron_update] id={}, not found or access denied", id));
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("schedule not found or access denied: {}", id)}));
        std::process::exit(1);
    };

    // Parse new schedule value
    let (schedule_type, schedule_value) = if let Some(dt) = telegram::parse_relative_time_pub(at_value) {
        cron_debug(&format!("[handle_cron_update] id={}, parsed as relative → absolute: {}", id, dt.format("%Y-%m-%d %H:%M:%S")));
        ("absolute".to_string(), dt.format("%Y-%m-%d %H:%M:%S").to_string())
    } else if at_value.split_whitespace().count() == 5 {
        cron_debug(&format!("[handle_cron_update] id={}, parsed as cron: {}", id, at_value));
        ("cron".to_string(), at_value.to_string())
    } else if chrono::NaiveDateTime::parse_from_str(at_value, "%Y-%m-%d %H:%M:%S").is_ok() {
        cron_debug(&format!("[handle_cron_update] id={}, parsed as absolute datetime: {}", id, at_value));
        ("absolute".to_string(), at_value.to_string())
    } else {
        cron_debug(&format!("[handle_cron_update] id={}, invalid --at value: {:?}", id, at_value));
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("invalid --at value: {}", at_value)}));
        std::process::exit(1);
    };

    // Update and write back
    let mut updated = entry.clone();
    updated.schedule = schedule_value.clone();
    updated.schedule_type = schedule_type.clone();
    updated.last_run = None; // Reset last_run so it triggers again

    cron_debug(&format!("[handle_cron_update] id={}, writing: type={}, schedule={}, last_run=None", id, schedule_type, schedule_value));
    telegram::write_schedule_entry_pub(&updated).unwrap_or_else(|e| {
        cron_debug(&format!("[handle_cron_update] id={}, write failed: {}", id, e));
        eprintln!("{}", serde_json::json!({"status":"error","message":format!("{}", e)}));
        std::process::exit(1);
    });

    cron_debug(&format!("[handle_cron_update] id={}, updated successfully", id));
    println!("{}", serde_json::json!({"status":"ok","id":id,"schedule":schedule_value}));
}

fn print_version() {
    println!("cokacdir {}", VERSION);
}

fn handle_ccserver(tokens: Vec<String>) {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    let title = format!("  cokacdir v{}  |  Telegram Bot Server  ", VERSION);
    let width = title.chars().count();
    println!();
    println!("  ┌{}┐", "─".repeat(width));
    println!("  │{}│", title);
    println!("  └{}┘", "─".repeat(width));
    println!();

    if tokens.len() == 1 {
        println!("  ▸ Bot instance : 1");
        println!("  ▸ Status       : Connecting...");
        println!();
        rt.block_on(services::telegram::run_bot(&tokens[0]));
    } else {
        println!("  ▸ Bot instances : {}", tokens.len());
        println!("  ▸ Status        : Connecting...");
        println!();
        rt.block_on(async {
            let mut handles = Vec::new();
            for (i, token) in tokens.into_iter().enumerate() {
                handles.push(tokio::spawn(async move {
                    println!("  ✓ Bot #{} connected", i + 1);
                    services::telegram::run_bot(&token).await;
                }));
            }
            for handle in handles {
                let _ = handle.await;
            }
        });
    }
}

fn handle_prompt(prompt: &str) {
    use crate::ui::theme::Theme;

    // Check if Claude is available
    if !claude::is_claude_available() {
        eprintln!("Error: Claude CLI is not available.");
        eprintln!("Please install Claude CLI: https://claude.ai/cli");
        return;
    }

    // Execute Claude command
    let current_dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let response = claude::execute_command(prompt, None, &current_dir, None, None);

    if !response.success {
        eprintln!("Error: {}", response.error.unwrap_or_else(|| "Unknown error".to_string()));
        return;
    }

    let content = response.response.unwrap_or_default();

    // Normalize empty lines first
    let normalized = normalize_consecutive_empty_lines(&content);

    // Render markdown
    let theme = Theme::default();
    let md_theme = MarkdownTheme::from_theme(&theme);
    let lines = render_markdown(&normalized, md_theme);

    // Remove consecutive empty lines from rendered output
    let mut prev_was_empty = false;
    for line in lines {
        let is_empty = is_line_empty(&line);
        if is_empty {
            if !prev_was_empty {
                println!();
            }
            prev_was_empty = true;
        } else {
            let content: String = line.spans.iter()
                .map(|s| s.content.as_ref())
                .collect();
            println!("{}", content);
            prev_was_empty = false;
        }
    }
}

/// Normalize consecutive empty lines to maximum of one
fn normalize_consecutive_empty_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut result_lines: Vec<&str> = Vec::new();
    let mut prev_was_empty = false;

    for line in lines {
        let is_empty = line.chars().all(|c| c.is_whitespace());
        if is_empty {
            if !prev_was_empty {
                result_lines.push("");
            }
            prev_was_empty = true;
        } else {
            result_lines.push(line);
            prev_was_empty = false;
        }
    }

    result_lines.join("\n")
}

fn main() -> io::Result<()> {
    // Initialize debug flag from environment variable
    claude::init_debug_from_env();

    // Handle command line arguments
    let args: Vec<String> = env::args().collect();
    let mut design_mode = false;
    let mut start_paths: Vec<std::path::PathBuf> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-v" | "--version" => {
                print_version();
                return Ok(());
            }
            "--prompt" => {
                if i + 1 >= args.len() {
                    eprintln!("Error: --prompt requires a text argument");
                    eprintln!("Usage: cokacdir --prompt \"your question\"");
                    return Ok(());
                }
                handle_prompt(&args[i + 1]);
                return Ok(());
            }
            "--base64" => {
                if i + 1 >= args.len() {
                    std::process::exit(1);
                }
                handle_base64(&args[i + 1]);
                return Ok(());
            }
            "--ccserver" => {
                let tokens: Vec<String> = args[i + 1..].iter()
                    .filter(|a| !a.starts_with('-'))
                    .cloned()
                    .collect();
                if tokens.is_empty() {
                    eprintln!("Error: --ccserver requires at least one token argument");
                    eprintln!("Usage: cokacdir --ccserver <TOKEN> [TOKEN2] ...");
                    return Ok(());
                }
                handle_ccserver(tokens);
                return Ok(());
            }
            "--currenttime" => {
                println!("{}", serde_json::json!({"status":"ok","time":chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()}));
                return Ok(());
            }
            "--cron" => {
                cron_debug("=== --cron argument parsing START ===");
                cron_debug(&format!("  Raw args: {:?}", &args[i..]));
                // Parse: --cron "prompt" --at "time" --chat ID --key KEY [--once] [--session SID]
                let mut prompt: Option<String> = None;
                let mut at_value: Option<String> = None;
                let mut chat_id: Option<i64> = None;
                let mut key: Option<String> = None;
                let mut once = false;
                let mut session_id: Option<String> = None;
                let mut j = i + 1;
                while j < args.len() {
                    match args[j].as_str() {
                        "--at" => {
                            if j + 1 < args.len() { at_value = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        "--chat" => {
                            if j + 1 < args.len() { chat_id = args[j + 1].parse().ok(); j += 2; }
                            else { j += 1; }
                        }
                        "--key" => {
                            if j + 1 < args.len() { key = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        "--session" => {
                            if j + 1 < args.len() { session_id = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        "--once" => { once = true; j += 1; }
                        _ if prompt.is_none() && !args[j].starts_with("--") => {
                            prompt = Some(args[j].clone()); j += 1;
                        }
                        _ => { j += 1; }
                    }
                }
                cron_debug(&format!("  Parsed: prompt={:?}, at={:?}, chat_id={:?}, key={:?}, once={}, session_id={:?}",
                    prompt, at_value, chat_id, key, once, session_id));
                match (prompt, at_value, chat_id, key) {
                    (Some(p), Some(at), Some(cid), Some(k)) => {
                        cron_debug("  All required args present, calling handle_cron_register");
                        handle_cron_register(&p, &at, cid, &k, once, session_id.as_deref());
                    }
                    _ => {
                        cron_debug("  ERROR: Missing required arguments");
                        eprintln!("{}", serde_json::json!({"status":"error","message":"--cron requires \"prompt\", --at \"time\", --chat <ID>, --key <HASH>"}));
                    }
                }
                cron_debug("=== --cron argument parsing END ===");
                return Ok(());
            }
            "--cron-context" => {
                // Background process: extract context summary and update schedule
                let remaining: Vec<String> = args[i+1..].to_vec();
                handle_cron_context(&remaining);
                return Ok(());
            }
            "--cron-list" => {
                let mut chat_id: Option<i64> = None;
                let mut key: Option<String> = None;
                let mut j = i + 1;
                while j < args.len() {
                    match args[j].as_str() {
                        "--chat" => {
                            if j + 1 < args.len() { chat_id = args[j + 1].parse().ok(); j += 2; }
                            else { j += 1; }
                        }
                        "--key" => {
                            if j + 1 < args.len() { key = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        _ => { j += 1; }
                    }
                }
                match (chat_id, key) {
                    (Some(cid), Some(k)) => handle_cron_list(cid, &k),
                    _ => {
                        eprintln!("{}", serde_json::json!({"status":"error","message":"--cron-list requires --chat <ID> --key <HASH>"}));
                    }
                }
                return Ok(());
            }
            "--cron-remove" => {
                let mut sched_id: Option<String> = None;
                let mut chat_id: Option<i64> = None;
                let mut key: Option<String> = None;
                let mut j = i + 1;
                while j < args.len() {
                    match args[j].as_str() {
                        "--chat" => {
                            if j + 1 < args.len() { chat_id = args[j + 1].parse().ok(); j += 2; }
                            else { j += 1; }
                        }
                        "--key" => {
                            if j + 1 < args.len() { key = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        _ if sched_id.is_none() && !args[j].starts_with("--") => {
                            sched_id = Some(args[j].clone()); j += 1;
                        }
                        _ => { j += 1; }
                    }
                }
                match (sched_id, chat_id, key) {
                    (Some(sid), Some(cid), Some(k)) => handle_cron_remove(&sid, cid, &k),
                    _ => {
                        eprintln!("{}", serde_json::json!({"status":"error","message":"--cron-remove requires <ID> --chat <ID> --key <HASH>"}));
                    }
                }
                return Ok(());
            }
            "--cron-update" => {
                let mut sched_id: Option<String> = None;
                let mut at_value: Option<String> = None;
                let mut chat_id: Option<i64> = None;
                let mut key: Option<String> = None;
                let mut j = i + 1;
                while j < args.len() {
                    match args[j].as_str() {
                        "--at" => {
                            if j + 1 < args.len() { at_value = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        "--chat" => {
                            if j + 1 < args.len() { chat_id = args[j + 1].parse().ok(); j += 2; }
                            else { j += 1; }
                        }
                        "--key" => {
                            if j + 1 < args.len() { key = Some(args[j + 1].clone()); j += 2; }
                            else { j += 1; }
                        }
                        _ if sched_id.is_none() && !args[j].starts_with("--") => {
                            sched_id = Some(args[j].clone()); j += 1;
                        }
                        _ => { j += 1; }
                    }
                }
                match (sched_id, at_value, chat_id, key) {
                    (Some(sid), Some(at), Some(cid), Some(k)) => handle_cron_update(&sid, &at, cid, &k),
                    _ => {
                        eprintln!("{}", serde_json::json!({"status":"error","message":"--cron-update requires <ID> --at \"time\" --chat <ID> --key <HASH>"}));
                    }
                }
                return Ok(());
            }
            "--sendfile" => {
                // Parse: --sendfile <PATH> --chat <ID> --key <TOKEN>
                let mut file_path: Option<String> = None;
                let mut chat_id: Option<i64> = None;
                let mut key: Option<String> = None;
                let mut j = i + 1;
                while j < args.len() {
                    match args[j].as_str() {
                        "--chat" => {
                            if j + 1 < args.len() {
                                chat_id = args[j + 1].parse().ok();
                                j += 2;
                            } else {
                                j += 1;
                            }
                        }
                        "--key" => {
                            if j + 1 < args.len() {
                                key = Some(args[j + 1].clone());
                                j += 2;
                            } else {
                                j += 1;
                            }
                        }
                        _ if file_path.is_none() && !args[j].starts_with("--") => {
                            file_path = Some(args[j].clone());
                            j += 1;
                        }
                        _ => { j += 1; }
                    }
                }
                match (file_path, chat_id, key) {
                    (Some(fp), Some(cid), Some(k)) => {
                        handle_sendfile(&fp, cid, &k);
                    }
                    _ => {
                        eprintln!("{}", serde_json::json!({"status":"error","message":"--sendfile requires <PATH>, --chat <ID>, and --key <HASH>"}));
                    }
                }
                return Ok(());
            }
            "--design" => {
                design_mode = true;
            }
            arg if arg.starts_with('-') => {
                eprintln!("Unknown option: {}", arg);
                eprintln!("Use --help for usage information");
                return Ok(());
            }
            path => {
                // Treat as a directory path
                let p = std::path::PathBuf::from(path);
                let resolved = if p.is_absolute() {
                    p
                } else {
                    env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")).join(p)
                };
                start_paths.push(resolved);
            }
        }
        i += 1;
    }

    // Setup panic hook to restore terminal on panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste,
            crossterm::cursor::Show
        );
        original_hook(panic_info);
    }));

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Clear screen before entering alternate screen
    execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Detect terminal image protocol (must be after alternate screen, before event loop)
    let picker = {
        let mut p = ratatui_image::picker::Picker::from_termios()
            .unwrap_or_else(|_| ratatui_image::picker::Picker::new((8, 16)));
        p.guess_protocol();
        p
    };

    // Load settings and create app state
    let (settings, settings_error) = match config::Settings::load_with_error() {
        Ok(s) => (s, None),
        Err(e) => (config::Settings::default(), Some(e)),
    };
    let mut app = App::with_settings(settings);
    app.image_picker = Some(picker);
    app.design_mode = design_mode;

    // Override panels with command-line paths if provided
    if !start_paths.is_empty() {
        app.set_panels_from_paths(start_paths);
    }

    // Show settings load error if any
    if let Some(err) = settings_error {
        app.show_message(&format!("Settings error: {} (using defaults)", err));
    }

    // Show design mode message if active
    if design_mode {
        app.show_message("Design mode: theme hot-reload enabled");
    }

    // Run app
    let result = run_app(&mut terminal, &mut app);

    // Save settings before exit
    app.save_settings();

    // Save last directory for shell cd (skip remote paths)
    if !app.active_panel().is_remote() {
        let last_dir = app.active_panel().path.display().to_string();
        if let Some(config_dir) = config::Settings::config_dir() {
            let lastdir_path = config_dir.join("lastdir");
            let _ = std::fs::write(&lastdir_path, &last_dir);
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0),
        crossterm::cursor::Show
    )?;

    if let Err(err) = result {
        eprintln!("Error: {}", err);
    }

    // Print goodbye message
    print_goodbye_message();

    Ok(())
}

fn print_goodbye_message() {
    // Check for updates
    check_for_updates();

    println!("Thank you for using COKACDIR! 🙏");
    println!();
    println!("If you found this useful, consider checking out my other content:");
    println!("  📺 YouTube: https://www.youtube.com/@코드깎는노인");
    println!("  📚 Classes: https://cokac.com/");
    println!();
    println!("Happy coding!");
}

fn check_for_updates() {
    let current_version = env!("CARGO_PKG_VERSION");

    // Fetch latest version from GitHub (with timeout)
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time", "3",
            "https://raw.githubusercontent.com/kstost/cokacdir/refs/heads/main/Cargo.toml"
        ])
        .output();

    let latest_version = match output {
        Ok(output) if output.status.success() => {
            let content = String::from_utf8_lossy(&output.stdout);
            parse_version_from_cargo_toml(&content)
        }
        _ => None,
    };

    if let Some(latest) = latest_version {
        if is_newer_version(&latest, current_version) {
            println!("┌──────────────────────────────────────────────────────────────────────────┐");
            println!("│  🚀 New version available: v{} (current: v{})                            ", latest, current_version);
            println!("│                                                                          │");
            println!("│  Update with:                                                            │");
            println!("│  /bin/bash -c \"$(curl -fsSL https://cokacdir.cokac.com/install.sh)\"      │");
            println!("└──────────────────────────────────────────────────────────────────────────┘");
            println!();
        }
    }
}

fn parse_version_from_cargo_toml(content: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("version") {
            // Parse: version = "x.x.x"
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if start < end {
                        return Some(line[start + 1..end].to_string());
                    }
                }
            }
        }
    }
    None
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };

    let latest_parts = parse(latest);
    let current_parts = parse(current);

    for i in 0..latest_parts.len().max(current_parts.len()) {
        let l = latest_parts.get(i).copied().unwrap_or(0);
        let c = current_parts.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        } else if l < c {
            return false;
        }
    }
    false
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        // Check if full redraw is needed (after terminal mode command like vim)
        if app.needs_full_redraw {
            terminal.clear()?;
            app.needs_full_redraw = false;
        }

        terminal.draw(|f| ui::draw::draw(f, app))?;

        // For AI screen, FileInfo with calculation, ImageViewer loading, diff comparing, file operation progress, or remote spinner, use fast polling
        let is_file_info_calculating = app.current_screen == Screen::FileInfo
            && app.file_info_state.as_ref().map(|s| s.is_calculating).unwrap_or(false);
        let is_image_loading = app.current_screen == Screen::ImageViewer
            && app.image_viewer_state.as_ref().map(|s| s.is_loading).unwrap_or(false);
        let is_diff_comparing = app.current_screen == Screen::DiffScreen
            && app.diff_state.as_ref().map(|s| s.is_comparing).unwrap_or(false);
        let is_dedup_active = app.current_screen == Screen::DedupScreen
            && app.dedup_screen_state.as_ref().map(|s| !s.is_complete).unwrap_or(false);
        let is_progress_active = app.file_operation_progress
            .as_ref()
            .map(|p| p.is_active)
            .unwrap_or(false);
        let is_remote_spinner = app.remote_spinner.is_some();

        let poll_timeout = if is_progress_active || is_dedup_active {
            Duration::from_millis(16) // ~60fps for smooth real-time updates
        } else if is_remote_spinner {
            Duration::from_millis(100) // Fast polling for spinner animation
        } else if app.current_screen == Screen::AIScreen || app.is_ai_mode() || is_file_info_calculating || is_image_loading || is_diff_comparing {
            Duration::from_millis(100) // Fast polling for spinner animation
        } else {
            Duration::from_millis(250)
        };

        // Poll for AI responses if on AI screen or AI mode (panel)
        if app.current_screen == Screen::AIScreen || app.is_ai_mode() {
            if let Some(ref mut state) = app.ai_state {
                // poll_response()가 true를 반환하면 새 내용이 추가된 것
                let has_new_content = state.poll_response();
                if has_new_content {
                    app.refresh_panels();
                }
            }
        }

        // Poll for file info calculation if on FileInfo screen
        if app.current_screen == Screen::FileInfo {
            if let Some(ref mut state) = app.file_info_state {
                state.poll();
            }
        }

        // Poll for image loading if on ImageViewer screen
        if app.current_screen == Screen::ImageViewer {
            if let Some(ref mut state) = app.image_viewer_state {
                let was_loading = state.is_loading;
                state.poll();
                // Create inline protocol when loading completes
                if was_loading && !state.is_loading && state.image.is_some() {
                    if let Some(ref mut picker) = app.image_picker {
                        if picker.protocol_type != ratatui_image::picker::ProtocolType::Halfblocks {
                            let img = state.image.as_ref().expect("checked above").clone();
                            state.inline_protocol = Some(picker.new_resize_protocol(img));
                            state.use_inline = true;
                        }
                    }
                }
            }
        }

        // Poll for diff comparison progress if on DiffScreen
        if app.current_screen == Screen::DiffScreen {
            if let Some(ref mut state) = app.diff_state {
                let just_completed = state.poll();
                if just_completed && !state.has_differences() {
                    app.diff_state = None;
                    app.current_screen = Screen::FilePanel;
                    app.show_message("No differences found");
                }
            }
        }

        // Poll for remote spinner completion
        app.poll_remote_spinner();

        // Check for theme file changes (hot-reload, only in design mode)
        if app.design_mode && app.theme_watch_state.check_for_changes() {
            app.reload_theme();
        }

        // Poll for file operation progress
        let progress_message: Option<String> = if let Some(ref mut progress) = app.file_operation_progress {
            let still_active = progress.poll();
            if !still_active {
                // Operation completed - extract result info before releasing borrow
                let msg = if let Some(ref result) = progress.result {
                    // Special handling for Tar - show archive name
                    if progress.operation_type == crate::services::file_ops::FileOperationType::Tar {
                        if result.failure_count == 0 {
                            if let Some(ref archive_name) = app.pending_tar_archive {
                                Some(format!("Created: {}", archive_name))
                            } else {
                                Some(format!("Archived {} file(s)", result.success_count))
                            }
                        } else {
                            Some(format!("Error: {}", result.last_error.as_deref().unwrap_or("Archive failed")))
                        }
                    } else if progress.operation_type == crate::services::file_ops::FileOperationType::Untar {
                        if result.failure_count == 0 {
                            if let Some(ref extract_dir) = app.pending_extract_dir {
                                Some(format!("Extracted to: {}", extract_dir))
                            } else {
                                Some(format!("Extracted {} file(s)", result.success_count))
                            }
                        } else {
                            Some(format!("Error: {}", result.last_error.as_deref().unwrap_or("Extract failed")))
                        }
                    } else {
                        let op_name = match progress.operation_type {
                            crate::services::file_ops::FileOperationType::Copy => "Copied",
                            crate::services::file_ops::FileOperationType::Move => "Moved",
                            crate::services::file_ops::FileOperationType::Tar => "Archived",
                            crate::services::file_ops::FileOperationType::Untar => "Extracted",
                            crate::services::file_ops::FileOperationType::Download => "Downloaded",
                            crate::services::file_ops::FileOperationType::Encrypt => "Encrypted",
                            crate::services::file_ops::FileOperationType::Decrypt => "Decrypted",
                        };
                        let total = result.success_count + result.failure_count;
                        if result.failure_count == 0 {
                            Some(format!("{} {} file(s)", op_name, result.success_count))
                        } else {
                            Some(format!("{} {}/{}. Error: {}",
                                op_name,
                                result.success_count,
                                total,
                                result.last_error.as_deref().unwrap_or("Unknown error")
                            ))
                        }
                    }
                } else {
                    None
                };
                msg
            } else {
                None
            }
        } else {
            None
        };

        // Handle progress completion (outside of borrow)
        if progress_message.is_some() {
            // 원격 다운로드 완료 → 편집기/뷰어 열기
            if let Some(pending) = app.pending_remote_open.take() {
                app.file_operation_progress = None;
                app.dialog = None;

                // tmp 파일 존재 확인으로 성공/실패 판단
                let tmp_exists = match &pending {
                    crate::ui::app::PendingRemoteOpen::Editor { tmp_path, .. } => tmp_path.exists(),
                    crate::ui::app::PendingRemoteOpen::ImageViewer { tmp_path } => tmp_path.exists(),
                };

                if !tmp_exists {
                    if let Some(msg) = progress_message {
                        app.show_message(&msg);
                    } else {
                        app.show_message("Download failed");
                    }
                } else {
                    match pending {
                        crate::ui::app::PendingRemoteOpen::Editor { tmp_path, panel_index, remote_path } => {
                            let mut editor = crate::ui::file_editor::EditorState::new();
                            editor.set_syntax_colors(app.theme.syntax);
                            match editor.load_file(&tmp_path) {
                                Ok(_) => {
                                    editor.remote_origin = Some(crate::ui::file_editor::RemoteEditOrigin {
                                        panel_index,
                                        remote_path,
                                    });
                                    app.editor_state = Some(editor);
                                    app.current_screen = Screen::FileEditor;
                                }
                                Err(e) => {
                                    app.show_message(&format!("Cannot open file: {}", e));
                                }
                            }
                        }
                        crate::ui::app::PendingRemoteOpen::ImageViewer { tmp_path } => {
                            if !crate::ui::image_viewer::supports_true_color() {
                                app.pending_large_image = Some(tmp_path);
                                app.dialog = Some(crate::ui::app::Dialog {
                                    dialog_type: crate::ui::app::DialogType::TrueColorWarning,
                                    input: String::new(),
                                    cursor_pos: 0,
                                    message: "Terminal doesn't support true color. Open anyway?".to_string(),
                                    completion: None,
                                    selected_button: 1,
                                    selection: None,
                                    use_md5: false,
                                });
                            } else {
                                app.image_viewer_state = Some(
                                    crate::ui::image_viewer::ImageViewerState::new(&tmp_path)
                                );
                                app.current_screen = Screen::ImageViewer;
                            }
                        }
                    }
                }
            } else {
                if let Some(msg) = progress_message {
                    app.show_message(&msg);
                }
                // Focus on created tar archive if applicable
                if let Some(archive_name) = app.pending_tar_archive.take() {
                    app.refresh_panels();
                    if let Some(idx) = app.active_panel().files.iter().position(|f| f.name == archive_name) {
                        app.active_panel_mut().selected_index = idx;
                    }
                // Focus on extracted directory if applicable
                } else if let Some(extract_dir) = app.pending_extract_dir.take() {
                    app.refresh_panels();
                    if let Some(idx) = app.active_panel().files.iter().position(|f| f.name == extract_dir) {
                        app.active_panel_mut().selected_index = idx;
                    }
                // Focus on first pasted file (by panel's sorted order) if applicable
                } else if let Some(paste_names) = app.pending_paste_focus.take() {
                    app.refresh_panels();
                    // Find the first file in the panel's sorted list that matches any pasted name
                    if let Some(idx) = app.active_panel().files.iter().position(|f| paste_names.contains(&f.name)) {
                        app.active_panel_mut().selected_index = idx;
                    }
                } else {
                    app.refresh_panels();
                }
                app.file_operation_progress = None;
                app.dialog = None;
            }
        }

        // Check for key events with timeout
        if event::poll(poll_timeout)? {
            // Block all input while remote spinner is active
            if app.remote_spinner.is_some() {
                let ev = event::read()?;
                if let Event::Key(key) = ev {
                    if key.code == KeyCode::Esc {
                        app.remote_spinner = None;
                        app.show_message("Connection cancelled");
                    }
                }
                continue;
            }
            match event::read()? {
                Event::Key(key) => {
                    match app.current_screen {
                        Screen::FilePanel => {
                            if handle_panel_input(app, key.code, key.modifiers) {
                                return Ok(());
                            }
                        }
                        Screen::FileViewer => {
                            ui::file_viewer::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::FileEditor => {
                            ui::file_editor::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::FileInfo => {
                            ui::file_info::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::ProcessManager => {
                            ui::process_manager::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::Help => {
                            if ui::help::handle_input(app, key.code) {
                                app.current_screen = Screen::FilePanel;
                            }
                        }
                        Screen::AIScreen => {
                            if let Some(ref mut state) = app.ai_state {
                                if ui::ai_screen::handle_input(state, key.code, key.modifiers, &app.keybindings) {
                                    // Save session to file before leaving
                                    state.save_session_to_file();
                                    app.current_screen = Screen::FilePanel;
                                    app.ai_state = None;
                                    // Refresh panels in case AI modified files
                                    app.refresh_panels();
                                }
                            }
                        }
                        Screen::SystemInfo => {
                            if ui::system_info::handle_input(&mut app.system_info_state, key.code, key.modifiers, &app.keybindings) {
                                app.current_screen = Screen::FilePanel;
                            }
                        }
                        Screen::ImageViewer => {
                            // 다이얼로그가 열려있으면 다이얼로그 입력 처리
                            if app.dialog.is_some() {
                                ui::dialogs::handle_dialog_input(app, key.code, key.modifiers);
                            } else {
                                ui::image_viewer::handle_input(app, key.code, key.modifiers);
                            }
                        }
                        Screen::SearchResult => {
                            let result = ui::search_result::handle_input(
                                &mut app.search_result_state,
                                key.code,
                                key.modifiers,
                                &app.keybindings,
                            );
                            match result {
                                Some(crate::keybindings::SearchResultAction::Open) => {
                                    app.goto_search_result();
                                }
                                Some(crate::keybindings::SearchResultAction::Close) => {
                                    app.search_result_state.active = false;
                                    app.current_screen = Screen::FilePanel;
                                }
                                _ => {}
                            }
                        }
                        Screen::DiffScreen => {
                            ui::diff_screen::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::DiffFileView => {
                            ui::diff_file_view::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::GitScreen => {
                            ui::git_screen::handle_input(app, key.code, key.modifiers);
                        }
                        Screen::DedupScreen => {
                            if let Some(ref mut state) = app.dedup_screen_state {
                                if ui::dedup_screen::handle_input(state, key.code, key.modifiers) {
                                    app.current_screen = Screen::FilePanel;
                                    app.dedup_screen_state = None;
                                    app.refresh_panels();
                                }
                            }
                        }
                    }
                }
                Event::Paste(text) => {
                    match app.current_screen {
                        Screen::AIScreen => {
                            if let Some(ref mut state) = app.ai_state {
                                ui::ai_screen::handle_paste(state, &text);
                            }
                        }
                        Screen::FilePanel => {
                            // AI mode with focus on AI panel
                            if app.is_ai_mode() && app.ai_panel_index == Some(app.active_panel_index) {
                                if let Some(ref mut state) = app.ai_state {
                                    ui::ai_screen::handle_paste(state, &text);
                                }
                            } else if app.dialog.is_some() {
                                ui::dialogs::handle_paste(app, &text);
                            } else if app.advanced_search_state.active {
                                ui::advanced_search::handle_paste(&mut app.advanced_search_state, &text);
                            }
                        }
                        Screen::FileEditor => {
                            ui::file_editor::handle_paste(app, &text);
                        }
                        Screen::ImageViewer => {
                            if app.dialog.is_some() {
                                ui::dialogs::handle_paste(app, &text);
                            }
                        }
                        Screen::GitScreen => {
                            if let Some(ref mut state) = app.git_screen_state {
                                ui::git_screen::handle_paste(state, &text);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
}

fn handle_panel_input(app: &mut App, code: KeyCode, modifiers: KeyModifiers) -> bool {
    // AI 모드일 때: active_panel이 AI 패널 쪽이면 AI로 입력 전달, 아니면 파일 패널 조작
    if app.is_ai_mode() {
        let ai_has_focus = app.ai_panel_index == Some(app.active_panel_index);
        if app.keybindings.panel_action(code, modifiers) == Some(PanelAction::SwitchPanel) {
            // AI fullscreen 모드에서는 패널 전환 차단
            let ai_fullscreen = app.ai_state.as_ref().map_or(false, |s| s.ai_fullscreen);
            if !ai_fullscreen {
                app.switch_panel();
            }
            return false;
        }
        if ai_has_focus {
            if let Some(ref mut state) = app.ai_state {
                if ui::ai_screen::handle_input(state, code, modifiers, &app.keybindings) {
                    // AI 화면 종료 요청
                    app.close_ai_screen();
                }
            }
            return false;
        }
        // ai_has_focus가 false면 아래 파일 패널 로직으로 진행
    }

    // Handle advanced search dialog first
    if app.advanced_search_state.active {
        if let Some(criteria) = ui::advanced_search::handle_input(&mut app.advanced_search_state, code, modifiers, &app.keybindings) {
            app.execute_advanced_search(&criteria);
        }
        return false;
    }

    // Handle dialog input first
    if app.dialog.is_some() {
        return ui::dialogs::handle_dialog_input(app, code, modifiers);
    }


    // Look up action from keybindings
    if let Some(action) = app.keybindings.panel_action(code, modifiers) {
        match action {
            PanelAction::Quit => return true,
            PanelAction::MoveUp => app.move_cursor(-1),
            PanelAction::MoveDown => app.move_cursor(1),
            PanelAction::PageUp => app.move_cursor(-10),
            PanelAction::PageDown => app.move_cursor(10),
            PanelAction::GoHome => app.cursor_to_start(),
            PanelAction::GoEnd => app.cursor_to_end(),
            PanelAction::Open => app.enter_selected(),
            PanelAction::ParentDir => {
                if app.diff_first_panel.is_some() {
                    app.diff_first_panel = None;
                    app.show_message("Diff cancelled");
                } else {
                    app.go_to_parent();
                }
            }
            PanelAction::SwitchPanel => app.switch_panel(),
            PanelAction::SwitchPanelLeft => app.switch_panel_left(),
            PanelAction::SwitchPanelRight => app.switch_panel_right(),
            PanelAction::ToggleSelect => app.toggle_selection(),
            PanelAction::SelectAll => app.toggle_all_selection(),
            PanelAction::SelectByExtension => app.select_by_extension(),
            PanelAction::SelectUp => app.move_cursor_with_selection(-1),
            PanelAction::SelectDown => app.move_cursor_with_selection(1),
            PanelAction::Copy => app.clipboard_copy(),
            PanelAction::Cut => app.clipboard_cut(),
            PanelAction::Paste => app.clipboard_paste(),
            PanelAction::SortByName => app.toggle_sort_by_name(),
            PanelAction::SortByType => app.toggle_sort_by_type(),
            PanelAction::SortBySize => app.toggle_sort_by_size(),
            PanelAction::SortByDate => app.toggle_sort_by_date(),
            PanelAction::Help => app.show_help(),
            PanelAction::FileInfo => app.show_file_info(),
            PanelAction::Edit => app.edit_file(),
            PanelAction::Mkdir => app.show_mkdir_dialog(),
            PanelAction::Mkfile => app.show_mkfile_dialog(),
            PanelAction::Delete => app.show_delete_dialog(),
            PanelAction::ProcessManager => app.show_process_manager(),
            PanelAction::Rename => app.show_rename_dialog(),
            PanelAction::Tar => app.show_tar_dialog(),
            PanelAction::Search => app.show_search_dialog(),
            PanelAction::GoToPath => app.show_goto_dialog(),
            PanelAction::AddPanel => app.add_panel(),
            PanelAction::GoHomeDir => app.goto_home(),
            PanelAction::Refresh => app.refresh_panels(),
            PanelAction::GitLogDiff => app.show_git_log_diff_dialog(),
            PanelAction::StartDiff => app.start_diff(),
            PanelAction::ClosePanel => app.close_panel(),
            PanelAction::AIScreen => app.show_ai_screen(),
            PanelAction::Settings => app.show_settings_dialog(),
            PanelAction::GitScreen => app.show_git_screen(),
            PanelAction::ToggleBookmark => app.toggle_bookmark(),
            PanelAction::SetHandler => app.show_handler_dialog(),
            PanelAction::EncryptAll => app.show_encrypt_dialog(),
            PanelAction::DecryptAll => app.show_decrypt_dialog(),
            PanelAction::RemoveDuplicates => app.show_dedup_screen(),
            #[cfg(target_os = "macos")]
            PanelAction::OpenInFinder => app.open_in_finder(),
            #[cfg(target_os = "macos")]
            PanelAction::OpenInVSCode => app.open_in_vscode(),
        }
    }
    false
}
