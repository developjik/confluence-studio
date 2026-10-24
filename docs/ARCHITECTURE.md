# Confluence Studio — Architecture & Module Contracts

Tauri 2 desktop app: Rust backend (src-tauri), React 19 + TypeScript frontend (src).
Confluence Cloud 관리(탐색/생성/수정/삭제), Markdown export 잡+스케줄(기존 Python 앱 의미론 이식),
Damon daemon(protocol v2 WS) thin-client AI 채팅.

## Golden rules (모든 기여자)

1. **수정 금지 파일**(integrator 소유): src-tauri/Cargo.toml, package.json, src-tauri/src/lib.rs, src-tauri/src/main.rs, src-tauri/tauri.conf.json, src-tauri/src/credentials.rs. 필요한 의존성은 이미 선언돼 있다. 부족하면 계약 문서에 맞춰 순수 Rust로 해결한다.
2. 아래 "Module contracts"의 **공개 API(이름·시그니처·의미)를 정확히 구현**한다. 통합 계층이 이 계약에 맞춰 작성된다.
3. 모든 DTO: #[derive(Debug, Clone, Serialize, Deserialize)] + #[serde(rename_all = "camelCase")]. JSON 키는 camelCase로 프론트와 통일한다.
4. 에러: 모듈 내부는 thiserror enum. Tauri 커맨드에서 String으로 변환될 것이므로 Display 메시지가 사용자 친화적이어야 한다.
5. 비동기: tokio. HTTP는 reqwest(rustls). 절대 async 안에서 blocking I/O 금지.
6. 테스트: 모듈 파일 내 #[cfg(test)] + src-tauri/tests/ 통합 테스트. cd src-tauri && cargo test 가 전부 통과해야 한다. clippy(default) 경과 0 권장.
7. 코드 스타일: rustfmt 기본. 주석은 영어 또는 한국어 자유.

## Tauri command contract (integrator가 와이어링, 전원 이 계약에 맞춤)

명령 이름(snake_case, Tauri가 JS camelCase→snake_case 자동 변환). 반환은 모두 camelCase JSON.

### 연결
| command | args | returns |
|---|---|---|
| save_connection | {baseUrl, email, token} | void (keyring 저장) |
| get_connection | — | {baseUrl, email} 또는 null |
| test_connection | — | {ok, siteName?, serverTime?} — 오류면 throw(문자열) |
| clear_connection | — | void |

### 스페이스/페이지
| command | args | returns |
|---|---|---|
| list_spaces | — | [{id, key, name, type}] |
| get_page_tree | {spaceId} | PageTreeNode (전체 트리, 각 노드 children 포함) |
| get_page | {pageId} | PageDetail |
| create_page | {spaceId, parentId?, title, markdown} | PageDetail |
| update_page | {pageId, title, markdown, version} | PageDetail |
| delete_page | {pageId} | void |

PageDetail = {id, spaceId, parentId?, title, status, version, markdown, webUrl, createdAt?, updatedAt?}
PageTreeNode = {id, title, status, children: [PageTreeNode]}

### 잡/스케줄
| command | args | returns |
|---|---|---|
| list_jobs | — | [Job] |
| save_job | {job: JobInput} | Job |
| delete_job | {jobId} | void |
| run_job | {jobId} | {runId} (비동기 시작) |
| cancel_job_run | {jobId} | {cancelled: bool} |
| list_job_runs | {jobId, limit?} | [RunRecord] |

### Damon
| command | args | returns |
|---|---|---|
| damon_status | — | {running, url?, version?, backends: [{id, available}]} |
| damon_list_sessions | {limit?} | [{sessionId, title, backend, createdAt, cwd}] |
| damon_create_session | {backend?, title?} | {sessionId, backend} |
| damon_prompt | {sessionId, prompt} | void (스트림은 이벤트로) |
| damon_cancel_turn | {sessionId} | {cancelled} |
| damon_permission_respond | {sessionId, requestId, behavior: "allow"|"deny"|"always"} | void |
| damon_delete_session | {sessionId} | {deleted} |
| damon_rename_session | {sessionId, title} | {renamed} |

### Tauri events (backend → frontend emit)
| event | payload |
|---|---|
| export://progress | {jobId, runId, message, done, total} |
| export://run-finished | {jobId, runId, status, summary} |
| damon://event | {sessionId, event} (event = Damon StreamEvent 그대로, camelCase 유지) |
| damon://turn-done | {sessionId, turnId, stopReason} |
| confluence://changed | {kind: "created"|"updated"|"deleted", pageId?, spaceId?} |

## Module: confluence (소유 파일: src-tauri/src/confluence/**)

하위 파일: mod.rs, models.rs, client.rs, convert.rs.

