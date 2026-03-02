import { useEffect, useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { motion } from 'framer-motion'
import { ArrowLeft, Github, Monitor, Bot, Terminal, Copy, Check, Rocket, MessageCircle, Download, LogIn, Search, Clock } from 'lucide-react'
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

function Hl({ children }: { children: React.ReactNode }) {
  return <span className="text-yellow-400 bg-yellow-400/10 rounded px-0.5">{children}</span>
}

function CopyBlock({ code, label, children }: { code: string; label?: string; children?: React.ReactNode }) {
  const [copied, setCopied] = useState(false)
  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(code)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {}
  }
  return (
    <div className="my-4">
      {label && <p className="text-xs text-zinc-500 mb-1 font-mono">{label}</p>}
      <div className="relative group bg-bg-card border border-zinc-800 rounded-lg overflow-hidden hover:border-accent-cyan/30 transition-colors">
        <div className="flex items-start justify-between px-4 py-3 gap-2">
          <pre className="overflow-x-auto min-w-0 flex-1 font-mono text-accent-cyan text-xs sm:text-sm whitespace-pre leading-relaxed">
            {children || code}
          </pre>
          <button
            onClick={handleCopy}
            className="p-2 rounded-md bg-bg-elevated hover:bg-zinc-700 transition-colors shrink-0 mt-0.5"
            aria-label="Copy to clipboard"
          >
            {copied ? (
              <Check className="w-4 h-4 text-accent-green" />
            ) : (
              <Copy className="w-4 h-4 text-zinc-400 group-hover:text-white" />
            )}
          </button>
        </div>
      </div>
    </div>
  )
}

function SectionCard({ icon: Icon, title, step, children }: {
  icon: typeof Terminal
  title: string
  step: number
  children: React.ReactNode
}) {
  return (
    <motion.section
      initial={{ opacity: 0, y: 20 }}
      whileInView={{ opacity: 1, y: 0 }}
      viewport={{ once: true }}
      transition={{ duration: 0.5 }}
      className="mb-10"
    >
      <div className="flex items-center gap-3 mb-5">
        <div className="flex-shrink-0 w-10 h-10 rounded-full bg-accent-cyan/20 border border-accent-cyan/50 flex items-center justify-center text-accent-cyan font-bold text-lg">
          {step}
        </div>
        <div className="flex items-center gap-2">
          <Icon className="w-5 h-5 text-accent-cyan" />
          <h2 className="text-xl sm:text-2xl font-bold text-white">{title}</h2>
        </div>
      </div>
      <div className="ml-0 sm:ml-[52px] text-zinc-300 text-sm sm:text-base leading-relaxed space-y-4">
        {children}
      </div>
    </motion.section>
  )
}

function InlineStep({ n, children }: { n: number; children: React.ReactNode }) {
  return (
    <div className="flex gap-3 py-2">
      <span className="flex-shrink-0 w-6 h-6 rounded-full bg-accent-purple/20 border border-accent-purple/40 flex items-center justify-center text-accent-purple font-bold text-xs">
        {n}
      </span>
      <div className="text-zinc-300 text-sm sm:text-base leading-relaxed">{children}</div>
    </div>
  )
}

