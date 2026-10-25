import { useCallback, useEffect, useRef, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Pencil, Play, Plus, RefreshCw, Square, Trash2, X } from "lucide-react";
import { api, EVENTS, onEvent } from "../api";
import type {
  ExportProgressEvent,
  ExportRunFinishedEvent,
  Job,
  JobInput,
  RunRecord,
  Schedule,
  ScheduleKind,
  SpaceSummary,
} from "../types";
import { EmptyState, ErrorBanner, Loading, formatTime } from "./common";
import { Modal } from "./Modal";

const WEEKDAYS: Array<{ n: number; label: string }> = [
  { n: 1, label: "월" },
  { n: 2, label: "화" },
  { n: 3, label: "수" },
  { n: 4, label: "목" },
  { n: 5, label: "금" },
  { n: 6, label: "토" },
  { n: 7, label: "일" },
];

const SCHEDULE_KINDS: ScheduleKind[] = ["Daily", "MultipleDaily", "Weekdays", "Weekly", "Monthly"];

const KIND_LABEL: Record<ScheduleKind, string> = {
  Daily: "매일",
  MultipleDaily: "하루 여러 번",
  Weekdays: "평일",
  Weekly: "주간",
  Monthly: "매월",
};

function defaultJobInput(spaces: SpaceSummary[]): JobInput {
  return {
    name: "",
    spaceId: spaces.length > 0 ? spaces[0].id : "",
    rootPageId: null,
    targetDir: "",
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
      enabled: false,
      kind: "Daily",
      times: ["09:00"],
      daysOfWeek: [1, 2, 3, 4, 5],
      dayOfMonth: null,
      timezone: "Asia/Seoul",
      paused: false,
    },
  };
}

function cloneJob(job: Job): JobInput {
  return JSON.parse(JSON.stringify(job)) as JobInput;
}

function describeSchedule(job: Job): string {
  const s = job.schedule;
  if (s.paused) return "일시정지";
  if (!s.enabled) return "수동 전용";
  let when = KIND_LABEL[s.kind];
  if (s.kind === "Weekly") {
    const labels =
      WEEKDAYS.filter((w) => s.daysOfWeek.includes(w.n))
        .map((w) => w.label)
        .join("") || "?";
    when += " " + labels;
  }
  if (s.kind === "Monthly" && s.dayOfMonth) when += " " + s.dayOfMonth + "일";
  return when + " · " + s.times.join(", ") + " · " + s.timezone;
}

