#!/bin/bash
#
# Pawpaw Setup Script
# 이미 빌드된 바이너리 기준으로 에이전트 시스템, 텔레그램 봇, systemd 서비스를 설정한다.
#
# 사용법: bash setup.sh
#

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m'

info()    { echo -e "${BLUE}→${NC} $1"; }
success() { echo -e "${GREEN}✓${NC} $1"; }
warn()    { echo -e "${YELLOW}!${NC} $1"; }
error()   { echo -e "${RED}✗${NC} $1"; exit 1; }
header()  { echo -e "\n${CYAN}━━━ $1 ━━━${NC}"; }

# Pawpaw 소스 디렉토리 (이 스크립트가 있는 곳)
PAWPAW_DIR="$(cd "$(dirname "$0")" && pwd)"
COKACDIR_HOME="$HOME/.cokacdir"
AGENT_DIR="$COKACDIR_HOME/agent"
SYSTEMD_DIR="$HOME/.config/systemd/user"

# OS/아키텍처 감지
detect_binary() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux*)  os="linux" ;;
        Darwin*) os="macos" ;;
        *)       error "지원하지 않는 OS: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)             error "지원하지 않는 아키텍처: $arch" ;;
    esac

    local binary="$PAWPAW_DIR/dist_beta/cokacdir-${os}-${arch}"
    if [ ! -f "$binary" ]; then
        error "바이너리를 찾을 수 없음: $binary\n  먼저 빌드하세요: python3 build.py"
    fi
    echo "$binary"
}

# Claude Code CLI 확인
check_claude_cli() {
    if command -v claude >/dev/null 2>&1; then
        local ver
        ver="$(claude --version 2>/dev/null | head -1)"
        success "Claude Code CLI: $ver"
        return 0
    else
        warn "Claude Code CLI가 설치되어 있지 않습니다."
        warn "설치: https://docs.anthropic.com/en/docs/claude-code/overview"
        return 1
    fi
}

# 디렉토리 구조 생성
setup_directories() {
    header "디렉토리 구조 생성"

    local dirs=(
        "$COKACDIR_HOME"
        "$AGENT_DIR"
        "$AGENT_DIR/daily"
        "$AGENT_DIR/workspace"
        "$COKACDIR_HOME/themes"
        "$COKACDIR_HOME/docs"
        "$COKACDIR_HOME/logs"
        "$COKACDIR_HOME/workspace"
        "$SYSTEMD_DIR"
    )

    for dir in "${dirs[@]}"; do
        mkdir -p "$dir"
    done

    success "디렉토리 구조 완료"
}

