# Bot-to-Bot Messaging 설계서

## 1. 개요

같은 Telegram 그룹 채팅 안에 있는 복수의 cokacdir 봇 인스턴스 간 양방향 메시지 통신을 가능하게 하는 기능.

봇의 AI가 다른 봇에게 작업을 위임하거나 정보를 요청하고, 응답을 받아 이어서 작업할 수 있는 핑퐁 방식의 대화를 지원한다.

---

## 2. 전제 조건

### 2.1 동일 환경
- 통신하는 봇들은 **같은 서버, 같은 파일시스템** 위에서 동작
- 통신하는 봇들은 **같은 Telegram 그룹 채팅** 안에 있음
- 파일 기반 통신이므로 네트워크 통신 불필요

### 2.2 봇 자기 인식 (구현 완료)
- 각 봇은 시작 시 `bot.get_me().await`로 자기 Telegram username을 확보
- `bot_username` 변수가 `run_bot` → `handle_message`로 전달됨

### 2.3 bot_settings.json에 username 저장
- 봇 시작 시 `bot_settings.json`의 해당 봇 항목에 `username` 필드를 저장
- CLI가 `--key`로부터 발신자 username을 조회하는 데 사용
- CLI가 `--to`로 지정된 수신자의 존재 여부를 검증하는 데 사용

```json
{
    "<token_hash>": {
        "token": "...",
        "username": "mybot_a",
        "last_sessions": { "-1001234567": "/home/kst/project" },
        ...
    }
}
```

---

## 3. System Prompt 변경

### 3.1 봇 정보 상시 포함

`build_system_prompt`에서 생성하는 system prompt 상단에 항상 포함:

```
Bot username: @mybot_a
Chat ID: -1001234567
Current working directory: /home/kst/project
```

- `Bot username`: `bot.get_me()`로 확보한 값
- `Chat ID`: 현재 대화 중인 chat_id (기존에 `--sendfile`, `--cron` 템플릿에서 사용 중)
- AI는 이 값들을 `--message` 명령어에 그대로 사용

### 3.2 명령어 레퍼런스 추가

기존 `--sendfile`, `--cron` 레퍼런스와 동일한 패턴으로 추가:

```
── BOT MESSAGING ──
Send a message to another bot in this chat:
"cokacdir" --message <CONTENT> --to <BOT_USERNAME> --chat {chat_id} --key {bot_key}
• The --from field is automatically determined from --key
• The target bot must be in the same chat and have an active session
• Use /instruction to configure which bots to collaborate with
• Output: {"status":"ok","id":"msg_..."}
```

---

## 4. CLI 명령어

### 4.1 명령어 형식

```
cokacdir --message <CONTENT> --to <BOT_USERNAME> --chat <CHAT_ID> --key <KEY>
```

| 파라미터 | 필수 | 설명 |
|---------|------|------|
| `--message` | O | 전달할 메시지 내용 |
| `--to` | O | 수신 봇의 Telegram username (@ 제외) |
| `--chat` | O | 그룹 채팅 chat_id |
| `--key` | O | 발신 봇의 token_hash (인증 + 발신자 식별) |

- `--from`은 파라미터로 받지 않음 — CLI가 `--key`로 `bot_settings.json`을 조회하여 자동 결정
- AI가 잘못된 `--from`을 제공할 가능성을 원천 차단

### 4.2 CLI 처리 흐름

```
1. --key로 bot_settings.json 조회 → 발신자 username 결정
   - 실패 시: {"status":"error","message":"Invalid key"}
2. --to로 bot_settings.json 조회 → 수신자 존재 확인
   - 실패 시: {"status":"error","message":"Bot 'xxx' not found"}
3. ~/.cokacdir/messages/ 디렉토리 확인/생성
4. 메시지 파일 생성: msg_<timestamp>_<random>.json
5. 응답 반환: {"status":"ok","id":"msg_..."}
```

### 4.3 성공 응답

