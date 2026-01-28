# COKACDIR

Norton Commander style dual-panel file manager for terminal with AI-powered natural language commands.

**Terminal File Manager for Vibe Coders** - An easy terminal explorer for vibe coders who are scared of the terminal.

## Features

- **Blazing Fast**: Written in Rust for maximum performance. ~10ms startup, ~5MB memory usage, ~4MB static binary with zero runtime dependencies.
- **AI-Powered Commands**: Natural language file operations powered by Claude AI. Press `.` and describe what you want.
- **Dual-Panel Navigation**: Classic Norton Commander style interface
- **Keyboard Driven**: Full keyboard navigation designed for power users
- **Built-in Viewer & Editor**: View and edit files directly without leaving the application
- **Process Manager**: Monitor and manage system processes. Sort by CPU, memory, or PID.

## Installation

### Quick Install (Recommended)

```bash
/bin/bash -c "$(curl -fsSL https://cokacdir.cokac.com/install.sh)"
```

Then run:

```bash
cokacdir
```

### From Source

```bash
# Clone the repository
git clone https://github.com/kstost/cokacdir.git
cd cokacdir

# Build release version
cargo build --release

# Run
./target/release/cokacdir
```

### Cross-Platform Build

Build for multiple platforms using the included build system:

```bash
# Build for current platform
python3 build.py

# Build for macOS (arm64 + x86_64)
python3 build.py --macos

# Build for all platforms
python3 build.py --all

# Check build tools status
python3 build.py --status
```

See [build_manual.md](build_manual.md) for detailed build instructions.

## Enable AI Commands (Optional)

Install Claude Code to unlock natural language file operations:

```bash
npm install -g @anthropic-ai/claude-code
```

Learn more at [docs.anthropic.com](https://docs.anthropic.com/en/docs/claude-code)

## Keyboard Shortcuts

### Navigation

| Key | Action |
|-----|--------|
| `↑`/`↓` | Navigate files |
| `Enter` | Open directory |
| `Esc` | Parent directory |
| `Tab` | Switch panels |
| `Home`/`End` | First / Last item |
| `PgUp`/`PgDn` | Move 10 lines |
| `/` | Go to path |

### File Operations

| Key | Action |
|-----|--------|
| `c` | Copy |
| `m` | Move |
| `k` | Create directory |
| `x` | Delete |
| `r` | Rename |

### View & Tools

| Key | Action |
|-----|--------|
| `h` | Help |
| `o` | File info |
| `v` | View file |
| `e` | Edit file |
| `p` | Process manager |

### Selection & AI

| Key | Action |
|-----|--------|
| `Space` | Select file |
| `*` | Select all |
| `n` / `s` / `d` | Sort by name / size / date |
| `.` | AI command |
| `q` | Quit |

### File Viewer

| Key | Action |
|-----|--------|
| `q` | Close viewer |
| `/` | Search |
| `n` | Next match |
| `N` | Previous match |
| `↑`/`↓`/`j`/`k` | Scroll |
| `PgUp`/`PgDn` | Page scroll |
| `g`/`G` | Go to start/end |

### File Editor

| Key | Action |
|-----|--------|
| `Ctrl+S` | Save |
| `Ctrl+Q` | Quit (warns if unsaved) |
| `Ctrl+X` | Discard changes and quit |
| `Arrows` | Navigate |
| `Tab` | Insert spaces |

### Process Manager

| Key | Action |
|-----|--------|
| `k` | Kill process (SIGTERM) |
| `9` | Force kill (SIGKILL) |
| `r` | Refresh list |
| `p` | Sort by PID |
| `c` | Sort by CPU |
| `m` | Sort by memory |
| `n` | Sort by command name |
| `Esc` | Close |

## Supported Platforms

- macOS (Apple Silicon & Intel)
- Linux (x86_64 & ARM64)

## License

MIT License

## Author

cokac <monogatree@gmail.com>

Homepage: https://cokacdir.cokac.com
