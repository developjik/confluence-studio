// Demo backend — sample data behind the exact same `api` surface as the real
// Tauri commands, so the whole UI can run in a plain browser with
// `pnpm dev:demo` (no Tauri runtime, no Confluence site, no Damon daemon).
//
// Everything here is in-memory and resets on reload. It is a showcase of the
// UI and data flow only: nothing is written to disk and nothing reaches
// Confluence or Damon. `App` renders a DEMO badge so screenshots can never be
// mistaken for real site data.
import type { StudioApi } from "../api";
import type {
  ConnectionInfo,
  DamonCreateSessionResult,
  DamonDeleteSessionResult,
  DamonSession,
  DamonStatus,
  ExportProgressEvent,
  ExportRunFinishedEvent,
  Job,
  JobInput,
  PageDetail,
  PageTreeNode,
  RunRecord,
  SpaceSummary,
} from "../types";
import { EVENTS } from "../types";

// ---------------------------------------------------------------------------
// Tiny event bus standing in for `listen()` from @tauri-apps/api/event
// ---------------------------------------------------------------------------

type Listener = (payload: unknown) => void;
const listeners = new Map<string, Set<Listener>>();

export function demoOnEvent<T>(event: string, handler: (payload: T) => void): () => void {
  let set = listeners.get(event);
  if (!set) {
    set = new Set();
    listeners.set(event, set);
  }
  const wrapped = handler as unknown as Listener;
  set.add(wrapped);
  return () => {
    set?.delete(wrapped);
  };
}

function emit(event: string, payload: unknown): void {
  const set = listeners.get(event);
  if (!set) return;
  for (const handler of [...set]) handler(payload);
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));
const nowIso = (offsetMs = 0) => new Date(Date.now() + offsetMs).toISOString();
const DAY = 86_400_000;

// ---------------------------------------------------------------------------
// Sample spaces / pages
// ---------------------------------------------------------------------------

const SITE = "https://acme-demo.atlassian.net/wiki";

const SPACES: SpaceSummary[] = [
  { id: "1001", key: "ENG", name: "Engineering Wiki", type: "global" },
  { id: "1002", key: "PRD", name: "Product Specs", type: "global" },
  { id: "1003", key: "DSGN", name: "Design System", type: "global" },
];

const TREES: Record<string, PageTreeNode> = {
  "1001": {
    id: "1001",
    title: "Engineering Wiki",
    status: "current",
    children: [
      {
        id: "1101",
        title: "Architecture Decision Records",
        status: "current",
        children: [
          { id: "1102", title: "ADR-014 · Tauri 2 채택", status: "current", children: [] },
          { id: "1103", title: "ADR-015 · Markdown 단일 소스", status: "current", children: [] },
        ],
      },
      {
        id: "1104",
        title: "On-call Runbook",
        status: "current",
        children: [{ id: "1105", title: "Incident 2026-11-08 회고", status: "current", children: [] }],
      },
      { id: "1106", title: "Frontend Playbook", status: "current", children: [] },
    ],
  },
  "1002": {
    id: "1002",
    title: "Product Specs",
    status: "current",
    children: [
      {
        id: "1201",
        title: "Confluence Studio v0.2 범위",
        status: "current",
        children: [{ id: "1202", title: "릴리스 체크리스트", status: "current", children: [] }],
      },
    ],
  },
  "1003": {
    id: "1003",
    title: "Design System",
    status: "current",
    children: [
      { id: "1301", title: "Design Tokens", status: "current", children: [] },
      { id: "1302", title: "Dark theme 규칙", status: "current", children: [] },
    ],
  },
};