export function JobsPanel({ spaces }: { spaces: SpaceSummary[] }) {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);
  const [form, setForm] = useState<JobInput | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<Job | null>(null);
  const [runs, setRuns] = useState<RunRecord[]>([]);
  const [runsLoading, setRunsLoading] = useState(false);
  const [runsError, setRunsError] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, ExportProgressEvent>>({});
  const [finished, setFinished] = useState<Record<string, ExportRunFinishedEvent>>({});
  const [runningJobs, setRunningJobs] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string | null>(null);

  const selectedJobIdRef = useRef<string | null>(null);
  useEffect(() => {
    selectedJobIdRef.current = selectedJobId;
  }, [selectedJobId]);

  const spaceName = (id: string): string => spaces.find((s) => s.id === id)?.name ?? id;

  const loadJobs = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setJobs(await api.listJobs());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  const loadRuns = useCallback(async (jobId: string) => {
    setRunsLoading(true);
    setRunsError(null);
    try {
      setRuns(await api.listJobRuns(jobId, 20));
    } catch (e) {
      setRunsError(e instanceof Error ? e.message : String(e));
    } finally {
      setRunsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadJobs();
  }, [loadJobs]);

  useEffect(() => {
    if (selectedJobId) void loadRuns(selectedJobId);
    else setRuns([]);
  }, [selectedJobId, loadRuns]);

  useEffect(() => {
    const cleanups: Array<() => void> = [];
    void (async () => {
      cleanups.push(
        await onEvent<ExportProgressEvent>(EVENTS.exportProgress, (ev) => {
          setProgress((prev) => ({ ...prev, [ev.jobId]: ev }));
        }),
      );
      cleanups.push(
        await onEvent<ExportRunFinishedEvent>(EVENTS.exportRunFinished, (ev) => {
          setFinished((prev) => ({ ...prev, [ev.jobId]: ev }));
          setRunningJobs((prev) => {
            const next = new Set(prev);
            next.delete(ev.jobId);
            return next;
          });
          setProgress((prev) => {
            const next = { ...prev };
            delete next[ev.jobId];
            return next;
          });
          if (selectedJobIdRef.current === ev.jobId) void loadRuns(ev.jobId);
        }),
      );
    })();
    return () => {
      cleanups.forEach((fn) => fn());
    };
  }, [loadRuns]);

  const selectedJob = jobs.find((j) => j.id === selectedJobId) ?? null;

  async function handleRun(job: Job) {
    setActionError(null);
    setRunningJobs((prev) => new Set(prev).add(job.id));
    try {
      await api.runJob(job.id);
    } catch (e) {
      setRunningJobs((prev) => {
        const next = new Set(prev);
        next.delete(job.id);
        return next;
      });
      setActionError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handleCancelRun(job: Job) {
    try {
      const res = await api.cancelJobRun(job.id);
      if (res.cancelled) {
        setRunningJobs((prev) => {
          const next = new Set(prev);
          next.delete(job.id);
          return next;
        });
      }
    } catch (e) {
      setActionError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handleSave() {
    if (!form) return;
    if (!form.name.trim()) {
      setFormError("잡 이름을 입력하세요.");
      return;
    }
    if (!form.spaceId) {
      setFormError("대상 스페이스를 선택하세요.");
      return;
    }
    if (!form.targetDir.trim()) {
      setFormError("내보낼 폴더를 지정하세요.");
      return;
    }
    setSaving(true);
    setFormError(null);
    try {
      const saved = await api.saveJob(form);
      setForm(null);
      setSelectedJobId(saved.id);
      await loadJobs();
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete(job: Job) {
    try {
      await api.deleteJob(job.id);
      setConfirmDelete(null);
      if (selectedJobId === job.id) setSelectedJobId(null);
      await loadJobs();
    } catch (e) {
      setConfirmDelete(null);
      setActionError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handlePickDir() {
    if (!form) return;
    try {
      const dir = await openDialog({ directory: true, title: "내보낼 폴더 선택" });
      if (typeof dir === "string" && dir) setForm({ ...form, targetDir: dir });
    } catch (e) {
      setFormError("폴더 선택 실패: " + (e instanceof Error ? e.message : String(e)));
    }
  }

  function patchSchedule(patch: Partial<Schedule>) {
    if (!form) return;
    setForm({ ...form, schedule: { ...form.schedule, ...patch } });
  }

  function renderTimesEditor(f: JobInput) {
    return (
      <div className="times-editor">
        {f.schedule.times.map((t, i) => (
          <div key={i} className="time-row">
            <input
              type="time"
              className="input"
              style={{ width: 110 }}
              value={t}
              onChange={(e) => {
                const times = f.schedule.times.slice();
                times[i] = e.target.value || "00:00";
                patchSchedule({ times });
              }}
            />
            {f.schedule.times.length > 1 ? (
              <button
                className="icon-btn"
                aria-label="시간 삭제"
                onClick={() => patchSchedule({ times: f.schedule.times.filter((_, idx) => idx !== i) })}
              >
                <X size={13} />
              </button>
            ) : null}
          </div>
        ))}
        <button
          className="btn btn-ghost btn-xs"
          onClick={() => patchSchedule({ times: [...f.schedule.times, "12:00"] })}
        >
          <Plus size={12} /> 시간 추가
        </button>
      </div>
    );
  }

  function renderForm() {
    if (!form) return null;
    const f = form;
    return (
      <div className="job-form">
        <div className="job-form-head">
          <h3>{f.id ? "잡 편집" : "새 잡"}</h3>
          <button className="icon-btn" onClick={() => setForm(null)} aria-label="닫기">
            <X size={14} />
          </button>
        </div>
        {formError ? <ErrorBanner message={formError} /> : null}

        <div className="form-grid">
          <div className="field span-2">
            <label className="field-label">이름</label>
            <input
              className="input"
              value={f.name}
              onChange={(e) => setForm({ ...f, name: e.target.value })}
              placeholder="예: 팀 위키 주간 백업"
            />
          </div>
          <div className="field">
            <label className="field-label">스페이스</label>
            <select className="input" value={f.spaceId} onChange={(e) => setForm({ ...f, spaceId: e.target.value })}>
              <option value="">선택…</option>
              {spaces.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name} ({s.key})
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label className="field-label">루트 페이지 ID (선택)</label>
            <input
              className="input"
              value={f.rootPageId ?? ""}
              onChange={(e) => setForm({ ...f, rootPageId: e.target.value || null })}
              placeholder="비워두면 스페이스 전체"
            />
          </div>
          <div className="field span-2">
            <label className="field-label">내보낼 폴더</label>
            <div className="row">
              <input
                className="input"
                value={f.targetDir}
                onChange={(e) => setForm({ ...f, targetDir: e.target.value })}
                placeholder="/Users/you/Exports/wiki"
              />
              <button className="btn" onClick={() => void handlePickDir()}>
                <FolderOpen size={13} /> 선택
              </button>
            </div>
          </div>
        </div>

        <div className="form-section">내보내기 옵션</div>
        <div className="check-grid">
          <label className="check-row">
            <input type="checkbox" checked={f.includeAttachments} onChange={(e) => setForm({ ...f, includeAttachments: e.target.checked })} />
            첨부 파일 포함
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={f.exportOptions.skipUnchanged}
              onChange={(e) => setForm({ ...f, exportOptions: { ...f.exportOptions, skipUnchanged: e.target.checked } })}
            />
            변경 없는 페이지 건너뛰기
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={f.exportOptions.cleanupStale}
              onChange={(e) => setForm({ ...f, exportOptions: { ...f.exportOptions, cleanupStale: e.target.checked } })}
            />
            원격에서 삭제된 파일 정리
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={f.exportOptions.pageBreadcrumbs}
              onChange={(e) => setForm({ ...f, exportOptions: { ...f.exportOptions, pageBreadcrumbs: e.target.checked } })}
            />
            페이지 경로 표시
          </label>
          <label className="check-row">
            <input
              type="checkbox"
              checked={f.exportOptions.frontMatter}
              onChange={(e) => setForm({ ...f, exportOptions: { ...f.exportOptions, frontMatter: e.target.checked } })}
            />
            YAML front matter
          </label>
          <div className="check-row">
            <span className="field-label">동시 작업 수</span>
            <input
              className="input input-narrow"
              type="number"
              min={1}
              max={64}
              value={f.exportOptions.maxWorkers}
              onChange={(e) =>
                setForm({ ...f, exportOptions: { ...f.exportOptions, maxWorkers: Math.max(1, Number(e.target.value) || 1) } })
              }
            />
          </div>
        </div>

        <div className="form-section">스케줄</div>
        <div className="check-grid">
          <label className="check-row">
            <input type="checkbox" checked={f.enabled} onChange={(e) => setForm({ ...f, enabled: e.target.checked })} />
            잡 활성화
          </label>
          <label className="check-row">
            <input type="checkbox" checked={f.schedule.enabled} onChange={(e) => patchSchedule({ enabled: e.target.checked })} />
            스케줄 사용
          </label>
          <label className="check-row">
            <input type="checkbox" checked={f.schedule.paused} onChange={(e) => patchSchedule({ paused: e.target.checked })} />
            일시정지
          </label>
        </div>
        <div className="form-grid">
          <div className="field">
            <label className="field-label">반복</label>
            <select
              className="input"
              value={f.schedule.kind}
              onChange={(e) => patchSchedule({ kind: e.target.value as ScheduleKind })}
            >
              {SCHEDULE_KINDS.map((k) => (
                <option key={k} value={k}>
                  {KIND_LABEL[k]}
                </option>
              ))}
            </select>
          </div>
          <div className="field">
            <label className="field-label">시간대 (IANA)</label>
            <input
              className="input"
              value={f.schedule.timezone}
              onChange={(e) => patchSchedule({ timezone: e.target.value })}
              placeholder="Asia/Seoul"
            />
          </div>
        </div>
        <div className="field">
          <label className="field-label">실행 시간</label>
          {renderTimesEditor(f)}
        </div>
        {f.schedule.kind === "Weekly" ? (
          <div className="field">
            <label className="field-label">요일</label>
            <div className="chips">
              {WEEKDAYS.map((w) => (
                <button
                  key={w.n}
                  className={"chip" + (f.schedule.daysOfWeek.includes(w.n) ? " chip-on" : "")}
                  onClick={() =>
                    patchSchedule({
                      daysOfWeek: f.schedule.daysOfWeek.includes(w.n)
                        ? f.schedule.daysOfWeek.filter((d) => d !== w.n)
                        : [...f.schedule.daysOfWeek, w.n].sort((a, b) => a - b),
                    })
                  }
                >
                  {w.label}
                </button>
              ))}
            </div>
          </div>
        ) : null}
        {f.schedule.kind === "Monthly" ? (
          <div className="field">
            <label className="field-label">일 (1-31)</label>
            <input
              className="input input-narrow"
              type="number"
              min={1}
              max={31}
              value={f.schedule.dayOfMonth ?? 1}
              onChange={(e) =>
                patchSchedule({
                  dayOfMonth: e.target.value ? Math.min(31, Math.max(1, Number(e.target.value) || 1)) : null,
                })
              }
            />
          </div>
        ) : null}

        <div className="form-footer">
          <button className="btn" onClick={() => setForm(null)}>
            취소
          </button>
          <button className="btn btn-primary" disabled={saving} onClick={() => void handleSave()}>
            {saving ? "저장 중…" : "저장"}
          </button>
        </div>
      </div>
    );
  }

  function renderDetail(job: Job) {
    return (
      <div className="job-detail">
        <h3>{job.name}</h3>
        <div className="kv">
          <span className="kv-k">스페이스</span>
          <span className="kv-v">{spaceName(job.spaceId)}</span>
          <span className="kv-k">루트 페이지</span>
          <span className="kv-v">{job.rootPageId || "스페이스 전체"}</span>
          <span className="kv-k">폴더</span>
          <span className="kv-v mono">{job.targetDir}</span>
          <span className="kv-k">스케줄</span>
          <span className="kv-v">{describeSchedule(job)}</span>
          <span className="kv-k">첨부</span>
          <span className="kv-v">{job.includeAttachments ? "포함" : "제외"}</span>
          <span className="kv-k">작업자</span>
          <span className="kv-v">{job.exportOptions.maxWorkers}</span>
        </div>

        <div className="form-section">실행 이력</div>
        {runsLoading ? (
          <Loading label="이력 불러오는 중…" />
        ) : runsError ? (
          <ErrorBanner message={runsError} onRetry={() => void loadRuns(job.id)} />
        ) : runs.length === 0 ? (
          <p className="dim-note">실행 이력이 없습니다.</p>
        ) : (
          <table className="table">
            <thead>
              <tr>
                <th>상태</th>
                <th>페이지</th>
                <th>건너뜀</th>
                <th>첨부</th>
                <th>삭제</th>
                <th>종료 시각</th>
                <th>오류</th>
              </tr>
            </thead>
            <tbody>
              {runs.map((r) => (
                <tr key={r.runId}>
                  <td>
                    <span className={"badge" + (r.status === "success" ? " badge-ok" : r.status === "cancelled" ? " badge-dim" : " badge-err")}>
                      {r.status}
                    </span>
                  </td>
                  <td>{r.pagesWritten}</td>
                  <td>{r.pagesSkipped}</td>
                  <td>{r.attachmentsWritten}</td>
                  <td>{r.filesRemoved}</td>
                  <td className="dim">{formatTime(r.finishedAt)}</td>
                  <td className="table-error" title={r.error ?? ""}>
                    {r.error ?? ""}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel-header">
        <h2>Export 잡</h2>
        <div className="panel-actions">
          <button className="btn btn-ghost btn-xs" onClick={() => void loadJobs()}>
            <RefreshCw size={13} /> 새로고침
          </button>
          <button
            className="btn btn-primary btn-xs"
            onClick={() => {
              setForm(defaultJobInput(spaces));
              setFormError(null);
            }}
          >
            <Plus size={13} /> 새 잡
          </button>
        </div>
      </div>

      {error ? (
        <div className="panel-body-pad">
          <ErrorBanner message={error} onRetry={() => void loadJobs()} />
        </div>
      ) : null}
      {actionError ? (
        <div className="panel-body-pad">
          <ErrorBanner message={actionError} />
        </div>
      ) : null}

      <div className="jobs-layout">
        <div className="jobs-list">
          {loading && jobs.length === 0 ? (
            <Loading label="잡 목록 불러오는 중…" />
          ) : jobs.length === 0 && !error ? (
            <EmptyState title="잡이 없습니다" hint="'새 잡' 버튼으로 첫 내보내기 잡을 만들어 보세요." />
          ) : (
            jobs.map((job) => (
              <div
                key={job.id}
                className={"job-item" + (selectedJobId === job.id ? " selected" : "")}
                onClick={() => setSelectedJobId(job.id)}
              >
                <div className="job-item-top">
                  <span className="job-name">{job.name}</span>
                  <span className={"badge" + (job.enabled ? " badge-ok" : " badge-dim")}>
                    {job.enabled ? "활성" : "비활성"}
                  </span>
                </div>
                <div className="job-item-sub">
                  {spaceName(job.spaceId)} · {describeSchedule(job)}
                </div>

                {runningJobs.has(job.id) ? (
                  <div className="job-progress">
                    <span className="job-progress-text">
                      {progress[job.id]
                        ? progress[job.id].message + " (" + progress[job.id].done + "/" + progress[job.id].total + ")"
                        : "실행 준비 중…"}
                    </span>
                    {progress[job.id] && progress[job.id].total > 0 ? (
                      <div className="progress">
                        <div
                          className="progress-fill"
                          style={{
                            width: Math.min(100, Math.round((progress[job.id].done / progress[job.id].total) * 100)) + "%",
                          }}
                        />
                      </div>
                    ) : null}
                  </div>
                ) : null}

                {finished[job.id] && !runningJobs.has(job.id) ? (
                  <div className={"banner" + (finished[job.id].status === "success" ? " success" : " error") + " banner-flat"}>
                    {finished[job.id].status === "success" ? "완료" : "실패"} — {finished[job.id].summary}
                  </div>
                ) : null}

                <div
                  className="job-item-actions"
                  onClick={(e) => e.stopPropagation()}
                >
                  {runningJobs.has(job.id) ? (
                    <button className="icon-btn" title="실행 취소" onClick={() => void handleCancelRun(job)}>
                      <Square size={13} />
                    </button>
                  ) : (
                    <button className="icon-btn" title="지금 실행" onClick={() => void handleRun(job)}>
                      <Play size={13} />
                    </button>
                  )}
                  <button
                    className="icon-btn"
                    title="편집"
                    onClick={() => {
                      setForm(cloneJob(job));
                      setFormError(null);
                    }}
                  >
                    <Pencil size={13} />
                  </button>
                  <button className="icon-btn icon-btn-danger" title="삭제" onClick={() => setConfirmDelete(job)}>
                    <Trash2 size={13} />
                  </button>
                </div>
              </div>
            ))
          )}
        </div>

        <div className="jobs-detail">
          {form
            ? renderForm()
            : selectedJob
              ? renderDetail(selectedJob)
              : (
                  <EmptyState title="잡을 선택하세요" hint="왼쪽 목록에서 잡을 선택하거나 '새 잡'을 만드세요. '지금 실행'으로 즉시 내보낼 수 있습니다." />
                )}
        </div>
      </div>

      {confirmDelete ? (
        <Modal
          title="잡 삭제"
          danger
          onClose={() => setConfirmDelete(null)}
          footer={
            <>
              <button className="btn" onClick={() => setConfirmDelete(null)}>
                취소
              </button>
              <button className="btn btn-danger" onClick={() => void handleDelete(confirmDelete)}>
                삭제
              </button>
            </>
          }
        >
          <p>"{confirmDelete.name}" 잡을 삭제할까요? 이미 내보내진 로컬 파일은 삭제되지 않습니다.</p>
        </Modal>
      ) : null}
    </div>
  );
}
