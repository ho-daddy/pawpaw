import { useState, useEffect } from 'react'
import { Link } from 'react-router-dom'
import { motion, AnimatePresence } from 'framer-motion'
import {
  ArrowLeft, Github, Send, StopCircle, TerminalSquare, ShieldCheck, FileUp, Layers,
  Cloud, Apple, Monitor, BookOpen, CheckCircle, Download,
  Upload, BarChart3, Ban, Plus, Minus
} from 'lucide-react'
import { LanguageProvider, useLanguage } from '../tutorial/LanguageContext'

function LanguageToggle() {
  const { lang, setLang } = useLanguage()
  return (
    <div className="flex items-center border border-zinc-700 rounded-md overflow-hidden text-xs">
      <button
        onClick={() => setLang('en')}
        className={`px-2 py-1 font-semibold transition-colors ${
          lang === 'en'
            ? 'bg-accent-cyan/20 text-accent-cyan'
            : 'text-zinc-500 hover:text-zinc-300'
        }`}
      >
        EN
      </button>
      <div className="w-px h-4 bg-zinc-700" />
      <button
        onClick={() => setLang('ko')}
        className={`px-2 py-1 font-semibold transition-colors ${
          lang === 'ko'
            ? 'bg-accent-cyan/20 text-accent-cyan'
            : 'text-zinc-500 hover:text-zinc-300'
        }`}
      >
        KO
      </button>
    </div>
  )
}

function ChatBubble({ from, children }: { from: 'user' | 'bot'; children: React.ReactNode }) {
  const isUser = from === 'user'
  return (
    <motion.div
      initial={{ opacity: 0, y: 10 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3 }}
      className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}
    >
      <div
        className={`max-w-[85%] px-4 py-2.5 rounded-2xl text-sm leading-relaxed ${
          isUser
            ? 'bg-accent-cyan/20 border border-accent-cyan/30 text-zinc-200 rounded-br-sm'
            : 'bg-bg-card border border-zinc-700 text-zinc-300 rounded-bl-sm'
        }`}
      >
        {children}
      </div>
    </motion.div>
  )
}

interface ChatMessage {
  from: 'user' | 'bot'
  en: React.ReactNode
  ko: React.ReactNode
}

interface Scenario {
  id: string
  icon: typeof StopCircle
  label: string
  labelKo: string
  description: string
  descriptionKo: string
  keyPoints: { en: string; ko: string }[]
  messages: ChatMessage[]
  multiChat?: { botName: string; messages: ChatMessage[] }[]
}