const BODIES: Record<string, { title: string; spaceId: string; parentId: string | null; markdown: string; version: number; updatedDaysAgo: number }> = {
  "1101": {
    title: "Architecture Decision Records",
    spaceId: "1001",
    parentId: null,
    version: 12,
    updatedDaysAgo: 2,
    markdown: `# Architecture Decision Records

ENG 스페이스의 기술 결정은 **ADR** 한 건당 페이지 하나로 남깁니다.
회의록과 달리, ADR은 "왜 그렇게 정했는가"만 남기고 나머지는 링크로 넘깁니다.

## 규칙

- 하나의 결정 = 하나의 ADR 페이지
- 상태: \`proposed\` → \`accepted\` → \`superseded\`
- 뒤집힌 결정은 지우지 않고 \`superseded by ADR-0xx\`만 덧붙입니다

## 목록

| ADR | 제목 | 상태 | 결정일 |
| --- | --- | --- | --- |
| 014 | Tauri 2 채택 | accepted | 2026-10-21 |
| 015 | Markdown 단일 소스 | accepted | 2026-10-27 |
| 016 | 데몬 프로토콜 v2 고정 | proposed | 2026-11-12 |

## 참고

- 템플릿: \`docs/adr/template.md\`
- 리뷰: 결정을 뒤집는 ADR은 최소 2명의 승인이 필요합니다
`,
  },
  "1102": {
    title: "ADR-014 · Tauri 2 채택",
    spaceId: "1001",
    parentId: "1101",
    version: 7,
    updatedDaysAgo: 6,
    markdown: `# ADR-014 · Tauri 2 채택

- 상태: accepted
- 결정일: 2026-10-21

## 맥락

사내 위키를 브라우저 탭 대신 데스크톱 창에서 다루고 싶다는 요구가 반복되었습니다.
후보는 Electron과 Tauri 2였고, 판단 기준은 아래 세 가지였습니다.

1. 유휴 상태의 메모리 사용량
2. OS 키체인에 자격증명을 맡길 수 있는가
3. 배포 번들 크기

## 결정

**Tauri 2 + Rust**를 씁니다. 웹뷰는 OS가 제공하는 것을 그대로 사용하고,
Confluence API 호출·파일 쓰기·스케줄러는 Rust 쪽에서 처리합니다.

\`\`\`rust
// 프론트는 커맨드만 호출하고, 토큰은 웹뷰로 내려오지 않습니다.
#[tauri::command]
pub async fn test_connection() -> Result<TestResult, String> {
    let client = confluence_client().await?;
    client.test().await.map_err(|e| e.to_string())
}
\`\`\`

## 결과

- Chromium을 통째로 번들하지 않아도 됩니다
- 키체인 연동이 플랫폼 기능으로 바로 붙습니다
- 대신 웹뷰 엔진(macOS WebKit / Windows WebView2) 차이를 항상 의식해야 합니다
`,
  },
  "1103": {
    title: "ADR-015 · Markdown 단일 소스",
    spaceId: "1001",
    parentId: "1101",
    version: 4,
    updatedDaysAgo: 12,
    markdown: `# ADR-015 · Markdown 단일 소스

- 상태: accepted
- 결정일: 2026-10-27

## 결정

편집 화면은 **Markdown**만 다루고, 저장할 때 Rust 쪽에서 Confluence storage XHTML로 변환합니다.
원본은 언제나 Confluence에 남고, 로컬 export는 같은 변환 파이프라인을 재사용합니다.

## 왕복 규칙

- 제목, 문단, 리스트, 표, 코드 블록은 왕복해도 텍스트가 유실되지 않아야 합니다
- Confluence 매크로처럼 Markdown에 대응이 없는 요소는 인용 블록으로 축약합니다
- 완벽한 왕복은 목표가 아닙니다. 대신 유실되면 안 되는 것을 테스트로 고정합니다

> [!NOTE] 변환기는 \`src-tauri/src/confluence/convert.rs\` 한 곳에만 있습니다.
`,
  },
  "1104": {
    title: "On-call Runbook",
    spaceId: "1001",
    parentId: null,
    version: 23,
    updatedDaysAgo: 1,
    markdown: `# On-call Runbook

당직자는 **P1 인시던트의 단일 창구**입니다. 해결이 아니라 "지금 무엇을 하고 있는지"를 계속 알리는 것이 역할입니다.

## P1 대응 절차

- [ ] 인시던트 채널 개설 (\`#inc-YYYYMMDD-<keyword>\`)
- [ ] 영향 범위 1차 확인 — 고객 수, 리전, 마지막 정상 배포 시각
- [ ] 상태 페이지 초안 작성 (10분 안에)
- [ ] 30분마다 업데이트. 진전이 없어도 "아직 원인 조사 중"을 남깁니다
- [ ] 종료 시 이 스페이스에 회고 페이지 생성

## 에스컬레이션

| 등급 | 기준 | 연락 |
| --- | --- | --- |
| P1 | 결제·로그인 불가 | 당직자 → 인프라 리드 |
| P2 | 일부 기능 저하 | 당직자 단독 대응 |
| P3 | 문의성 | 주간 배치 |

## 자주 쓰는 명령

\`\`\`bash
# 최근 배포 확인
gh run list --workflow deploy.yml --limit 5
\`\`\`
`,
  },
  "1105": {
    title: "Incident 2026-11-08 회고",
    spaceId: "1001",
    parentId: "1104",
    version: 3,
    updatedDaysAgo: 9,
    markdown: `# Incident 2026-11-08 회고

- 영향: 42분간 로그인 리다이렉트 실패
- 등급: P1
- 원인: 환경 변수 이름 변경이 배포 파이프라인에만 반영됨

## 타임라인

- 09:12 첫 알림
- 09:19 원인 후보 확인
- 09:54 롤백 완료

## 다음에 할 일

- [x] 배포 전 환경 변수 대조 스텝 추가
- [ ] 스테이징에서 로그인 스모크 테스트 자동화
- [ ] 상태 페이지 템플릿에 "롤백 진행 중" 문구 추가
`,
  },
  "1106": {
    title: "Frontend Playbook",
    spaceId: "1001",
    parentId: null,
    version: 15,
    updatedDaysAgo: 3,
    markdown: `# Frontend Playbook

React 19 + TypeScript. 상태는 화면 가까이에 두고, 서버 상태는 명령 호출 결과로만 갱신합니다.

## 원칙

1. 스트리밍 응답과 에디터 입력이 같은 렌더를 공유하지 않게 컴포넌트 경계를 나눕니다
2. 실패는 조용히 삼키지 않고 **화면에 그대로** 보여줍니다 (재시도 버튼 포함)
3. 백엔드가 없어도 화면이 깨지지 않아야 합니다 — 모든 호출은 \`try/catch\`로 감쌉니다

## 자주 하는 실수

- \`useEffect\` 의존성에 객체를 넣어 매 렌더마다 재구독하는 것
- StrictMode 이중 마운트에서 리스너를 두 번 등록하는 것
`,
  },
  "1201": {
    title: "Confluence Studio v0.2 범위",
    spaceId: "1002",
    parentId: null,
    version: 9,
    updatedDaysAgo: 4,
    markdown: `# Confluence Studio v0.2 범위

## 목표

"위키를 열기 위해 브라우저 탭을 찾지 않는다"를 실제로 달성하는 것.

## 포함

- 스페이스/페이지 트리 탐색과 Markdown 편집
- 예약 export (스케줄 종류 5가지, IANA 타임존)
- Damon 데몬과의 스트리밍 채팅, 권한 요청 카드

## 제외

- 페이지 댓글과 첨부 편집
- 다중 사이트 동시 연결
- 모바일 빌드

## 지표

| 항목 | 목표 |
| --- | --- |
| 앱 시작 | 1초 이내 체감 |
| export 실패 | 재시도로 복구 가능 |
| 토큰 노출 | 0건 |
`,
  },
  "1202": {
    title: "릴리스 체크리스트",
    spaceId: "1002",
    parentId: "1201",
    version: 2,
    updatedDaysAgo: 5,
    markdown: `# 릴리스 체크리스트

- [ ] \`pnpm build\` 통과
- [ ] \`cargo test\` 통과
- [ ] macOS / Windows 번들 각각 실행 확인
- [ ] 키체인 저장·삭제 확인
- [ ] export 잡 1회 실제 실행
- [ ] CHANGELOG 갱신
`,
  },
  "1301": {
    title: "Design Tokens",
    spaceId: "1003",
    parentId: null,
    version: 6,
    updatedDaysAgo: 8,
    markdown: `# Design Tokens

색은 CSS 변수 하나로만 정의하고, 컴포넌트는 변수만 참조합니다.

\`\`\`css
:root {
  --bg: #0e1116;
  --bg-elev: #151a22;
  --text: #e8ecf3;
  --accent: #4d8dff;
}
\`\`\`

## 규칙

- 새 색을 추가하기 전에 기존 변수로 표현할 수 있는지 먼저 확인
- 대비비 4.5:1 미만 조합은 본문에 쓰지 않습니다
`,
  },
  "1302": {
    title: "Dark theme 규칙",
    spaceId: "1003",
    parentId: null,
    version: 3,
    updatedDaysAgo: 15,
    markdown: `# Dark theme 규칙

- 배경은 3단(\`--bg\`, \`--bg-elev\`, \`--bg-elev-2\`)까지만
- 테두리는 배경보다 한 단계 밝게
- 강조색은 파랑 하나, 위험은 빨강 하나만
- 그림자 대신 테두리로 층을 나눕니다
`,
  },
};

