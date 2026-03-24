//! Bridge mode: `cokacdir --bridge gemini`
//!
//! Runs as a subprocess that translates Claude-compatible arguments into
//! gemini-cli arguments, spawns gemini, and transforms its output back into
//! Claude-compatible stream-json / json on stdout.
//!
//! This module is the Rust port of the gemini sections of cokac-bridge.mjs.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use serde_json::Value;

// Simple v4-like UUID without external crate
fn gen_uuid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let pid = std::process::id() as u128;
    let val = nanos ^ (pid << 64);
    format!("{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (val >> 96) as u32,
        (val >> 80) as u16 & 0xffff,
        (val >> 68) as u16 & 0x0fff,
        ((val >> 52) as u16 & 0x3fff) | 0x8000,
        val as u64 & 0xffffffffffff,
    )
}

use crate::services::claude::debug_log_to;

fn bridge_debug(msg: &str) {
    debug_log_to("bridge.log", msg);
}

// ============================================================
// Parsed arguments (Claude-compatible flags)
// ============================================================

struct BridgeArgs {
    dangerously_skip_permissions: bool,
    verbose: bool,
    output_format: Option<String>,  // "json" | "stream-json"
    append_system_prompt_file: Option<String>,
    system_prompt_file: Option<String>,
    model: Option<String>,
    resume: Option<String>,
    tools: Option<String>,
    allowed_tools: Option<String>,
    positional: Vec<String>,
}

fn parse_bridge_args(args: &[String]) -> BridgeArgs {
    let mut parsed = BridgeArgs {
        dangerously_skip_permissions: false,
        verbose: false,
        output_format: None,
        append_system_prompt_file: None,
        system_prompt_file: None,
        model: None,
        resume: None,
        tools: None,
        allowed_tools: None,
        positional: Vec::new(),
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--print" => {}
            "--dangerously-skip-permissions" => parsed.dangerously_skip_permissions = true,
            "--verbose" => parsed.verbose = true,
            "--output-format" => {
                i += 1;
                if i < args.len() { parsed.output_format = Some(args[i].clone()); }
            }
            "--append-system-prompt-file" => {
                i += 1;
                if i < args.len() { parsed.append_system_prompt_file = Some(args[i].clone()); }
            }
            "--system-prompt-file" => {
                i += 1;
                if i < args.len() { parsed.system_prompt_file = Some(args[i].clone()); }
            }
            "--model" => {
                i += 1;
                if i < args.len() { parsed.model = Some(args[i].clone()); }
            }
            "--resume" | "-r" => {
                i += 1;
                if i < args.len() { parsed.resume = Some(args[i].clone()); }
            }
            "--tools" => {
                i += 1;
                if i < args.len() { parsed.tools = Some(args[i].clone()); }
            }
            "--allowed-tools" | "--allowedTools" => {
                i += 1;
                if i < args.len() { parsed.allowed_tools = Some(args[i].clone()); }
            }
            other => {
                if other.starts_with("--") {
                    // Unknown flag — skip its value if it looks like one
                    if i + 1 < args.len() && !args[i + 1].starts_with("--") {
                        i += 1;
                    }
                } else {
                    parsed.positional.push(args[i].clone());
                }
            }
        }
        i += 1;
    }
    parsed
}

// ============================================================
// Resolve gemini binary
// ============================================================

fn resolve_gemini_path() -> Option<String> {
    if let Ok(val) = std::env::var("COKAC_GEMINI_PATH") {
        if !val.is_empty() { return Some(val); }
    }

    #[cfg(unix)]
    {
        if let Ok(output) = Command::new("which").arg("gemini").output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !p.is_empty() { return Some(p); }
            }
        }
        if let Ok(output) = Command::new("bash").args(["-lc", "which gemini"]).output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !p.is_empty() { return Some(p); }
            }
        }
    }

    #[cfg(windows)]
    {
        if let Ok(output) = Command::new("where").arg("gemini").output() {
            if output.status.success() {
                let p = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Some(first) = p.lines().next() {
                    if !first.is_empty() { return Some(first.to_string()); }
                }
            }
        }
    }

    None
}

// ============================================================
// Build gemini-cli arguments from Claude-compatible args
// ============================================================

