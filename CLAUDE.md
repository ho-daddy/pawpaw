# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Pawpaw is a persistent AI agent with soul, memory, and autonomy — built on [cokacdir](https://github.com/kstost/cokacdir), a multi-panel terminal file manager written in Rust (~60K lines). The TUI app includes a file editor, image viewer, AI chat, Git integration, SSH/SFTP, encryption, Telegram/Discord bot, and process manager. Built on Ratatui + Crossterm.

**Core principle**: All AI providers are invoked as spawned CLI subprocesses (not SDK libraries) — zero additional API costs.

## CRITICAL: Do Not Change Design Without Permission

- **NEVER change product design/UX without explicit user request**
- Bug fix and design change are completely different things
- If you identify a "potential improvement" or "UX issue", only REPORT it - do NOT implement
- When user says "fix it", fix only the BUGS, not your suggestions
- If you think design change is needed, ASK FIRST before implementing
- Violating this rule wastes user's time and breaks trust

## Build Guidelines

- **IMPORTANT: Only build when the user explicitly requests it**
- Never run build commands automatically after code changes
- Never run build commands to "verify" or "check" code
- Do not use `cargo build`, `python3 build.py`, or any build commands unless user asks
- Focus only on code modifications; user handles all builds manually

### Build Commands (when requested)

Build uses a Python-based cross-compilation framework. See `build_manual.md` for full details.

```bash
python3 build.py                # Native release build
python3 build.py --debug        # Debug build (faster compile)
python3 build.py --all          # All platforms (except Windows)
python3 build.py --windows      # Windows builds
python3 build.py --setup        # Install all build tools
python3 build.py --status       # Check tool status
```

Output binaries go to `dist/` (release) or `dist_beta/` (development).

### Running Tests

```bash
cargo test                          # All tests
cargo test <test_name>              # Single test (e.g., test_copy_single_file)
cargo test -- --nocapture           # With stdout output
```

Tests are in `tests/file_operations.rs` (integration tests for file ops using `tempfile` crate).

## Version Management

- Version is defined in `Cargo.toml` (line 3: `version = "x.x.x"`)
- All version displays use `env!("CARGO_PKG_VERSION")` macro to read from Cargo.toml
- To update version: only modify `Cargo.toml`, all other locations reflect automatically
- Never hardcode version strings in source code

## Theme Color System

- All color definitions must use `Color::Indexed(number)` format directly
- Each UI element must have its own uniquely named color field, even if the color value is the same as another element
- Never reference another element's color (e.g., don't use `theme.bg_selected` for viewer search input)
- Define dedicated color fields in the appropriate Colors struct (e.g., `ViewerColors.search_input_text`)
- Color values may be duplicated across fields, but names must be unique and semantically meaningful

### Theme File Locations

- **Source of truth**: `src/ui/theme.rs` - theme color values and JSON comments are defined here
- **Generated files**: `~/.cokacdir/themes/*.json` - user config files generated at runtime
- Always modify `src/ui/theme.rs` for theme changes (never edit generated JSON directly)
- JSON comment format: `"__field__": "description"` - these comments are also defined in `to_json()` in theme.rs

## Architecture

### CLI Mode Dispatch

`src/main.rs` handles CLI argument parsing and dispatches to mutually exclusive modes:
- **Default (TUI)**: Interactive multi-panel file manager with event loop
- **`--prompt <TEXT>`**: Direct AI query — spawns Claude CLI subprocess, renders markdown to stdout
- **`--bridge <BACKEND>`**: AI provider bridge — translates Claude-compatible args to other provider CLIs (e.g., gemini)
- **`--ccserver <TOKEN>...`**: Bot server — runs persistent Telegram/Discord bots with HTTP API proxy
- **`--cron` / `--cron-list` / `--cron-remove` / `--cron-update`**: Schedule management for bot automation
- **`--voice <TEXT>`**: Voice chat interface for voice interaction
- **`--message`**: Inter-bot messaging
- **`--read_chat_log`**: Read group chat shared context logs
- **`--design`**: Theme hot-reload mode for development

TUI startup: `init_bin_path()` → `deploy_docs()` → config init → `enable_raw_mode()` → `App::new()` → render/event loop → cleanup

### Module Layout

- **`src/main.rs`** - Entry point, CLI arg parsing, top-level handler functions
- **`src/config.rs`** - Settings management (`~/.cokacdir/settings.json`)
- **`src/keybindings.rs`** - Keyboard event mapping and action dispatch