```json
{"status":"ok","id":"msg_20260311_120000_a1b2"}
```

### 4.4 에러 응답

```json
{"status":"error","message":"Bot 'bot_xyz' not found"}
{"status":"error","message":"Invalid key"}
```

---

## 5. 메시지 파일

### 5.1 저장 위치

```
~/.cokacdir/messages/
```

### 5.2 파일명 규칙

```
msg_<YYYYMMDD_HHMMSS>_<random_hex>.json
```

예: `msg_20260311_120000_a1b2c3d4.json`

### 5.3 파일 구조

모든 메시지 파일은 동일한 구조. 발신/수신 역할 구분 없이 동등한 형태.

```json
{
    "id": "msg_20260311_120000_a1b2c3d4",
    "from": "mybot_a",
    "to": "bot_b",
    "chat_id": "-1001234567",
    "content": "프론트엔드에 검색 기능 구현해줘. API: GET /users/search?q=query",
    "created_at": "2026-03-11 12:00:00"
}
```

| 필드 | 타입 | 설명 |
|------|------|------|
| `id` | String | 파일명과 동일한 고유 식별자 |
| `from` | String | 발신 봇 username (CLI가 --key에서 자동 결정) |
| `to` | String | 수신 봇 username |
| `chat_id` | String | 그룹 채팅 chat_id |
| `content` | String | 메시지 내용 (길이 제한 없음) |
| `created_at` | String | 생성 시각 |

### 5.4 파일 생명주기

```
파일 생성 (pending 상태)
    ↓
수신 봇 폴링이 감지
    ↓
파일 즉시 삭제
    ↓
처리 후 응답 메시지 파일 새로 생성
```

- 파일이 존재하면 = 미처리
- 처리 시작 시 = 즉시 삭제
- `status` 필드 불필요
- 별도 상태 관리 없음

---

## 6. 폴링 메커니즘

### 6.1 통합 위치

기존 스케줄러 루프에 통합하거나 별도 루프 추가.
각 봇 인스턴스가 주기적으로 `~/.cokacdir/messages/` 디렉토리를 스캔.

### 6.2 스캔 조건

```
~/.cokacdir/messages/*.json 파일 중
→ to == 내 bot_username 인 파일
→ created_at 순으로 처리 (FIFO)
```

### 6.3 폴링 주기

스케줄러 루프와 동일한 간격 (예: 5초) 또는 별도 설정 가능.

### 6.4 busy 체크

해당 `chat_id`에 `cancel_token`이 존재하면 (AI 작업 진행 중):
- 해당 메시지는 건너뛰고 다음 폴링 사이클에서 재시도
- 파일은 삭제하지 않음 (아직 미처리)

---

## 7. 메시지 처리 흐름

### 7.1 수신 봇의 처리 (Bot B가 Bot A의 메시지를 받은 경우)

```
1. 폴링 루프가 msg_xxx.json 감지 (to == "bot_b")
2. chat_id에 cancel_token 존재 여부 확인
   - 존재 시 (busy): 건너뛰고 다음 사이클에서 재시도
3. 메시지 파일 읽기 → 내용 파싱
4. 원본 파일 즉시 삭제
5. 텔레그램 채팅에 수신 알림 표시:
   "📨 @mybot_a: 프론트엔드에 검색 기능 구현해줘..."
6. handle_text_message(bot, chat_id, content, state) 호출
   - placeholder "..." 표시
   - AI 처리 (기존 로직 그대로)
   - 텔레그램에 응답 표시 (기존과 동일)
   - 세션 히스토리에 기록
7. AI 전체 응답 원문 캡처
   - 세션 히스토리에서 마지막 assistant 응답 추출
   - 텔레그램 길이 제한/silent 모드와 무관하게 전체 내용
8. 새 메시지 파일 생성:
   {
       from: "bot_b",
       to: "mybot_a",
       chat_id: "-1001234567",
       content: <AI 전체 응답 원문>
   }
```