fn build_gemini_args(parsed: &BridgeArgs) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();

    // Output format
    if let Some(ref fmt) = parsed.output_format {
        match fmt.as_str() {
            "stream-json" => { args.push("--output-format".into()); args.push("stream-json".into()); }
            "json" => { args.push("--output-format".into()); args.push("json".into()); }
            _ => {}
        }
    }

    // Model
    if let Some(ref m) = parsed.model {
        args.push("--model".into());
        args.push(m.clone());
    }

    // Auto-approve
    if parsed.dangerously_skip_permissions {
        args.push("--yolo".into());
    }

    // Session resume
    if let Some(ref sid) = parsed.resume {
        args.push("--resume".into());
        args.push(sid.clone());
    }

    // Tools
    if let Some(ref t) = parsed.tools {
        args.push("--allowed-tools".into());
        args.push(t.clone());
    } else if let Some(ref t) = parsed.allowed_tools {
        args.push("--allowed-tools".into());
        args.push(t.clone());
    }

    // Verbose → debug
    if parsed.verbose {
        args.push("--debug".into());
    }

    // Prompt from positional args
    if !parsed.positional.is_empty() {
        args.push("-p".into());
        args.push(parsed.positional.join(" "));
    }

    args
}

fn build_gemini_env(parsed: &BridgeArgs) -> Vec<(String, String)> {
    let mut extra = Vec::new();
    let sp_file = parsed.append_system_prompt_file.as_ref()
        .or(parsed.system_prompt_file.as_ref());
    if let Some(path) = sp_file {
        let abs = if std::path::Path::new(path).is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map(|d| d.join(path).display().to_string())
                .unwrap_or_else(|_| path.clone())
        };
        extra.push(("GEMINI_SYSTEM_MD".into(), abs));
    }
    extra
}

// ============================================================
// Stream-json transformer: gemini → Claude format
// ============================================================

