// TypeScript mirrors of the Rust DTOs defined in docs/ARCHITECTURE.md.
// All JSON keys are camelCase (serde rename_all = "camelCase" on the Rust side).

// ---------- Connection ----------
export interface ConnectionInfo {
  baseUrl: string;
  email: string;
}

export interface SaveConnectionInput {
  baseUrl: string;
  email: string;
  token: string;
}

export interface TestConnectionResult {
  ok: boolean;
  siteName?: string | null;
  serverTime?: string | null;
}

// ---------- Spaces / pages ----------
export interface SpaceSummary {
  id: string;
  key: string;
  name: string;
  type: string;
}

export interface PageTreeNode {
  id: string;
  title: string;
  status: string;
  children: PageTreeNode[];
}

export interface PageDetail {
  id: string;
  spaceId: string;
  parentId?: string | null;
  title: string;
  status: string;
  version: number;
  markdown: string;
  webUrl: string;
  createdAt?: string | null;
  updatedAt?: string | null;
}

// ---------- Export jobs ----------
export type ScheduleKind = "Daily" | "MultipleDaily" | "Weekdays" | "Weekly" | "Monthly";

export interface ExportOptions {
  skipUnchanged: boolean;
  cleanupStale: boolean;
  pageBreadcrumbs: boolean;
  frontMatter: boolean;
  maxWorkers: number;
}

export interface Schedule {
  enabled: boolean;
  kind: ScheduleKind;
  /** ["HH:MM"] */
  times: string[];
  /** ISO weekday (Mon=1 .. Sun=7) */
  daysOfWeek: number[];
  dayOfMonth?: number | null;
  /** IANA timezone name */
  timezone: string;
  paused: boolean;
}

export interface Job {
  id: string;
  name: string;
  spaceId: string;
  rootPageId?: string | null;
  targetDir: string;
  includeAttachments: boolean;
  enabled: boolean;
  exportOptions: ExportOptions;
  schedule: Schedule;
}

/** Same shape as Job; id is optional so the backend can mint one for new jobs. */
export type JobInput = Omit<Job, "id"> & { id?: string };

export interface RunJobResult {
  runId: string;
}

export interface CancelJobRunResult {
  cancelled: boolean;
}

export type RunStatus = "success" | "failed" | "cancelled";

export interface RunRecord {
  runId: string;
  status: RunStatus | string;
  pagesWritten: number;
  pagesSkipped: number;
  attachmentsWritten: number;
  filesRemoved: number;
  startedAt: string;
  finishedAt: string;
  error?: string | null;
}

// ---------- Damon ----------
export interface DamonBackend {
  id: string;
  available: boolean;
}

export interface DamonStatus {
  running: boolean;
  url?: string | null;
  version?: string | null;
  backends: DamonBackend[];
}

export interface DamonSession {
  sessionId: string;
  title: string;
  backend: string;
  createdAt: string;
  cwd: string;
}

export interface DamonCreateSessionResult {
  sessionId: string;
  backend: string;
}

export interface DamonCancelTurnResult {
  cancelled: boolean;
}

export interface DamonDeleteSessionResult {
  deleted: boolean;
}

export interface DamonRenameSessionResult {
  renamed: boolean;
}

export type PermissionBehavior = "allow" | "deny" | "always";

export type DamonStreamEventType =
  | "turn_started"
  | "timeline"
  | "permission_requested"
  | "turn_completed"
  | "turn_failed"
  | "turn_canceled"
  | "attention_required";

/** Loose mirror of the Damon protocol v2 StreamEvent (unknown fields ignored). */
export interface DamonStreamEvent {
  type: DamonStreamEventType | string;
  /** timeline: assistant_message | reasoning | tool_call | ... */
  kind?: string;
  text?: string;
  /** permission_requested */
  requestId?: string;
  tool?: string;
  summary?: string;
  title?: string;
  reason?: string;
  turnId?: string;
  stopReason?: string;
  [key: string]: unknown;
}

// ---------- Tauri event payloads (backend -> frontend) ----------
export interface ExportProgressEvent {
  jobId: string;
  runId: string;
  message: string;
  done: number;
  total: number;
}

export interface ExportRunFinishedEvent {
  jobId: string;
  runId: string;
  status: string;
  summary: string;
}

export interface DamonEventPayload {
  sessionId: string;
  event: DamonStreamEvent;
}

export interface DamonTurnDonePayload {
  sessionId: string;
  turnId: string;
  stopReason: string;
}

export interface ConfluenceChangedEvent {
  kind: "created" | "updated" | "deleted";
  pageId?: string | null;
  spaceId?: string | null;
}

export const EVENTS = {
  exportProgress: "export://progress",
  exportRunFinished: "export://run-finished",
  damonEvent: "damon://event",
  damonTurnDone: "damon://turn-done",
  confluenceChanged: "confluence://changed",
} as const;