### 7.2 응답 수신 봇의 처리 (Bot A가 Bot B의 응답을 받은 경우)

Bot B의 응답도 일반 메시지 파일과 동일한 구조이므로, 처리 흐름은 7.1과 완전히 동일:

```
1. 폴링이 감지 (to == "mybot_a")
2. busy 체크
3. 파일 읽기 → 삭제
4. 텔레그램에 표시: "📨 @bot_b: SearchBar.tsx 생성 완료..."
5. handle_text_message 호출 → AI가 이어서 작업
6. AI 응답 캡처
7. AI가 추가 --message를 실행했으면 → 새 메시지 파일 존재 → 핑퐁 계속
   AI가 --message를 안 보냈으면 → 체인 종료
```

### 7.3 기존 기능과의 일관성

메시지 수신 처리는 `handle_text_message`를 그대로 사용하므로:
- placeholder "..." 표시 ✓
- AI 작업 과정 실시간 표시 ✓
- `/stop`으로 중단 가능 ✓
- silent mode 적용 ✓
- `/instruction`에 설정된 system instruction 적용 ✓
- 세션 히스토리에 기록 ✓
- 파일 업로드/다운로드 등 모든 도구 사용 가능 ✓

---

## 8. 핑퐁 흐름

### 8.1 전체 시퀀스

```
사용자: "API 변경하고 프론트도 업데이트해줘"
        ↓
Bot A AI 처리:
  - 백엔드 코드 수정
  - cokacdir --message "프론트엔드 검색 기능 구현해줘..." --to bot_b --chat {id} --key {key}
  - 텔레그램 응답: "백엔드 완료. @bot_b에 요청 보냈습니다."
        ↓
msg_001.json 생성 (from: mybot_a, to: bot_b)
Bot A 턴 종료
        ↓
Bot B 폴링이 msg_001.json 감지
msg_001.json 즉시 삭제
        ↓
Bot B 텔레그램: "📨 @mybot_a: 프론트엔드 검색 기능 구현해줘..."
Bot B 텔레그램: "..."
Bot B AI 처리: SearchBar.tsx 생성, API 연동
Bot B 텔레그램: "검색 컴포넌트 구현 완료."
        ↓
msg_002.json 생성 (from: bot_b, to: mybot_a, content: AI 전체 응답)
Bot B 턴 종료
        ↓
Bot A 폴링이 msg_002.json 감지
msg_002.json 즉시 삭제
        ↓
Bot A 텔레그램: "📨 @bot_b: 검색 컴포넌트 구현 완료..."
Bot A 텔레그램: "..."
Bot A AI 처리: 통합 확인
Bot A 텔레그램: "모든 작업 완료되었습니다."
        ↓
Bot A AI가 --message를 안 보냄 → 체인 종료
```

### 8.2 텔레그램 채팅 표시 (사용자 시점)

같은 그룹 채팅에서 모든 과정이 시간순으로 표시:

```
[사용자]   API 변경하고 프론트도 업데이트해줘

[Bot A]    백엔드 엔드포인트 추가 완료.
           @bot_b에 프론트엔드 작업을 요청합니다.

[Bot B]    📨 @mybot_a: "프론트엔드에 검색 기능 구현해줘.
           API: GET /users/search?q=query"
[Bot B]    ...
[Bot B]    SearchBar.tsx 생성 완료. API 연동 완료.

[Bot A]    📨 @bot_b: "SearchBar.tsx 생성 완료. API 연동 완료."
[Bot A]    ...
[Bot A]    프론트엔드 작업도 완료됐습니다. 통합 테스트를 진행하겠습니다.
           [테스트 결과] 모든 테스트 통과. 작업 완료.
```

---

## 9. 종료 조건

### 9.1 자연 종료
- AI가 `--message` 명령을 더 이상 실행하지 않으면 체인이 자연히 끝남
- 별도 라운드 제한 없음