fn emit(line: &str) {
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(line.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

fn emit_json(val: &Value) {
    emit(&serde_json::to_string(val).unwrap_or_default());
}

fn run_stream_json(parsed: &BridgeArgs, gemini_bin: &str) -> i32 {
    let child_args = build_gemini_args(parsed);
    let env_extra = build_gemini_env(parsed);

    bridge_debug(&format!("stream-json: spawning {} {:?}", gemini_bin, child_args));

    let mut cmd = Command::new(gemini_bin);
    cmd.args(&child_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in &env_extra {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[bridge] Failed to spawn gemini: {}", e);
            return 1;
        }
    };

    // Read stdin and pass as prompt if available
    if let Some(mut stdin_handle) = child.stdin.take() {
        // Read from our own stdin (cokacdir writes prompt here)
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        if !input.trim().is_empty() {
            let _ = stdin_handle.write_all(input.as_bytes());
        }
        drop(stdin_handle);
    }

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            eprintln!("[bridge] Failed to capture stdout");
            return 1;
        }
    };

    let reader = BufReader::new(stdout);
    let mut session_id = parsed.resume.clone().unwrap_or_default();
    let mut gemini_model = String::new();
    let mut last_text = String::new();
    let mut result_emitted = false;
    let start = std::time::Instant::now();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() { continue; }

        let json: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "init" => {
                if let Some(sid) = json.get("session_id").and_then(|v| v.as_str()) {
                    session_id = sid.to_string();
                }
                if session_id.is_empty() { session_id = gen_uuid(); }
                if let Some(m) = json.get("model").and_then(|v| v.as_str()) {
                    gemini_model = m.to_string();
                }
                emit_json(&serde_json::json!({
                    "type": "system",
                    "subtype": "init",
                    "cwd": std::env::current_dir().unwrap_or_default().display().to_string(),
                    "session_id": session_id,
                    "model": gemini_model,
                }));
            }
            "message" => {
                let role = json.get("role").and_then(|v| v.as_str()).unwrap_or("");
                if role == "assistant" {
                    if let Some(content) = json.get("content").and_then(|v| v.as_str()) {
                        let is_delta = json.get("delta").and_then(|v| v.as_bool()).unwrap_or(false);
                        if is_delta {
                            last_text.push_str(content);
                        } else {
                            last_text = content.to_string();
                        }
                        emit_json(&serde_json::json!({
                            "type": "assistant",
                            "message": {
                                "model": gemini_model,
                                "content": [{"type": "text", "text": content}]
                            },
                            "session_id": session_id,
                        }));
                    }
                }
            }
            "tool_use" => {
                let tool_id = json.get("tool_id").and_then(|v| v.as_str())
                    .unwrap_or("").to_string();
                let tool_name = json.get("tool_name").and_then(|v| v.as_str())
                    .unwrap_or("unknown").to_string();
                let params = json.get("parameters").cloned().unwrap_or(Value::Object(Default::default()));
                emit_json(&serde_json::json!({
                    "type": "assistant",
                    "message": {
                        "model": gemini_model,
                        "content": [{
                            "type": "tool_use",
                            "id": if tool_id.is_empty() { gen_uuid() } else { tool_id },
                            "name": tool_name,
                            "input": params,
                        }]
                    },
                    "session_id": session_id,
                }));
            }
            "tool_result" => {
                let is_error = json.get("status").and_then(|v| v.as_str()) == Some("error");
                let content = if is_error {
                    json.get("error").and_then(|v| v.get("message")).and_then(|v| v.as_str())
                        .or_else(|| json.get("output").and_then(|v| v.as_str()))
                        .unwrap_or("Tool error")
                } else {
                    json.get("output").and_then(|v| v.as_str()).unwrap_or("")
                };
                let tool_id = json.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                emit_json(&serde_json::json!({
                    "type": "user",
                    "message": {
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": tool_id,
                            "content": content,
                            "is_error": is_error,
                        }]
                    },
                }));
            }
            "error" => {
                let msg = json.get("message").and_then(|v| v.as_str())
                    .or_else(|| json.get("error").and_then(|v| v.as_str()))
                    .unwrap_or("Unknown error");
                emit_json(&serde_json::json!({
                    "type": "result",
                    "subtype": "error_during_execution",
                    "is_error": true,
                    "errors": [msg],
                    "session_id": session_id,
                }));
                result_emitted = true;
            }
            "result" => {
                result_emitted = true;
                let stats = json.get("stats").cloned().unwrap_or(Value::Object(Default::default()));
                let duration_ms = stats.get("duration_ms").and_then(|v| v.as_u64())
                    .unwrap_or_else(|| start.elapsed().as_millis() as u64);
                let usage = serde_json::json!({
                    "input_tokens": stats.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    "output_tokens": stats.get("output_tokens").and_then(|v| v.as_u64()).unwrap_or(0),
                    "cache_read_input_tokens": stats.get("cached").and_then(|v| v.as_u64()).unwrap_or(0),
                });

                let status = json.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if status == "success" {
                    emit_json(&serde_json::json!({
                        "type": "result",
                        "subtype": "success",
                        "is_error": false,
                        "duration_ms": duration_ms,
                        "num_turns": 1,
                        "result": last_text,
                        "stop_reason": "end_turn",
                        "session_id": session_id,
                        "usage": usage,
                    }));
                } else {
                    let err_msg = json.get("error").and_then(|v| v.get("message")).and_then(|v| v.as_str())
                        .or_else(|| json.get("message").and_then(|v| v.as_str()))
                        .unwrap_or("Unknown error");
                    emit_json(&serde_json::json!({
                        "type": "result",
                        "subtype": "error_during_execution",
                        "is_error": true,
                        "duration_ms": duration_ms,
                        "num_turns": 1,
                        "errors": [err_msg],
                        "session_id": session_id,
                        "usage": usage,
                    }));
                }
            }
            _ => {} // skip unknown
        }
    }

    let status = child.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);

    if !result_emitted {
        let duration_ms = start.elapsed().as_millis() as u64;
        if status == 0 {
            emit_json(&serde_json::json!({
                "type": "result",
                "subtype": "success",
                "is_error": false,
                "duration_ms": duration_ms,
                "num_turns": 1,
                "result": last_text,
                "stop_reason": "end_turn",
                "session_id": session_id,
            }));
        } else {
            emit_json(&serde_json::json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "duration_ms": duration_ms,
                "num_turns": 1,
                "errors": [format!("Process exited with code {}", status)],
                "session_id": session_id,
            }));
        }
    }

    status
}

// ============================================================
// JSON mode transformer: gemini → Claude format
// ============================================================

