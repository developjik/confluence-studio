import { Component, type ErrorInfo, type ReactNode } from "react";
import { AlertTriangle, Inbox, Loader2 } from "lucide-react";

let uidCounter = 0;

/** Small unique id generator (crypto.randomUUID when available). */
export function uid(): string {
  if (typeof crypto !== "undefined" && typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  uidCounter += 1;
  return "id-" + Date.now().toString(36) + "-" + uidCounter;
}

/** Normalize an ISO timestamp for display; falls back to the raw string. */
export function formatTime(iso: string | null | undefined): string {
  if (!iso) return "-";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString();
}

export function ErrorBanner({
  message,
  onRetry,
  retryLabel = "다시 시도",
}: {
  message: string;
  onRetry?: () => void;
  retryLabel?: string;
}) {
  return (
    <div className="banner error" role="alert">
      <AlertTriangle size={15} />
      <span className="banner-message">{message}</span>
      {onRetry && (
        <button className="btn btn-ghost btn-xs" onClick={onRetry}>
          {retryLabel}
        </button>
      )}
    </div>
  );
}

export function Loading({ label = "불러오는 중…" }: { label?: string }) {
  return (
    <div className="loading">
      <Loader2 size={14} className="spin" />
      {label}
    </div>
  );
}

export function EmptyState({
  icon,
  title,
  hint,
}: {
  icon?: ReactNode;
  title: string;
  hint?: string;
}) {
  return (
    <div className="empty-state">
      <div className="empty-icon">{icon ?? <Inbox size={22} />}</div>
      <div className="empty-title">{title}</div>
      {hint ? <div className="empty-hint">{hint}</div> : null}
    </div>
  );
}

interface ErrorBoundaryState {
  error: Error | null;
}

/** Last-resort guard so one broken panel never blanks the whole window. */
export class ErrorBoundary extends Component<{ children: ReactNode }, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[ErrorBoundary]", error, info);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="boundary-fallback">
          <AlertTriangle size={22} />
          <p>화면을 표시하는 중 오류가 발생했습니다.</p>
          <pre>{this.state.error.message}</pre>
          <button className="btn" onClick={() => this.setState({ error: null })}>
            다시 시도
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
