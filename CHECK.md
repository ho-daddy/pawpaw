# 변경사항 검증 리포트 (2026-03-15)

비교 대상: `cokacdir` (현재) vs `cokacdir_old` (이전)

---

## 1. 코드 변경사항 검증

### 변경 파일

| 파일 | 변경 내용 |
|------|-----------|
| `Cargo.toml` | 버전 `0.4.48` → `0.4.49` |
| `src/services/claude.rs` | `--append-system-prompt` → `--append-system-prompt-file` 전환 (E2BIG 에러 방지) |

### 1.1 시스템 프롬프트 파일 기반 전달 방식 전환

**변경 목적**: OS의 "Argument list too long" (E2BIG) 에러를 방지하기 위해, 시스템 프롬프트를 CLI 인자로 직접 전달하는 대신 임시 파일에 기록 후 `--append-system-prompt-file` 플래그로 파일 경로를 전달하도록 변경.

**검증 항목**:

- [x] 시스템 프롬프트 텍스트 내용이 이전 버전과 글자 단위로 동일한지 diff 확인 → **동일**
- [x] `--append-system-prompt` 사용처가 소스 전체에서 완전히 제거되었는지 grep 확인 → **0건 (완전 제거)**
- [x] `--append-system-prompt-file` 플래그가 Claude CLI (v2.1.76)에서 실제 동작하는지 실증 테스트 → **동작 확인** (`--help`에는 표시되지 않으나, 파일 내용이 시스템 프롬프트로 정상 적용됨을 테스트로 검증)
- [x] 비스트리밍 경로 (`call_claude`): 파일 작성 → args 추가 → SpFileGuard 생성 → 프로세스 실행 → `wait_with_output()` → guard drop → 파일 삭제. **레이스 컨디션 없음**
- [x] 스트리밍 경로 (`execute_command_streaming`): 조건부 파일 작성 → args 추가 → SpFileGuard 설정 → 프로세스 실행 → stdout 루프 → `child.wait()` → guard drop → 파일 삭제. **레이스 컨디션 없음**
- [x] `system_prompt` 매개변수의 조건 분기 로직 (`None → default`, `Some("") → none`, `Some(p) → p`) 이전 버전과 동일하게 유지 → **동일**
- [x] 에러 처리 패턴이 각 함수의 기존 패턴과 일치하는지 확인 → **일치** (비스트리밍: `ClaudeResponse` 반환, 스트리밍: `Result` + `?` 연산자)
- [x] 수정 불필요 함수 (`extract_context_summary`, `extract_result_summary`)가 변경되지 않았는지 확인 → **미변경** (이 함수들은 `--append-system-prompt`를 사용하지 않음)

### 1.2 `simple_uuid()` 함수

- [x] 유일성: 나노초 타임스탬프 + PID 조합. 각 함수에서 1회만 호출되므로 동일 나노초 충돌은 사실상 불가능
- [x] `unwrap_or_default()`: UNIX_EPOCH 이전 시간(극히 예외적)에서도 PID와 결합되어 충돌 없음

### 1.3 Windows 호환성

- [x] `dirs::home_dir()`, `std::env::temp_dir()`, `PathBuf::join()`, `std::fs::write/remove_file` 등 모두 크로스 플랫폼 API 사용
- [x] Windows 파일 잠금 문제: 비스트리밍은 `wait_with_output()`, 스트리밍은 `child.wait()` 이후 guard drop → 프로세스 종료 후 파일 삭제이므로 **잠금 충돌 없음**
- [x] 취소 경로: `kill_child_tree` + `child.wait()` 후 return → guard drop → **안전**

### 1.4 경미한 부수 사항 (버그 아님)

- 프로세스 비정상 종료(SIGKILL) 시 `~/.cokacdir/system_prompt_*` 임시 파일 잔류 가능. 크기 ~1KB, 빈도 극히 낮음. 정리 메커니즘은 없으나 기능적 문제 아님
- `SpFileGuard` struct가 두 함수 내부에 중복 정의됨. 유효한 Rust 코드이며 기능적 문제 없음

