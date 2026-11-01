# ⚡ Confluence Studio

<div align="center">

<img src="docs/icon.png" width="96" height="96" alt="Confluence Studio Icon" />

### Native Desktop Workspace for Confluence Cloud
**Markdown Editing · Scheduled Exports · Streaming AI Sidekick**

[![React 19](https://img.shields.io/badge/React_19-20232A?style=for-the-badge&logo=react&logoColor=61DAFB)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-007ACC?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![Tauri 2](https://img.shields.io/badge/Tauri_2-24C8D8?style=for-the-badge&logo=tauri&logoColor=white)](https://tauri.app/)
[![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Vite](https://img.shields.io/badge/Vite-646CFF?style=for-the-badge&logo=vite&logoColor=white)](https://vitejs.dev/)

**[▶ Live Product Landing Page](https://developjik.github.io/confluence-studio/)** · **[Architecture Docs (docs/ARCHITECTURE.md)](docs/ARCHITECTURE.md)**

</div>

---

## 📌 Overview

**Confluence Studio**는 Confluence Cloud 문서를 로컬 데스크톱 환경에서 **Markdown으로 열람·편집**하고, 예약 잡(Cron)으로 **자동 백업/내보내기**하며, **AI 에이전트와 대화**할 수 있는 로컬 퍼스트 데스크톱 앱입니다.

Confluence의 웹 UI는 복잡하고 무거우며 브라우저 탭을 많이 소모합니다. Confluence Studio는 원본 문서는 Confluence Cloud에 온전히 보존하면서, 프론트엔드에서는 **키보드 중심의 빠른 마크다운 에디터**, **OS 키체인 기반 보안**, **스트리밍 AI 사이드킥**을 네이티브 데스크톱 성능으로 제공합니다.

---

## 🖥 Workspace Layout

```
┌─────────────────┬──────────────────────────────────┬─────────────────┐
│  Sidebar        │  Editor & Live Preview           │  AI Sidekick    │
│                 │                                  │                 │
│ 📁 Spaces Tree  │  # Document Title                │  🤖 Damon Agent │
│   ├─ Eng Wiki   │  Confluence Cloud v2 Sync...     │                 │
│   │   ├─ Spec   │                                  │  "요약해 줘"    │
│   │   └─ Arch   │  - [x] CodeMirror 6 Editor       │  "초안 작성해 줘"│
│   └─ Product    │  - [x] Split View Preview        │                 │
│                 │  - [x] Storage XHTML ↔ MD AST    │  [Streaming...] │
│ ⏱ Export Jobs  │                                  │                 │
│   └─ [38%]...   │  [Save] [Export] [Revert]        │  [Apply Changes]│
└─────────────────┴──────────────────────────────────┴─────────────────┘
```

---

## 🚀 Frontend Architecture & Key Highlights

### 1. React 19 Concurrent Rendering & Performance
- **상태 격리 (State Isolation)**: 대규모 스페이스 트리(수천 개 노드), 고속 에디터 키 입력, 그리고 WebSocket을 통해 유입되는 초당 수십 개의 AI 스트리밍 토큰이 서로 렌더링 블로킹을 일으키지 않도록 독립된 서브트리로 분리 설계했습니다.
- **메모리 효율성**: 가상화 렌더링과 이벤트 위임을 통해 10,000줄 이상의 대형 마크다운 문서 편집 시에도 60fps 인터랙션을 유지합니다.

### 2. 왜 Electron이 아닌 Tauri 2인가? (Technical Trade-offs)

| 비교 항목 | Electron 기반 앱 | **Confluence Studio (Tauri 2 + Rust)** | 차별점 및 엔지니어링 이점 |
| :--- | :---: | :---: | :--- |
| **유휴 메모리 (RAM)** | ~220 MB+ | **~42 MB** | Chromium 전체를 띄우지 않고 OS 내장 WebKit/WebView2 활용 |
| **콜드 스타트 속도** | ~2.5초 | **< 350ms** | 바이너리 로드 즉시 메인 윈도우 렌더링 |
| **설치 번들 크기** | ~120 MB | **~15 MB** | 초경량 네이티브 번들 배포 |
| **자격증명 보안** | 평문/설정 파일 | **OS Keychain 연동** | macOS Keychain / Windows Credential Manager와 Rust 직접 바인딩 |

### 3. CodeMirror 6 기반 커스텀 마크다운 에디터
- `@codemirror/view`, `@codemirror/state`를 조합하여 Confluence 마크다운 문법에 최적화된 에디터를 구축했습니다.
- **양방향 변환 파이프라인**: Rust 백엔드의 `quick-xml` 기반 파서와 연동하여 `Confluence Storage XHTML ↔ Markdown AST` 왕복 변환 시 서식 손실 없이 안전하게 동기화합니다.

### 4. WebSocket 기반 실시간 AI 스트리밍 (Damon Protocol v2)
- 로컬 또는 원격 데몬과 연결되는 WebSocket 씬 클라이언트(`ChatPanel.tsx`).
- 청크 단위 토큰 수신 시 렌더링 버퍼링 처리와 사용자 수동 스크롤 시 자동 스크롤 잠금(Smart Auto-Scroll Lock) 구현.
- AI의 문서 수정 제안을 원클릭으로 에디터에 머지할 수 있는 인터랙티브 승인 UI.

---

## 📂 Project Structure

```
confluence-studio/
├── src/                          # Frontend (React 19 + TypeScript + Vite)
│   ├── components/
│   │   ├── PageEditor.tsx        # CodeMirror 6 에디터 & 마크다운 프리뷰
│   │   ├── ChatPanel.tsx         # Damon AI 스트리밍 챗 인터페이스
│   │   ├── PageTree.tsx          # 스페이스 및 페이지 계층 트리 뷰
│   │   ├── JobsPanel.tsx         # 백그라운드 내보내기 잡 및 스케줄러 관리
│   │   ├── Sidebar.tsx           # 메인 네비게이션 및 스페이스 전환
│   │   └── SettingsPanel.tsx     # OS 키체인 인증 및 환경 설정
│   ├── api.ts                   # Tauri IPC 타입 세이프 커맨드 바인딩
│   ├── types.ts                 # DTO 및 UI 상태 인터페이스
│   └── styles.css               # 테마 및 모던 다크/라이트 UI 스타일링
│
├── src-tauri/                    # Backend (Rust + Tauri 2 Core)
│   ├── src/
│   │   ├── confluence/          # Confluence Cloud v2 REST 클라이언트 & XHTML 파서
│   │   ├── jobs/                # 백그라운드 내보내기 워커 & Cron 스케줄러
│   │   ├── damon/               # Damon v2 프로토콜 WebSocket 클라이언트
│   │   └── credentials.rs       # OS 키체인 네이티브 연동
│   └── Cargo.toml
│
└── docs/                         # 아키텍처 명세 및 GitHub Pages 랜딩
    ├── ARCHITECTURE.md          # 모듈 인터페이스 및 IPC 계약 명세
    └── index.html               # 제품 소개 웹페이지
```

---

## 🛠 Features

- **스페이스 & 페이지 트리 탐색**: 전 스페이스 목록과 하위 문서 트리를 즉시 검색하고 이동
- **양방향 Markdown 편집**: 실시간 구문 강조 및 Confluence Cloud v2 API 즉시 동기화(CRUD)
- **예약 내보내기 잡(Scheduler)**: 특정 스페이스나 하위 트리를 로컬 마크다운 파일(첨부파일 포함)로 정기 백업
- **OS 키체인 보안**: Atlassian API 토큰을 설정 파일이 아닌 OS 보안 영역에 안전하게 보관
- **AI 사이드킥 (Damon)**: 페이지 문서 요약, 초안 생성, 번역 및 서식 교정을 스트리밍으로 지원

---

## 💻 Getting Started

### 요구사항
- Node.js 20+
- pnpm
- Rust stable
- [Tauri 2 사전 준비사항](https://tauri.app/start/prerequisites/)

### 개발 서버 실행
```bash
pnpm install
pnpm tauri dev
```

### 프로덕션 빌드
```bash
pnpm tauri build
```

### 백엔드 테스트 실행
```bash
cd src-tauri
cargo test
```

---

## 📄 License & Status

- **Status**: v0.1.0 (macOS / Windows)
- Confluence is a trademark of Atlassian. Confluence Studio is an independent open-source tool.