fn run_json_mode(parsed: &BridgeArgs, gemini_bin: &str) -> i32 {
    let child_args = build_gemini_args(parsed);
    let env_extra = build_gemini_env(parsed);

    bridge_debug(&format!("json: spawning {} {:?}", gemini_bin, child_args));

    let mut cmd = Command::new(gemini_bin);
    cmd.args(&child_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in &env_extra {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[bridge] Failed to spawn gemini: {}", e);
            return 1;
        }
    };

    // Pass stdin prompt
    if let Some(mut stdin_handle) = child.stdin.take() {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        if !input.trim().is_empty() {
            let _ = stdin_handle.write_all(input.as_bytes());
        }
        drop(stdin_handle);
    }

    // Collect all stdout
    let mut raw_output = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let _ = stdout.read_to_string(&mut raw_output);
    }

    let status = child.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1);
    let _start_fallback_ms = 0u64; // We don't have precise timing; gemini stats provide it

    let mut session_id = parsed.resume.clone()
        .unwrap_or_else(|| gen_uuid());
    let mut response_text = String::new();
    let mut duration_ms = 0u64;
    let mut usage = Value::Null;

    // gemini json: {"session_id":"...", "response":"...", "stats":{"models":{...},...}}
    if let Ok(json) = serde_json::from_str::<Value>(raw_output.trim()) {
        if let Some(sid) = json.get("session_id").and_then(|v| v.as_str()) {
            session_id = sid.to_string();
        }
        if let Some(resp) = json.get("response").and_then(|v| v.as_str()) {
            response_text = resp.to_string();
        }
        // Aggregate stats from all models
        if let Some(models) = json.get("stats").and_then(|s| s.get("models")).and_then(|v| v.as_object()) {
            let mut input_tokens = 0u64;
            let mut output_tokens = 0u64;
            let mut cached = 0u64;
            let mut total_latency = 0u64;
            for (_name, model_val) in models {
                if let Some(t) = model_val.get("tokens").and_then(|v| v.as_object()) {
                    input_tokens += t.get("prompt").and_then(|v| v.as_u64()).unwrap_or(0);
                    output_tokens += t.get("candidates").and_then(|v| v.as_u64()).unwrap_or(0);
                    cached += t.get("cached").and_then(|v| v.as_u64()).unwrap_or(0);
                }
                if let Some(api) = model_val.get("api").and_then(|v| v.as_object()) {
                    total_latency += api.get("totalLatencyMs").and_then(|v| v.as_u64()).unwrap_or(0);
                }
            }
            duration_ms = total_latency;
            usage = serde_json::json!({
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_read_input_tokens": cached,
            });
        }
    } else {
        // Fallback: raw text
        response_text = raw_output.trim().to_string();
    }

    if response_text.is_empty() && !raw_output.trim().is_empty() {
        response_text = raw_output.trim().to_string();
    }

    let is_error = status != 0;
    let output = serde_json::json!({
        "type": "result",
        "subtype": if is_error { "error_during_execution" } else { "success" },
        "is_error": is_error,
        "duration_ms": duration_ms,
        "num_turns": 1,
        "result": if response_text.is_empty() && is_error {
            format!("Process exited with code {}", status)
        } else {
            response_text
        },
        "stop_reason": if is_error { "error" } else { "end_turn" },
        "session_id": session_id,
        "usage": usage,
    });

    emit_json(&output);
    status
}

// ============================================================
// Text mode: passthrough
// ============================================================

fn run_text_mode(parsed: &BridgeArgs, gemini_bin: &str) -> i32 {
    let child_args = build_gemini_args(parsed);
    let env_extra = build_gemini_env(parsed);

    let mut cmd = Command::new(gemini_bin);
    cmd.args(&child_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (k, v) in &env_extra {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[bridge] Failed to spawn gemini: {}", e);
            return 1;
        }
    };

    if let Some(mut stdin_handle) = child.stdin.take() {
        let mut input = String::new();
        let _ = std::io::stdin().read_to_string(&mut input);
        if !input.trim().is_empty() {
            let _ = stdin_handle.write_all(input.as_bytes());
        }
        drop(stdin_handle);
    }

    child.wait().map(|s| s.code().unwrap_or(1)).unwrap_or(1)
}

// ============================================================
// Public entry point: called from main.rs when --bridge gemini
// ============================================================

/// Run bridge mode. `remaining_args` are the Claude-compatible flags after `--bridge gemini`.
pub fn run_gemini(remaining_args: &[String]) -> ! {
    let parsed = parse_bridge_args(remaining_args);

    let gemini_bin = match resolve_gemini_path() {
        Some(p) => p,
        None => {
            eprintln!("[bridge] gemini CLI not found. Is it installed?");
            std::process::exit(1);
        }
    };

    bridge_debug(&format!("gemini_bin={}", gemini_bin));

    let fmt = parsed.output_format.as_deref().unwrap_or("text");
    let exit_code = match fmt {
        "stream-json" => run_stream_json(&parsed, &gemini_bin),
        "json" => run_json_mode(&parsed, &gemini_bin),
        _ => run_text_mode(&parsed, &gemini_bin),
    };

    std::process::exit(exit_code);
}