function useScenarios(): Scenario[] {
  return [
    {
      id: 'stop',
      icon: StopCircle,
      label: 'Interrupt Anytime',
      labelKo: '언제든 중단',
      description: 'Noticed the AI going in the wrong direction? Send /stop to immediately halt the current task. You stay in full control — stop, redirect, and continue at any moment.',
      descriptionKo: 'AI가 잘못된 방향으로 진행 중인가요? /stop을 보내면 즉시 작업을 중단합니다. 중단하고, 방향을 바꾸고, 다시 진행하세요 — 항상 당신이 통제합니다.',
      keyPoints: [
        { en: 'Stop any running task instantly with /stop', ko: '/stop으로 실행 중인 작업 즉시 중단' },
        { en: 'Redirect AI to a different approach', ko: 'AI를 다른 방향으로 즉시 전환' },
        { en: 'No wasted time on wrong changes', ko: '잘못된 변경에 시간 낭비 없음' },
      ],
      messages: [
        {
          from: 'user',
          en: 'Refactor the database module to use async connections.',
          ko: '데이터베이스 모듈을 async 연결로 리팩토링해줘.',
        },
        {
          from: 'bot',
          en: <>
            <span className="text-zinc-400">Working on it... Modifying src/db.rs, src/pool.rs, src/queries.rs...</span>
          </>,
          ko: <>
            <span className="text-zinc-400">작업 중... src/db.rs, src/pool.rs, src/queries.rs 수정 중...</span>
          </>,
        },
        {
          from: 'user',
          en: <><code className="text-red-400 font-mono text-xs">/stop</code></>,
          ko: <><code className="text-red-400 font-mono text-xs">/stop</code></>,
        },
        {
          from: 'bot',
          en: <>
            <div className="flex items-center gap-1.5">
              <Ban className="w-3 h-3 text-red-400" />
              <span className="text-red-400 text-xs font-medium">Task stopped</span>
            </div>
          </>,
          ko: <>
            <div className="flex items-center gap-1.5">
              <Ban className="w-3 h-3 text-red-400" />
              <span className="text-red-400 text-xs font-medium">작업 중단됨</span>
            </div>
          </>,
        },
        {
          from: 'user',
          en: 'Don\'t change the connection pool. Only refactor the query functions.',
          ko: '커넥션 풀은 바꾸지 마. 쿼리 함수만 리팩토링해.',
        },
        {
          from: 'bot',
          en: <>
            <span className="text-accent-green">Done!</span> Refactored 8 query functions to async in src/queries.rs. Connection pool unchanged.
          </>,
          ko: <>
            <span className="text-accent-green">완료!</span> src/queries.rs의 쿼리 함수 8개를 async로 리팩토링했습니다. 커넥션 풀은 변경 없음.
          </>,
        },
      ],
    },
    {
      id: 'shell',
      icon: TerminalSquare,
      label: 'Direct Shell Access',
      labelKo: '직접 쉘 접근',
      description: 'Run shell commands directly on your server with the ! prefix. Results come back raw — completely separate from the AI conversation context. Perfect for quick server checks.',
      descriptionKo: '! 접두사로 서버에서 쉘 명령을 직접 실행하세요. 결과는 가공 없이 그대로 돌아오며, AI 대화 컨텍스트와 완전히 분리됩니다. 빠른 서버 확인에 완벽합니다.',
      keyPoints: [
        { en: 'Run any shell command with ! prefix', ko: '! 접두사로 쉘 명령 실행' },
        { en: 'Raw output — not processed by AI', ko: '가공 없는 원본 출력 — AI가 처리하지 않음' },
        { en: 'Separate from AI context — quick server checks', ko: 'AI 컨텍스트와 분리 — 빠른 서버 확인용' },
      ],
      messages: [
        {
          from: 'user',
          en: <><code className="text-accent-cyan font-mono text-xs">!df -h</code></>,
          ko: <><code className="text-accent-cyan font-mono text-xs">!df -h</code></>,
        },
        {
          from: 'bot',
          en: <>
            <div className="font-mono text-xs text-zinc-400 whitespace-pre leading-relaxed">Filesystem      Size  Used Avail Use% Mounted on{'\n'}/dev/xvda1       32G   18G   14G  57% /{'\n'}tmpfs           2.0G     0  2.0G   0% /dev/shm</div>
          </>,
          ko: <>
            <div className="font-mono text-xs text-zinc-400 whitespace-pre leading-relaxed">Filesystem      Size  Used Avail Use% Mounted on{'\n'}/dev/xvda1       32G   18G   14G  57% /{'\n'}tmpfs           2.0G     0  2.0G   0% /dev/shm</div>
          </>,
        },
        {
          from: 'user',
          en: <><code className="text-accent-cyan font-mono text-xs">!pm2 list</code></>,
          ko: <><code className="text-accent-cyan font-mono text-xs">!pm2 list</code></>,
        },
        {
          from: 'bot',
          en: <>
            <div className="font-mono text-xs text-zinc-400 whitespace-pre leading-relaxed">┌────┬──────────┬─────┬──────┬───────┐{'\n'}│ id │ name     │ mode│ pid  │ status│{'\n'}├────┼──────────┼─────┼──────┼───────┤{'\n'}│ 0  │ api      │ fork│ 1234 │ <span className="text-accent-green">online</span>│{'\n'}│ 1  │ worker   │ fork│ 1235 │ <span className="text-red-400">errored</span>│{'\n'}└────┴──────────┴─────┴──────┴───────┘</div>
          </>,
          ko: <>
            <div className="font-mono text-xs text-zinc-400 whitespace-pre leading-relaxed">┌────┬──────────┬─────┬──────┬───────┐{'\n'}│ id │ name     │ mode│ pid  │ status│{'\n'}├────┼──────────┼─────┼──────┼───────┤{'\n'}│ 0  │ api      │ fork│ 1234 │ <span className="text-accent-green">online</span>│{'\n'}│ 1  │ worker   │ fork│ 1235 │ <span className="text-red-400">errored</span>│{'\n'}└────┴──────────┴─────┴──────┴───────┘</div>
          </>,
        },
        {
          from: 'user',
          en: <><code className="text-accent-cyan font-mono text-xs">!pm2 restart worker</code></>,
          ko: <><code className="text-accent-cyan font-mono text-xs">!pm2 restart worker</code></>,
        },
        {
          from: 'bot',
          en: <>
            <div className="font-mono text-xs text-zinc-400">[PM2] Applying action restartProcessId on app [worker](ids: 1){'\n'}[PM2] [worker](1) ✓</div>
          </>,
          ko: <>
            <div className="font-mono text-xs text-zinc-400">[PM2] Applying action restartProcessId on app [worker](ids: 1){'\n'}[PM2] [worker](1) ✓</div>
          </>,
        },
      ],
    },
    {
      id: 'permissions',
      icon: ShieldCheck,
      label: 'Tool Permissions',
      labelKo: '권한 관리',
      description: 'Dynamically control what the AI can do. Use /allowed +tool to enable and /allowed -tool to disable. Check available tools with /availabletools and current permissions with /allowedtools. (Claude only — not supported in Codex mode)',
      descriptionKo: 'AI가 할 수 있는 일을 동적으로 제어하세요. /allowed +tool로 활성화, /allowed -tool로 비활성화. /availabletools로 전체 도구 확인, /allowedtools로 현재 권한 확인. (Claude 전용 — Codex 모드에서는 미지원)',
      keyPoints: [
        { en: '/allowed +tool to enable, -tool to disable', ko: '/allowed +tool로 활성화, -tool로 비활성화' },
        { en: '/availabletools to see all 20 tools', ko: '/availabletools로 전체 20개 도구 확인' },
        { en: 'Settings persist across bot restarts', ko: '설정이 봇 재시작 후에도 유지됨' },
      ],
      messages: [
        {
          from: 'user',
          en: <><code className="text-accent-cyan font-mono text-xs">/allowedtools</code></>,
          ko: <><code className="text-accent-cyan font-mono text-xs">/allowedtools</code></>,
        },
        {
          from: 'bot',
          en: <>
            <strong className="text-white text-xs">Allowed Tools (17)</strong>
            <div className="mt-1 font-mono text-xs text-zinc-400">
              Bash, Read, Edit, Write, Glob, Grep, Task, TaskOutput, TaskStop, WebFetch, WebSearch, NotebookEdit, Skill, TaskCreate, TaskGet, TaskUpdate, TaskList
            </div>
          </>,
          ko: <>
            <strong className="text-white text-xs">허용된 도구 (17)</strong>
            <div className="mt-1 font-mono text-xs text-zinc-400">
              Bash, Read, Edit, Write, Glob, Grep, Task, TaskOutput, TaskStop, WebFetch, WebSearch, NotebookEdit, Skill, TaskCreate, TaskGet, TaskUpdate, TaskList
            </div>
          </>,
        },
        {
          from: 'user',
          en: <><code className="text-accent-cyan font-mono text-xs">/allowed -Bash</code></>,
          ko: <><code className="text-accent-cyan font-mono text-xs">/allowed -Bash</code></>,
        },
        {
          from: 'bot',
          en: <>
            <div className="flex items-center gap-1.5">
              <Minus className="w-3 h-3 text-red-400" />
              <span className="text-red-400 text-xs font-medium">Removed:</span>
              <code className="text-accent-cyan font-mono text-xs">Bash</code>
            </div>
            <div className="mt-1 text-xs text-zinc-400">AI can no longer execute shell commands.</div>
          </>,
          ko: <>
            <div className="flex items-center gap-1.5">
              <Minus className="w-3 h-3 text-red-400" />
              <span className="text-red-400 text-xs font-medium">제거:</span>
              <code className="text-accent-cyan font-mono text-xs">Bash</code>
            </div>
            <div className="mt-1 text-xs text-zinc-400">AI가 더 이상 쉘 명령을 실행할 수 없습니다.</div>
          </>,
        },
        {
          from: 'user',
          en: <><code className="text-accent-cyan font-mono text-xs">/allowed +Bash</code></>,
          ko: <><code className="text-accent-cyan font-mono text-xs">/allowed +Bash</code></>,
        },
        {
          from: 'bot',
          en: <>
            <div className="flex items-center gap-1.5">
              <Plus className="w-3 h-3 text-accent-green" />
              <span className="text-accent-green text-xs font-medium">Added:</span>
              <code className="text-accent-cyan font-mono text-xs">Bash</code>
            </div>
            <div className="mt-1 text-xs text-zinc-400">AI can now execute shell commands again.</div>
          </>,
          ko: <>
            <div className="flex items-center gap-1.5">
              <Plus className="w-3 h-3 text-accent-green" />
              <span className="text-accent-green text-xs font-medium">추가:</span>
              <code className="text-accent-cyan font-mono text-xs">Bash</code>
            </div>
            <div className="mt-1 text-xs text-zinc-400">AI가 다시 쉘 명령을 실행할 수 있습니다.</div>
          </>,
        },
      ],
    },
    {
      id: 'files',
      icon: FileUp,
      label: 'File Management',
      labelKo: '파일 관리',
      description: 'Upload files from your phone for AI analysis, or download generated results. Perfect for quick data processing, log analysis, or document conversion tasks.',
      descriptionKo: '폰에서 파일을 업로드해 AI 분석을 받거나, 생성된 결과를 다운로드하세요. 빠른 데이터 처리, 로그 분석, 문서 변환 작업에 완벽합니다.',
      keyPoints: [
        { en: 'Upload files directly from your phone', ko: '폰에서 직접 파일 업로드' },
        { en: 'AI analyzes and processes uploaded data', ko: 'AI가 업로드된 데이터 분석 및 처리' },
        { en: 'Download results or generated files', ko: '결과물 또는 생성된 파일 다운로드' },
      ],
      messages: [
        {
          from: 'user',
          en: <>
            <div className="flex items-center gap-2 mb-1">
              <Upload className="w-3 h-3 text-accent-cyan" />
              <span className="text-xs text-accent-cyan">server_logs.txt</span>
              <span className="text-xs text-zinc-500">(2.4 MB)</span>
            </div>
            Analyze this log file. Find errors and summarize.
          </>,
          ko: <>
            <div className="flex items-center gap-2 mb-1">
              <Upload className="w-3 h-3 text-accent-cyan" />
              <span className="text-xs text-accent-cyan">server_logs.txt</span>
              <span className="text-xs text-zinc-500">(2.4 MB)</span>
            </div>
            이 로그 파일 분석해줘. 에러 찾고 요약해줘.
          </>,
        },
        {
          from: 'bot',
          en: <>
            <span className="text-zinc-400">Analyzing 24,832 log entries...</span>
            <div className="mt-2">
              <BarChart3 className="w-3 h-3 inline mr-1 text-accent-cyan" />
              <strong className="text-white">Analysis Complete</strong>
            </div>
            <div className="mt-1 text-xs space-y-1">
              <div><span className="text-red-400">23 errors</span> — mostly DB connection timeouts</div>
              <div><span className="text-yellow-400">156 warnings</span> — deprecated API calls</div>
              <div><span className="text-accent-green">24,653 normal</span></div>
            </div>
            <div className="mt-2 text-xs text-zinc-400">Peak error time: 14:30-15:00 UTC</div>
          </>,
          ko: <>
            <span className="text-zinc-400">24,832개 로그 항목 분석 중...</span>
            <div className="mt-2">
              <BarChart3 className="w-3 h-3 inline mr-1 text-accent-cyan" />
              <strong className="text-white">분석 완료</strong>
            </div>
            <div className="mt-1 text-xs space-y-1">
              <div><span className="text-red-400">23개 에러</span> — 대부분 DB 연결 타임아웃</div>
              <div><span className="text-yellow-400">156개 경고</span> — deprecated API 호출</div>
              <div><span className="text-accent-green">24,653개 정상</span></div>
            </div>
            <div className="mt-2 text-xs text-zinc-400">에러 피크 시간: 14:30-15:00 UTC</div>
          </>,
        },
        {
          from: 'user',
          en: 'Create a CSV report of just the errors with timestamps.',
          ko: '에러만 타임스탬프와 함께 CSV 리포트 만들어줘.',
        },
        {
          from: 'bot',
          en: <>
            <span className="text-accent-green">Report generated!</span>
            <div className="mt-2 flex items-center gap-2">
              <Download className="w-3 h-3 text-accent-cyan" />
              <span className="text-xs text-accent-cyan font-mono">error_report.csv</span>
              <span className="text-xs text-zinc-500">(4.2 KB)</span>
            </div>
            <div className="mt-1 text-xs text-zinc-400">23 error entries with timestamp, level, source, and message columns.</div>
          </>,
          ko: <>
            <span className="text-accent-green">리포트 생성 완료!</span>
            <div className="mt-2 flex items-center gap-2">
              <Download className="w-3 h-3 text-accent-cyan" />
              <span className="text-xs text-accent-cyan font-mono">error_report.csv</span>
              <span className="text-xs text-zinc-500">(4.2 KB)</span>
            </div>
            <div className="mt-1 text-xs text-zinc-400">23개 에러 항목 — timestamp, level, source, message 컬럼 포함.</div>
          </>,
        },
      ],
    },
    {
      id: 'multi',
      icon: Layers,
      label: 'Multi-Session',
      labelKo: '다중 세션',
      description: 'Create multiple Telegram bots — one per project. Each bot has its own token and runs independently, so you can work on different projects in parallel just by switching between Telegram chat windows.',
      descriptionKo: '프로젝트별로 텔레그램 봇을 만드세요. 각 봇은 자체 토큰으로 독립 실행되어, 텔레그램 채팅 창만 전환하면 여러 프로젝트를 병렬로 작업할 수 있습니다.',
      keyPoints: [
        { en: 'One bot per project — fully isolated', ko: '프로젝트당 봇 하나 — 완전히 격리' },
        { en: 'Switch projects by switching Telegram chats', ko: '텔레그램 채팅 전환으로 프로젝트 전환' },
        { en: 'Each bot keeps independent context & history', ko: '각 봇이 독립적 컨텍스트 & 기록 유지' },
      ],
      multiChat: [
        {
          botName: 'api-bot',
          messages: [
            { from: 'user' as const, en: <><code className="text-accent-cyan font-mono text-xs">/start ~/project-api</code></>, ko: <><code className="text-accent-cyan font-mono text-xs">/start ~/project-api</code></> },
            { from: 'bot' as const, en: <>Session started at <code className="text-accent-cyan font-mono text-xs">~/project-api</code></>, ko: <><code className="text-accent-cyan font-mono text-xs">~/project-api</code>에서 세션 시작</> },
            { from: 'user' as const, en: 'Add JWT authentication to the API.', ko: 'API에 JWT 인증 추가해줘.' },
            { from: 'bot' as const, en: <><span className="text-accent-green">Done!</span> Added JWT auth middleware with token validation and refresh endpoint.</>, ko: <><span className="text-accent-green">완료!</span> JWT 인증 미들웨어와 토큰 검증, 갱신 엔드포인트를 추가했습니다.</> },
          ],
        },
        {
          botName: 'web-bot',
          messages: [
            { from: 'user' as const, en: <><code className="text-accent-cyan font-mono text-xs">/start ~/project-web</code></>, ko: <><code className="text-accent-cyan font-mono text-xs">/start ~/project-web</code></> },
            { from: 'bot' as const, en: <>Session started at <code className="text-accent-cyan font-mono text-xs">~/project-web</code></>, ko: <><code className="text-accent-cyan font-mono text-xs">~/project-web</code>에서 세션 시작</> },
            { from: 'user' as const, en: 'Add a login page that connects to the API.', ko: 'API에 연결되는 로그인 페이지 만들어줘.' },
            { from: 'bot' as const, en: <><span className="text-accent-green">Done!</span> Created login page with form validation, API integration, and error handling.</>, ko: <><span className="text-accent-green">완료!</span> 폼 검증, API 연동, 에러 처리가 포함된 로그인 페이지를 생성했습니다.</> },
          ],
        },
      ],
      messages: [],
    },
  ]
}

