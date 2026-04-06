# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

COKACDIR is a multi-panel terminal file manager written in Rust (~60K lines) with built-in file editor, image viewer, AI chat, Git integration, SSH/SFTP, encryption, Telegram/Discord bot, and process manager. Built on Ratatui + Crossterm.

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

Output binaries go to `dist/`.

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

### Entry Point & Startup

`src/main.rs` handles CLI argument parsing and launches the TUI event loop. Key startup flow:
1. `init_bin_path()` / `deploy_docs()` / config init
2. CLI mode dispatch: TUI (default), `--prompt` (AI), `--ccserver` (bot), `--bridge` (AI provider), etc.
3. TUI mode: `enable_raw_mode()` -> `App::new()` -> render/event loop -> cleanup

### Module Layout

- **`src/main.rs`** - Entry point, CLI arg parsing, top-level handler functions
- **`src/config.rs`** - Settings management (`~/.cokacdir/settings.json`)
- **`src/keybindings.rs`** - Keyboard event mapping and action dispatch

**`src/ui/`** - TUI rendering (Ratatui-based):
- `app.rs` - Main application state machine and event loop (largest UI file)
- `draw.rs` - Low-level rendering primitives
- `dialogs.rs` - All modal dialogs (create, delete, rename, etc.)
- `ai_screen.rs` - AI chat interface
- `file_viewer.rs` / `file_editor.rs` - Text viewing and editing with syntax highlighting
- `diff_screen.rs` / `diff_file_view.rs` - Side-by-side diff
- `git_screen.rs` - Git operations UI
- `theme.rs` / `theme_loader.rs` - Color system (100+ fields, JSON theme loading)
- `syntax.rs` - Syntax highlighting engine

**`src/services/`** - Backend business logic:
- `file_ops.rs` - File operations with progress tracking
- `claude.rs`, `codex.rs`, `gemini.rs`, `opencode.rs` - AI provider bridges
- `telegram.rs` - Telegram bot server (largest file, ~10K lines)
- `messenger_bridge.rs` / `bridge.rs` - AI message routing
- `remote.rs` / `remote_transfer.rs` - SSH/SFTP connections
- `process.rs` - Process monitoring
- `dedup.rs` - Duplicate file detection

**`src/enc/`** - AES-256-CBC encryption (mod.rs, crypto.rs, naming.rs, error.rs)

**`src/utils/`** - Helpers (markdown rendering, path formatting)

### Key Patterns

- **Async**: Tokio runtime for AI streaming, file transfers, bot operations
- **Platform-specific code**: Uses `cfg!(target_os = ...)` conditionals (no Cargo feature flags)
- **Clippy lints**: `unwrap_used` and `expect_used` are warnings - handle Results/Options properly
- **TLS**: Uses `rustls` everywhere (no OpenSSL dependency)
- **Config directory**: `~/.cokacdir/` (settings, themes, docs, database, schedules)