### models.rs
- pub struct ConfluenceAuth { pub base_url: String, pub email: String, pub token: String } (crate::credentials가 사용 — 반드시 이 위치에 정의)
- SpaceSummary {id, key, name, type}, PageTreeNode, PageDetail (위 계약), PageVersion {number, message?}

### client.rs — ConfluenceClient
- new(auth: ConfluenceAuth) -> Self, with_timeout(secs) 옵션.
- Basic auth(email:token base64). 모든 엔드포인트는 <base_url>/wiki/api/v2/... (base_url에 /wiki가 붙어 있으면 중복 금지 — 정규화).
- 비동기 함수:
  - test() -> Result<TestResult> — GET /wiki/api/v2/spaces?limit=1 이 200인지 확인.
  - list_spaces() -> Result<Vec<SpaceSummary>> — GET /wiki/api/v2/spaces?limit=50, _links.next 커서 페이지네이션(cursor 파라미터).
  - page_tree(space_id) -> Result<PageTreeNode> — GET /wiki/api/v2/spaces/{id}/pages (최상위) 후 재귀로 GET /wiki/api/v2/pages/{id}/children (커서 페이지네이션). 트리 완성. 순차 재귀 OK(병렬화 선택).
  - get_page(page_id) -> Result<PageDetail> — GET /wiki/api/v2/pages/{id}?body-format=storage. markdown 필드는 convert::storage_to_markdown 결과.
  - create_page(space_id, parent_id: Option<String>, title, markdown) -> Result<PageDetail> — POST /wiki/api/v2/pages, body {spaceId, status:"current", parentId?, title, body:{representation:"storage", value}}. value는 convert::markdown_to_storage 결과.
  - update_page(page_id, title, markdown, version_number) -> Result<PageDetail> — PUT /wiki/api/v2/pages/{id}, body {id, status:"current", title, body:{...}, version:{number: version_number+1}}. 반환은 새 version.
  - delete_page(page_id) -> Result<()> — DELETE /wiki/api/v2/pages/{id}.
  - export 지원용: get_descendants(page_id) -> Result<Vec<PageBrief>>(GET /wiki/api/v2/pages/{id}/descendants, 커서), get_page_storage(page_id) -> Result<PageStorage{storage, id, title, version, parentId?}>, list_attachments(content_id) -> Result<Vec<Attachment{ id, title, downloadUrl, mediaType }>>(GET /wiki/rest/api/content/{id}/child/attachment?limit=200, 커서), download_attachment(download_url) -> Result<Vec<u8>>(reqwest, base_url 상대 경로 처리).
- 에러 매핑: 401/403 → "인증 실패(이메일/API 토큰 확인)", 404 → "페이지/스페이스 없음", 그 외 상태코드+본문 요약.

### convert.rs — storage XHTML ↔ markdown
- pub fn storage_to_markdown(storage: &str) -> String, pub fn markdown_to_storage(md: &str) -> String.
- quick-xml 기반 파서. 지원 서브셋(양방향 최선 노력):
  - h1-h6, p, br, strong/b, em/i, code(inline), pre(코드블록, 언어 attr), a(href), ac:link → 일반 링크 텍스트화, ul/ol/li(중첩 2뎁스), blockquote, hr, table/thead/tbody/tr/th/td(GFM 테이블), img, ac:image(attachment) → ![](filename), ac:structured-macro → blockquote로 축약(&gt; [!INFO] + 내부 텍스트).
  - markdown→storage: headings/paragraph/lists(중첩)/code fence(언어 있으면 pre에 data-lang)/inline(bold/italic/code/link/img)/blockquote/hr/table(GFM). 변환 불가 요소는 텍스트 보존. 완벽한 왕복은 강요하지 않지만 round-trip 시 제목/리스트/본문 텍스트는 유실되지 않아야 한다.
- 단위 테스트: 각 요소 변환 케이스 + round-trip 텍스트 보존.

### 통합 테스트 (src-tauri/tests/confluence_client.rs, wiremock)
스페이스 목록(커서 2페이지), 페이지 트리(3레벨), get(→markdown 변환 포함), create/update/delete 요청 바디 검증(storage representation, version+1), 401 에러 메시지.

## Module: jobs (소유 파일: src-tauri/src/jobs/**)

하위 파일: mod.rs, models.rs, store.rs, scheduler.rs, export.rs, history.rs.

### models.rs — 기존 Python 앱 의미론 이식