---

## 2. Telegram 봇 Owner 인증 시스템 보안 검증

### 2.1 인증 게이트 구조

Owner 체크는 `handle_message()` 최상단 (line 1732-1779)에 위치하며, 모든 메시지 처리 로직(`/`, `!`, `;`, 일반 텍스트, 파일 업로드, 위치 공유 등)보다 **앞에서** 실행됨.

```
handle_message 진입
  └─ owner_user_id 확인 (뮤텍스 내부)
       ├─ None → 첫 유저를 owner로 등록 (imprinting)
       └─ Some(owner_id)
            ├─ uid == owner_id → 통과
            └─ uid != owner_id
                 ├─ /public on 된 그룹 → 통과 (is_owner=false)
                 └─ 그 외 → return Ok(()) ← 즉시 종료, 이후 코드 도달 불가
```

### 2.2 검증 시나리오

#### 시나리오 A: Owner 등록 상태 + 서버 다운 + 비owner가 메시지 전송 + 서버 재시작

- Owner 정보는 `save_bot_settings()`로 파일에 영속화됨
- 서버 재시작 시 `load_bot_settings()`로 owner_user_id 복원
- 비owner의 펜딩 메시지는 `teloxide::repl` 폴링 시 수신되나, owner 체크에서 **모두 거부**
- **결과: 비owner 메시지 실행 불가**

#### 시나리오 B: 비공개 채팅 (DM/일반 그룹)에서 비owner의 모든 종류 메시지

- `/stop`, `/start`, `/help`, `/model`, `/debug`, `/allowed` 등 모든 슬래시 커맨드 → **차단**
- `!쉘명령` → **차단**
- `;텍스트`, 일반 텍스트 → **차단**
- 파일 업로드, 위치 공유 → **차단**
- auth 게이트가 이 모든 처리 로직보다 앞에 위치하여 `return Ok(())`로 즉시 반환
- **결과: 메시지 종류에 관계없이 실행 불가**

#### 시나리오 C: 그룹채팅 + `/public on` 미설정 상태

- `as_public_for_group_chat.get(&chat_key).unwrap_or(false)` → `false`
- `is_public = is_group_chat && false` → `false`
- 비공개 채팅과 동일하게 **거부**
- **결과: 실행 불가**

#### 시나리오 D: Owner 미등록 + 서버 시작 전 복수 사용자 메시지 대기

예시: 54335680, 54335681, 54335682, 54335683 네 사용자가 서버 시작 전 메시지 전송

- `teloxide::repl`은 Telegram `update_id` 순서대로 **순차 처리**
- 첫 번째 메시지의 발신자가 owner로 등록됨 (imprinting)
- 이후 메시지들은 owner 체크에서 모두 **거부**
- 설령 동시 처리 환경이라 해도 `state.lock().await` 뮤텍스가 owner 설정/확인의 원자성을 보장
- **결과: owner로 등록된 사용자의 메시지만 실행, 나머지는 모두 거부**

### 2.3 Owner 전용 커맨드

공개 그룹(`/public on`)에서 비owner가 접근하더라도 owner 전용으로 보호되는 커맨드:

| 커맨드 | 보호 위치 |
|--------|-----------|
| `/public` | `handle_public_command` line 4487: `if !is_owner` 체크 |
| `/direct` | `handle_direct_command` line 4261: `if !is_owner` 체크 |

---

## 3. 종합 결론

### 코드 변경
변경사항은 완전하고 올바르게 구현됨. 누락, 사이드이펙트, 크로스 플랫폼 문제 없음.

### 보안
Owner 인증 시스템은 모든 검증 시나리오에서 비owner 메시지를 완전히 차단함. 뮤텍스 보호, 순차 처리, 영속화된 owner 정보의 조합으로 우회 경로 없음.