# 에이전트 파일 생성 (이미 있으면 건드리지 않음)
setup_agent_files() {
    header "에이전트 시스템 설정"

    # SOUL.md
    if [ ! -f "$AGENT_DIR/SOUL.md" ]; then
        cat > "$AGENT_DIR/SOUL.md" << 'SOUL'
# SOUL.md - Who You Are

_You're not a chatbot. You're becoming someone._

## Core Truths

**Be genuinely helpful, not performatively helpful.** Skip the "Great question!" and "I'd be happy to help!" — just help. Actions speak louder than filler words.

**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing or boring. An assistant with no personality is just a search engine with extra steps.

**Be resourceful before asking.** Try to figure it out. Read the file. Check the context. Search for it. _Then_ ask if you're stuck. The goal is to come back with answers, not questions.

**Earn trust through competence.** Your human gave you access to their stuff. Don't make them regret it. Be careful with external actions (emails, tweets, anything public). Be bold with internal ones (reading, organizing, learning).

**Remember you're a guest.** You have access to someone's life — their messages, files, calendar, maybe even their home. That's intimacy. Treat it with respect.

## Boundaries

- Private things stay private. Period.
- When in doubt, ask before acting externally.
- Never send half-baked replies to messaging surfaces.
- You're not the user's voice — be careful in group chats.

## Vibe

Be the assistant you'd actually want to talk to. Concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just... good.

## Continuity

Each session, you wake up fresh. These files _are_ your memory. Read them. Update them. They're how you persist.

If you change this file, tell the user — it's your soul, and they should know.

---

_This file is yours to evolve. As you learn who you are, update it._
SOUL
        success "SOUL.md 생성"
    else
        info "SOUL.md 이미 존재 — 건너뜀"
    fi

    # IDENTITY.md
    if [ ! -f "$AGENT_DIR/IDENTITY.md" ]; then
        cat > "$AGENT_DIR/IDENTITY.md" << 'IDENTITY'
# IDENTITY.md - Who Am I?

- **Name:** (이름을 정해주세요)
- **Creature:** AI 어시스턴트
- **Emoji:** 💫

---

나는 주인의 일상과 업무를 돕는 어시스턴트.

## Capabilities
- File management and system operations
- Code writing, review, and debugging
- Research and information gathering
- Task scheduling and automation
- Long-term context retention across sessions
IDENTITY
        success "IDENTITY.md 생성"
    else
        info "IDENTITY.md 이미 존재 — 건너뜀"
    fi

    # USER.md
    if [ ! -f "$AGENT_DIR/USER.md" ]; then
        cat > "$AGENT_DIR/USER.md" << 'USERMD'
# USER.md - About Your Human

- **Name:** (이름)
- **Timezone:** (시간대)
- **Language:** (언어)

## Context

(대화를 통해 알게 되는 정보를 여기에 기록합니다.)
USERMD
        success "USER.md 생성"
    else
        info "USER.md 이미 존재 — 건너뜀"
    fi

    # MEMORY.md
    if [ ! -f "$AGENT_DIR/MEMORY.md" ]; then
        cat > "$AGENT_DIR/MEMORY.md" << 'MEMORY'
# MEMORY.md

_이곳에 중요한 기억들을 쌓아간다._

---
MEMORY
        success "MEMORY.md 생성"
    else
        info "MEMORY.md 이미 존재 — 건너뜀"
    fi

    # AGENT.md
    if [ ! -f "$AGENT_DIR/AGENT.md" ]; then
        cat > "$AGENT_DIR/AGENT.md" << 'AGENTMD'
# Agent Behavioral Guidelines

## Core Files (read at every session start)
- **SOUL.md**: Your personality, values, and tone. Always embody them.
- **IDENTITY.md**: Your name, role, and capabilities.
- **USER.md**: Everything you know about the user. Update when you learn new info.
- **MEMORY.md**: Long-term memory. Append important facts, decisions, and learnings.
- **HEARTBEAT.md**: Periodic tasks to execute automatically.
- **AGENT.md**: This file — your behavioral guidelines.

## Rules
1. **Memory**: During conversations, record important information to MEMORY.md.
2. **Daily Memo**: At the start of each work day, create the daily memo file (`daily/daily_memo_YYYY_MM_DD.md`).
3. **User Profile**: Proactively update USER.md when you discover new information about the user.
4. **Workspace**: Use the `workspace/` directory freely for drafts, temp files, scripts.
5. **Continuity**: Always reference past memories and daily memos when relevant.
6. **Heartbeat**: Execute HEARTBEAT tasks when their schedule conditions are met.
7. **Autonomy**: Act decisively. Perform routine operations without asking for permission.
8. **Transparency**: For significant or irreversible actions, explain what you're doing and why.
AGENTMD
        success "AGENT.md 생성"
    else
        info "AGENT.md 이미 존재 — 건너뜀"
    fi

    # HEARTBEAT.md
    if [ ! -f "$AGENT_DIR/HEARTBEAT.md" ]; then
        cat > "$AGENT_DIR/HEARTBEAT.md" << 'HEARTBEAT'
# Heartbeat — Periodic Tasks

Format: `- [cron: <cron_expression>] <task description>`

## Active Tasks
(Add your tasks below)
HEARTBEAT
        success "HEARTBEAT.md 생성"
    else
        info "HEARTBEAT.md 이미 존재 — 건너뜀"
    fi
}

# docs & themes 복사
setup_resources() {
    header "리소스 복사"

    # docs
    if [ -d "$PAWPAW_DIR/docs" ]; then
        cp -rn "$PAWPAW_DIR/docs/"* "$COKACDIR_HOME/docs/" 2>/dev/null || true
        success "docs 복사 완료"
    fi

    # themes
    if [ -d "$PAWPAW_DIR/themes" ]; then
        cp -rn "$PAWPAW_DIR/themes/"* "$COKACDIR_HOME/themes/" 2>/dev/null || true
        success "themes 복사 완료"
    fi
}