function WorkflowsPageInner() {
  const { t } = useLanguage()
  const scenarios = useScenarios()
  const [activeTab, setActiveTab] = useState(0)

  useEffect(() => {
    window.scrollTo(0, 0)
  }, [])

  const active = scenarios[activeTab]

  return (
    <div className="min-h-screen bg-bg-dark">
      {/* Top navigation bar */}
      <header className="fixed top-0 left-0 right-0 z-30 bg-bg-dark/80 backdrop-blur-md border-b border-zinc-800">
        <div className="max-w-5xl mx-auto px-4 h-16 flex items-center justify-between">
          <div className="flex items-center gap-4">
            <Link
              to="/"
              className="flex items-center gap-2 text-zinc-400 hover:text-white transition-colors"
            >
              <ArrowLeft className="w-4 h-4" />
              <span className="text-sm">Home</span>
            </Link>
            <div className="hidden sm:block h-5 w-px bg-zinc-700" />
            <Link to="/" className="hidden sm:block">
              <span className="gradient-text font-bold text-lg">cokacdir</span>
            </Link>
          </div>

          <div className="flex items-center gap-3">
            <LanguageToggle />
            <span className="text-white font-semibold flex items-center gap-2">
              <Send className="w-4 h-4 text-accent-cyan" />
              <span className="hidden sm:inline">{t('Workflows', '워크플로우')}</span>
            </span>
          </div>

          <a
            href="https://github.com/kstost/cokacdir"
            target="_blank"
            rel="noopener noreferrer"
            className="flex items-center gap-2 text-zinc-400 hover:text-white transition-colors"
          >
            <Github className="w-4 h-4" />
            <span className="text-sm hidden sm:inline">GitHub</span>
          </a>
        </div>
      </header>

      {/* Main content */}
      <main className="max-w-5xl mx-auto px-4 pt-24 pb-16">
        {/* Page header */}
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5 }}
          className="mb-10 text-center"
        >
          <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-accent-cyan/10 border border-accent-cyan/30 text-accent-cyan text-sm font-medium mb-4">
            <Send className="w-4 h-4" />
            {t('Telegram Bot Workflows', '텔레그램 봇 워크플로우')}
          </div>
          <h1 className="text-3xl sm:text-4xl lg:text-5xl font-extrabold text-white mb-4">
            {t('See It in Action', '실제 사용 모습')}
          </h1>
          <p className="text-base sm:text-lg text-zinc-400 leading-relaxed max-w-2xl mx-auto">
            {t(
              'Real-world scenarios showing how you can use the Telegram bot to manage code, deploy, review, and more — all from your phone.',
              '텔레그램 봇으로 코드 관리, 배포, 리뷰 등을 처리하는 실전 시나리오를 확인하세요 — 모든 것을 폰에서.'
            )}
          </p>
        </motion.div>

        {/* Scenario tabs */}
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5, delay: 0.1 }}
          className="mb-8"
        >
          <div className="flex flex-wrap justify-center gap-2">
            {scenarios.map((s, i) => {
              const Icon = s.icon
              const isActive = i === activeTab
              return (
                <button
                  key={s.id}
                  onClick={() => setActiveTab(i)}
                  className={`flex items-center gap-2 px-4 py-2.5 rounded-lg text-sm font-medium transition-all duration-200 cursor-pointer ${
                    isActive
                      ? 'bg-accent-cyan/20 border border-accent-cyan/40 text-accent-cyan shadow-lg shadow-accent-cyan/10'
                      : 'bg-bg-card border border-zinc-800 text-zinc-400 hover:text-zinc-200 hover:border-zinc-600'
                  }`}
                >
                  <Icon className="w-4 h-4" />
                  <span className="hidden sm:inline">{t(s.label, s.labelKo)}</span>
                  <span className="sm:hidden">{t(s.label.split(' ')[0], s.labelKo.split(' ')[0])}</span>
                </button>
              )
            })}
          </div>
        </motion.div>

        {/* Active scenario content */}
        <AnimatePresence mode="wait">
          <motion.div
            key={active.id}
            initial={{ opacity: 0, y: 15 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -15 }}
            transition={{ duration: 0.3 }}
            className="grid grid-cols-1 lg:grid-cols-5 gap-8 items-start"
          >
            {/* Left: Chat UI (3/5) */}
            <div className="lg:col-span-3">
              {active.multiChat ? (
                <div className="space-y-4">
                  {active.multiChat.map((chat) => (
                    <div key={chat.botName} className="relative">
                      <div className="absolute inset-0 bg-gradient-to-r from-accent-cyan/20 via-primary/10 to-accent-cyan/20 rounded-xl blur-lg opacity-20" />
                      <div className="relative bg-bg-dark border border-zinc-700 rounded-xl overflow-hidden shadow-2xl">
                        <div className="flex items-center gap-3 px-4 py-2.5 bg-bg-card border-b border-zinc-800">
                          <div className="w-7 h-7 rounded-full bg-accent-cyan/20 flex items-center justify-center">
                            <Send className="w-3.5 h-3.5 text-accent-cyan" />
                          </div>
                          <div>
                            <div className="text-sm text-white font-medium">{chat.botName}</div>
                            <div className="text-xs text-zinc-500">online</div>
                          </div>
                        </div>
                        <div className="p-3 space-y-2.5">
                          {chat.messages.map((msg, i) => (
                            <ChatBubble key={`${chat.botName}-${i}`} from={msg.from}>
                              {t(msg.en, msg.ko)}
                            </ChatBubble>
                          ))}
                        </div>
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="relative">
                  <div className="absolute inset-0 bg-gradient-to-r from-accent-cyan/20 via-primary/10 to-accent-cyan/20 rounded-xl blur-lg opacity-30" />
                  <div className="relative bg-bg-dark border border-zinc-700 rounded-xl overflow-hidden shadow-2xl">
                    {/* Title bar */}
                    <div className="flex items-center gap-3 px-4 py-3 bg-bg-card border-b border-zinc-800">
                      <div className="w-8 h-8 rounded-full bg-accent-cyan/20 flex items-center justify-center">
                        <Send className="w-4 h-4 text-accent-cyan" />
                      </div>
                      <div>
                        <div className="text-sm text-white font-medium">cokacdir Bot</div>
                        <div className="text-xs text-zinc-500">online</div>
                      </div>
                    </div>

                    {/* Chat body */}
                    <div className="p-4 space-y-3 min-h-[340px]">
                      {active.messages.map((msg, i) => (
                        <ChatBubble key={`${active.id}-${i}`} from={msg.from}>
                          {t(msg.en, msg.ko)}
                        </ChatBubble>
                      ))}
                    </div>

                    {/* Input bar */}
                    <div className="px-4 py-3 bg-bg-card border-t border-zinc-800">
                      <div className="flex items-center gap-2 px-3 py-2 bg-bg-dark rounded-lg border border-zinc-700">
                        <span className="text-zinc-600 text-sm flex-1">{t('Type a message...', '메시지를 입력하세요...')}</span>
                        <Send className="w-4 h-4 text-zinc-600" />
                      </div>
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* Right: Description + Key Points (2/5) */}
            <div className="lg:col-span-2 space-y-6">
              {/* Scenario title */}
              <div>
                <div className="flex items-center gap-3 mb-3">
                  <div className="w-10 h-10 rounded-lg bg-accent-cyan/10 border border-accent-cyan/20 flex items-center justify-center">
                    <active.icon className="w-5 h-5 text-accent-cyan" />
                  </div>
                  <h2 className="text-xl font-bold text-white">
                    {t(active.label, active.labelKo)}
                  </h2>
                </div>
                <p className="text-zinc-400 text-sm leading-relaxed">
                  {t(active.description, active.descriptionKo)}
                </p>
              </div>

              {/* Key points */}
              <div className="bg-bg-card border border-zinc-800 rounded-xl p-5">
                <h3 className="text-sm font-semibold text-zinc-300 mb-3 uppercase tracking-wider">
                  {t('Key Points', '핵심 포인트')}
                </h3>
                <div className="space-y-3">
                  {active.keyPoints.map((kp, i) => (
                    <div key={i} className="flex items-start gap-3">
                      <CheckCircle className="w-4 h-4 text-accent-green mt-0.5 flex-shrink-0" />
                      <span className="text-sm text-zinc-300">{t(kp.en, kp.ko)}</span>
                    </div>
                  ))}
                </div>
              </div>

              {/* Scenario indicator */}
              <div className="flex items-center gap-2">
                {scenarios.map((_, i) => (
                  <button
                    key={i}
                    onClick={() => setActiveTab(i)}
                    className={`h-1.5 rounded-full transition-all duration-300 cursor-pointer ${
                      i === activeTab
                        ? 'w-8 bg-accent-cyan'
                        : 'w-3 bg-zinc-700 hover:bg-zinc-500'
                    }`}
                    aria-label={`Scenario ${i + 1}`}
                  />
                ))}
              </div>
            </div>
          </motion.div>
        </AnimatePresence>

        {/* CTA section */}
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.5 }}
          className="mt-16 bg-bg-card border border-zinc-800 rounded-xl p-6 sm:p-8 text-center"
        >
          <h3 className="text-xl font-bold text-white mb-2">
            {t('Ready to Try?', '시작할 준비가 되셨나요?')}
          </h3>
          <p className="text-zinc-400 text-sm mb-6 max-w-lg mx-auto">
            {t(
              'Set up your own cokacdir Telegram bot and start controlling AI agents from anywhere.',
              '나만의 cokacdir 텔레그램 봇을 설정하고 어디서든 AI 에이전트를 제어하세요.'
            )}
          </p>
          <div className="flex flex-wrap justify-center gap-3">
            <Link
              to="/ec2"
              className="inline-flex items-center gap-2 px-6 py-3 rounded-lg bg-accent-cyan text-bg-dark font-bold hover:bg-accent-cyan/90 shadow-lg shadow-accent-cyan/25 transition-all duration-200"
            >
              <Cloud className="w-4 h-4" />
              {t('EC2 Setup', 'EC2 설정')}
            </Link>
            <Link
              to="/macos"
              className="inline-flex items-center gap-2 px-6 py-3 rounded-lg bg-accent-purple text-white font-bold hover:bg-accent-purple/90 shadow-lg shadow-accent-purple/25 transition-all duration-200"
            >
              <Apple className="w-4 h-4" />
              {t('macOS Setup', 'macOS 설정')}
            </Link>
            <Link
              to="/windows"
              className="inline-flex items-center gap-2 px-6 py-3 rounded-lg bg-zinc-800 border border-zinc-600 text-zinc-200 font-bold hover:bg-zinc-700 transition-all duration-200"
            >
              <Monitor className="w-4 h-4" />
              {t('Windows Setup', 'Windows 설정')}
            </Link>
            <Link
              to="/tutorial"
              className="inline-flex items-center gap-2 px-6 py-3 rounded-lg border border-zinc-700 text-zinc-300 font-semibold hover:text-white hover:border-zinc-500 transition-all duration-200"
            >
              <BookOpen className="w-4 h-4" />
              {t('Tutorial', '튜토리얼')}
            </Link>
          </div>
        </motion.div>

        {/* Bottom navigation */}
        <div className="mt-12 pt-8 border-t border-zinc-800 flex flex-col sm:flex-row items-center justify-between gap-4">
          <Link
            to="/"
            className="flex items-center gap-2 text-zinc-400 hover:text-accent-cyan transition-colors"
          >
            <ArrowLeft className="w-4 h-4" />
            {t('Back to Home', '홈으로 돌아가기')}
          </Link>
          <Link
            to="/tutorial"
            className="flex items-center gap-2 text-zinc-400 hover:text-accent-cyan transition-colors"
          >
            {t('Beginner Tutorial →', '초보자 튜토리얼 →')}
          </Link>
        </div>
      </main>
    </div>
  )
}

export default function WorkflowsPage() {
  return (
    <LanguageProvider>
      <WorkflowsPageInner />
    </LanguageProvider>
  )
}
