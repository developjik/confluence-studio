# CI 셋업 (1분)

**상태: 활성화됨 (2026-11-18).** `.github/workflows/ci.yml`이 커밋되어 `master` 푸시와 모든 PR에서 `ci` 워크플로가 돕니다. 아래는 그 파일의 원본과, 다른 저장소에 같은 셋업을 옮길 때를 위한 기록입니다.

워크플로 원본은 [ci.workflow.yml](ci.workflow.yml)에 같은 내용으로 들어 있습니다. Pickle의 GitHub App에는 `workflows` 권한이 없어서 `.github/workflows/ci.yml`을 직접 푸시할 수 없습니다(푸시와 Contents API 모두 `refusing to allow a GitHub App to create or update workflow` / `Resource not accessible by integration`으로 거절됩니다). 그래서 파일은 `docs/`에 두고, 활성화는 웹에서 했습니다.

## 방법 1 — 웹에서 파일 추가 (권장, 1분)

1. https://github.com/developjik/confluence-studio/new/master 로 이동합니다. (또는 저장소에서 `.github/workflows` 폴더로 들어가 **Add file → Create new file**)
2. 파일 이름 칸에 `.github/workflows/ci.yml`을 입력합니다. `/`를 치면 폴더가 만들어집니다.
3. [ci.workflow.yml](ci.workflow.yml)의 `name: ci`부터 끝까지 복사해 붙여넣습니다. (파일 맨 위 `# CI workflow — copy to ...` 주석 4줄은 빼도 됩니다.)
4. **Commit changes** → 커밋 메시지는 `ci: add workflow` 정도면 됩니다.
5. **Actions** 탭에서 `ci` 워크플로가 도는 것을 확인합니다.

## 방법 2 — 로컬에서 푸시 (PAT 사용)

`workflow` 스코프가 있는 개인 액세스 토큰으로:

```sh
mkdir -p .github/workflows
cp docs/ci.workflow.yml .github/workflows/ci.yml
git add .github/workflows/ci.yml
git commit -m "ci: add workflow"
git push
```

## 워크플로가 하는 일

| 잡 | 러너 | 내용 |
| --- | --- | --- |
| `web` | ubuntu-latest | `pnpm install --frozen-lockfile` → `tsc --noEmit` → `vite build`, `dist/`를 아티팩트로 업로드 |
| `rust` | macos-latest | `dist/` 아티팩트를 내려받은 뒤 `cargo test` (`src-tauri`) |

- `rust`가 `dist/`를 먼저 받는 이유: `src-tauri/src/lib.rs`의 `tauri::generate_context!()`가 컴파일 시점에 `frontendDist`(`../dist`)를 임베드하므로, 프런트엔드 산출물 없이는 `cargo test`가 빌드되지 않습니다.
- 러너가 macOS인 이유: `keyring`이 `apple-native`/`windows-native` 피처로만 빌드되어 `credentials` 테스트가 실제 OS 키체인을 필요로 합니다. Linux에서는 `libsecret` 피처와 webkit2gtk/gtk 개발 패키지가 추가로 필요합니다.
- `damon_live` 통합 테스트는 `DAMON_LIVE=1`이 없으면 스스로 skip하므로 CI에서 데몬 없이 돌아갑니다.
- 트리거: `master` 푸시, 모든 PR. 같은 ref의 이전 실행은 취소됩니다.

## 확인된 로컬 검증

깨끗한 클론에서 아래를 실행해 통과를 확인했습니다(2026-11-18).

```sh
pnpm install --frozen-lockfile   # pnpm 10.26.1
pnpm exec tsc --noEmit           # TypeScript 6.0.3, 오류 없음
pnpm build                       # vite 8.3.5, dist/ 생성 (500 kB 청크 경고만)
```

`cargo test`는 로컬에서 검증하지 못했고(개발 라이브러리 부재), 워크플로의 첫 실행이 곧 검증이었습니다 — 2026-11-18 첫 실행에서 `web`·`rust` 두 잡 모두 통과했습니다.