pub struct Job {
  id: String, name: String,
  space_id: String,               // export 대상 스페이스
  root_page_id: Option<String>,   // 있으면 그 페이지 하위만
  target_dir: String,
  include_attachments: bool,
  enabled: bool,
  export_options: ExportOptions,
  schedule: Schedule,
}
pub struct ExportOptions {
  skip_unchanged: bool,   // default true
  cleanup_stale: bool,    // default true
  page_breadcrumbs: bool, // default true — 페이지 상단 "&gt; 상위1 &gt; 상위2" 경로 줄
  front_matter: bool,     // default true — YAML front matter(title, page_id, url, version, exported_at)
  max_workers: u32,       // default 8
}
pub enum ScheduleKind { Daily, MultipleDaily, Weekdays, Weekly, Monthly }
pub struct Schedule {
  enabled: bool,            // default false
  kind: ScheduleKind,       // default Daily
  times: Vec<String>,       // ["HH:MM"], default ["09:00"], 정렬·중복제거
  days_of_week: Vec<u32>,   // ISO(월=1..일=7), default [1,2,3,4,5]
  day_of_month: Option<u32>,
  timezone: String,         // IANA, default "Asia/Seoul" (chrono-tz)
  paused: bool,
}

이식 규칙(원본 scheduler.py): daily/multiple_daily=매일, weekdays=평일(월-금), weekly=days_of_week 포함일, monthly=day_of_month(그 달에 그 날짜가 없으면 미발생). 발생 판정은 occurrences_between(job, start_utc, end_utc): schedule tz로 변환, 각 매칭 일자 × times 중 start < t <= end 인 것들만. misfire=skip, concurrency=skip_if_running.

### store.rs
- JobStore::load(app_data_dir) -> Self (jobs.json 없으면 빈), list() -> Vec&lt;Job&gt;, save(&Job) (upsert by id, id 없으면 uuid), delete(id), persist() (tmp+rename 원자적 쓰기). serde camelCase.

### scheduler.rs
- pub struct Scheduler — occurrences_between(job, start, end) -> Vec&lt;DateTime&lt;Utc&gt;&gt; (위 규칙), run_loop(store, runner, tick, shutdown): 30초 tick, 마지막 tick 이후 occurrence 수집, 실행 중 잡은 skip(skip_if_running), 완료/실패는 history 기록. 실행은 스폰(백그라운드 태스크)하고 tick 루프는 막히지 않게.
- 단위 테스트(순수 로직 위주): daily 09:00 KST 경계, weekly days_of_week, monthly 31일 없는 달, disabled/paused → 발생 0, times 중복/정렬.

### export.rs — 네이티브 export 엔진 (의존성 주입으로 테스트)

pub struct ExportProgress { message: String, done: u32, total: u32 }
pub trait PageSource {   // 테스트용 목 구현 가능
  fn fetch_tree(&mut self, job: &Job) -> Result&lt;Vec&lt;PageData&gt;&gt;;
  fn fetch_attachment(&mut self, att: &AttachmentRef) -> Result&lt;Vec&lt;u8&gt;&gt;;
}
pub struct PageData { id, title, parent_title_path: Vec&lt;String&gt;, storage: String, attachments: Vec&lt;AttachmentRef&gt; }
pub struct ExportOutcome { run_id, status: "success"|"failed"|"cancelled", pages_written, pages_skipped, attachments_written, files_removed, started_at, finished_at, error: Option&lt;String&gt; }
pub async fn run_export(job, source: &mut dyn PageSource, progress, cancel: &AtomicBool) -> ExportOutcome

동작: 트리 fetch → 각 페이지 markdown 변환(crate::confluence::convert::storage_to_markdown 사용) → 디렉터리 구조: target/space명/상위경로/NN - Title.md (파일명 sanitize, NN은 같은 부모 내 순번) → front_matter/breadcrumbs 옵션 → skip_unchanged: 내용 sha256이 target/.confluence-studio/state.json 기록과 같으면 skip → cleanup_stale: state에 없는(원격에 사라진) 로컬 파일 삭제 → 첨부는 attachments/ 하위 저장. 실행 중 잡 추적(running set) 함수 포함 — cancel_job_run 계약과 맞을 것.

### history.rs
- append_run(target_dir, &ExportOutcome) — target/.confluence-studio/run_history.json (최근 100건), list_runs(target_dir) -> Vec&lt;ExportOutcome&gt;.
- 테스트: export.rs + store + scheduler + history 전부 실제 임시 디렉터리 기반 단위 테스트.

## Module: damon (소유 파일: src-tauri/src/damon/**)

하위 파일: mod.rs, protocol.rs, client.rs.

### protocol.rs — v2 프레임 모델
- Request {id, method, params}, Response {id, result? 또는 error: {code, message, data?}}, Push {event, sessionId?, data}, Hello {hello: {protocol, daemon, version, backends[], methods[], permissionTimeoutSecs}}.
- StreamEvent: type이 "turn_started" | "timeline" | "permission_requested" | "turn_completed" | "turn_failed" | "turn_canceled" | "attention_required" 중 하나. timeline은 kind: "assistant_message"|"reasoning"|"tool_call"|... 와 text. serde는 알 수 없는 필드 무시, camelCase.
- 사용 메서드: hello, backend.list, session.create {backend?, cwd?}, session.list {limit?}, session.delete {sessionId}, session.rename {sessionId, title}, turn.start {sessionId, prompt}(응답은 턴 종료 시 {turnId, stopReason, usage?}), turn.cancel {sessionId}, permission.respond {sessionId, requestId, response: {behavior}}. 오류 코드 -32001/-32003/-32004/-32006 등 타입 매핑.

