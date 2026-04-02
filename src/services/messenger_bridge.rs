//! Messenger Bridge: translates external messengers into Telegram Bot API format.
//!
//! Architecture:
//!   External Messenger ←→ MessengerBackend ←→ TG Bot API Proxy ←→ telegram.rs (unchanged)
//!
//! The proxy runs a local HTTP server that implements the Telegram Bot API subset
//! used by telegram.rs. teloxide connects to this proxy instead of the real
//! Telegram API, enabling any messenger to reuse the existing telegram.rs logic
//! without modification.
//!
//! Discord bots are launched via `--ccserver` (auto-detected by token format).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicI64, Ordering};

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use serde_json::{json, Value};

// ============================================================
// Common types
// ============================================================

/// Bot identity information (returned by getMe)
pub struct BotInfo {
    pub id: i64,
    pub username: String,
    pub first_name: String,
}

/// An incoming message from the external messenger
pub struct IncomingMessage {
    /// Mapped chat ID (must be stable for the same chat/channel)
    pub chat_id: i64,
    /// Mapped message ID (unique within the chat)
    pub message_id: i32,
    /// Sender's user ID
    pub from_id: u64,
    /// Sender's display name
    pub from_first_name: String,
    /// Sender's username (optional)
    pub from_username: Option<String>,
    /// Text content
    pub text: Option<String>,
    /// Whether this is a group/channel (vs DM)
    pub is_group: bool,
    /// Group/channel title (required if is_group)
    pub group_title: Option<String>,
    /// File attachment
    pub document: Option<FileAttachment>,
    /// Photo attachments
    pub photo: Option<Vec<PhotoAttachment>>,
    /// Caption for media
    pub caption: Option<String>,
}

pub struct FileAttachment {
    pub file_id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub file_size: Option<u64>,
}

pub struct PhotoAttachment {
    pub file_id: String,
    pub width: u32,
    pub height: u32,
    pub file_size: Option<u64>,
}

/// Result of sending a message through the backend
pub struct SentMessage {
    pub message_id: i32,
    pub chat_id: i64,
    pub text: Option<String>,
}

/// File info for downloads
pub struct FileInfo {
    pub file_id: String,
    pub file_path: String,
    pub file_size: Option<u64>,
}

// ============================================================
// MessengerBackend trait
// ============================================================

#[async_trait]
pub trait MessengerBackend: Send + Sync {
    /// Backend name (e.g., "discord", "slack", "console")
    fn name(&self) -> &str;

    /// Initialize the backend and return bot info
    async fn init(&mut self) -> Result<BotInfo, String>;

    /// Start listening for incoming messages, sending them through `tx`.
    /// This should spawn a background task and return immediately.
    async fn start(&self, tx: mpsc::Sender<IncomingMessage>) -> Result<(), String>;

    /// Send a text message to a chat
    async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<SentMessage, String>;

    /// Edit an existing message
    async fn edit_message(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<SentMessage, String>;

    /// Delete a message
    async fn delete_message(&self, chat_id: i64, message_id: i32) -> Result<bool, String>;

    /// Send a file/document
    async fn send_document(
        &self,
        chat_id: i64,
        data: &[u8],
        filename: &str,
        caption: Option<&str>,
    ) -> Result<SentMessage, String>;

    /// Get file info for downloading
    async fn get_file(&self, file_id: &str) -> Result<FileInfo, String>;

    /// Download file data by file_path (returned from get_file)
    async fn get_file_data(&self, file_path: &str) -> Result<Vec<u8>, String>;
}

// ============================================================
// HTTP helpers
// ============================================================

struct HttpRequest {
    path: String,
    content_type: String,
    body: Vec<u8>,
}

/// Maximum request body size (100 MB — covers Telegram's 50 MB file upload limit)
const MAX_BODY_SIZE: usize = 100 * 1024 * 1024;

async fn read_http_request(
    reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> Option<HttpRequest> {
    /// Maximum header line size (16 KB — well above any realistic HTTP header)
    const MAX_HEADER_LINE: usize = 16 * 1024;

    let mut request_line = String::new();
    match reader.read_line(&mut request_line).await {
        Ok(0) => return None,
        Err(_) => return None,
        _ => {}
    }
    if request_line.len() > MAX_HEADER_LINE {
        return None;
    }

    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let path = parts[1].to_string();

    let mut content_length: Option<usize> = None;
    let mut content_type = String::new();
    let mut chunked = false;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line).await {
            Ok(0) => return None,
            Err(_) => return None,
            _ => {}
        }
        if line.len() > MAX_HEADER_LINE {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_lowercase();
        if let Some(val) = lower.strip_prefix("content-length:") {
            content_length = val.trim().parse().ok();
        } else if lower.starts_with("content-type:") {
            content_type = trimmed["content-type:".len()..].trim().to_string();
        } else if lower.starts_with("transfer-encoding:") {
            if lower.contains("chunked") {
                chunked = true;
            }
        }
    }

    let body = if chunked {
        // Read chunked transfer encoding
        let mut body = Vec::new();
        loop {
            let mut size_line = String::new();
            match reader.read_line(&mut size_line).await {
                Ok(0) => break,
                Err(_) => return None,
                _ => {}
            }
            let chunk_size = match usize::from_str_radix(size_line.trim(), 16) {
                Ok(s) => s,
                Err(_) => return None,
            };
            if chunk_size == 0 {
                // Read trailing \r\n after final chunk
                let mut trailing = String::new();
                let _ = reader.read_line(&mut trailing).await;
                break;
            }
            if body.len() + chunk_size > MAX_BODY_SIZE {
                return None;
            }
            let mut chunk = vec![0u8; chunk_size];
            if reader.read_exact(&mut chunk).await.is_err() {
                return None;
            }
            body.extend_from_slice(&chunk);
            // Read trailing \r\n after chunk data
            let mut trailing = String::new();
            let _ = reader.read_line(&mut trailing).await;
        }
        body
    } else {
        let cl = content_length.unwrap_or(0);
        if cl > MAX_BODY_SIZE {
            return None;
        }
        let mut body = vec![0u8; cl];
        if cl > 0 {
            if reader.read_exact(&mut body).await.is_err() {
                return None;
            }
        }
        body
    };

    Some(HttpRequest {
        path,
        content_type,
        body,
    })
}

fn http_json_response(status: u16, body: &[u8]) -> Vec<u8> {
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let header = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        status, status_text, body.len()
    );
    let mut resp = header.into_bytes();
    resp.extend_from_slice(body);
    resp
}

