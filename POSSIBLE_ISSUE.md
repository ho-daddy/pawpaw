# Windows 지원 구현 — 잠재적 이슈 목록

이 문서는 Windows 지원을 위한 `#[cfg(unix)]`/`#[cfg(windows)]` 조건부 컴파일 적용 후 발생할 수 있는 문제를 정리한 것입니다.

---

## Critical

### ~~1. `src/ui/app.rs` — `/tmp` 하드코딩 폴백~~ (해결됨)

- **상태**: 해결 완료
- **수정 내용**: `/tmp` 및 `std::env::temp_dir()` 폴백을 모두 제거. `home_dir()` 실패 시 해당 기능을 중단하도록 변경. 추가로 `main()` 시작 시 `home_dir()` 존재를 확인하여 앱 실행 자체를 차단.
- **수정 범위**: `app.rs` 3곳, `remote_transfer.rs` 2곳, `telegram.rs` 2곳 — 총 7곳

### 2. `src/services/remote_transfer.rs` — SSH_ASKPASS 환경변수 플랫폼 분기 없음

- **위치**: askpass 스크립트를 설정하는 `env("SSH_ASKPASS", ...)`, `env("DISPLAY", ...)` 호출부
- **내용**: `SSH_ASKPASS`, `SSH_ASKPASS_REQUIRE`, `DISPLAY` 환경변수를 무조건 설정
- **문제**: Windows의 OpenSSH는 `SSH_ASKPASS` 메커니즘이 다르게 동작할 수 있음. `DISPLAY` 환경변수는 Windows에서 의미 없음
- **완화**: Windows에서 rsync/SSH를 사용하는 경우 자체가 드물고, OpenSSH for Windows가 해당 변수를 무시할 수 있음

### 3. `src/services/file_ops.rs` — 보호 경로 목록이 Unix 전용

- **위치**: `SENSITIVE_PATHS` 상수 (line ~939)
- **내용**: `/etc`, `/sys`, `/proc`, `/boot` 등 Unix 전용 경로만 나열
- **문제**: Windows에서 `C:\Windows`, `C:\Windows\System32` 등 민감한 경로가 보호되지 않음
- **완화**: symlink을 통한 tar 아카이브 보안 체크이며, Windows에서 symlink 생성 자체가 권한 필요

---

## Medium

### 4. `src/ui/system_info.rs` — wmic 명령 폐기 예정

- **위치**: Windows `#[cfg(windows)]` 블록 전체 (uptime, memory, CPU, disk)
- **내용**: `wmic` 명령을 사용하여 시스템 정보 조회
- **문제**: Microsoft가 Windows 11 21H1부터 `wmic`를 폐기(deprecated) 처리. 향후 버전에서 제거될 수 있음
- **대안**: PowerShell의 `Get-CimInstance` 사용 (`powershell -Command "Get-CimInstance Win32_OperatingSystem | ..."`)
- **완화**: 현재 Windows 10/11에서는 아직 사용 가능. 실패 시 빈 값으로 graceful 처리됨

### 5. `src/enc/mod.rs` — `FileTimes` API 최소 Rust 버전 요구

- **위치**: `#[cfg(not(unix))]` mtime 복원 블록
- **내용**: `std::fs::FileTimes::new()` 및 `File::set_times()` 사용
- **문제**: Rust 1.75.0에서 안정화된 API. 프로젝트의 MSRV(Minimum Supported Rust Version)가 이보다 낮으면 컴파일 실패
- **완화**: Cargo.toml에 `rust-version`이 명시되지 않았고, 다른 의존성들도 최신 버전 사용 중이므로 실질적 문제 가능성 낮음

### 6. `src/services/process.rs` — tasklist CSV 파싱의 로케일 의존성

- **위치**: `parse_tasklist_csv_line()` 함수
- **내용**: `tasklist /FO CSV /V` 출력을 파싱하여 필드를 고정 위치로 접근
- **문제**: 비영어 Windows에서 필드 순서나 헤더 이름이 다를 수 있음. 메모리 값의 "K" 접미사도 로케일에 따라 다를 수 있음 (예: 독일어에서 쉼표 대신 마침표)
- **완화**: CSV 형식(`/FO CSV`)은 로케일과 무관하게 고정 구조이므로 실제 문제 발생 가능성은 낮음. 다만 메모리 값의 천단위 구분자는 로케일 의존적

### 7. `src/services/process.rs` — Windows taskkill 권한 문제

- **위치**: `kill_process_with_verification()`, `force_kill_process_with_verification()` Windows 버전
- **내용**: `taskkill /PID <pid>` 사용
- **문제**: 관리자 권한 없이 다른 사용자의 프로세스를 종료할 수 없음. Unix의 `EPERM`처럼 구체적 에러 코드 분류가 없고 stderr 메시지만 반환
- **완화**: stderr 메시지를 그대로 사용자에게 표시하므로 기능적으로는 문제 없음

### 8. `src/services/remote_transfer.rs` — Windows askpass `.bat` 파일의 패스워드 특수문자