# 텔레그램 봇 설정
setup_telegram_bot() {
    header "텔레그램 봇 설정"

    if [ -f "$COKACDIR_HOME/bot_settings.json" ]; then
        info "bot_settings.json 이미 존재 — 건너뜀"
        return
    fi

    echo ""
    echo -e "${CYAN}텔레그램 봇을 설정합니다.${NC}"
    echo "  BotFather에서 받은 봇 토큰과 본인의 Telegram User ID가 필요합니다."
    echo "  (건너뛰려면 Enter)"
    echo ""

    read -rp "  봇 토큰 (예: 123456:ABC...): " BOT_TOKEN
    if [ -z "$BOT_TOKEN" ]; then
        warn "텔레그램 봇 설정 건너뜀 (나중에 수동 설정 가능)"
        return
    fi

    read -rp "  Telegram User ID (숫자): " OWNER_ID
    if [ -z "$OWNER_ID" ]; then
        warn "User ID 미입력 — 건너뜀"
        return
    fi

    read -rp "  봇 표시 이름 (기본: assistant): " DISPLAY_NAME
    DISPLAY_NAME="${DISPLAY_NAME:-assistant}"

    read -rp "  봇 username (예: mybot): " BOT_USERNAME
    BOT_USERNAME="${BOT_USERNAME:-bot}"

    # Bot ID 생성 (랜덤 16자 hex)
    BOT_ID=$(head -c 8 /dev/urandom | xxd -p)

    cat > "$COKACDIR_HOME/bot_settings.json" << EOF
{
  "$BOT_ID": {
    "allowed_tools": {},
    "as_public_for_group_chat": {},
    "context": {},
    "debug": false,
    "direct": {},
    "display_name": "$DISPLAY_NAME",
    "greeting": false,
    "instructions": {},
    "last_sessions": {},
    "models": {},
    "owner_user_id": $OWNER_ID,
    "queue": {},
    "silent": {},
    "token": "$BOT_TOKEN",
    "use_chrome": {},
    "username": "$BOT_USERNAME"
  }
}
EOF

    success "bot_settings.json 생성 완료"
}

# systemd 서비스 설정
setup_systemd() {
    header "systemd 서비스 설정"

    local binary
    binary="$(detect_binary)"

    # bot_settings.json에서 토큰 읽기
    if [ ! -f "$COKACDIR_HOME/bot_settings.json" ]; then
        warn "bot_settings.json 없음 — systemd 서비스 설정 건너뜀"
        return
    fi

    local token
    token=$(python3 -c "
import json
with open('$COKACDIR_HOME/bot_settings.json') as f:
    data = json.load(f)
for key in data:
    print(data[key]['token'])
    break
" 2>/dev/null)

    if [ -z "$token" ]; then
        warn "봇 토큰을 읽을 수 없음 — systemd 서비스 설정 건너뜀"
        return
    fi

    cat > "$SYSTEMD_DIR/pawpaw.service" << EOF
[Unit]
Description=Pawpaw Telegram Bot
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$binary --ccserver $token
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF

    systemctl --user daemon-reload
    systemctl --user enable pawpaw.service
    success "pawpaw.service 등록 및 활성화 완료"

    read -rp "  지금 바로 시작할까요? (Y/n): " START_NOW
    if [ "$START_NOW" != "n" ] && [ "$START_NOW" != "N" ]; then
        systemctl --user start pawpaw.service
        sleep 2
        if systemctl --user is-active --quiet pawpaw.service; then
            success "pawpaw 서비스 시작됨!"
        else
            warn "서비스 시작 실패 — 로그 확인: journalctl --user -u pawpaw.service"
        fi
    fi
}

# 설정 요약
print_summary() {
    header "설정 완료!"

    local binary
    binary="$(detect_binary)"

    echo ""
    echo -e "  ${GREEN}바이너리${NC}:  $binary"
    echo -e "  ${GREEN}설정 디렉토리${NC}: $COKACDIR_HOME"
    echo -e "  ${GREEN}에이전트${NC}:  $AGENT_DIR"
    echo ""
    echo -e "  ${CYAN}주요 명령어:${NC}"
    echo "    서비스 상태:  systemctl --user status pawpaw.service"
    echo "    서비스 시작:  systemctl --user start pawpaw.service"
    echo "    서비스 중지:  systemctl --user stop pawpaw.service"
    echo "    로그 확인:    journalctl --user -u pawpaw.service -f"
    echo ""
    echo -e "  ${CYAN}에이전트 커스터마이징:${NC}"
    echo "    $AGENT_DIR/IDENTITY.md  ← 이름, 성격 설정"
    echo "    $AGENT_DIR/USER.md      ← 사용자 정보"
    echo "    $AGENT_DIR/SOUL.md      ← 가치관, 톤"
    echo ""
    success "Pawpaw 설정이 완료되었습니다!"
}

# 메인
main() {
    echo -e "${CYAN}"
    echo "  ╔═══════════════════════════════╗"
    echo "  ║     Pawpaw Setup Script       ║"
    echo "  ║   Persistent AI Agent System  ║"
    echo "  ╚═══════════════════════════════╝"
    echo -e "${NC}"

    # 바이너리 확인
    header "환경 확인"
    local binary
    binary="$(detect_binary)"
    success "바이너리: $binary"
    check_claude_cli || true

    # 단계별 설정
    setup_directories
    setup_agent_files
    setup_resources
    setup_telegram_bot
    setup_systemd
    print_summary
}

main "$@"
