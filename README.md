# Confluence Studio

A native desktop workspace for Confluence Cloud: browse and edit pages in Markdown, run scheduled Markdown export jobs, and chat with an AI agent — all in one Tauri 2 app with a Rust backend.

> Confluence Cloud 문서를 Markdown으로 편집·내보내고, 예약 잡으로 자동 백업하고, AI 에이전트와 대화하는 네이티브 데스크톱 앱 (Tauri 2 + Rust + React 19).

## Why

Teams keep their knowledge in Confluence but want it in plain Markdown files: portable, diffable, and greppable. Confluence Studio treats Confluence as the source of truth while giving you a native, keyboard-first editor, an export scheduler with the semantics of the original [confluence-export-desktop](https://github.com/) Python tool, and a built-in AI sidekick for drafting and editing pages.

## Features

- **Spaces & page tree** — browse every space, expand the full page tree, open any page instantly.
- **Markdown editing, Confluence storage underneath** — pages round-trip through a quick-xml based `storage XHTML ↔ Markdown` converter; what you edit is what gets saved as real Confluence versions.
- **Create / update / delete** — full CRUD against the Confluence Cloud v2 REST API from the editor.
- **Scheduled export jobs** — export a space (or a page subtree) to local Markdown with attachments; options for skip-unchanged, stale cleanup, breadcrumbs and front matter, with bounded parallel workers.
- **Schedules** — Daily, multiple-times-daily, weekdays, weekly or monthly, with timezone support and run history.
- **AI chat (Damon)** — thin-client chat over the Damon daemon protocol v2 WebSocket: sessions, streaming replies, tool-permission prompts.
- **Credentials in the OS keychain** — your API token lives in macOS Keychain / Windows Credential Manager, never in a config file.

## Screenshot

![Confluence Studio](docs/screenshot.png)

## Getting started

Requirements: Node 20+, pnpm, Rust stable, platform webview deps ([Tauri 2 prerequisites](https://tauri.app/start/prerequisites/)).

    pnpm install
    pnpm tauri dev

Build a production bundle:

    pnpm tauri build

## Architecture

- `src/` — React 19 + TypeScript + Vite frontend (CodeMirror editor, react-markdown preview)
- `src-tauri/src/confluence/` — Confluence Cloud v2 client + storage↔Markdown converter
- `src-tauri/src/jobs/` — export jobs, scheduler, history (semantics ported from the Python original)
- `src-tauri/src/damon/` — Damon protocol v2 WebSocket thin client

Module contracts and the full command/event API: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Testing

    cd src-tauri && cargo test

Unit + integration tests (wiremock-backed Confluence API, Damon protocol, jobs/scheduler).

## Acknowledgments

- Export semantics ported from the original confluence-export-desktop (Python).
- AI chat powered by the Damon daemon (protocol v2).

## Status

v0.1.0 — macOS primary; not affiliated with Atlassian. Confluence is a trademark of Atlassian.