// ---------------------------------------------------------------------------
// Mutable in-memory state
// ---------------------------------------------------------------------------

const pages = new Map<string, PageDetail>();
for (const [id, body] of Object.entries(BODIES)) {
  pages.set(id, {
    id,
    spaceId: body.spaceId,
    parentId: body.parentId,
    title: body.title,
    status: "current",
    version: body.version,
    markdown: body.markdown,
    webUrl: `${SITE}/spaces/${body.spaceId}/pages/${id}`,
    createdAt: nowIso(-90 * DAY),
    updatedAt: nowIso(-body.updatedDaysAgo * DAY),
  });
}

const trees: Record<string, PageTreeNode> = JSON.parse(JSON.stringify(TREES)) as Record<string, PageTreeNode>;

function findNode(node: PageTreeNode, id: string): PageTreeNode | null {
  if (node.id === id) return node;
  for (const child of node.children) {
    const hit = findNode(child, id);
    if (hit) return hit;
  }
  return null;
}

function removeNode(node: PageTreeNode, id: string): boolean {
  const idx = node.children.findIndex((c) => c.id === id);
  if (idx >= 0) {
    node.children.splice(idx, 1);
    return true;
  }
  return node.children.some((c) => removeNode(c, id));
}

const jobs: Job[] = [
  {
    id: "job-eng-weekly",
    name: "팀 위키 주간 백업",
    spaceId: "1001",
    rootPageId: null,
    targetDir: "/Users/ingwon/Exports/eng-wiki",
    includeAttachments: true,
    enabled: true,
    exportOptions: {
      skipUnchanged: true,
      cleanupStale: true,
      pageBreadcrumbs: true,
      frontMatter: true,
      maxWorkers: 8,
    },
    schedule: {
      enabled: true,
      kind: "Weekly",
      times: ["09:00"],
      daysOfWeek: [1],
      dayOfMonth: null,
      timezone: "Asia/Seoul",
      paused: false,
    },
  },
  {
    id: "job-prd-daily",
    name: "제품 스펙 데일리 스냅샷",
    spaceId: "1002",
    rootPageId: "1201",
    targetDir: "/Users/ingwon/Exports/prd",
    includeAttachments: false,
    enabled: true,
    exportOptions: {
      skipUnchanged: true,
      cleanupStale: false,
      pageBreadcrumbs: false,
      frontMatter: true,
      maxWorkers: 4,
    },
    schedule: {
      enabled: true,
      kind: "Daily",
      times: ["07:30", "19:30"],
      daysOfWeek: [1, 2, 3, 4, 5],
      dayOfMonth: null,
      timezone: "Asia/Seoul",
      paused: false,
    },
  },
];