### client.rs — DamonClient
- discover() -> Option&lt;DaemonEndpoint&gt;: ~/.damon/daemon.json ({port, pid, version, tls}) 읽고 http://127.0.0.1:{port}/health ({status, backends[]}) 프로브 → 살아있으면 endpoint, 아니면 None. (파일은 힌트일 뿐 — 반드시 health로 검증)
- connect(url, token: Option&lt;String&gt;) -> Result&lt;Self&gt;: tokio-tungstenite, Authorization: Bearer 헤더(토큰 있을 때). 연결 후 hello push 대기(timeout 3s, 실패해도 치명 아님). 내부: pending request map(id→oneshot), 이벤트 broadcast channel(tokio::sync::broadcast), 수신 루프 태스크.
- call(method, params) -> Result&lt;serde_json::Value&gt; (id 배정, 응답 대기, error→typed DamonError), create_session(backend: Option&lt;&str&gt;, cwd: Option&lt;&str&gt;), turn_start(session_id, prompt) -> TurnResult{turn_id, stop_reason, usage}(이벤트는 subscribe로), cancel_turn, respond_permission, list_sessions, delete_session, rename_session, subscribe_events() -> broadcast::Receiver&lt;DamonPush&gt;.
- Clone 가능한 핸들(Arc 내부) 권장. 재접속은 v1에서 선택.

### 통합 테스트 (src-tauri/tests/damon_client.rs, axum WS mock)
mock daemon: /ws 에서 hello push → session.create 응답 → turn.start 요청 수신 시 session.event 스트림(turn_started, timeline assistant_message 2청크, turn_completed) 후 응답 {turnId, stopReason:"completed"}. 클라이언트로: connect → create_session → subscribe → turn_start → 스트리밍 텍스트 누적("He"+"llo"="Hello") → stopReason completed 검증. + permission_requested → permission.respond 프레임 검증 1개, + error 코드 매핑 케이스 1개(-32003).

## Frontend (소유 파일: src/** 전부; index.html, vite.config.ts, tsconfig* 수정 금지)

- pnpm build(tsc && vite build) exit 0 필수(타입 에러 0). pnpm dev 로 띄워 수동 확인 가능(백엔드 커맨드 미완성 시 오류 표시로 graceful degrade — 화면 깨지지 않게 try/catch + 안내).
- src/types.ts: 위 Rust DTO의 TS 미러(camelCase). src/api.ts: invoke/listen 래퍼(타입 안전, 오류 문자열 그대로 throw).
- 화면(단일 창, 좌측 사이드바 + 메인 뷰 전환):
  1. Sidebar: 스페이스 목록(선택 시 페이지 트리, 펼침/접힘), 새 페이지 버튼, 뷰 전환(페이지/잡/채팅/설정).
  2. PageEditor: CodeMirror(@uiw/react-codemirror + lang-markdown) 마크다운 편집 + 미리보기 토글(react-markdown+remark-gfm), 저장(create_page/update_page), 삭제(확인 모달), 제목 입력, 상태바(버전/dirty 표시).
  3. JobsPanel: 잡 목록 + 편집 폼(이름, 대상 스페이스/루트 페이지, target_dir 폴더 선택 @tauri-apps/plugin-dialog, 첨부 포함, export 옵션, 스케줄 폼: kind별 필드 + timezone), 지금 실행 + 진행 표시(export://progress), 실행 이력(list_job_runs).
  4. ChatPanel: Damon 상태(damon_status), 세션 목록/선택/새로 만들기(backend 선택), 메시지 스트림(damon://event timeline assistant_message append, reasoning은 접힘 블록), 권한 요청 카드(허용/거절 → damon_permission_respond), 입력창(Shift+Enter 개행), 진행 중 취소 버튼.
  5. SettingsPanel: base_url/email/token 입력, 저장(save_connection), 연결 테스트(test_connection), 연결 해제(clear_connection).
- 디자인: 다크 테마 기본, CSS 변수(src/styles.css 단일 파일, 컴포넌트별 클래스), lucide-react 아이콘. 외부 UI 라이브러리 금지(이미 설치된 것만).
- 컴포넌트 단위는 자유롭되 위 5영역은 파일로 존재할 것(components/Sidebar.tsx, components/PageEditor.tsx, components/JobsPanel.tsx, components/ChatPanel.tsx, components/SettingsPanel.tsx).
