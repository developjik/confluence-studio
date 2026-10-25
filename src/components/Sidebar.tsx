import { useCallback, useEffect, useRef, useState } from "react";
import { Bot, Briefcase, FileText, LayoutGrid, Plus, RefreshCw, Settings as SettingsIcon } from "lucide-react";
import type { PageTreeNode, SpaceSummary } from "../types";
import { ErrorBanner, Loading } from "./common";
import { PageTree } from "./PageTree";

export type View = "pages" | "jobs" | "chat" | "settings";

interface SidebarProps {
  view: View;
  onViewChange(v: View): void;
  spaces: SpaceSummary[];
  spacesLoading: boolean;
  spacesError: string | null;
  onRefreshSpaces(): void;
  currentSpaceId: string | null;
  tree: PageTreeNode | null;
  treeLoading: boolean;
  treeError: string | null;
  onRetryTree(): void;
  onSelectSpace(spaceId: string | null): void;
  selectedPageId: string | null;
  onSelectPage(pageId: string): void;
  onNewPage(parentId: string | null): void;
}

export function Sidebar(props: SidebarProps) {
  const {
    view, onViewChange, spaces, spacesLoading, spacesError, onRefreshSpaces,
    currentSpaceId, tree, treeLoading, treeError, onRetryTree, onSelectSpace,
    selectedPageId, onSelectPage, onNewPage,
  } = props;

  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const initForRef = useRef<string>("");

  // When a different tree arrives (space switch), expand its first level once.
  useEffect(() => {
    if (!tree) {
      initForRef.current = "";
      return;
    }
    if (initForRef.current !== tree.id) {
      initForRef.current = tree.id;
      setExpanded(new Set(tree.children.map((c) => c.id)));
    }
  }, [tree]);

  const toggle = useCallback((id: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const currentSpace = spaces.find((s) => s.id === currentSpaceId) ?? null;

  return (
    <aside className="sidebar">
      <div className="brand">
        <span className="brand-mark"><LayoutGrid size={15} /></span>
        <span className="brand-name">Confluence Studio</span>
      </div>

      <nav className="nav">
        <button className={"nav-btn" + (view === "pages" ? " active" : "")} onClick={() => onViewChange("pages")}>
          <FileText size={14} /> 페이지
        </button>
        <button className={"nav-btn" + (view === "jobs" ? " active" : "")} onClick={() => onViewChange("jobs")}>
          <Briefcase size={14} /> 잡
        </button>
        <button className={"nav-btn" + (view === "chat" ? " active" : "")} onClick={() => onViewChange("chat")}>
          <Bot size={14} /> 채팅
        </button>
        <button className={"nav-btn" + (view === "settings" ? " active" : "")} onClick={() => onViewChange("settings")}>
          <SettingsIcon size={14} /> 설정
        </button>
      </nav>

      <div className="side-section">
        <div className="side-head">
          <span>스페이스</span>
          <button className="icon-btn" title="새로고침" onClick={onRefreshSpaces}>
            <RefreshCw size={12} />
          </button>
        </div>
        {spacesError ? <ErrorBanner message={spacesError} onRetry={onRefreshSpaces} /> : null}
        {spacesLoading ? (
          <div className="skeleton-stack">
            {[0, 1, 2].map((i) => (
              <div key={i} className="skeleton" />
            ))}
          </div>
        ) : spaces.length === 0 && !spacesError ? (
          <p className="dim-note">스페이스가 없습니다. 설정에서 연결을 확인하세요.</p>
        ) : (
          <div className="space-list">
            {spaces.map((s) => (
              <button
                key={s.id}
                className={"space-item" + (currentSpaceId === s.id ? " selected" : "")}
                onClick={() => onSelectSpace(currentSpaceId === s.id ? null : s.id)}
                title={s.name}
              >
                <span className="space-key mono">{s.key}</span>
                <span className="space-name">{s.name}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      {currentSpaceId ? (
        <div className="side-section side-tree">
          <div className="side-head">
            <span>페이지{currentSpace ? " · " + currentSpace.key : ""}</span>
            <button className="icon-btn" title="루트에 새 페이지" onClick={() => onNewPage(null)}>
              <Plus size={12} />
            </button>
          </div>
          {treeError ? <ErrorBanner message={treeError} onRetry={onRetryTree} /> : null}
          {treeLoading ? (
            <Loading label="트리 불러오는 중…" />
          ) : tree ? (
            <PageTree
              node={tree}
              depth={0}
              selectedId={selectedPageId}
              expanded={expanded}
              onToggle={toggle}
              onSelect={onSelectPage}
              onAddChild={onNewPage}
            />
          ) : !treeError ? (
            <p className="dim-note">트리를 불러오지 못했습니다.</p>
          ) : null}
        </div>
      ) : null}
    </aside>
  );
}