function WindowsPageInner() {
  const navigate = useNavigate()
  const { t } = useLanguage()

  useEffect(() => {
    window.scrollTo(0, 0)
  }, [])

  return (
    <div className="min-h-screen bg-bg-dark">
      {/* Top navigation bar */}
      <header className="fixed top-0 left-0 right-0 z-30 bg-bg-dark/80 backdrop-blur-md border-b border-zinc-800">
        <div className="max-w-4xl mx-auto px-4 h-16 flex items-center justify-between">
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
              <Monitor className="w-4 h-4 text-accent-cyan" />
              <span className="hidden sm:inline">Windows Setup Guide</span>
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
      <main className="max-w-4xl mx-auto px-4 pt-24 pb-16">
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.5 }}
        >
          {/* Page title */}
          <div className="mb-12 text-center">
            <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full bg-accent-cyan/10 border border-accent-cyan/30 text-accent-cyan text-sm font-medium mb-4">
              <Monitor className="w-4 h-4" />
              Windows
            </div>
            <h1 className="text-3xl sm:text-4xl lg:text-5xl font-extrabold text-white mb-4">
              {t('cokacdir on Windows', 'Windows에서 cokacdir')}
              <br />
              <span className="gradient-text">{t('Setup Guide', '셋업 가이드')}</span>
            </h1>
            <p className="text-lg text-zinc-400 leading-relaxed max-w-2xl mx-auto">
              {t(
                'Install cokacdir on a Windows PC with Claude Code or Codex CLI and use it anywhere via Telegram bot.',
                'Claude Code 또는 Codex CLI가 설치된 Windows PC에 cokacdir을 설치하고, 텔레그램 봇으로 어디서나 사용하는 가이드입니다.'
              )}
            </p>
          </div>

          {/* Prerequisites */}
          <motion.div
            initial={{ opacity: 0, y: 20 }}
            whileInView={{ opacity: 1, y: 0 }}
            viewport={{ once: true }}
            className="mb-12 p-5 rounded-xl border border-accent-purple/30 bg-accent-purple/5"
          >
            <h3 className="text-lg font-semibold text-white mb-3 flex items-center gap-2">
              <Rocket className="w-5 h-5 text-accent-purple" />
              {t('2 Prerequisites', '준비물 2가지')}
            </h3>
            <div className="grid sm:grid-cols-2 gap-3">
              <div className="flex items-center gap-3 p-3 rounded-lg bg-bg-card border border-zinc-800">
                <Monitor className="w-5 h-5 text-accent-cyan flex-shrink-0" />
                <div>
                  <p className="text-white font-medium text-sm">{t('Windows PC (x64 or ARM)', 'Windows PC (x64 또는 ARM)')}</p>
                  <p className="text-zinc-500 text-xs">{t('PowerShell accessible', 'PowerShell 사용 가능')}</p>
                </div>
              </div>
              <div className="flex items-center gap-3 p-3 rounded-lg bg-bg-card border border-zinc-800">
                <Bot className="w-5 h-5 text-accent-cyan flex-shrink-0" />
                <div>
                  <p className="text-white font-medium text-sm">{t('Telegram Bot Token', '텔레그램 봇 토큰')}</p>
                  <p className="text-zinc-500 text-xs">{t('Issued by BotFather', 'BotFather 발급')}</p>
                </div>
              </div>
            </div>
          </motion.div>

          {/* Step 1: Check System Type */}
          <SectionCard icon={Search} title={t('Check System Type', '시스템 종류 확인')} step={1}>
            <p>
              {t(
                'First, check whether your PC is ARM or x64. This determines which cokacdir binary to download.',
                '먼저 PC가 ARM인지 x64인지 확인합니다. 이에 따라 다운로드할 cokacdir 바이너리가 달라집니다.'
              )}
            </p>
            <InlineStep n={1}>
              {t(
                <>Click the <strong className="text-white">search icon</strong> on the taskbar and search for <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">system</code>.</>,
                <>작업표시줄의 <strong className="text-white">검색 아이콘</strong>을 클릭하고 <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">시스템</code>을 검색합니다.</>
              )}
            </InlineStep>
            <InlineStep n={2}>
              {t(
                <>Open <strong className="text-white">System Information</strong> and check the <strong className="text-white">System Type</strong> field.</>,
                <><strong className="text-white">시스템 정보</strong>를 열고 <strong className="text-white">시스템 종류</strong> 항목을 확인합니다.</>
              )}
            </InlineStep>
            <InlineStep n={3}>
              {t(
                <>If it says <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">ARM</code>, it's ARM. If it says <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">x64</code>, it's x64.</>,
                <><code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">ARM</code>이면 ARM, <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">x64</code>이면 x64입니다.</>
              )}
            </InlineStep>
          </SectionCard>

          {/* Step 2: Install Git */}
          <SectionCard icon={Download} title={t('Install Git', 'Git 설치')} step={2}>
            <div className="p-4 rounded-lg border border-zinc-700 bg-zinc-800/30">
              <p className="text-sm text-zinc-400">
                {t(
                  <>If Git is already installed, you can <strong className="text-white">skip this step</strong>.</>,
                  <>Git이 이미 설치되어 있다면 <strong className="text-white">이 단계를 건너뛸 수 있습니다</strong>.</>
                )}
              </p>
            </div>
            <p>
              {t(
                <>Go to <a href="https://git-scm.com/downloads/win" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline font-medium">git-scm.com/downloads/win</a> and download the <strong className="text-white">Standalone Installer</strong> for your system type (64-bit or ARM64).</>,
                <><a href="https://git-scm.com/downloads/win" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline font-medium">git-scm.com/downloads/win</a>에 접속하여 시스템 종류에 맞는 <strong className="text-white">Standalone Installer</strong>를 다운로드합니다 (64-bit 또는 ARM64).</>
              )}
            </p>
            <p>
              {t(
                'Run the installer and follow the default settings to complete the installation.',
                '설치 프로그램을 실행하고 기본 설정 그대로 설치를 완료합니다.'
              )}
            </p>
          </SectionCard>

          {/* Step 3: Install AI Provider */}
          <SectionCard icon={Terminal} title={t('Install AI Provider', 'AI 제공자 설치')} step={3}>
            <div className="p-4 rounded-lg border border-accent-cyan/20 bg-accent-cyan/5">
              <p className="text-sm text-zinc-300">
                {t(
                  <>cokacdir supports <strong className="text-white">Claude Code</strong> and <strong className="text-white">Codex CLI</strong>. Install at least one. You can also install both.</>,
                  <>cokacdir은 <strong className="text-white">Claude Code</strong>와 <strong className="text-white">Codex CLI</strong>를 지원합니다. 최소 하나를 설치하세요. 둘 다 설치해도 됩니다.</>
                )}
              </p>
            </div>

            {/* Claude Code */}
            <div className="mt-6 p-5 rounded-xl border border-zinc-700 bg-zinc-800/20">
              <h4 className="text-base font-semibold text-white mb-3 flex items-center gap-2">
                <span className="px-2 py-0.5 rounded bg-accent-purple/20 text-accent-purple text-xs font-bold">A</span>
                Claude Code
              </h4>
              <p className="text-sm text-zinc-300">
                {t(
                  'Open PowerShell and run the following command.',
                  'PowerShell을 열고 아래 명령어를 실행합니다.'
                )}
              </p>
              <CopyBlock code={`irm https://claude.ai/install.ps1 | iex`} label="PowerShell" />
              <p className="text-sm text-zinc-300">
                {t(
                  'Add Claude Code to your PATH.',
                  'Claude Code를 PATH에 추가합니다.'
                )}
              </p>
              <CopyBlock code={`[Environment]::SetEnvironmentVariable("Path", [Environment]::GetEnvironmentVariable("Path", "User") + ";$env:USERPROFILE\\.local\\bin", "User")`} label="PowerShell" />
            </div>

            {/* Codex CLI */}
            <div className="mt-4 p-5 rounded-xl border border-zinc-700 bg-zinc-800/20">
              <h4 className="text-base font-semibold text-white mb-3 flex items-center gap-2">
                <span className="px-2 py-0.5 rounded bg-accent-green/20 text-accent-green text-xs font-bold">B</span>
                Codex CLI
              </h4>
              <InlineStep n={1}>
                {t(
                  <>Install Node.js from <a href="https://nodejs.org/ko/download" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline font-medium">nodejs.org/download</a>. Download and run the installer for your system.</>,
                  <><a href="https://nodejs.org/ko/download" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline font-medium">nodejs.org/download</a>에서 Node.js를 다운로드하여 설치합니다.</>
                )}
              </InlineStep>
              <InlineStep n={2}>
                {t(
                  'Open PowerShell and set the execution policy.',
                  'PowerShell을 열고 실행 정책을 설정합니다.'
                )}
              </InlineStep>
              <CopyBlock code={`Set-ExecutionPolicy -Scope CurrentUser -ExecutionPolicy RemoteSigned`} label="PowerShell" />
              <InlineStep n={3}>
                {t(
                  'Install Codex CLI globally.',
                  'Codex CLI를 전역으로 설치합니다.'
                )}
              </InlineStep>
              <CopyBlock code={`npm i -g @openai/codex`} label="PowerShell" />
            </div>
          </SectionCard>

          {/* Step 4: Log in to AI Provider */}
          <SectionCard icon={LogIn} title={t('Log in to AI Provider', 'AI 제공자 로그인')} step={4}>
            <div className="p-4 rounded-lg border border-zinc-700 bg-zinc-800/30">
              <p className="text-sm text-zinc-400">
                {t(
                  <>If you are already logged in, you can <strong className="text-white">skip this step</strong>.</>,
                  <>이미 로그인되어 있다면 <strong className="text-white">이 단계를 건너뛸 수 있습니다</strong>.</>
                )}
              </p>
            </div>

            <p>
              {t(
                'Close and reopen PowerShell, then run the login command for the provider you installed.',
                'PowerShell을 닫고 다시 열어서, 설치한 제공자의 로그인 명령어를 실행합니다.'
              )}
            </p>

            {/* Claude Code */}
            <div className="mt-4 p-5 rounded-xl border border-zinc-700 bg-zinc-800/20">
              <h4 className="text-base font-semibold text-white mb-3 flex items-center gap-2">
                <span className="px-2 py-0.5 rounded bg-accent-purple/20 text-accent-purple text-xs font-bold">A</span>
                Claude Code
              </h4>
              <CopyBlock code="claude" label="PowerShell" />
            </div>

            {/* Codex CLI */}
            <div className="mt-4 p-5 rounded-xl border border-zinc-700 bg-zinc-800/20">
              <h4 className="text-base font-semibold text-white mb-3 flex items-center gap-2">
                <span className="px-2 py-0.5 rounded bg-accent-green/20 text-accent-green text-xs font-bold">B</span>
                Codex CLI
              </h4>
              <CopyBlock code="codex" label="PowerShell" />
            </div>

            <p className="text-zinc-500 text-sm">
              {t(
                'Follow the on-screen instructions to complete the login process.',
                '화면에 나오는 안내에 따라 로그인 절차를 완료합니다.'
              )}
            </p>
          </SectionCard>

          {/* Step 5: Download cokacdir */}
          <SectionCard icon={Download} title={t('Download cokacdir', 'cokacdir 다운로드')} step={5}>
            <p>
              {t(
                'Download the cokacdir binary that matches your system type.',
                '시스템 종류에 맞는 cokacdir 바이너리를 다운로드합니다.'
              )}
            </p>

            <div className="my-4 p-4 rounded-lg border border-accent-purple/30 bg-accent-purple/5">
              <p className="text-sm font-semibold text-white mb-2">{t('For x64 systems:', 'x64 시스템인 경우:')}</p>
              <CopyBlock code={`irm -Uri "https://github.com/kstost/cokacdir/raw/refs/heads/main/dist/cokacdir-windows-x86_64.exe" -OutFile "$env:USERPROFILE\\cokacdir.exe"`} label="PowerShell">
{`irm -Uri "https://github.com/kstost/cokacdir/raw/refs/heads/main/dist/cokacdir-windows-x86_64.exe" -OutFile "$env:USERPROFILE\\cokacdir.exe"`}
              </CopyBlock>
            </div>

            <div className="my-4 p-4 rounded-lg border border-accent-purple/30 bg-accent-purple/5">
              <p className="text-sm font-semibold text-white mb-2">{t('For ARM systems:', 'ARM 시스템인 경우:')}</p>
              <CopyBlock code={`irm -Uri "https://github.com/kstost/cokacdir/raw/refs/heads/main/dist/cokacdir-windows-aarch64.exe" -OutFile "$env:USERPROFILE\\cokacdir.exe"`} label="PowerShell">
{`irm -Uri "https://github.com/kstost/cokacdir/raw/refs/heads/main/dist/cokacdir-windows-aarch64.exe" -OutFile "$env:USERPROFILE\\cokacdir.exe"`}
              </CopyBlock>
            </div>

            <p className="text-zinc-500 text-sm">
              {t(
                <>The file will be downloaded to <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">C:\Users\[YourName]\cokacdir.exe</code>.</>,
                <>파일이 <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">C:\Users\[사용자명]\cokacdir.exe</code>에 다운로드됩니다.</>
              )}
            </p>
          </SectionCard>

          {/* Step 6: Telegram Bot Token */}
          <SectionCard icon={Bot} title={t('Create Telegram Bot', '텔레그램 봇 토큰 발급')} step={6}>
            <p>
              {t(
                <>Go to <a href="https://t.me/botfather" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline font-medium">@BotFather</a> and press <strong className="text-white">START BOT</strong> to begin.</>,
                <><a href="https://t.me/botfather" target="_blank" rel="noopener noreferrer" className="text-accent-cyan hover:underline font-medium">@BotFather</a>에서 <strong className="text-white">START BOT</strong>을 눌러 대화를 시작합니다.</>
              )}
            </p>

            <InlineStep n={1}>
              {t(
                <>Type <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">/newbot</code>.</>,
                <><code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">/newbot</code>을 입력합니다.</>
              )}
            </InlineStep>
            <InlineStep n={2}>
              {t(
                <>Set the bot's <strong className="text-white">name</strong> and <strong className="text-white">username</strong>.<br /><span className="text-zinc-500">e.g. name: 'mybot', username: 'my_cokac_bot'</span></>,
                <>Bot의 <strong className="text-white">name</strong>과 <strong className="text-white">username</strong>을 정합니다.<br /><span className="text-zinc-500">예: name은 '코깎봇', username은 'cokac_bot'</span></>
              )}
            </InlineStep>
            <InlineStep n={3}>
              {t('A token will be issued in the following format:', '토큰이 발급됩니다. 아래와 같은 형식입니다:')}
            </InlineStep>

            <div className="my-2 px-4 py-3 bg-bg-card border border-zinc-800 rounded-lg">
              <code className="font-mono text-sm text-yellow-400">123456789:ABCdefGHIjklMNOpqrsTUVwxyz</code>
            </div>
            <p className="text-zinc-500 text-sm">{t('Copy this token.', '이 토큰을 복사해 둡니다.')}</p>
          </SectionCard>

          {/* Step 7: Run cokacdir */}
          <SectionCard icon={Terminal} title={t('Run cokacdir', 'cokacdir 실행')} step={7}>
            <p>
              {t(
                <>Open the folder where you want the AI to work in. <strong className="text-white">Right-click</strong> inside the folder and select <strong className="text-white">"Open in Terminal"</strong>.</>,
                <>AI가 작업할 폴더를 엽니다. 폴더 안에서 <strong className="text-white">우클릭</strong>하고 <strong className="text-white">"터미널에서 열기"</strong>를 선택합니다.</>
              )}
            </p>
            <p>
              {t(
                'Then run the following command with your Telegram bot token.',
                '그리고 아래 명령어에 텔레그램 봇 토큰을 넣어 실행합니다.'
              )}
            </p>

            <CopyBlock code={`& "$env:USERPROFILE\\cokacdir.exe" --ccserver <텔레그램봇토큰>`} label="PowerShell">
{`& "$env:USERPROFILE\\cokacdir.exe" --ccserver `}<Hl>{'<텔레그램봇토큰>'}</Hl>
            </CopyBlock>

            <div className="mt-4 p-4 rounded-lg border border-accent-cyan/20 bg-accent-cyan/5">
              <p className="text-sm text-zinc-300">
                {t(
                  <>Replace <Hl>{'<텔레그램봇토큰>'}</Hl> with the actual token issued by BotFather and run the command.</>,
                  <><Hl>{'<텔레그램봇토큰>'}</Hl> 부분을 BotFather에서 발급받은 실제 토큰으로 바꿔 넣고 실행하세요.</>
                )}
              </p>
            </div>
          </SectionCard>

          {/* Step 8: Use the Bot */}
          <SectionCard icon={MessageCircle} title={t('Use the Bot', '봇 사용하기')} step={8}>
            <InlineStep n={1}>
              {t(
                <>Go to <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">https://t.me/[your bot username]</code> and press <strong className="text-white">START BOT</strong> to begin chatting with the bot.</>,
                <><code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">https://t.me/[앞서 정한 username]</code> 으로 접속해서 <strong className="text-white">START BOT</strong>을 눌러 봇과의 대화를 시작합니다.</>
              )}
            </InlineStep>
            <InlineStep n={2}>
              {t(
                <>Type <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">/start</code> to launch the AI agent.</>,
                <><code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">/start</code> 를 입력하면 AI 에이전트를 실행한 것과 같은 상태가 됩니다.</>
              )}
            </InlineStep>
            <InlineStep n={3}>
              {t(
                <>Say hello! For example, try <strong className="text-white">"Hello!"</strong></>,
                <>인사를 해보세요! 예를 들어 <strong className="text-white">"안녕!"</strong></>
              )}
            </InlineStep>

            <div className="mt-2 flex flex-col sm:flex-row gap-3">
              <button
                onClick={() => {
                  navigate('/telegram-tutorial', { state: { scrollTo: 'telegram-workflow' } })
                }}
                className="inline-flex items-center gap-2 px-4 py-2 rounded-lg border border-accent-cyan/30 bg-accent-cyan/5 text-accent-cyan text-sm font-medium hover:bg-accent-cyan/10 transition-colors cursor-pointer"
              >
                <MessageCircle className="w-4 h-4" />
                {t('Workflow in Practice — Learn More →', '실전 사용 워크플로우 — 자세히 보기 →')}
              </button>
              <button
                onClick={() => {
                  navigate('/telegram-tutorial', { state: { scrollTo: 'telegram-commands' } })
                }}
                className="inline-flex items-center gap-2 px-4 py-2 rounded-lg border border-accent-cyan/30 bg-accent-cyan/5 text-accent-cyan text-sm font-medium hover:bg-accent-cyan/10 transition-colors cursor-pointer"
              >
                <Terminal className="w-4 h-4" />
                {t('Available Commands — Learn More →', '사용 가능한 명령어 — 자세히 보기 →')}
              </button>
            </div>
          </SectionCard>

          {/* Divider */}
          <div className="flex items-center gap-4 my-12">
            <div className="flex-1 h-px bg-zinc-800" />
            <span className="text-zinc-500 text-sm font-medium">{t('All Done', '모든 설정 끝')}</span>
            <div className="flex-1 h-px bg-zinc-800" />
          </div>

          {/* Step 9: Auto-start (Optional) */}
          <SectionCard icon={Clock} title={t('Auto-start on Login (Optional)', '자동 실행 등록 (선택사항)')} step={9}>
            <p>
              {t(
                <>You can register cokacdir to start automatically when you log in to Windows using <strong className="text-white">Task Scheduler</strong>.</>,
                <><strong className="text-white">작업 스케줄러</strong>를 사용하여 Windows 로그인 시 cokacdir이 자동으로 시작되도록 등록할 수 있습니다.</>
              )}
            </p>

            <div className="my-4 p-4 rounded-lg border border-yellow-500/20 bg-yellow-500/5">
              <p className="text-sm text-zinc-300">
                {t(
                  <>The following command requires <strong className="text-white">administrator privileges</strong>. Right-click PowerShell and select <strong className="text-white">"Run as Administrator"</strong>.</>,
                  <>아래 명령어는 <strong className="text-white">관리자 권한</strong>이 필요합니다. PowerShell을 <strong className="text-white">우클릭</strong>하고 <strong className="text-white">"관리자 권한으로 실행"</strong>을 선택하세요.</>
                )}
              </p>
            </div>

            <CopyBlock code={`$action = New-ScheduledTaskAction -Execute "$env:USERPROFILE\\cokacdir.exe" -Argument "--ccserver <텔레그램봇토큰>" -WorkingDirectory "$env:USERPROFILE"
$trigger = New-ScheduledTaskTrigger -AtLogon
Register-ScheduledTask -TaskName "cokacdir" -Action $action -Trigger $trigger -RunLevel Highest -Force`} label="PowerShell (Administrator)">
{`$action = New-ScheduledTaskAction -Execute "$env:USERPROFILE\\cokacdir.exe" -Argument "--ccserver `}<Hl>{'<텔레그램봇토큰>'}</Hl>{`" -WorkingDirectory "$env:USERPROFILE"
$trigger = New-ScheduledTaskTrigger -AtLogon
Register-ScheduledTask -TaskName "cokacdir" -Action $action -Trigger $trigger -RunLevel Highest -Force`}
            </CopyBlock>

            <div className="mt-4 p-4 rounded-lg border border-accent-cyan/20 bg-accent-cyan/5">
              <p className="text-sm text-zinc-300">
                {t(
                  <>Replace <Hl>{'<텔레그램봇토큰>'}</Hl> with the actual token issued by BotFather.</>,
                  <><Hl>{'<텔레그램봇토큰>'}</Hl> 부분을 BotFather에서 발급받은 실제 토큰으로 바꿔 넣으세요.</>
                )}
              </p>
            </div>

            <div className="mt-4 p-4 rounded-lg border border-zinc-700 bg-zinc-800/30">
              <p className="text-sm text-zinc-400">
                {t(
                  <>To remove the auto-start, run: <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">Unregister-ScheduledTask -TaskName "cokacdir" -Confirm:$false</code></>,
                  <>자동 실행을 해제하려면: <code className="px-1.5 py-0.5 bg-bg-elevated rounded text-accent-cyan text-sm">Unregister-ScheduledTask -TaskName "cokacdir" -Confirm:$false</code></>
                )}
              </p>
            </div>
          </SectionCard>

        </motion.div>

        {/* Bottom navigation */}
        <div className="mt-16 pt-8 border-t border-zinc-800 flex flex-col sm:flex-row items-center justify-between gap-4">
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
            Beginner Tutorial →
          </Link>
        </div>
      </main>
    </div>
  )
}

export default function WindowsPage() {
  return (
    <LanguageProvider>
      <WindowsPageInner />
    </LanguageProvider>
  )
}