const runs: Record<string, RunRecord[]> = {
  "job-eng-weekly": [
    {
      runId: "run-0042",
      status: "success",
      pagesWritten: 5,
      pagesSkipped: 12,
      attachmentsWritten: 3,
      filesRemoved: 1,
      startedAt: nowIso(-2 * DAY),
      finishedAt: nowIso(-2 * DAY + 9_400),
      error: null,
    },
    {
      runId: "run-0041",
      status: "success",
      pagesWritten: 2,
      pagesSkipped: 15,
      attachmentsWritten: 0,
      filesRemoved: 0,
      startedAt: nowIso(-9 * DAY),
      finishedAt: nowIso(-9 * DAY + 7_100),
      error: null,
    },
  ],
  "job-prd-daily": [
    {
      runId: "run-0137",
      status: "failed",
      pagesWritten: 0,
      pagesSkipped: 0,
      attachmentsWritten: 0,
      filesRemoved: 0,
      startedAt: nowIso(-1 * DAY),
      finishedAt: nowIso(-1 * DAY + 2_300),
      error: "인증 실패(이메일/API 토큰 확인)",
    },
    {
      runId: "run-0136",
      status: "success",
      pagesWritten: 2,
      pagesSkipped: 0,
      attachmentsWritten: 0,
      filesRemoved: 0,
      startedAt: nowIso(-1 * DAY - 12 * 3_600_000),
      finishedAt: nowIso(-1 * DAY - 12 * 3_600_000 + 1_800),
      error: null,
    },
  ],
};