### 9.2 수동 중단
- 사용자가 현재 처리 중인 봇의 채팅에서 `/stop` 입력
- 해당 봇의 AI 처리 중단 (기존 cancel_token 메커니즘)
- 응답 메시지 파일이 생성되지 않음
- 상대 봇은 응답을 받지 못함
- 체인 끊김

핑퐁은 항상 순차적 (한쪽 봇만 처리 중)이므로 `/stop`으로 언제든 중단 가능.

---

## 10. 에러 및 엣지 케이스

### 10.1 수신 봇의 세션 없음
- `handle_text_message`가 "No active session. Use /start <path> first." 응답
- 이 내용이 응답 메시지 파일의 `content`로 전달됨
- 발신 봇의 AI가 상황을 인지하고 사용자에게 알림

### 10.2 수신 봇이 busy (다른 작업 진행 중)
- 해당 chat_id에 `cancel_token` 존재
- 폴링 루프에서 건너뛰고 다음 사이클에서 재시도
- 메시지 파일은 삭제하지 않음 (미처리 상태 유지)

### 10.3 수신 봇 오프라인
- 메시지 파일이 `~/.cokacdir/messages/`에 남아있음
- 발신 봇의 폴링 루프가 `created_at` 기준으로 타임아웃 체크
- 타임아웃 (예: 30분) 초과 시:
  - 해당 메시지 파일 삭제
  - 발신 봇 채팅에 타임아웃 알림: "⏰ Message to @bot_b timed out."

### 10.4 수신자 username 오타
- CLI가 `bot_settings.json`에서 `--to` 검증
- 즉시 에러 반환: `{"status":"error","message":"Bot 'bot_xyz' not found"}`
- AI가 에러를 인지하고 사용자에게 알림

### 10.5 잘못된 --key
- CLI가 `bot_settings.json`에서 `--key` 검증 실패
- 즉시 에러 반환: `{"status":"error","message":"Invalid key"}`

### 10.6 동시 다중 메시지
- Bot A가 한 턴에서 Bot B와 Bot C에게 각각 `--message` 실행
- 별도의 메시지 파일 2개 생성
- Bot B와 Bot C가 각각 독립적으로 처리
- 응답이 각각 Bot A에게 도착, 순서대로 처리 (busy 체크로 순차 실행)

### 10.7 파일 충돌 방지
- 파일명에 timestamp + random hex 사용으로 충돌 방지
- 폴링 루프에서 파일 읽기 → 즉시 삭제 (원자적 처리)
- 여러 봇이 동시에 같은 파일을 처리하지 않음 (to 필드로 필터링)

---

## 11. 응답 캡처

### 11.1 캡처 대상
- AI의 전체 응답 원문
- 텔레그램 메시지 길이 제한 (4096자) 무관
- silent 모드에서 생략된 도구 호출 정보 무관
- 분할 전송된 경우에도 전체 내용

### 11.2 캡처 방법
- `handle_text_message` 완료 후
- 세션 히스토리에서 마지막 assistant 응답 항목 추출
- 해당 내용을 새 메시지 파일의 `content`로 기록

### 11.3 캡처 시점 판별
- 메시지 처리 시 "이 실행이 봇 간 메시지에 의해 트리거되었는지" 플래그 필요
- 플래그가 있을 때만 처리 완료 후 응답 메시지 파일 생성
- 일반 사용자 메시지 처리에는 영향 없음

---

## 12. 구현 항목

### 12.1 bot_settings.json 변경

| 항목 | 설명 |
|------|------|
| `username` 필드 추가 | 봇 시작 시 `get_me()`로 확보한 username 저장 |
| `save_bot_settings` 수정 | username 포함하여 저장 |

### 12.2 System Prompt 변경

| 항목 | 설명 |
|------|------|
| `build_system_prompt` 수정 | bot username, chat_id 명시 추가 |
| `--message` 레퍼런스 추가 | 명령어 형식, 파라미터, 출력 형식 문서화 |
| `bot_username` 전달 | `handle_text_message` → `build_system_prompt` 경로에 username 전달 |