**`src/ui/`** - TUI rendering (Ratatui-based):
- `app.rs` - Main application state machine (~6650 lines). 14 screens via `Screen` enum, separate `DialogType` enum for modals. Event loop driven by 100ms Crossterm tick + keyboard events.
- `draw.rs` - Low-level rendering primitives
- `dialogs.rs` - All modal dialogs (create, delete, rename, etc.)
- `ai_screen.rs` - AI chat interface
- `file_viewer.rs` / `file_editor.rs` - Text viewing and editing with syntax highlighting
- `image_viewer.rs` - Image rendering (Kitty, iTerm2, Sixel protocols)
- `diff_screen.rs` / `diff_file_view.rs` - Side-by-side diff
- `git_screen.rs` - Git operations UI
- `advanced_search.rs` / `search_result.rs` - Advanced file search UI
- `dedup_screen.rs` - Duplicate file detection UI
- `file_info.rs` / `system_info.rs` - File and system info panels
- `process_manager.rs` - Process manager UI
- `panel.rs` - Panel layout primitives
- `help.rs` - Help screen
- `theme.rs` / `theme_loader.rs` - Color system (100+ fields, JSON theme loading)
- `syntax.rs` - Syntax highlighting engine

**`src/services/`** - Backend business logic:
- `file_ops.rs` - File operations with progress tracking via mpsc channels (`ProgressMessage`)
- `claude.rs`, `codex.rs`, `gemini.rs`, `opencode.rs` - AI provider subprocess wrappers (spawned via `Command::new`, communicate over stdin/stdout pipes)
- `agent.rs` - Persistent agent system: merges `~/.cokacdir/agent/{SOUL,IDENTITY,USER,MEMORY,AGENT,HEARTBEAT}.md` into system prompt. MEMORY truncated to 8K chars. Daily memos in `agent/daily/`
- `context_trigger.rs` - 5-layer semantic context system: matches keywords from frontmatter `triggers` fields to auto-load relevant context from `projects/*/wiki.md` and `knowledge/wiki/*.md`
- `telegram.rs` - Telegram bot server (largest file, ~10.2K lines)
- `messenger_bridge.rs` / `bridge.rs` - Multi-messenger abstraction: MessengerBackend trait → HTTP proxy → Telegram Bot API (enables Discord/Slack without modifying telegram.rs)
- `remote.rs` / `remote_transfer.rs` - SSH/SFTP connections via `russh` (no OpenSSH dependency)
- `process.rs` - Process monitoring via `/proc` (Unix)
- `dedup.rs` - Duplicate file detection with MD5 hashing

**`src/enc/`** - AES-256-CBC encryption (mod.rs, crypto.rs, naming.rs, error.rs)

**`src/utils/`** - Helpers (markdown rendering, path formatting)

### Language & Localization

- Code (variable names, comments in source) is in English
- User-facing documentation (README, build manual, commit messages) is primarily in Korean
- Commit messages are typically written in Korean — follow this convention

### Key Patterns

- **AI provider discovery**: Uses `which <provider>` with fallback to `bash -lc "which <provider>"` for SSH sessions without loaded profiles
- **AI system prompts**: Written to temp files in `~/.cokacdir/system_prompt/`, passed via `--append-system-prompt-file`
- **Async**: Tokio runtime for AI streaming, file transfers, bot operations (TUI rendering is synchronous)
- **Platform-specific code**: Uses `cfg!(target_os = ...)` conditionals (no Cargo feature flags)
- **Clippy lints**: `unwrap_used` and `expect_used` are warnings - handle Results/Options properly
- **TLS**: Uses `rustls` everywhere (no OpenSSL dependency)
- **Config directory**: `~/.cokacdir/` (settings, themes, docs, database, schedules, agent/, system_prompt/, bot_settings.json, .cokacdir.db)
- **Binary self-resolution**: `std::env::current_exe()` cached in `OnceLock` at startup, used for respawning
- **Agent file hierarchy**: `~/.cokacdir/agent/` contains `SOUL.md`, `IDENTITY.md`, `USER.md`, `MEMORY.md`, `AGENT.md`, `HEARTBEAT.md` — all merged into AI system prompts by `agent.rs`. MEMORY auto-summarizes when exceeding 50KB. Daily memos stored in `agent/daily/`
- **Cross-compilation**: Build system uses Zig as the C cross-compiler toolchain, installed locally in `builder/tools/` to avoid polluting the system environment