const sessions: DamonSession[] = [
  {
    sessionId: "sess-7c1f",
    title: "온콜 런북 P1 절차 요약",
    backend: "claude",
    createdAt: nowIso(-3 * 3_600_000),
    cwd: "/Users/ingwon/work/confluence-studio",
  },
  {
    sessionId: "sess-2b90",
    title: "ADR-014 근거 정리",
    backend: "codex",
    createdAt: nowIso(-26 * 3_600_000),
    cwd: "/Users/ingwon/work/confluence-studio",
  },
];

const cancelledTurns = new Set<string>();
let sequence = 0;

// ---------------------------------------------------------------------------
// Damon demo turn: streams a canned, technical answer
// ---------------------------------------------------------------------------

const REPLY = `**On-call Runbook · P1 절차 (요약)**

1. \`#inc-YYYYMMDD-<keyword>\` 채널을 열고 당직자가 단일 창구가 됩니다.
2. 영향 범위를 1차 확인합니다 — 고객 수, 리전, 마지막 정상 배포 시각.
3. 10분 안에 상태 페이지 초안을 올립니다.
4. **30분마다** 업데이트합니다. 진전이 없어도 "원인 조사 중"을 남깁니다.
5. 종료 후 \`On-call Runbook\` 하위에 회고 페이지를 만듭니다.

에스컬레이션은 P1(결제·로그인 불가)일 때만 인프라 리드에게 넘깁니다.
관련 페이지: **On-call Runbook** (ENG) · **Incident 2026-11-08 회고**`;

async function streamTurn(sessionId: string, turnId: string, reply: string): Promise<void> {
  emit(EVENTS.damonEvent, {
    sessionId,
    event: { type: "turn_started", turnId },
  });
  await sleep(180);
  if (cancelledTurns.has(turnId)) return;
  const chunks = reply.match(/[\s\S]{1,24}/g) ?? [reply];
  for (const chunk of chunks) {
    if (cancelledTurns.has(turnId)) return;
    emit(EVENTS.damonEvent, {
      sessionId,
      event: { type: "timeline", kind: "assistant_message", text: chunk },
    });
    await sleep(55);
  }
  if (cancelledTurns.has(turnId)) return;
  emit(EVENTS.damonEvent, {
    sessionId,
    event: { type: "turn_completed", turnId },
  });
  emit(EVENTS.damonTurnDone, {
    sessionId,
    turnId,
    stopReason: "completed",
    error: undefined,
  });
}

// ---------------------------------------------------------------------------
// Export run simulation
// ---------------------------------------------------------------------------

const runningJobs = new Set<string>();

