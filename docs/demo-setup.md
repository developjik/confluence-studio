# 라이브 데모 배포 셋업 (2분)

이 저장소의 웹 데모는 완성된 상태로 `docs/deploy-demo.workflow.yml`에 들어 있습니다. Pickle의 GitHub App에는 `workflows` 권한이 없어 `.github/workflows/`에 직접 푸시할 수 없으므로, 아래 두 단계만 웹에서 해 주시면 됩니다.

## 1) 워크플로 추가

1. https://github.com/developjik/confluence-studio/new/master 로 이동합니다.
2. 파일 이름 칸에 `.github/workflows/deploy-demo.yml`을 입력합니다. (`/`를 치면 폴더가 만들어집니다.)
3. [deploy-demo.workflow.yml](deploy-demo.workflow.yml)의 `name: deploy-demo`부터 끝까지 복사해 붙여넣습니다. (맨 위 `#` 주석 8줄은 빼도 됩니다.)
4. **Commit changes**.

## 2) Pages 소스를 GitHub Actions로 변경

**Settings → Pages → Build and deployment → Source**를 `Deploy from a branch` → **`GitHub Actions`**로 바꿉니다.

기존에는 `master` 브랜치의 `/docs` 폴더(랜딩 페이지)를 그대로 서빙하고 있었습니다. 소스를 바꾸면 루트는 **데모 앱**, 랜딩 페이지는 **`/landing/`**로 옮겨집니다.

## 워크플로가 하는 일

| 잡 | 내용 |
| --- | --- |
| `build` | `pnpm install --frozen-lockfile` → `pnpm build:demo`(`tsc` + `vite build --mode demo`, `VITE_BASE_PATH=/confluence-studio/`) → 랜딩 페이지를 `dist/landing/`으로 복사 → `actions/upload-pages-artifact` |
| `deploy` | `actions/deploy-pages`로 GitHub Pages 배포 (`environment: github-pages`) |

- `--mode demo`가 `.env.demo`의 `VITE_DEMO=1`을 읽어 `src/api.ts`를 `src/demo` 인메모리 백엔드로 바꿉니다. Tauri 런타임·Confluence 사이트·Damon 데몬이 없어도 화면 전체가 동작합니다.
- `VITE_BASE_PATH=/confluence-studio/`가 `vite.config.ts`의 `base`가 되어, 프로젝트 Pages 하위 경로에서 에셋이 404 없이 로드됩니다.
- 결과: https://developjik.github.io/confluence-studio/ (데모), https://developjik.github.io/confluence-studio/landing/ (랜딩)

## 확인된 로컬 검증 (2026-11-18)

```sh
pnpm install --frozen-lockfile
VITE_BASE_PATH=/confluence-studio/ pnpm build:demo   # dist/ 에 base 경로 반영, 데모 백엔드 포함
```

빌드된 `dist/index.html`의 스크립트·CSS 경로가 모두 `/confluence-studio/assets/...`로 나오는 것을 확인했습니다.
