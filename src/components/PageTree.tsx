import { ChevronDown, ChevronRight, FilePlus2 } from "lucide-react";
import type { PageTreeNode } from "../types";

interface PageTreeProps {
  node: PageTreeNode;
  depth: number;
  selectedId: string | null;
  expanded: ReadonlySet<string>;
  onToggle(id: string): void;
  onSelect(id: string): void;
  onAddChild(parentId: string): void;
}

export function PageTree(props: PageTreeProps) {
  const { node, depth, selectedId, expanded, onToggle, onSelect, onAddChild } = props;
  const hasChildren = node.children.length > 0;
  const isOpen = expanded.has(node.id);
  return (
    <div>
      <div className={"tree-row" + (selectedId === node.id ? " selected" : "")} style={{ paddingLeft: 4 + depth * 14 }}>
        <button
          className="tree-chevron"
          onClick={() => onToggle(node.id)}
          disabled={!hasChildren}
          aria-label={isOpen ? "접기" : "펼치기"}
        >
          {hasChildren ? (isOpen ? <ChevronDown size={13} /> : <ChevronRight size={13} />) : <span className="tree-dot" />}
        </button>
        <button className="tree-label" onClick={() => onSelect(node.id)} title={node.title}>
          <span className={"tree-status" + (node.status === "current" ? " ok" : "")} />
          <span className="tree-title">{node.title}</span>
        </button>
        <button
          className="icon-btn tree-add"
          onClick={() => onAddChild(node.id)}
          aria-label="하위 페이지 추가"
          title="하위 페이지 추가"
        >
          <FilePlus2 size={12} />
        </button>
      </div>
      {isOpen &&
        node.children.map((child) => (
          <PageTree key={child.id} {...props} node={child} depth={depth + 1} />
        ))}
    </div>
  );
}