async function simulateRun(job: Job, runId: string): Promise<void> {
  const total = 9;
  for (let done = 1; done <= total; done++) {
    if (!runningJobs.has(job.id)) {
      // cancelled
      const cancelled: RunRecord = {
        runId,
        status: "cancelled",
        pagesWritten: done - 1,
        pagesSkipped: 0,
        attachmentsWritten: 0,
        filesRemoved: 0,
        startedAt: nowIso(-done * 300),
        finishedAt: nowIso(),
      };
      (runs[job.id] ??= []).unshift(cancelled);
      emit(EVENTS.exportRunFinished, {
        jobId: job.id,
        runId,
        status: "cancelled",
        summary: `취소됨 · ${done - 1}개 페이지 저장`,
      } satisfies ExportRunFinishedEvent);
      return;
    }
    emit(EVENTS.exportProgress, {
      jobId: job.id,
      runId,
      message: `페이지 내보내는 중… (${done}/${total})`,
      done,
      total,
    } satisfies ExportProgressEvent);
    await sleep(220);
  }
  const record: RunRecord = {
    runId,
    status: "success",
    pagesWritten: total,
    pagesSkipped: 4,
    attachmentsWritten: 2,
    filesRemoved: 0,
    startedAt: nowIso(-total * 220),
    finishedAt: nowIso(),
  };
  (runs[job.id] ??= []).unshift(record);
  runningJobs.delete(job.id);
  emit(EVENTS.exportRunFinished, {
    jobId: job.id,
    runId,
    status: "success",
    summary: `${total}개 페이지 · 첨부 2개 · 건너뜀 4개`,
  } satisfies ExportRunFinishedEvent);
}

// ---------------------------------------------------------------------------
// The demo api — same signatures as the Tauri command wrapper
// ---------------------------------------------------------------------------