fn http_file_response(data: &[u8], content_type: &str) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        content_type,
        data.len()
    );
    let mut resp = header.into_bytes();
    resp.extend_from_slice(data);
    resp
}

// ============================================================
// Multipart / URL-encoded parsers
// ============================================================

/// Find the first occurrence of `needle` in `haystack` (byte-level search).
fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_multipart_to_json(content_type: &str, body: &[u8]) -> Value {
    let boundary = content_type
        .split("boundary=")
        .nth(1)
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    if boundary.is_empty() {
        return json!({});
    }

    let mut result = serde_json::Map::new();
    let delim = format!("--{}", boundary);
    let delim_bytes = delim.as_bytes();
    let sep = b"\r\n\r\n";

    // Walk through the body finding delimiter-separated parts
    let mut search_from = 0;
    let mut parts: Vec<(usize, usize)> = Vec::new(); // (start, end) of each part body

    while let Some(d_pos) = find_bytes(&body[search_from..], delim_bytes) {
        let abs_d = search_from + d_pos;
        // Content starts after delimiter + \r\n
        let after_delim = abs_d + delim_bytes.len();
        if after_delim >= body.len() {
            break;
        }
        // Check for closing "--" (end marker)
        if body[after_delim..].starts_with(b"--") {
            break;
        }
        // Skip \r\n after delimiter
        let content_start = if body[after_delim..].starts_with(b"\r\n") {
            after_delim + 2
        } else {
            after_delim
        };

        // Find next delimiter to determine part end
        if let Some(next_d) = find_bytes(&body[content_start..], delim_bytes) {
            let part_end = content_start + next_d;
            // Strip trailing \r\n before delimiter
            let trimmed_end = if part_end >= 2 && &body[part_end - 2..part_end] == b"\r\n" {
                part_end - 2
            } else {
                part_end
            };
            parts.push((content_start, trimmed_end));
            search_from = content_start + next_d;
        } else {
            // Last part (no next delimiter found)
            parts.push((content_start, body.len()));
            break;
        }
    }

    for &(start, end) in &parts {
        if start >= end {
            continue;
        }
        let part = &body[start..end];

        // Split headers from content at \r\n\r\n
        let header_end = match find_bytes(part, sep) {
            Some(pos) => pos,
            None => continue,
        };

        let header_str = std::str::from_utf8(&part[..header_end]).unwrap_or("");
        let content = &part[header_end + sep.len()..];

        let name = extract_header_param(header_str, "name");
        let filename = extract_header_param(header_str, "filename");

        if let Some(name) = name {
            if let Some(fname) = filename {
                // File field: encode as base64 to preserve binary content
                use base64::{Engine as _, engine::general_purpose::STANDARD};
                result.insert("_filename".to_string(), json!(fname));
                result.insert("_file_data_b64".to_string(), json!(STANDARD.encode(content)));
            } else {
                // Text field
                let text = std::str::from_utf8(content).unwrap_or("");
                result.insert(name, json!(text));
            }
        }
    }

    // Convert numeric string fields to numbers
    for key in &["chat_id", "message_id", "offset", "limit", "timeout"] {
        if let Some(Value::String(s)) = result.get(*key) {
            if let Ok(n) = s.parse::<i64>() {
                result.insert(key.to_string(), json!(n));
            }
        }
    }

    Value::Object(result)
}

fn extract_header_param(headers: &str, param: &str) -> Option<String> {
    let search = format!("{}=\"", param);
    let start = headers.find(&search)?;
    let rest = &headers[start + search.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn parse_urlencoded_to_json(body: &[u8]) -> Value {
    let body_str = String::from_utf8_lossy(body);
    let mut result = serde_json::Map::new();
    for pair in body_str.split('&') {
        if let Some((key, value)) = pair.split_once('=') {
            let decoded = simple_url_decode(value);
            if let Ok(n) = decoded.parse::<i64>() {
                result.insert(key.to_string(), json!(n));
            } else if decoded == "true" {
                result.insert(key.to_string(), json!(true));
            } else if decoded == "false" {
                result.insert(key.to_string(), json!(false));
            } else {
                result.insert(key.to_string(), json!(decoded));
            }
        }
    }
    Value::Object(result)
}

fn simple_url_decode(s: &str) -> String {
    let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
    let mut chars = s.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let hex: Vec<u8> = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                if let Ok(decoded) = u8::from_str_radix(
                    std::str::from_utf8(&hex).unwrap_or(""),
                    16,
                ) {
                    bytes.push(decoded);
                    continue;
                }
            }
            // Malformed percent-encoding: keep original
            bytes.push(b'%');
            bytes.extend_from_slice(&hex);
        } else if b == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).to_string())
}

// ============================================================
// Proxy State
// ============================================================

struct ProxyState {
    backend: Arc<dyn MessengerBackend>,
    bot_info: BotInfo,
    update_rx: Mutex<mpsc::Receiver<IncomingMessage>>,
    update_id_counter: AtomicI64,
    /// Expected bot token — requests with a mismatched token are rejected.
    expected_token: String,
}