- **위치**: `create_askpass_script()` Windows 블록
- **내용**: `@echo off\necho {password}\n` 형태로 .bat 파일 생성
- **문제**: 패스워드에 `&`, `|`, `>`, `<`, `^`, `%` 등 CMD 특수문자가 있으면 echo가 제대로 동작하지 않음
- **수정 방안**: `echo` 대신 `echo.` 또는 특수문자 이스케이핑(`^&`, `^|` 등) 적용 필요

### 9. `src/services/telegram.rs` — Windows에서 cmd /c 명령의 stdin 처리

- **위치**: shell command execution `#[cfg(windows)]` 블록
- **내용**: `cmd /c` 로 명령 실행 시 `stdin(Stdio::null())` 사용
- **문제**: `cmd /c`는 일부 대화형 명령에서 stdin이 null이면 즉시 종료될 수 있음
- **완화**: Telegram 봇의 명령 실행은 비대화형이므로 실질적 문제 가능성 낮음

---

## Low

### 10. `src/ui/app.rs` — Windows xcopy의 디렉토리 복사 차이

- **위치**: git diff용 `cp -a` → `xcopy` 교체
- **내용**: `xcopy /e /h /k /q /y /i` 사용
- **문제**: `xcopy`는 심볼릭 링크를 따르지 않고, 보안 ACL을 완전히 복사하지 못함. `cp -a`와 완전한 동등성은 없음
- **완화**: git diff 용도의 임시 복사이므로 심볼릭 링크나 ACL은 중요하지 않음
- **대안**: `robocopy /MIR /COPY:DAT` 가 더 정확하지만 종료 코드 처리가 복잡

### 11. `src/ui/app.rs` — `explorer` 명령 경로

- **위치**: `open_in_explorer()` 함수
- **내용**: `Command::new("explorer")` 사용
- **문제**: 극히 드물지만 `explorer.exe`가 PATH에 없는 환경이 있을 수 있음
- **완화**: `explorer.exe`는 `C:\Windows`에 있고 이 경로는 항상 PATH에 포함됨

### 12. `src/config.rs` — Windows 핸들러의 `set /p`와 `pause` 명령

- **위치**: Windows 기본 확장자 핸들러
- **내용**: `set /p a=...` (확인 프롬프트), `pause >nul` (키 입력 대기)
- **문제**: PowerShell에서 이 핸들러를 실행하면 `set /p`가 동작하지 않음
- **완화**: 핸들러는 `execute_terminal_command()`를 통해 `cmd /c`로 실행되므로 CMD 환경이 보장됨

### 13. `src/ui/system_info.rs` — wmic 출력의 빈 줄 처리

- **위치**: Windows `load_disk_info()`, `SystemData::load()`
- **내용**: wmic CSV 출력에서 `skip(1)`로 첫 줄만 건너뜀
- **문제**: wmic 출력은 빈 줄로 시작하고, 줄 끝에 `\r`이 포함될 수 있음
- **완화**: `parts.len() >= 5` 체크와 `total_size == 0` 체크로 빈 줄/헤더줄이 걸러지고, `trim()`으로 `\r` 처리됨

### 14. `src/services/process.rs` — tasklist 메모리 파싱

- **위치**: `parse_tasklist_csv_line()` 내 `mem_str` 처리
- **내용**: `fields[4].replace(" K", "").replace(",", "")` 로 메모리 값 파싱
- **문제**: 일부 Windows 로케일에서 "K" 대신 다른 단위 접미사 사용 가능. 천단위 구분자가 "."일 수 있음
- **완화**: `unwrap_or(0)` 으로 파싱 실패 시 0으로 처리되므로 크래시는 없음

### 15. `src/services/claude.rs` — npm 글로벌 경로 탐색

- **위치**: Windows `resolve_claude_path()` 내 npm fallback
- **내용**: `npm root -g`로 글로벌 모듈 경로를 찾고 `cli.js` 존재 확인
- **문제**: `cli.js` 경로를 직접 반환하면 Node.js 런타임 없이는 실행 불가. 또한 npm이 설치되어 있지 않으면 `cmd /c npm root -g` 자체가 실패
- **완화**: 첫 번째 `where claude`에서 대부분 해결되고, fallback 실패 시 `None` 반환으로 graceful 처리

---

## 이 구현에서 변경하지 않은 영역 (향후 고려사항)

| 영역 | 내용 |
|------|------|
| ~~`app.rs` `/tmp` 폴백 6곳~~ | 해결됨 — `/tmp` 및 `temp_dir()` 폴백 제거, `main()`에서 `home_dir()` 사전 체크 추가 |
| `file_ops.rs` symlink 처리 | 이미 `#[cfg(unix)]`/`#[cfg(not(unix))]` 분기 존재 |
| `file_ops.rs` 특수 파일 체크 | block device, socket 등은 이미 `#[cfg(unix)]` 가드 있음 |
| PowerShell 지원 | 현재 Windows 쉘은 CMD(`cmd /c`) 기준. PowerShell 직접 지원은 미포함 |
| Windows Terminal 색상 | crossterm이 Windows Terminal/conhost 모두 지원하므로 별도 처리 불필요 |
