import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { EVENTS } from "./types";
import type {
  CancelJobRunResult,
  ConnectionInfo,
  DamonCancelTurnResult,
  DamonCreateSessionResult,
  DamonDeleteSessionResult,
  DamonRenameSessionResult,
  DamonSession,
  DamonStatus,
  Job,
  JobInput,
  PageDetail,
  PageTreeNode,
  PermissionBehavior,
  RunJobResult,
  RunRecord,
  SaveConnectionInput,
  SpaceSummary,
  TestConnectionResult,
} from "./types";

function toErrorMessage(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

/**
 * Typed invoke wrapper. Tauri command errors arrive as plain strings,
 * so they are normalized into Error objects whose message is shown as-is.
 * When the backend command is not wired yet (or running in a plain browser),
 * the promise rejects — callers are expected to catch and degrade gracefully.
 */
async function cmd<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    throw new Error(toErrorMessage(e));
  }
}

export const api = {
  // ---------- connection ----------
  saveConnection(input: SaveConnectionInput): Promise<void> {
    return cmd("save_connection", { ...input });
  },
  getConnection(): Promise<ConnectionInfo | null> {
    return cmd("get_connection");
  },
  testConnection(): Promise<TestConnectionResult> {
    return cmd("test_connection");
  },
  clearConnection(): Promise<void> {
    return cmd("clear_connection");
  },

  // ---------- spaces / pages ----------
  listSpaces(): Promise<SpaceSummary[]> {
    return cmd("list_spaces");
  },
  getPageTree(spaceId: string): Promise<PageTreeNode> {
    return cmd("get_page_tree", { spaceId });
  },
  getPage(pageId: string): Promise<PageDetail> {
    return cmd("get_page", { pageId });
  },
  createPage(input: {
    spaceId: string;
    parentId?: string | null;
    title: string;
    markdown: string;
  }): Promise<PageDetail> {
    return cmd("create_page", { ...input });
  },
  updatePage(input: {
    pageId: string;
    title: string;
    markdown: string;
    version: number;
  }): Promise<PageDetail> {
    return cmd("update_page", { ...input });
  },
  deletePage(pageId: string): Promise<void> {
    return cmd("delete_page", { pageId });
  },

  // ---------- jobs ----------
  listJobs(): Promise<Job[]> {
    return cmd("list_jobs");
  },
  saveJob(job: JobInput): Promise<Job> {
    return cmd("save_job", { job });
  },
  deleteJob(jobId: string): Promise<void> {
    return cmd("delete_job", { jobId });
  },
  runJob(jobId: string): Promise<RunJobResult> {
    return cmd("run_job", { jobId });
  },
  cancelJobRun(jobId: string): Promise<CancelJobRunResult> {
    return cmd("cancel_job_run", { jobId });
  },
  listJobRuns(jobId: string, limit?: number): Promise<RunRecord[]> {
    return cmd("list_job_runs", { jobId, limit });
  },

  // ---------- damon ----------
  damonStatus(): Promise<DamonStatus> {
    return cmd("damon_status");
  },
  damonListSessions(limit?: number): Promise<DamonSession[]> {
    return cmd("damon_list_sessions", { limit });
  },
  damonCreateSession(input: {
    backend?: string | null;
    title?: string | null;
  }): Promise<DamonCreateSessionResult> {
    return cmd("damon_create_session", { ...input });
  },
  damonPrompt(sessionId: string, prompt: string): Promise<void> {
    return cmd("damon_prompt", { sessionId, prompt });
  },
  damonCancelTurn(sessionId: string): Promise<DamonCancelTurnResult> {
    return cmd("damon_cancel_turn", { sessionId });
  },
  damonPermissionRespond(
    sessionId: string,
    requestId: string,
    behavior: PermissionBehavior,
  ): Promise<void> {
    return cmd("damon_permission_respond", { sessionId, requestId, behavior });
  },
  damonDeleteSession(sessionId: string): Promise<DamonDeleteSessionResult> {
    return cmd("damon_delete_session", { sessionId });
  },
  damonRenameSession(sessionId: string, title: string): Promise<DamonRenameSessionResult> {
    return cmd("damon_rename_session", { sessionId, title });
  },
};

/**
 * Typed event subscription. Returns an unlisten function.
 * Outside of the Tauri runtime (e.g. plain 'vite dev' in a browser) listen()
 * cannot register; we log once and return a no-op so panels keep rendering.
 */
export async function onEvent<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  try {
    return await listen<T>(event, (e) => handler(e.payload));
  } catch (e) {
    console.warn("[api] event listener unavailable: " + event + " (" + toErrorMessage(e) + ")");
    return () => undefined;
  }
}

export { EVENTS };
