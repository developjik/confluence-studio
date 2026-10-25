import { useEffect, useState } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { markdown as markdownLang, markdownLanguage } from "@codemirror/lang-markdown";
import { languages } from "@codemirror/language-data";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ExternalLink, Eye, FileText, Pencil, Save, Trash2 } from "lucide-react";
import { api } from "../api";
import type { PageDetail } from "../types";
import { EmptyState, ErrorBanner, Loading, formatTime } from "./common";
import { Modal } from "./Modal";

export interface CreateTarget {
  spaceId: string;
  parentId: string | null;
  parentTitle: string | null;
}

interface PageEditorProps {
  mode: "view" | "create";
  page: PageDetail | null;
  createTarget: CreateTarget | null;
  loading: boolean;
  loadError: string | null;
  onRetry(): void;
  onSaved(page: PageDetail): void;
  onDeleted(pageId: string): void;
  onCancelCreate(): void;
}

export function PageEditor(props: PageEditorProps) {
  const { mode, page, createTarget, loading, loadError, onRetry, onSaved, onDeleted, onCancelCreate } = props;

  const [title, setTitle] = useState("");
  const [markdown, setMarkdown] = useState("");
  const [preview, setPreview] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);

  // Reset local editing state whenever the editing target changes.
  const identity =
    mode === "create"
      ? "create:" + (createTarget ? createTarget.parentId ?? "root" : "none")
      : "view:" + (page ? page.id : "none");

  useEffect(() => {
    if (mode === "view") {
      setTitle(page ? page.title : "");
      setMarkdown(page ? page.markdown : "");
    } else {
      setTitle("");
      setMarkdown("");
    }
    setError(null);
    setConfirmDelete(false);
    setPreview(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity]);

  const dirty =
    mode === "view"
      ? page != null && (title !== page.title || markdown !== page.markdown)
      : title.trim() !== "" || markdown !== "";

  if (mode === "view" && loading) {
    return (
      <div className="panel">
        <Loading label="페이지를 불러오는 중…" />
      </div>
    );
  }
  if (mode === "view" && loadError) {
    return (
      <div className="panel">
        <div className="panel-body-pad" style={{ paddingTop: 14 }}>
          <ErrorBanner message={loadError} onRetry={onRetry} />
        </div>
        <EmptyState
          title="페이지를 불러올 수 없습니다"
          hint="연결 설정(설정 뷰)과 페이지 접근 권한을 확인하세요."
          icon={<FileText size={22} />}
        />
      </div>
    );
  }
  if (mode === "view" && !page) {
    return (
      <div className="panel">
        <EmptyState
          title="페이지를 선택하세요"
          hint="왼쪽 사이드바에서 스페이스를 선택한 뒤 페이지를 고르세요. 새 페이지는 사이드바의 + 버튼으로 만들 수 있습니다."
          icon={<FileText size={22} />}
        />
      </div>
    );
  }
  if (mode === "create" && !createTarget) {
    return (
      <div className="panel">
        <EmptyState title="새 페이지" hint="먼저 사이드바에서 스페이스를 선택하세요." />
      </div>
    );
  }

  async function handleSave() {
    if (!title.trim()) {
      setError("제목을 입력하세요.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const saved =
        mode === "create" && createTarget
          ? await api.createPage({
              spaceId: createTarget.spaceId,
              parentId: createTarget.parentId,
              title: title.trim(),
              markdown,
            })
          : await api.updatePage({
              pageId: page ? page.id : "",
              title: title.trim(),
              markdown,
              version: page ? page.version : 0,
            });
      onSaved(saved);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleDelete() {
    if (!page) return;
    setSaving(true);
    setError(null);
    try {
      await api.deletePage(page.id);
      onDeleted(page.id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
      setConfirmDelete(false);
    }
  }

  return (
    <div className="panel">
      <div className="editor-header">
        <input
          className="title-input"
          value={title}
          placeholder="제목"
          onChange={(e) => setTitle(e.target.value)}
        />
        <div className="editor-actions">
          <button
            className={"icon-btn" + (preview ? " on" : "")}
            title={preview ? "편집 모드" : "미리보기"}
            onClick={() => setPreview((p) => !p)}
          >
            {preview ? <Pencil size={14} /> : <Eye size={14} />}
          </button>
          {mode === "view" && page && page.webUrl ? (
            <a
              className="icon-btn"
              href={page.webUrl}
              target="_blank"
              rel="noreferrer"
              title="Confluence에서 열기"
            >
              <ExternalLink size={14} />
            </a>
          ) : null}
          {mode === "create" ? (
            <button className="btn btn-ghost btn-xs" onClick={onCancelCreate}>
              취소
            </button>
          ) : null}
          {mode === "view" && page ? (
            <button
              className="icon-btn icon-btn-danger"
              title="페이지 삭제"
              onClick={() => setConfirmDelete(true)}
            >
              <Trash2 size={14} />
            </button>
          ) : null}
          <button
            className="btn btn-primary btn-xs"
            disabled={saving || (mode === "view" && !dirty)}
            onClick={() => void handleSave()}
          >
            <Save size={13} />
            {saving ? "저장 중…" : mode === "create" ? "생성" : "저장"}
          </button>
        </div>
      </div>

      {mode === "create" && createTarget ? (
        <div className="editor-context">
          새 페이지 · 위치: {createTarget.parentTitle ?? "스페이스 루트"}
        </div>
      ) : null}

      {error ? <div className="panel-body-pad"><ErrorBanner message={error} /></div> : null}

      <div className="editor-body">
        {preview ? (
          <div className="md-preview md">
            <Markdown remarkPlugins={[remarkGfm]}>{markdown || "_내용이 없습니다._"}</Markdown>
          </div>
        ) : (
          <div className="editor-cm">
            <CodeMirror
              value={markdown}
              height="100%"
              theme="dark"
              extensions={[markdownLang({ base: markdownLanguage, codeLanguages: languages })]}
              onChange={setMarkdown}
              placeholder="마크다운으로 입력하세요…"
            />
          </div>
        )}
      </div>

      <div className="editor-statusbar">
        {mode === "view" && page ? (
          <>
            <span className="badge badge-dim">v{page.version}</span>
            <span className={"badge" + (dirty ? " badge-accent" : " badge-ok")}>
              {dirty ? "수정됨" : "저장됨"}
            </span>
            <span className="dim">{page.status}</span>
            {page.updatedAt ? <span className="dim">갱신 {formatTime(page.updatedAt)}</span> : null}
          </>
        ) : (
          <span className="badge badge-accent">새 페이지</span>
        )}
        <span className="dim mono">{markdown.length}자</span>
      </div>

      {confirmDelete && page ? (
        <Modal
          title="페이지 삭제"
          danger
          onClose={() => setConfirmDelete(false)}
          footer={
            <>
              <button className="btn" onClick={() => setConfirmDelete(false)}>
                취소
              </button>
              <button className="btn btn-danger" disabled={saving} onClick={() => void handleDelete()}>
                삭제
              </button>
            </>
          }
        >
          <p>"{page.title}" 페이지를 삭제할까요? Confluence 휴지통 정책에 따라 복구가 제한될 수 있습니다.</p>
        </Modal>
      ) : null}
    </div>
  );
}