impl ProxyState {
    fn next_update_id(&self) -> i64 {
        self.update_id_counter.fetch_add(1, Ordering::Relaxed)
    }
}

// ============================================================
// Proxy Server
// ============================================================

async fn run_proxy_server(state: Arc<ProxyState>, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let state = state.clone();
                tokio::spawn(handle_connection(state, stream));
            }
            Err(e) => {
                eprintln!("  [bridge] accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(state: Arc<ProxyState>, stream: tokio::net::TcpStream) {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);

    while let Some(req) = read_http_request(&mut reader).await {
        let resp_bytes = route_request(&state, &req).await;
        if write_half.write_all(&resp_bytes).await.is_err() {
            break;
        }
        if write_half.flush().await.is_err() {
            break;
        }
    }
}

async fn route_request(state: &ProxyState, req: &HttpRequest) -> Vec<u8> {
    let path = &req.path;
    let unauthorized = json!({"ok": false, "description": "Unauthorized"});

    // File download: /file/bot<token>/<file_path>
    if path.starts_with("/file/bot") {
        let parts: Vec<&str> = path.splitn(4, '/').collect();
        // parts = ["", "file", "bot<token>", "<file_path>"]
        if parts.len() >= 4 {
            // Verify token: "bot<token>" → strip "bot" prefix
            let token = parts[2].strip_prefix("bot").unwrap_or("");
            if token != state.expected_token {
                return http_json_response(401, unauthorized.to_string().as_bytes());
            }
            return handle_file_download(state, parts[3]).await;
        }
        let err = json!({"ok": false, "description": "Invalid file path"});
        return http_json_response(400, err.to_string().as_bytes());
    }

    // API method: /bot<token>/<method>
    let (token, method) = extract_token_and_method(path);

    // Verify token
    if token != state.expected_token {
        return http_json_response(401, unauthorized.to_string().as_bytes());
    }

    if method.is_empty() {
        let err = json!({"ok": false, "description": "Unknown method"});
        return http_json_response(404, err.to_string().as_bytes());
    }

    let body_json = parse_request_body(&req.content_type, &req.body);
    if method == "SendDocument" || method == "sendDocument" {
        eprintln!("  [bridge-proxy] SendDocument: content_type={:?}, body_len={}, parsed_keys={:?}",
            req.content_type, req.body.len(),
            body_json.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    }
    let result = handle_api_method(state, method, &body_json).await;
    http_json_response(200, result.to_string().as_bytes())
}

/// Extract token and method from path like `/bot<token>/sendMessage` → `("<token>", "sendMessage")`
fn extract_token_and_method(path: &str) -> (&str, &str) {
    let after_bot = match path.find("/bot") {
        Some(pos) => &path[pos + 4..],
        None => return ("", ""),
    };
    match after_bot.find('/') {
        Some(pos) => (&after_bot[..pos], &after_bot[pos + 1..]),
        None => (after_bot, ""),
    }
}

fn parse_request_body(content_type: &str, body: &[u8]) -> Value {
    if content_type.contains("multipart/form-data") {
        parse_multipart_to_json(content_type, body)
    } else if content_type.contains("application/x-www-form-urlencoded") {
        parse_urlencoded_to_json(body)
    } else {
        // Default: try JSON
        serde_json::from_slice(body).unwrap_or(json!({}))
    }
}

// ============================================================
// API Method Router
// ============================================================

async fn handle_api_method(state: &ProxyState, method: &str, body: &Value) -> Value {
    // teloxide 0.13 uses PascalCase (GetMe, SendMessage, etc.)
    match method {
        "GetMe" | "getMe" => handle_get_me(state),
        "SetMyCommands" | "setMyCommands" => json!({"ok": true, "result": true}),
        "GetUpdates" | "getUpdates" => handle_get_updates(state, body).await,
        "SendMessage" | "sendMessage" => handle_send_message(state, body).await,
        "EditMessageText" | "editMessageText" => handle_edit_message(state, body).await,
        "DeleteMessage" | "deleteMessage" => handle_delete_message(state, body).await,
        "SendDocument" | "sendDocument" => handle_send_document(state, body).await,
        "GetFile" | "getFile" => handle_get_file(state, body).await,
        "SendChatAction" | "sendChatAction" => json!({"ok": true, "result": true}),
        "GetWebhookInfo" | "getWebhookInfo" => json!({"ok": true, "result": {"url": "", "has_custom_certificate": false, "pending_update_count": 0}}),
        _ => json!({"ok": true, "result": true}),
    }
}

// ============================================================
// Endpoint Handlers
// ============================================================

fn handle_get_me(state: &ProxyState) -> Value {
    json!({
        "ok": true,
        "result": {
            "id": state.bot_info.id,
            "is_bot": true,
            "first_name": state.bot_info.first_name,
            "username": state.bot_info.username,
            "can_join_groups": true,
            "can_read_all_group_messages": true,
            "supports_inline_queries": false,
        }
    })
}

async fn handle_get_updates(state: &ProxyState, body: &Value) -> Value {
    let offset = body.get("offset").and_then(|v| v.as_i64()).unwrap_or(0);
    let timeout_secs = body.get("timeout").and_then(|v| v.as_u64()).unwrap_or(0);
    let limit = body.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;

    // Negative offset: flush/discard all pending messages (startup sequence)
    if offset < 0 {
        let mut rx = state.update_rx.lock().await;
        while rx.try_recv().is_ok() {}
        return json!({"ok": true, "result": []});
    }

    // limit=0 is a confirmation/flush request — return empty
    if limit == 0 {
        return json!({"ok": true, "result": []});
    }

    let mut updates = Vec::new();
    let mut rx = state.update_rx.lock().await;

    // Drain immediately available messages
    while updates.len() < limit {
        match rx.try_recv() {
            Ok(msg) => updates.push(incoming_to_update(state, msg)),
            Err(_) => break,
        }
    }

    // If nothing yet and timeout > 0, wait for the first message
    if updates.is_empty() && timeout_secs > 0 {
        let duration = std::time::Duration::from_secs(timeout_secs);
        match tokio::time::timeout(duration, rx.recv()).await {
            Ok(Some(msg)) => {
                updates.push(incoming_to_update(state, msg));
                // Drain any more that arrived while we waited
                while updates.len() < limit {
                    match rx.try_recv() {
                        Ok(msg) => updates.push(incoming_to_update(state, msg)),
                        Err(_) => break,
                    }
                }
            }
            _ => {} // Timeout or channel closed
        }
    }

    json!({"ok": true, "result": updates})
}

async fn handle_send_message(state: &ProxyState, body: &Value) -> Value {
    let chat_id = body.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let parse_mode = body.get("parse_mode").and_then(|v| v.as_str());

    match state.backend.send_message(chat_id, text, parse_mode).await {
        Ok(sent) => json!({
            "ok": true,
            "result": make_bot_message_json(state, sent.message_id, chat_id,
                sent.text.as_deref().unwrap_or(text))
        }),
        Err(e) => json!({"ok": false, "description": e}),
    }
}

async fn handle_edit_message(state: &ProxyState, body: &Value) -> Value {
    let chat_id = body.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let message_id = body.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
    let parse_mode = body.get("parse_mode").and_then(|v| v.as_str());

    match state
        .backend
        .edit_message(chat_id, message_id, text, parse_mode)
        .await
    {
        Ok(_) => json!({
            "ok": true,
            "result": make_bot_message_json(state, message_id, chat_id, text)
        }),
        Err(e) => json!({"ok": false, "description": e}),
    }
}

async fn handle_delete_message(state: &ProxyState, body: &Value) -> Value {
    let chat_id = body.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
    let message_id = body.get("message_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

    match state.backend.delete_message(chat_id, message_id).await {
        Ok(_) => json!({"ok": true, "result": true}),
        Err(e) => json!({"ok": false, "description": e}),
    }
}

async fn handle_send_document(state: &ProxyState, body: &Value) -> Value {
    let chat_id = body.get("chat_id").and_then(|v| v.as_i64()).unwrap_or(0);
    eprintln!("  [bridge-proxy] send_document: chat_id={}, has_filename={}, has_file_data={}",
        chat_id, body.get("_filename").is_some(), body.get("_file_data_b64").is_some());
    let caption = body.get("caption").and_then(|v| v.as_str());
    let filename = body
        .get("_filename")
        .and_then(|v| v.as_str())
        .unwrap_or("file");
    let file_data = body
        .get("_file_data_b64")
        .and_then(|v| v.as_str())
        .and_then(|s| {
            use base64::{Engine as _, engine::general_purpose::STANDARD};
            STANDARD.decode(s).ok()
        })
        .unwrap_or_default();

    match state
        .backend
        .send_document(chat_id, &file_data, filename, caption)
        .await
    {
        Ok(sent) => json!({
            "ok": true,
            "result": make_bot_message_json(state, sent.message_id, chat_id,
                caption.unwrap_or(""))
        }),
        Err(e) => json!({"ok": false, "description": e}),
    }
}

async fn handle_get_file(state: &ProxyState, body: &Value) -> Value {
    let file_id = body.get("file_id").and_then(|v| v.as_str()).unwrap_or("");

    match state.backend.get_file(file_id).await {
        Ok(info) => json!({
            "ok": true,
            "result": {
                "file_id": info.file_id,
                "file_unique_id": info.file_id,
                "file_size": info.file_size,
                "file_path": info.file_path,
            }
        }),
        Err(e) => json!({"ok": false, "description": e}),
    }
}

async fn handle_file_download(state: &ProxyState, file_path: &str) -> Vec<u8> {
    match state.backend.get_file_data(file_path).await {
        Ok(data) => {
            let ct = if file_path.ends_with(".jpg") || file_path.ends_with(".jpeg") {
                "image/jpeg"
            } else if file_path.ends_with(".png") {
                "image/png"
            } else if file_path.ends_with(".pdf") {
                "application/pdf"
            } else {
                "application/octet-stream"
            };
            http_file_response(&data, ct)
        }
        Err(e) => {
            let err = json!({"ok": false, "description": e});
            http_json_response(404, err.to_string().as_bytes())
        }
    }
}

// ============================================================
// JSON Builders (Telegram-compatible format)
// ============================================================

/// Convert IncomingMessage to Telegram Update JSON
fn incoming_to_update(state: &ProxyState, msg: IncomingMessage) -> Value {
    let update_id = state.next_update_id();
    let ts = chrono::Local::now().timestamp();

    let chat = if msg.is_group {
        json!({
            "id": msg.chat_id,
            "type": "supergroup",
            "title": msg.group_title.as_deref().unwrap_or("Group"),
        })
    } else {
        json!({
            "id": msg.chat_id,
            "type": "private",
            "first_name": msg.from_first_name,
        })
    };

    let mut from = json!({
        "id": msg.from_id,
        "is_bot": false,
        "first_name": msg.from_first_name,
    });
    if let Some(uname) = &msg.from_username {
        from["username"] = json!(uname);
    }

    let mut message = json!({
        "message_id": msg.message_id,
        "from": from,
        "chat": chat,
        "date": ts,
    });

    if let Some(text) = &msg.text {
        message["text"] = json!(text);
    }
    if let Some(caption) = &msg.caption {
        message["caption"] = json!(caption);
    }
    if let Some(doc) = &msg.document {
        message["document"] = json!({
            "file_id": doc.file_id,
            "file_unique_id": doc.file_id,
            "file_name": doc.file_name,
            "mime_type": doc.mime_type,
            "file_size": doc.file_size,
        });
    }
    if let Some(photos) = &msg.photo {
        let arr: Vec<Value> = photos
            .iter()
            .map(|p| {
                json!({
                    "file_id": p.file_id,
                    "file_unique_id": p.file_id,
                    "width": p.width,
                    "height": p.height,
                    "file_size": p.file_size,
                })
            })
            .collect();
        message["photo"] = json!(arr);
    }

    json!({
        "update_id": update_id,
        "message": message,
    })
}

/// Build a Telegram Message JSON for bot-sent messages (used in sendMessage/editMessage responses)
fn make_bot_message_json(state: &ProxyState, msg_id: i32, chat_id: i64, text: &str) -> Value {
    let chat = if chat_id < 0 {
        json!({
            "id": chat_id,
            "type": "supergroup",
            "title": "Group",
        })
    } else {
        json!({
            "id": chat_id,
            "type": "private",
            "first_name": state.bot_info.first_name,
        })
    };

    json!({
        "message_id": msg_id,
        "from": {
            "id": state.bot_info.id,
            "is_bot": true,
            "first_name": state.bot_info.first_name,
            "username": state.bot_info.username,
        },
        "chat": chat,
        "date": chrono::Local::now().timestamp(),
        "text": text,
    })
}

// ============================================================
// Console Backend (for testing)
// ============================================================

struct ConsoleBackend {
    msg_id_counter: Arc<AtomicI32>,
}

impl ConsoleBackend {
    fn new() -> Self {
        Self {
            msg_id_counter: Arc::new(AtomicI32::new(1)),
        }
    }
}

#[async_trait]
impl MessengerBackend for ConsoleBackend {
    fn name(&self) -> &str {
        "console"
    }

    async fn init(&mut self) -> Result<BotInfo, String> {
        Ok(BotInfo {
            id: 100,
            username: "console_bot".to_string(),
            first_name: "ConsoleBot".to_string(),
        })
    }

    async fn start(&self, tx: mpsc::Sender<IncomingMessage>) -> Result<(), String> {
        let counter = self.msg_id_counter.clone();

        tokio::task::spawn_blocking(move || {
            use std::io::BufRead;
            let stdin = std::io::stdin();
            let reader = stdin.lock();

            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => break,
                };
                let text = line.trim().to_string();
                if text.is_empty() {
                    continue;
                }

                let msg_id = counter.fetch_add(1, Ordering::Relaxed);
                let msg = IncomingMessage {
                    chat_id: 1,
                    message_id: msg_id,
                    from_id: 1000,
                    from_first_name: "ConsoleUser".to_string(),
                    from_username: Some("console_user".to_string()),
                    text: Some(text),
                    is_group: false,
                    group_title: None,
                    document: None,
                    photo: None,
                    caption: None,
                };

                if tx.blocking_send(msg).is_err() {
                    break;
                }
            }
        });

        Ok(())
    }

    async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        _parse_mode: Option<&str>,
    ) -> Result<SentMessage, String> {
        let clean = strip_html(text);
        println!("\n{}\n", clean);
        let msg_id = self.msg_id_counter.fetch_add(1, Ordering::Relaxed);
        Ok(SentMessage {
            message_id: msg_id,
            chat_id,
            text: Some(text.to_string()),
        })
    }

    async fn edit_message(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        _parse_mode: Option<&str>,
    ) -> Result<SentMessage, String> {
        let clean = strip_html(text);
        // Overwrite previous line with updated text
        print!("\x1b[2K\r{}", clean);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        Ok(SentMessage {
            message_id,
            chat_id,
            text: Some(text.to_string()),
        })
    }

    async fn delete_message(&self, _chat_id: i64, _message_id: i32) -> Result<bool, String> {
        Ok(true)
    }

    async fn send_document(
        &self,
        chat_id: i64,
        data: &[u8],
        filename: &str,
        caption: Option<&str>,
    ) -> Result<SentMessage, String> {
        let dir = std::env::temp_dir().join("cokacdir_bridge");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(filename);
        let _ = std::fs::write(&path, data);
        println!(
            "\n[File: {} ({} bytes) → {}]",
            filename,
            data.len(),
            path.display()
        );
        if let Some(cap) = caption {
            println!("  {}", cap);
        }
        let msg_id = self.msg_id_counter.fetch_add(1, Ordering::Relaxed);
        Ok(SentMessage {
            message_id: msg_id,
            chat_id,
            text: None,
        })
    }

    async fn get_file(&self, _file_id: &str) -> Result<FileInfo, String> {
        Err("Console backend does not support file downloads".to_string())
    }

    async fn get_file_data(&self, _file_path: &str) -> Result<Vec<u8>, String> {
        Err("Console backend does not support file downloads".to_string())
    }
}

