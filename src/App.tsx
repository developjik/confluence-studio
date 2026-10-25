import { useCallback, useEffect, useRef, useState } from "react";
import { Settings as SettingsIcon } from "lucide-react";
import { api, EVENTS, onEvent } from "./api";
import type { ConfluenceChangedEvent, PageDetail, PageTreeNode, SpaceSummary } from "./types";
import { ErrorBoundary } from "./components/common";
import { Sidebar, type View } from "./components/Sidebar";
import { PageEditor, type CreateTarget } from "./components/PageEditor";
import { JobsPanel } from "./components/JobsPanel";
import { ChatPanel } from "./components/ChatPanel";
import { SettingsPanel } from "./components/SettingsPanel";

function findTreeNode(node: PageTreeNode | null, id: string): PageTreeNode | null {
  if (!node) return null;
  if (node.id === id) return node;
  for (const child of node.children) {
    const hit = findTreeNode(child, id);
    if (hit) return hit;
  }
  return null;
}

export default function App() {
  const [view, setView] = useState<View>("pages");
  const [spaces, setSpaces] = useState<SpaceSummary[]>([]);
  const [spacesLoading, setSpacesLoading] = useState(false);
  const [spacesError, setSpacesError] = useState<string | null>(null);
  const [currentSpaceId, setCurrentSpaceId] = useState<string | null>(null);
  const [tree, setTree] = useState<PageTreeNode | null>(null);
  const [treeLoading, setTreeLoading] = useState(false);
  const [treeError, setTreeError] = useState<string | null>(null);
  const [selectedPage, setSelectedPage] = useState<PageDetail | null>(null);
  const [pageLoading, setPageLoading] = useState(false);
  const [pageError, setPageError] = useState<string | null>(null);
  const [editorMode, setEditorMode] = useState<"view" | "create">("view");
  const [createTarget, setCreateTarget] = useState<CreateTarget | null>(null);
  const [hasConnection, setHasConnection] = useState<boolean | null>(null);

  const currentSpaceRef = useRef<string | null>(null);
  const selectedPageIdRef = useRef<string | null>(null);
  const lastPageIdRef = useRef<string | null>(null);
  useEffect(() => {
    currentSpaceRef.current = currentSpaceId;
  }, [currentSpaceId]);

  const loadSpaces = useCallback(async () => {
    setSpacesLoading(true);
    setSpacesError(null);
    try {
      setSpaces(await api.listSpaces());
    } catch (e) {
      setSpacesError(e instanceof Error ? e.message : String(e));
    } finally {
      setSpacesLoading(false);
    }
  }, []);

  const loadTree = useCallback(async (spaceId: string) => {
    setTreeLoading(true);
    setTreeError(null);
    try {
      setTree(await api.getPageTree(spaceId));
    } catch (e) {
      setTree(null);
      setTreeError(e instanceof Error ? e.message : String(e));
    } finally {
      setTreeLoading(false);
    }
  }, []);

  useEffect(() => {
    api
      .getConnection()
      .then((c) => setHasConnection(c != null))
      .catch(() => setHasConnection(false));
    void loadSpaces();
  }, [loadSpaces]);

  const selectPage = useCallback(async (pageId: string) => {
    setEditorMode("view");
    setCreateTarget(null);
    lastPageIdRef.current = pageId;
    selectedPageIdRef.current = pageId;
    setPageLoading(true);
    setPageError(null);
    setSelectedPage(null);
    try {
      const page = await api.getPage(pageId);
      setSelectedPage(page);
      selectedPageIdRef.current = page.id;
    } catch (e) {
      setPageError(e instanceof Error ? e.message : String(e));
    } finally {
      setPageLoading(false);
    }
  }, []);

  // Refresh everything relevant when the backend reports Confluence changes.
  useEffect(() => {
    const cleanups: Array<() => void> = [];
    void (async () => {
      cleanups.push(
        await onEvent<ConfluenceChangedEvent>(EVENTS.confluenceChanged, (ev) => {
          void loadSpaces();
          const sid = currentSpaceRef.current;
          if (sid) void loadTree(sid);
          if (ev.kind === "deleted" && ev.pageId && selectedPageIdRef.current === ev.pageId) {
            setSelectedPage(null);
            setEditorMode("view");
            setCreateTarget(null);
          }
        }),
      );
    })();
    return () => {
      cleanups.forEach((fn) => fn());
    };
  }, [loadSpaces, loadTree]);

  const handleSelectSpace = useCallback(
    (spaceId: string | null) => {
      setCurrentSpaceId(spaceId);
      setSelectedPage(null);
      setEditorMode("view");
      setCreateTarget(null);
      selectedPageIdRef.current = null;
      setTree(null);
      setTreeError(null);
      setTreeLoading(false);
      if (spaceId) void loadTree(spaceId);
    },
    [loadTree],
  );

  const handleNewPage = useCallback(
    (parentId: string | null) => {
      if (!currentSpaceId) return;
      const parent = parentId ? findTreeNode(tree, parentId) : null;
      setEditorMode("create");
      setCreateTarget({ spaceId: currentSpaceId, parentId, parentTitle: parent ? parent.title : null });
      setSelectedPage(null);
    },
    [currentSpaceId, tree],
  );

  const handleSaved = useCallback(
    (page: PageDetail) => {
      setSelectedPage(page);
      setEditorMode("view");
      setCreateTarget(null);
      selectedPageIdRef.current = page.id;
      const sid = currentSpaceRef.current;
      if (sid) void loadTree(sid);
    },
    [loadTree],
  );

  const handleDeleted = useCallback(() => {
    setSelectedPage(null);
    setEditorMode("view");
    setCreateTarget(null);
    selectedPageIdRef.current = null;
    const sid = currentSpaceRef.current;
    if (sid) void loadTree(sid);
  }, [loadTree]);

  const handleConnectionChanged = useCallback(() => {
    api
      .getConnection()
      .then((c) => setHasConnection(c != null))
      .catch(() => setHasConnection(null));
    void loadSpaces();
  }, [loadSpaces]);

  return (
    <ErrorBoundary>
      <div className="app">
        <Sidebar
          view={view}
          onViewChange={setView}
          spaces={spaces}
          spacesLoading={spacesLoading}
          spacesError={spacesError}
          onRefreshSpaces={() => void loadSpaces()}
          currentSpaceId={currentSpaceId}
          tree={tree}
          treeLoading={treeLoading}
          treeError={treeError}
          onRetryTree={() => {
            const sid = currentSpaceRef.current;
            if (sid) void loadTree(sid);
          }}
          onSelectSpace={handleSelectSpace}
          selectedPageId={editorMode === "view" ? selectedPage?.id ?? null : null}
          onSelectPage={(pid) => void selectPage(pid)}
          onNewPage={handleNewPage}
        />

        <main className="main">
          {view === "pages" ? (
            hasConnection === false ? (
              <div className="no-connection">
                <h2>Confluence 연결이 필요합니다</h2>
                <p>스페이스와 페이지를 불러오려면 먼저 연결 정보를 설정하세요.</p>
                <button className="btn btn-primary" onClick={() => setView("settings")}>
                  <SettingsIcon size={14} /> 설정 열기
                </button>
              </div>
            ) : (
              <PageEditor
                mode={editorMode}
                page={selectedPage}
                createTarget={createTarget}
                loading={pageLoading}
                loadError={pageError}
                onRetry={() => {
                  const pid = lastPageIdRef.current;
                  if (pid) void selectPage(pid);
                }}
                onSaved={handleSaved}
                onDeleted={handleDeleted}
                onCancelCreate={() => {
                  setEditorMode("view");
                  setCreateTarget(null);
                }}
              />
            )
          ) : null}
          {view === "jobs" ? <JobsPanel spaces={spaces} /> : null}
          {view === "chat" ? <ChatPanel /> : null}
          {view === "settings" ? <SettingsPanel onConnectionChanged={handleConnectionChanged} /> : null}
        </main>
      </div>
    </ErrorBoundary>
  );
}
