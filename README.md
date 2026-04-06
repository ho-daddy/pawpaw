# Pawpaw

**A persistent AI agent with soul, memory, and autonomy — powered by CLI.**

Pawpaw is a terminal-based personal assistant that controls Claude Code (and other AI coding agents) through direct CLI execution — no API keys, no additional costs. It maintains persistent identity, long-term memory, daily journals, and scheduled tasks across sessions, turning a stateless coding agent into a continuous personal companion.

Built on [cokacdir](https://github.com/kstost/cokacdir), a Rust terminal file manager.

## What Makes Pawpaw Different

Most AI integrations are stateless — every conversation starts from zero. Pawpaw gives your agent **continuity**:

- **Soul & Identity**: The agent reads `SOUL.md` and `IDENTITY.md` at every session start, maintaining consistent personality and behavior
- **User Memory**: `USER.md` stores what the agent learns about you — your name, preferences, projects, working style
- **Long-Term Memory**: `MEMORY.md` accumulates facts, decisions, and learnings across all sessions
- **Daily Journal**: Automatic `daily_memo_YYYY_MM_DD.md` files track daily work, conversations, and follow-ups
- **Heartbeat**: Scheduled tasks via cron expressions in `HEARTBEAT.md` — the agent acts autonomously on a timer
- **Session Continuity**: Automatic session resume across restarts via `LAST_SESSION.txt`

## Core Principle

**Everything runs through CLI — no API calls.** AI providers are controlled by spawning their CLI binaries as child processes:

- `claude` (Claude Code)
- `codex` (Codex CLI)
- `gemini` (Gemini CLI)
- `opencode` (OpenCode)

This means Pawpaw runs within each agent's existing subscription (or free tier) with **zero additional API costs**.

## Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) toolchain
- [Python 3](https://www.python.org/) (for build script)
- One of: [Claude Code](https://claude.ai/code), [Codex CLI](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli), or [OpenCode](https://github.com/opencode-ai/opencode) installed and available in PATH

### Install & Run

**1. Clone and build:**

```bash
git clone https://github.com/ho-daddy/pawpaw.git
cd pawpaw
python3 build.py            # Native release build (debug: --debug)
```

빌드가 완료되면 `dist_beta/` 디렉토리에 바이너리가 생성됩니다.

**2. Run the built binary:**

```bash
# Linux
./dist_beta/cokacdir-linux-x86_64       # x86_64
./dist_beta/cokacdir-linux-aarch64      # ARM64

# macOS
./dist_beta/cokacdir-macos-aarch64      # Apple Silicon
./dist_beta/cokacdir-macos-x86_64       # Intel
```

**3. Initialize the agent:**

앱이 실행되면 `.` 키를 눌러 AI 화면으로 진입한 뒤, `/agent init` 명령어를 입력하세요.
`~/.cokacdir/agent/` 에 에이전트 설정 파일들이 생성되며, 이를 편집하여 에이전트를 커스터마이즈할 수 있습니다.

```
~/.cokacdir/agent/
├── SOUL.md          # Personality, values, communication style
├── IDENTITY.md      # Name, role, capabilities
├── USER.md          # What the agent knows about you
├── MEMORY.md        # Long-term memory (auto-summarized at 50KB)
├── AGENT.md         # Behavioral guidelines
├── HEARTBEAT.md     # Scheduled tasks (cron format)
├── workspace/       # Agent's free working directory
└── daily/           # Daily memo files
```

### Agent Commands

| Command | Description |
|---------|-------------|
| `/agent init` | Initialize agent files and directories |
| `/agent status` | Show agent status, paths, and session info |
| `/agent reset-session` | Clear saved session and start fresh |
| `/agent memory` | Show memory size and summarization status |

## Features

### From cokacdir (Terminal File Manager)

- **Multi-panel navigation** with keyboard-first design
- **Built-in editor** with syntax highlighting (20+ languages)
- **Image viewer** (Kitty, iTerm2, Sixel)
- **Git integration** (status, commit, log, branch, diff)
- **SSH/SFTP** remote file access
- **AES-256 file encryption**
- **Process manager**, **diff viewer**, **duplicate detection**
- **Telegram bot** for remote AI sessions
- **Customizable themes** (light/dark with JSON color config)

### Pawpaw Agent System

- **Persistent identity** — consistent personality across all sessions
- **Long-term memory** — accumulates and auto-summarizes when too large
- **Daily journaling** — automatic daily memos with work logs
- **Heartbeat scheduler** — cron-based autonomous task execution
- **Session resume** — picks up where you left off
- **Full autonomy** — runs with `--dangerously-skip-permissions` for uninterrupted operation
- **Workspace** — dedicated directory for agent drafts and working files

## HEARTBEAT.md Format

Define scheduled tasks using cron expressions:

```markdown
## Active Tasks
- [cron: 0 9 * * *] Create today's daily memo and review yesterday's work
- [cron: 0 18 * * *] Summarize today's work in the daily memo
- [cron: 0 0 * * 0] Weekly review — summarize the week and update MEMORY.md
```

## Supported Platforms

- macOS (Apple Silicon & Intel)
- Linux (x86_64 & ARM64)
- Windows (x86_64 & ARM64)

## Tech Stack

- **Language**: Rust (~60K lines)
- **TUI**: Ratatui + Crossterm
- **Async**: Tokio
- **TLS**: rustls (no OpenSSL dependency)
- **Build**: Python-based cross-compilation (Zig toolchain)

## License

MIT License

## Credits

Based on [cokacdir](https://github.com/kstost/cokacdir) by [cokac](mailto:monogatree@gmail.com).

## Disclaimer

THIS SOFTWARE IS PROVIDED "AS IS," WITHOUT WARRANTY OF ANY KIND. The user assumes full responsibility for all consequences arising from the use of this software. AI agents running in autonomous mode may execute system commands — use with caution.