export const demoApi: StudioApi = {
  // ---------- connection ----------
  async saveConnection(): Promise<void> {
    /* in-memory only */
  },
  async getConnection(): Promise<ConnectionInfo | null> {
    return { baseUrl: "https://acme-demo.atlassian.net", email: "demo@acme-demo.atlassian.net" };
  },
  async testConnection() {
    await sleep(350);
    return { ok: true, siteName: "Acme Demo", serverTime: nowIso() };
  },
  async clearConnection(): Promise<void> {
    /* in-memory only */
  },

  // ---------- spaces / pages ----------
  async listSpaces(): Promise<SpaceSummary[]> {
    await sleep(120);
    return [...SPACES];
  },
  async getPageTree(spaceId: string): Promise<PageTreeNode> {
    await sleep(140);
    const tree = trees[spaceId];
    if (!tree) throw new Error("페이지/스페이스 없음");
    return JSON.parse(JSON.stringify(tree)) as PageTreeNode;
  },
  async getPage(pageId: string): Promise<PageDetail> {
    await sleep(110);
    const page = pages.get(pageId);
    if (!page) throw new Error("페이지/스페이스 없음");
    return { ...page };
  },
  async createPage(input): Promise<PageDetail> {
    await sleep(180);
    const id = String(9000 + ++sequence);
    const page: PageDetail = {
      id,
      spaceId: input.spaceId,
      parentId: input.parentId ?? null,
      title: input.title,
      status: "current",
      version: 1,
      markdown: input.markdown,
      webUrl: `${SITE}/spaces/${input.spaceId}/pages/${id}`,
      createdAt: nowIso(),
      updatedAt: nowIso(),
    };
    pages.set(id, page);
    const tree = trees[input.spaceId];
    if (tree) {
      const parent = input.parentId ? findNode(tree, input.parentId) : tree;
      parent?.children.push({ id, title: page.title, status: "current", children: [] });
    }
    return { ...page };
  },
  async updatePage(input): Promise<PageDetail> {
    await sleep(180);
    const page = pages.get(input.pageId);
    if (!page) throw new Error("페이지/스페이스 없음");
    const next: PageDetail = {
      ...page,
      title: input.title,
      markdown: input.markdown,
      version: page.version + 1,
      updatedAt: nowIso(),
    };
    pages.set(page.id, next);
    const tree = trees[next.spaceId];
    const node = tree ? findNode(tree, next.id) : null;
    if (node) node.title = next.title;
    return { ...next };
  },
  async deletePage(pageId: string): Promise<void> {
    await sleep(150);
    const page = pages.get(pageId);
    pages.delete(pageId);
    if (page) {
      const tree = trees[page.spaceId];
      if (tree) removeNode(tree, pageId);
    }
  },

  // ---------- jobs ----------
  async listJobs(): Promise<Job[]> {
    await sleep(120);
    return JSON.parse(JSON.stringify(jobs)) as Job[];
  },
  async saveJob(input: JobInput): Promise<Job> {
    await sleep(160);
    const job: Job = { ...(JSON.parse(JSON.stringify(input)) as JobInput), id: input.id || `job-${++sequence}` } as Job;
    const idx = jobs.findIndex((j) => j.id === job.id);
    if (idx >= 0) jobs[idx] = job;
    else jobs.push(job);
    return JSON.parse(JSON.stringify(job)) as Job;
  },
  async deleteJob(jobId: string): Promise<void> {
    await sleep(120);
    const idx = jobs.findIndex((j) => j.id === jobId);
    if (idx >= 0) jobs.splice(idx, 1);
    delete runs[jobId];
  },
  async runJob(jobId: string) {
    await sleep(90);
    const job = jobs.find((j) => j.id === jobId);
    if (!job) throw new Error("잡을 찾을 수 없습니다");
    const runId = `run-${1000 + ++sequence}`;
    runningJobs.add(jobId);
    void simulateRun(job, runId);
    return { runId };
  },
  async cancelJobRun(jobId: string) {
    await sleep(80);
    const wasRunning = runningJobs.delete(jobId);
    return { cancelled: wasRunning };
  },
  async listJobRuns(jobId: string): Promise<RunRecord[]> {
    await sleep(110);
    return (runs[jobId] ?? []).map((r) => ({ ...r }));
  },

  // ---------- damon ----------
  async damonStatus(): Promise<DamonStatus> {
    await sleep(90);
    return {
      running: true,
      url: "ws://127.0.0.1:4310/ws",
      version: "0.8.0",
      backends: [
        { id: "claude", available: true },
        { id: "codex", available: true },
      ],
    };
  },
  async damonListSessions(): Promise<DamonSession[]> {
    await sleep(110);
    return sessions.map((s) => ({ ...s }));
  },
  async damonCreateSession(input): Promise<DamonCreateSessionResult> {
    await sleep(200);
    const session: DamonSession = {
      sessionId: `sess-${++sequence}`,
      title: input.title ?? "새 세션",
      backend: input.backend ?? "claude",
      createdAt: nowIso(),
      cwd: "/Users/ingwon/work/confluence-studio",
    };
    sessions.unshift(session);
    return { sessionId: session.sessionId, backend: session.backend };
  },
  async damonPrompt(sessionId: string, prompt: string): Promise<void> {
    await sleep(60);
    if (!prompt.trim()) throw new Error("빈 프롬프트");
    const turnId = `turn-${++sequence}`;
    void streamTurn(sessionId, turnId, REPLY);
  },
  async damonCancelTurn(sessionId: string) {
    await sleep(60);
    const last = [...cancelledTurns];
    void last;
    const turnId = `turn-${sequence}`;
    cancelledTurns.add(turnId);
    emit(EVENTS.damonTurnDone, { sessionId, turnId, stopReason: "canceled", error: undefined });
    return { cancelled: true };
  },
  async damonPermissionRespond(): Promise<void> {
    await sleep(60);
  },
  async damonDeleteSession(sessionId: string): Promise<DamonDeleteSessionResult> {
    await sleep(100);
    const idx = sessions.findIndex((s) => s.sessionId === sessionId);
    if (idx >= 0) sessions.splice(idx, 1);
    return { deleted: idx >= 0 };
  },
  async damonRenameSession(sessionId: string, title: string) {
    await sleep(100);
    const session = sessions.find((s) => s.sessionId === sessionId);
    if (session) session.title = title;
    return { renamed: session != null };
  },
};