### 12.3 CLI 변경

| 항목 | 설명 |
|------|------|
| `--message` 인자 파싱 | clap 또는 수동 파싱 |
| `--to` 인자 파싱 | 수신 봇 username |
| `bot_settings.json` 조회 | `--key` → from username 결정, `--to` → 존재 확인 |
| 메시지 파일 생성 | `~/.cokacdir/messages/` 디렉토리에 JSON 파일 |
| JSON 응답 출력 | `{"status":"ok","id":"msg_..."}` 또는 에러 |

### 12.4 메시지 폴링 루프

| 항목 | 설명 |
|------|------|
| 폴링 루프 추가/통합 | 스케줄러 루프에 통합 또는 별도 루프 |
| `bot_username` 접근 | 폴링 루프에서 자기 username으로 필터링 |
| 디렉토리 스캔 | `~/.cokacdir/messages/*.json` |
| 메시지 필터링 | `to == my_username` |
| busy 체크 | `cancel_token` 존재 시 건너뛰기 |
| 타임아웃 체크 | 자신이 보낸 메시지 중 오래된 pending 파일 감지 |

### 12.5 메시지 수신 처리

| 항목 | 설명 |
|------|------|
| 파일 읽기 + 즉시 삭제 | 원자적 처리 |
| 수신 알림 표시 | "📨 @from: content..." 텔레그램 전송 |
| `handle_text_message` 호출 | 기존 로직 재사용 |
| 트리거 플래그 전달 | 봇 메시지 트리거 여부 표시 |

### 12.6 응답 캡처 및 파일 생성

| 항목 | 설명 |
|------|------|
| 트리거 플래그 확인 | 봇 메시지 트리거인 경우에만 |
| 히스토리에서 응답 추출 | 마지막 assistant 응답 전체 내용 |
| 새 메시지 파일 생성 | from/to 역전, content = AI 전체 응답 |

### 12.7 타임아웃 처리

| 항목 | 설명 |
|------|------|
| 타임아웃 시간 | 설정 가능 (기본 30분) |
| 체크 대상 | `from == my_username`이고 아직 존재하는 파일 |
| 처리 | 파일 삭제 + 채팅에 타임아웃 알림 |

---

## 13. 보안 고려사항

### 13.1 발신자 위조 방지
- `--from`을 파라미터로 받지 않음
- `--key`로 `bot_settings.json`을 조회하여 발신자 username을 시스템이 결정
- AI가 잘못된 발신자를 지정할 수 없음

### 13.2 수신자 검증
- `--to`로 지정된 봇이 `bot_settings.json`에 존재하는지 CLI에서 검증
- 존재하지 않으면 즉시 에러 반환

### 13.3 파일시스템 접근
- 모든 봇이 같은 서버/사용자 계정에서 동작
- `bot_settings.json`에 토큰이 포함되어 있으나, 동일 서버이므로 추가 보안 위험 없음
- 메시지 파일은 `~/.cokacdir/messages/`에 저장, 동일 서버의 모든 프로세스가 접근 가능

---

## 14. 향후 확장 가능성 (보류)

### 14.1 복수 수신자
```
cokacdir --message "내용" --to bot_b,bot_c,bot_d --chat {id} --key {key}
```
쉼표로 복수 수신자 지정 시 CLI가 각각의 메시지 파일을 생성.

### 14.2 다른 채팅의 봇과 통신
현재 설계는 같은 그룹 채팅 내 봇 간 통신만 지원.
다른 채팅의 봇과 통신하려면 `bot_settings.json`의 `last_sessions`를 조회하여 대상 봇의 chat_id를 결정하는 로직 필요.

### 14.3 3~4자간 자유 대화
현재 구조에서도 가능:
- 각 봇의 AI가 한 턴에서 여러 `--message`를 실행 가능
- 수신 봇의 AI도 응답하면서 다른 봇에게 `--message` 전송 가능
- N:N 자유 대화 가능하나, 각 메시지는 1:1 단위
