import { useCallback, useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Bot, Plus, RefreshCw, Send, Square, Wrench, Zap } from "lucide-react";
import { api, EVENTS, onEvent } from "../api";
import type {
  DamonEventPayload,
  DamonSession,
  DamonStatus,
  DamonStreamEvent,
  DamonTurnDonePayload,
  PermissionBehavior,
} from "../types";
import { EmptyState, ErrorBanner, Loading, uid } from "./common";

interface ChatEntry {
  id: string;
  role: "user" | "assistant" | "reasoning" | "tool" | "system";
  text: string;
  meta?: string;
  streaming?: boolean;
}

interface PermissionRequest {
  requestId: string;
  tool: string;
  summary: string;
}

export function ChatPanel() {
  const [status, setStatus] = useState<DamonStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [sessions, setSessions] = useState<DamonSession[]>([]);
  const [sessionsLoading, setSessionsLoading] = useState(true);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [entriesBySession, setEntriesBySession] = useState<Record<string, ChatEntry[]>>({});
  const [permissions, setPermissions] = useState<PermissionRequest[]>([]);
  const [streaming, setStreaming] = useState(false);
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [newBackend, setNewBackend] = useState("");
  const [creating, setCreating] = useState(false);

  const sessionIdRef = useRef<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    sessionIdRef.current = sessionId;
  }, [sessionId]);

  const entries: ChatEntry[] =
    sessionId != null ? entriesBySession[sessionId] ?? [] : [];

  const reloadStatus = useCallback(async () => {
    setStatusError(null);
    try {
      setStatus(await api.damonStatus());
    } catch (e) {
      setStatus(null);
      setStatusError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const reloadSessions = useCallback(async () => {
    setSessionsLoading(true);
    try {
      const list = await api.damonListSessions(50);
      setSessions(list);
      setSessionId((prev) =>
        prev != null && list.some((s) => s.sessionId === prev)
          ? prev
          : list.length > 0
            ? list[0].sessionId
            : null,
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSessionsLoading(false);
    }
  }, []);

  useEffect(() => {
    void reloadStatus();
    void reloadSessions();
  }, [reloadStatus, reloadSessions]);

  // Permission cards belong to the currently open session only.
  useEffect(() => {
    setPermissions([]);
  }, [sessionId]);

  const finalizeTurn = useCallback((sid: string, stopReason: string, error?: string) => {
    setEntriesBySession((prev) => {
      const list = (prev[sid] ?? []).map((e) => (e.streaming ? { ...e, streaming: false } : e));
      if (stopReason !== "completed") {
        const detail = error ? (stopReason === "canceled" ? " (" + error + ")" : error) : "";
        list.push({
          id: uid(),
          role: "system",
          text: stopReason === "canceled" ? "턴이 취소되었습니다." + detail : "턴 종료: " + stopReason + detail,
        });
      }
      return { ...prev, [sid]: list };
    });
    if (sessionIdRef.current === sid) setStreaming(false);
  }, []);

  function applyDamonEvent(sid: string, ev: DamonStreamEvent) {
    setEntriesBySession((prev) => {
      const list = (prev[sid] ?? []).slice();
      let lastAssistant = -1;
      for (let i = list.length - 1; i >= 0; i--) {
        if (list[i].role === "assistant") {
          lastAssistant = i;
          break;
        }
      }
      if (ev.type === "timeline") {
        if (ev.kind === "reasoning") {
          const last = list.length > 0 ? list[list.length - 1] : null;
          if (last && last.role === "reasoning") {
            list[list.length - 1] = { ...last, text: last.text + (ev.text ?? "") };
          } else {
            list.push({ id: uid(), role: "reasoning", text: ev.text ?? "" });
          }
        } else if (ev.kind === "assistant_message") {
          const incoming = ev.text ?? "";
          if (lastAssistant >= 0 && list[lastAssistant].streaming) {
            const target = list[lastAssistant];
            // 데몬은 전체 스냅샷을 보낼 수도 있고 델타를 보낼 수도 있다:
            // 누적본이 incoming의 접두사면 스냅샷 → 교체, 아니면 델타 → append.
            const isSnapshot = incoming.startsWith(target.text) && target.text.length > 0;
            const text = isSnapshot ? incoming : target.text + incoming;
            list[lastAssistant] = { ...target, text, streaming: true };
          } else {
            list.push({ id: uid(), role: "assistant", text: incoming, streaming: true });
          }
        } else {
          list.push({ id: uid(), role: "tool", text: ev.text ?? "", meta: ev.kind });
        }
      } else if (ev.type === "permission_requested") {
        list.push({
          id: uid(),
          role: "system",
          text: "권한 요청이 도착했습니다. 아래 카드에서 허용/거절하세요.",
        });
      } else if (ev.type === "turn_failed") {
        list.push({ id: uid(), role: "system", text: "턴 실패: " + (ev.reason ?? "알 수 없는 오류") });
      }
      return { ...prev, [sid]: list };
    });
  }

  useEffect(() => {
    const cleanups: Array<() => void> = [];
    let cancelled = false; // cleanup이 listen 해결 전에 실행되면(StrictMode 이중 마운트 등) 즉시 해제한다
    void (async () => {
      const un1 = await onEvent<DamonEventPayload>(EVENTS.damonEvent, ({ sessionId: sid, event: ev }) => {
        applyDamonEvent(sid, ev);
        if (ev.type === "permission_requested" && sessionIdRef.current === sid) {
          setPermissions((prev) => [
            ...prev.filter((p) => p.requestId !== ev.requestId),
            { requestId: ev.requestId ?? uid(), tool: ev.tool ?? "", summary: ev.summary ?? "" },
          ]);
        }
      });
      if (cancelled) { un1(); return; }
      cleanups.push(un1);
      const un2 = await onEvent<DamonTurnDonePayload>(EVENTS.damonTurnDone, ({ sessionId: sid, stopReason, error }) => {
        finalizeTurn(sid, stopReason, error);
        if (sessionIdRef.current === sid) setPermissions([]);
        void reloadSessions();
      });
      if (cancelled) { un2(); return; }
      cleanups.push(un2);
    })();
    return () => {
      cancelled = true;
      cleanups.forEach((fn) => fn());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [finalizeTurn, reloadSessions]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [entries.length, streaming]);

  async function handleSend() {
    const sid = sessionId;
    const text = input.trim();
    if (!sid || !text || streaming) return;
    setInput("");
    setError(null);
    setEntriesBySession((prev) => ({
      ...prev,
      [sid]: [
        ...(prev[sid] ?? []),
        { id: uid(), role: "user" as const, text },
        { id: uid(), role: "assistant" as const, text: "", streaming: true },
      ],
    }));
    setStreaming(true);
    try {
      await api.damonPrompt(sid, text);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      setEntriesBySession((prev) => {
        const list = (prev[sid] ?? []).map((en) => (en.streaming ? { ...en, streaming: false } : en));
        list.push({ id: uid(), role: "system", text: "전송 실패: " + msg });
        return { ...prev, [sid]: list };
      });
      setStreaming(false);
    }
  }

  async function handleCancel() {
    const sid = sessionId;
    if (!sid) return;
    try {
      await api.damonCancelTurn(sid);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      finalizeTurn(sid, "canceled");
    }
  }

  async function handleCreateSession() {
    setCreating(true);
    setError(null);
    try {
      const res = await api.damonCreateSession({ backend: newBackend || undefined, title: null });
      setSessions((prev) => [
        {
          sessionId: res.sessionId,
          title: "새 세션",
          backend: res.backend,
          createdAt: new Date().toISOString(),
          cwd: "",
        },
        ...prev,
      ]);
      setSessionId(res.sessionId);
      setEntriesBySession((prev) => ({
        ...prev,
        [res.sessionId]: [{ id: uid(), role: "system", text: "세션이 생성되었습니다 (" + res.backend + ")." }],
      }));
      void reloadStatus();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setCreating(false);
    }
  }

  async function respond(requestId: string, behavior: PermissionBehavior) {
    const sid = sessionId;
    if (!sid) return;
    try {
      await api.damonPermissionRespond(sid, requestId, behavior);
      setPermissions((prev) => prev.filter((p) => p.requestId !== requestId));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="chat-panel">
      <div className="chat-sessions">
        <div className="chat-sessions-head">
          <span>세션</span>
          <div style={{ display: "flex", gap: 2 }}>
            <button className="icon-btn" title="새로고침" onClick={() => { void reloadStatus(); void reloadSessions(); }}>
              <RefreshCw size={12} />
            </button>
            <button className="icon-btn" title="새 세션" disabled={creating} onClick={() => void handleCreateSession()}>
              <Plus size={12} />
            </button>
          </div>
        </div>
        <div className="field">
          <select className="input input-xs" value={newBackend} onChange={(e) => setNewBackend(e.target.value)}>
            <option value="">기본 백엔드</option>
            {(status?.backends ?? []).map((b) => (
              <option key={b.id} value={b.id} disabled={!b.available}>
                {b.id}
                {b.available ? "" : " (사용 불가)"}
              </option>
            ))}
          </select>
        </div>
        {sessionsLoading && sessions.length === 0 ? (
          <Loading label="세션 불러오는 중…" />
        ) : sessions.length === 0 ? (
          <p className="dim-note">
            세션이 없습니다. 위 + 버튼으로 새 세션을 만드세요. 백엔드를 먼저 선택할 수 있습니다.
          </p>
        ) : (
          sessions.map((s) => (
            <button
              key={s.sessionId}
              className={"session-item" + (sessionId === s.sessionId ? " selected" : "")}
              onClick={() => setSessionId(s.sessionId)}
              title={s.title + " · " + s.cwd}
            >
              <span className="session-title">{s.title || "(제목 없음)"}</span>
              <span className="session-sub">{s.backend}</span>
            </button>
          ))
        )}
      </div>

      <div className="chat-main">
        <div className="chat-status">
          {statusError ? (
            <ErrorBanner message={statusError} onRetry={() => void reloadStatus()} />
          ) : status == null ? (
            <Loading label="Damon 상태 확인 중…" />
          ) : status.running ? (
            <span className="badge badge-ok">Damon 실행 중{status.version ? " · v" + status.version : ""}</span>
          ) : (
            <span className="badge badge-err">Damon 데몬 미실행</span>
          )}
          {(status?.backends ?? []).length > 0 ? (
            <span className="backends">
              {(status?.backends ?? []).map((b) => (
                <span
                  key={b.id}
                  className={"chip" + (b.available ? " chip-on" : "")}
                  title={b.available ? "사용 가능" : "사용 불가"}
                >
                  {b.id}
                </span>
              ))}
            </span>
          ) : null}
          {!statusError && status != null && !status.running ? (
            <span className="dim">데몬을 시작한 뒤 새로고침하세요.</span>
          ) : null}
        </div>

        <div className="messages">
          {entries.length === 0 ? (
            <EmptyState
              title="메시지 없음"
              hint="세션을 선택하고 프롬프트를 입력해 Damon 에이전트와 대화하세요."
              icon={<Bot size={22} />}
            />
          ) : null}
          {entries.map((entry) => {
            if (entry.role === "user") {
              return (
                <div key={entry.id} className="msg msg-user">
                  {entry.text}
                </div>
              );
            }
            if (entry.role === "assistant") {
              return (
                <div key={entry.id} className="msg msg-assistant md">
                  <Markdown remarkPlugins={[remarkGfm]}>{entry.text}</Markdown>
                  {entry.streaming ? <span className="cursor">▍</span> : null}
                </div>
              );
            }
            if (entry.role === "reasoning") {
              return (
                <details key={entry.id} className="reasoning">
                  <summary>추론 과정</summary>
                  <pre>{entry.text}</pre>
                </details>
              );
            }
            if (entry.role === "tool") {
              return (
                <details key={entry.id} className="tool-event">
                  <summary>
                    <Wrench size={12} /> {entry.meta || "tool"}
                  </summary>
                  <pre>{entry.text}</pre>
                </details>
              );
            }
            return (
              <div key={entry.id} className="msg msg-system">
                {entry.text}
              </div>
            );
          })}
          <div ref={bottomRef} />
        </div>

        {permissions.map((p) => (
          <div key={p.requestId} className="permission-card">
            <div className="permission-text">
              <Zap size={14} />
              <span>
                권한 요청{p.tool ? ": " + p.tool : ""}
                {p.summary ? " — " + p.summary : ""}
              </span>
            </div>
            <div className="permission-actions">
              <button className="btn btn-primary btn-xs" onClick={() => void respond(p.requestId, "allow")}>
                허용
              </button>
              <button className="btn btn-xs" onClick={() => void respond(p.requestId, "always")}>
                항상 허용
              </button>
              <button className="btn btn-danger btn-xs" onClick={() => void respond(p.requestId, "deny")}>
                거절
              </button>
            </div>
          </div>
        ))}

        {error ? (
          <div style={{ padding: "0 16px 8px" }}>
            <ErrorBanner message={error} />
          </div>
        ) : null}

        <div className="composer">
          <textarea
            className="composer-input"
            rows={2}
            value={input}
            disabled={sessionId == null}
            placeholder={sessionId ? "메시지 입력… (Enter 전송 · Shift+Enter 줄바꿈)" : "세션을 만들거나 선택하세요"}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                void handleSend();
              }
            }}
          />
          {streaming ? (
            <button className="icon-btn composer-cancel" title="턴 취소" onClick={() => void handleCancel()}>
              <Square size={14} />
            </button>
          ) : (
            <button
              className="icon-btn composer-send"
              title="전송"
              disabled={sessionId == null || !input.trim()}
              onClick={() => void handleSend()}
            >
              <Send size={14} />
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