/// Convert Telegram HTML to Discord Markdown, preserving formatting.
/// Handles: `<b>`, `<i>`, `<code>`, `<pre>`, and HTML entities.
fn telegram_html_to_discord(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut chars = html.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            for tc in chars.by_ref() {
                if tc == '>' {
                    break;
                }
                tag.push(tc);
            }
            match tag.as_str() {
                "b" => result.push_str("**"),
                "/b" => result.push_str("**"),
                "i" => result.push('*'),
                "/i" => result.push('*'),
                "code" => result.push('`'),
                "/code" => result.push('`'),
                "pre" => {
                    if !result.is_empty() && !result.ends_with('\n') {
                        result.push('\n');
                    }
                    result.push_str("```\n");
                }
                "/pre" => {
                    if !result.ends_with('\n') {
                        result.push('\n');
                    }
                    result.push_str("```");
                }
                _ => {} // strip unknown tags
            }
        } else if c == '&' {
            let mut entity = String::new();
            for ec in chars.by_ref() {
                if ec == ';' {
                    break;
                }
                entity.push(ec);
            }
            match entity.as_str() {
                "lt" => result.push('<'),
                "gt" => result.push('>'),
                "amp" => result.push('&'),
                "quot" => result.push('"'),
                _ => {
                    result.push('&');
                    result.push_str(&entity);
                    result.push(';');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

fn strip_html(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

// ============================================================
// Discord Backend
// ============================================================

/// File metadata stored for later download via get_file / get_file_data
#[derive(Clone)]
struct StoredFile {
    url: String,
    #[allow(dead_code)]
    filename: String,
    #[allow(dead_code)]
    mime_type: Option<String>,
    size: Option<u64>,
}

/// Shared state between Discord EventHandler and DiscordBackend methods.
/// Uses std::sync::Mutex (not tokio) because critical sections are very short
/// (HashMap lookups/inserts only, no I/O).
struct DiscordState {
    msg_counter: AtomicI32,
    file_counter: AtomicI32,
    /// telegram msg_id → (discord_channel_id, discord_message_id)
    tg_to_discord: std::sync::Mutex<HashMap<i32, (u64, u64)>>,
    /// (chat_id, discord_message_id) → telegram msg_id
    discord_to_tg: std::sync::Mutex<HashMap<(i64, u64), i32>>,
    /// file_id string → stored file info
    files: std::sync::Mutex<HashMap<String, StoredFile>>,
}

impl DiscordState {
    fn new() -> Self {
        Self {
            msg_counter: AtomicI32::new(1),
            file_counter: AtomicI32::new(1),
            tg_to_discord: std::sync::Mutex::new(HashMap::new()),
            discord_to_tg: std::sync::Mutex::new(HashMap::new()),
            files: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Get or create a Telegram-compatible i32 message ID for a Discord message.
    fn map_message_id(&self, chat_id: i64, discord_msg_id: u64) -> i32 {
        let key = (chat_id, discord_msg_id);
        let mut d2t = self.discord_to_tg.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(&id) = d2t.get(&key) {
            return id;
        }
        let new_id = self.msg_counter.fetch_add(1, Ordering::Relaxed);
        d2t.insert(key, new_id);
        drop(d2t);
        self.tg_to_discord
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(new_id, (chat_id_to_channel_u64(chat_id), discord_msg_id));
        new_id
    }

    /// Resolve a Telegram message ID back to (discord_channel_id, discord_message_id).
    fn resolve_message_id(&self, tg_msg_id: i32) -> Option<(u64, u64)> {
        self.tg_to_discord
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&tg_msg_id)
            .copied()
    }

    /// Store a Discord file URL for later download, returning a bridge file_id.
    fn store_file(
        &self,
        url: String,
        filename: String,
        mime_type: Option<String>,
        size: Option<u64>,
    ) -> String {
        let id = self.file_counter.fetch_add(1, Ordering::Relaxed);
        let file_id = format!("df_{}", id);
        self.files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                file_id.clone(),
                StoredFile {
                    url,
                    filename,
                    mime_type,
                    size,
                },
            );
        file_id
    }

    fn get_stored_file(&self, file_id: &str) -> Option<StoredFile> {
        self.files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(file_id)
            .cloned()
    }
}

/// Convert Discord channel ID to Telegram-compatible chat ID.
/// Guild channels → negative (triggers group chat logic), DM → positive (private chat).
fn discord_chat_id(channel_id: u64, is_guild: bool) -> i64 {
    let id = (channel_id & 0x7FFFFFFFFFFFFFFF) as i64;
    if is_guild {
        -id
    } else {
        id
    }
}

/// Convert Telegram chat ID back to Discord channel ID (u64).
/// Guards against zero (serenity ChannelId requires NonZeroU64).
fn chat_id_to_channel_u64(chat_id: i64) -> u64 {
    let v = chat_id.unsigned_abs();
    if v == 0 { 1 } else { v }
}

struct DiscordBackend {
    token: String,
    http: Option<Arc<serenity::http::Http>>,
    state: Arc<DiscordState>,
}

impl DiscordBackend {
    fn new(token: String) -> Self {
        Self {
            token,
            http: None,
            state: Arc::new(DiscordState::new()),
        }
    }

    fn http(&self) -> Result<&Arc<serenity::http::Http>, String> {
        self.http
            .as_ref()
            .ok_or_else(|| "Discord not initialized".to_string())
    }
}

/// serenity EventHandler that converts Discord messages to IncomingMessage
struct DiscordHandler {
    tx: mpsc::Sender<IncomingMessage>,
    state: Arc<DiscordState>,
}

#[async_trait]
impl serenity::all::EventHandler for DiscordHandler {
    async fn message(&self, ctx: serenity::all::Context, msg: serenity::all::Message) {
        // Ignore bot messages (including our own)
        if msg.author.bot {
            return;
        }

        let is_guild = msg.guild_id.is_some();
        let chat_id = discord_chat_id(msg.channel_id.get(), is_guild);
        let tg_msg_id = self.state.map_message_id(chat_id, msg.id.get());

        // Classify first attachment as image (photo) or generic file (document)
        let first_att = msg.attachments.first();
        let is_image = first_att
            .map(|a| {
                a.content_type
                    .as_ref()
                    .map(|ct| ct.starts_with("image/"))
                    .unwrap_or(false)
                    && a.width.is_some()
            })
            .unwrap_or(false);

        let (document, photo) = if let Some(att) = first_att {
            let file_id = self.state.store_file(
                att.url.clone(),
                att.filename.clone(),
                att.content_type.clone(),
                Some(att.size as u64),
            );
            if is_image {
                (
                    None,
                    Some(vec![PhotoAttachment {
                        file_id,
                        width: att.width.unwrap_or(0) as u32,
                        height: att.height.unwrap_or(0) as u32,
                        file_size: Some(att.size as u64),
                    }]),
                )
            } else {
                (
                    Some(FileAttachment {
                        file_id,
                        file_name: Some(att.filename.clone()),
                        mime_type: att.content_type.clone(),
                        file_size: Some(att.size as u64),
                    }),
                    None,
                )
            }
        } else {
            (None, None)
        };

        let has_attachment = document.is_some() || photo.is_some();
        let text = if msg.content.is_empty() {
            None
        } else {
            Some(msg.content.clone())
        };

        // Guild name from cache (falls back to "Discord")
        let group_title = if is_guild {
            msg.guild_id
                .and_then(|gid| ctx.cache.guild(gid).map(|g| g.name.clone()))
                .or_else(|| Some("Discord".to_string()))
        } else {
            None
        };

        let incoming = IncomingMessage {
            chat_id,
            message_id: tg_msg_id,
            from_id: msg.author.id.get(),
            from_first_name: msg.author.name.clone(),
            from_username: Some(msg.author.name.clone()),
            text: if has_attachment { None } else { text.clone() },
            is_group: is_guild,
            group_title,
            document,
            photo,
            caption: if has_attachment { text } else { None },
        };

        let _ = self.tx.send(incoming).await;
    }

    async fn ready(&self, _ctx: serenity::all::Context, ready: serenity::all::Ready) {
        println!(
            "  ✓ Discord gateway: {} ({})",
            ready.user.name, ready.user.id
        );
    }
}

#[async_trait]
impl MessengerBackend for DiscordBackend {
    fn name(&self) -> &str {
        "discord"
    }

    async fn init(&mut self) -> Result<BotInfo, String> {
        let http = Arc::new(serenity::http::Http::new(&self.token));
        let user = http
            .get_current_user()
            .await
            .map_err(|e| format!("Discord auth failed: {}", e))?;

        self.http = Some(http);

        Ok(BotInfo {
            id: user.id.get() as i64,
            username: user.name.clone(),
            first_name: user.name.clone(),
        })
    }

    async fn start(&self, tx: mpsc::Sender<IncomingMessage>) -> Result<(), String> {
        let handler = DiscordHandler {
            tx,
            state: self.state.clone(),
        };

        let intents = serenity::all::GatewayIntents::GUILD_MESSAGES
            | serenity::all::GatewayIntents::DIRECT_MESSAGES
            | serenity::all::GatewayIntents::MESSAGE_CONTENT;

        let mut client = serenity::all::Client::builder(&self.token, intents)
            .event_handler(handler)
            .await
            .map_err(|e| format!("Discord client error: {}", e))?;

        tokio::spawn(async move {
            if let Err(e) = client.start().await {
                eprintln!("  ✗ Discord gateway error: {}", e);
            }
        });

        Ok(())
    }

    async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<SentMessage, String> {
        let http = self.http()?;
        let channel = serenity::all::ChannelId::new(chat_id_to_channel_u64(chat_id));
        let clean = match parse_mode {
            Some("Html") | Some("HTML") | Some("html") => telegram_html_to_discord(text),
            Some(_) => strip_html(text),
            None => text.to_string(),
        };
        // Discord rejects empty messages
        let clean = if clean.trim().is_empty() {
            "\u{200b}".to_string() // zero-width space
        } else {
            clean
        };

        // Discord 2000 char limit — split if needed
        let chunks = split_discord_message(&clean);
        let mut last_msg_id = 0i32;

        for chunk in &chunks {
            let sent = channel
                .say(http.as_ref(), chunk)
                .await
                .map_err(|e| format!("Discord send: {}", e))?;
            last_msg_id = self.state.map_message_id(chat_id, sent.id.get());
        }

        Ok(SentMessage {
            message_id: last_msg_id,
            chat_id,
            text: Some(clean),
        })
    }

    async fn edit_message(
        &self,
        chat_id: i64,
        message_id: i32,
        text: &str,
        parse_mode: Option<&str>,
    ) -> Result<SentMessage, String> {
        let http = self.http()?;
        let (channel_u64, discord_msg_u64) = self
            .state
            .resolve_message_id(message_id)
            .ok_or_else(|| format!("Unknown msg ID: {}", message_id))?;
        let channel = serenity::all::ChannelId::new(channel_u64);
        let msg_id = serenity::all::MessageId::new(discord_msg_u64);
        let clean = match parse_mode {
            Some("Html") | Some("HTML") | Some("html") => telegram_html_to_discord(text),
            Some(_) => strip_html(text),
            None => text.to_string(),
        };

        // Discord rejects empty messages
        let clean = if clean.trim().is_empty() {
            "\u{200b}".to_string()
        } else {
            clean
        };

        // Truncate for Discord's 2000 char limit (streaming edits may exceed)
        let display = if clean.len() > 2000 {
            let mut end = 1997;
            while end > 0 && !clean.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…", &clean[..end])
        } else {
            clean.clone()
        };

        let edit = serenity::all::EditMessage::new().content(&display);
        channel
            .edit_message(http.as_ref(), msg_id, edit)
            .await
            .map_err(|e| format!("Discord edit: {}", e))?;

        Ok(SentMessage {
            message_id,
            chat_id,
            text: Some(clean),
        })
    }

    async fn delete_message(&self, _chat_id: i64, message_id: i32) -> Result<bool, String> {
        let http = self.http()?;
        let (channel_u64, discord_msg_u64) = self
            .state
            .resolve_message_id(message_id)
            .ok_or_else(|| format!("Unknown msg ID: {}", message_id))?;
        let channel = serenity::all::ChannelId::new(channel_u64);
        let msg_id = serenity::all::MessageId::new(discord_msg_u64);

        channel
            .delete_message(http.as_ref(), msg_id)
            .await
            .map_err(|e| format!("Discord delete: {}", e))?;
        Ok(true)
    }

    async fn send_document(
        &self,
        chat_id: i64,
        data: &[u8],
        filename: &str,
        caption: Option<&str>,
    ) -> Result<SentMessage, String> {
        let http = self.http()?;
        let channel = serenity::all::ChannelId::new(chat_id_to_channel_u64(chat_id));

        let attachment = serenity::all::CreateAttachment::bytes(data.to_vec(), filename);
        let mut builder = serenity::all::CreateMessage::new().add_file(attachment);
        if let Some(cap) = caption {
            let clean = strip_html(cap);
            if clean.len() <= 2000 {
                builder = builder.content(clean);
            }
        }

        let sent = channel
            .send_message(http.as_ref(), builder)
            .await
            .map_err(|e| format!("Discord send_document: {}", e))?;
        let tg_msg_id = self.state.map_message_id(chat_id, sent.id.get());

        Ok(SentMessage {
            message_id: tg_msg_id,
            chat_id,
            text: None,
        })
    }

    async fn get_file(&self, file_id: &str) -> Result<FileInfo, String> {
        let stored = self
            .state
            .get_stored_file(file_id)
            .ok_or_else(|| format!("File not found: {}", file_id))?;
        Ok(FileInfo {
            file_id: file_id.to_string(),
            file_path: stored.url,
            file_size: stored.size,
        })
    }

    async fn get_file_data(&self, file_path: &str) -> Result<Vec<u8>, String> {
        // file_path is a Discord CDN URL stored by store_file
        let resp = reqwest::get(file_path)
            .await
            .map_err(|e| format!("Download failed: {}", e))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("Read failed: {}", e))?;
        Ok(bytes.to_vec())
    }
}

/// Split text into Discord-compatible chunks (max 2000 chars each).
/// Tries to split at newlines or spaces for readability.
fn split_discord_message(text: &str) -> Vec<String> {
    const MAX: usize = 2000;
    if text.len() <= MAX {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut pos = 0;
    while pos < text.len() {
        if text.len() - pos <= MAX {
            chunks.push(text[pos..].to_string());
            break;
        }
        let mut end = pos + MAX;
        while !text.is_char_boundary(end) && end > pos {
            end -= 1;
        }
        let chunk = &text[pos..end];
        let split = chunk
            .rfind('\n')
            .or_else(|| chunk.rfind(' '))
            .map(|p| pos + p + 1);
        let split = match split {
            Some(s) if s > pos => s,
            _ => end,
        };
        chunks.push(text[pos..split].to_string());
        pos = split;
    }
    if chunks.is_empty() {
        chunks.push(text.to_string());
    }
    chunks
}

// ============================================================
// Public entry point
// ============================================================

/// Run the messenger bridge.
///
/// `backend_name`: "console", "discord", etc.
/// `args`: backend-specific arguments
pub async fn run_bridge(backend_name: &str, args: &[String]) {
    let mut backend: Box<dyn MessengerBackend> = match backend_name {
        "console" => Box::new(ConsoleBackend::new()),
        "discord" => {
            let token = match args.first() {
                Some(t) => t.clone(),
                None => {
                    eprintln!("Error: Discord bridge requires a bot token");
                    eprintln!("Usage: cokacdir --ccserver <DISCORD_BOT_TOKEN>");
                    std::process::exit(1);
                }
            };
            Box::new(DiscordBackend::new(token))
        }
        other => {
            eprintln!(
                "Error: Unknown messenger backend '{}'. Supported: console, discord",
                other
            );
            std::process::exit(1);
        }
    };

    // Initialize backend
    let bot_info = match backend.init().await {
        Ok(info) => {
            println!("  ✓ Backend: {} (@{})", info.first_name, info.username);
            info
        }
        Err(e) => {
            eprintln!("  ✗ Backend init failed: {}", e);
            std::process::exit(1);
        }
    };

    // Message channel: backend → proxy → teloxide
    let (tx, rx) = mpsc::channel(256);

    // Start backend listener
    let backend_arc: Arc<dyn MessengerBackend> = Arc::from(backend);
    {
        let backend_clone = backend_arc.clone();
        tokio::spawn(async move {
            if let Err(e) = backend_clone.start(tx).await {
                eprintln!("  ✗ Backend listener error: {}", e);
            }
        });
    }

    // Generate a stable bridge token for telegram.rs settings storage.
    // Hash the real token to avoid exposing it in URL paths and debug logs.
    let token_discriminator = args.first()
        .map(|t| crate::services::telegram::token_hash(t))
        .unwrap_or_else(|| "default".to_string());
    let bridge_token = format!("bridge_{}_{}", backend_name, token_discriminator);

    // Proxy state
    let state = Arc::new(ProxyState {
        backend: backend_arc,
        bot_info,
        update_rx: Mutex::new(rx),
        update_id_counter: AtomicI64::new(1),
        expected_token: bridge_token.clone(),
    });

    // Bind local proxy server
    let listener = match TcpListener::bind("127.0.0.1:0").await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("  ✗ Failed to bind proxy server: {}", e);
            std::process::exit(1);
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let api_url = format!("http://127.0.0.1:{}", port);
    println!("  ✓ Proxy: {}", api_url);

    // Start proxy server
    let proxy_state = state.clone();
    tokio::spawn(run_proxy_server(proxy_state, listener));

    // Run the existing telegram bot logic — it connects to our proxy
    crate::services::telegram::run_bot(&bridge_token, Some(&api_url)).await;
}
