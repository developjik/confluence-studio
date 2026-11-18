# 데스크톱 릴리스 셋업 (Tauri 2 → GitHub Release)

**상태: 활성화·검증됨 (2026-11-18).** `.github/workflows/release.yml`(커밋 [8538fb6](../../commit/8538fb6))이 `v*` 태그 푸시와 수동 실행(`workflow_dispatch`)에서 동작합니다. 원본은 [release.workflow.yml](release.workflow.yml)에 같은 내용으로 들어 있습니다.

- 워크플로: https://github.com/developjik/confluence-studio/blob/master/.github/workflows/release.yml
- 첫 검증 실행(수동, master): https://github.com/developjik/confluence-studio/actions/runs/44281592679 — 3개 번들 잡 모두 성공, **릴리스는 만들지 않고** 아티팩트만 남겼습니다.

GitHub App(`workflows` 권한 없음) 때문에 `.github/workflows/`는 `git push`로 못 올립니다. 그래서 파일은 `docs/`에 원본으로 두고, 활성화는 웹 업로더로 했습니다.

## 트리거

| 트리거 | 하는 일 |
| --- | --- |
| `push` — 태그 `v*` | 번들 3종 빌드 후 **초안(draft) 릴리스** 생성·자산 업로드. 초안은 직접 **Publish** 하기 전까지 공개되지 않습니다. |
| `workflow_dispatch` (수동) | 빌드만 하고 번들을 워크플로 아티팩트로 남깁니다. 릴리스는 만들지 않습니다. |

## 잡 구성

| 잡 | 러너 | 내용 |
| --- | --- | --- |
| `preflight` | ubuntu-latest | `v*` 태그일 때 태그 이름과 `src-tauri/tauri.conf.json`의 `version`이 일치하는지 검사 (`v0.1.0` ↔ `0.1.0`). 다르면 즉시 실패. |
| `bundle (macos-arm64)` | macos-latest (arm64) | `--target aarch64-apple-darwin` |
| `bundle (macos-x64)` | macos-latest (arm64) | `--target x86_64-apple-darwin` 교차 빌드 |
| `bundle (windows-x64)` | windows-latest | 기본 타겟(msvc) |

공통 스텝: `actions/checkout@v4` → `pnpm/action-setup@v4`(pnpm 10) → `actions/setup-node@v4`(Node 22, pnpm 캐시) → `dtolnay/rust-toolchain@stable`(macOS는 두 apple 타겟 추가) → `Swatinem/rust-cache@v2`(`src-tauri -> target`, 매트릭스별 key) → `pnpm install --frozen-lockfile` → `tauri-apps/tauri-action@v1`(`tauriScript: pnpm tauri`).

- `tauri build`가 `beforeBuildCommand`(`pnpm build` = `tsc && vite build`)를 돌려 `dist/`를 만든 뒤 Rust가 `../dist`를 임베드합니다.
- Linux 잡은 의도적으로 없습니다. `src-tauri/Cargo.toml`의 `keyring`이 `apple-native`·`windows-native` 피처로만 빌드되어 리눅스 키체인 백엔드가 없기 때문입니다(README "알려진 한계"와 같은 이유).
- macOS 두 잡이 같은 초안 릴리스를 동시에 만들 수 있어 `retryAttempts: 2`로 재시도를 둡니다.

## 산출물

`bundle.targets: "all"` 설정대로:

- macOS: `Confluence Studio_<version>_aarch64.dmg` / `..._x64.dmg` (+ `.app`)
- Windows: `Confluence Studio_<version>_x64-setup.exe` (NSIS), `..._x64_en-US.msi` (WiX)

수동 실행에서는 이 번들이 워크플로 아티팩트(`darwin-aarch64-dmg`, `darwin-aarch64-app`, `darwin-x64-dmg`, `darwin-x64-app`, `windows-x64-nsis`, `windows-x64-msi`)로 남습니다.

## 릴리스 내는 법

1. `src-tauri/tauri.conf.json`과 `package.json`의 `version`을 올리고 `master`에 머지합니다. (앱 번들 버전은 태그가 아니라 `tauri.conf.json`에서 옵니다.)
2. 같은 커밋에 태그를 밀거나 GitHub **Releases → Draft a new release**에서 `v0.2.0` 같은 태그를 새로 만듭니다.
   ```sh
   git tag v0.2.0 && git push origin v0.2.0
   ```
3. `release` 워크플로가 돌아 **초안 릴리스**를 만듭니다. 자산과 본문을 확인한 뒤 **Publish release**를 누르면 `/releases/latest`에 올라갑니다.

## 검증 결과 (2026-11-18)

- 로컬: `actionlint 1.7.7`로 `release.yml`·`ci.yml`·`deploy-demo.yml` 무경고, YAML 파싱 정상, `preflight` 셸 로직 일치/불일치 두 경우 테스트 통과.
- 수동 실행 [#44281592679](https://github.com/developjik/confluence-studio/actions/runs/44281592679) (master `8538fb6`): `version check` 성공(태그 검사는 수동 실행이라 skip), `bundle` 3개 모두 성공. 총 11분 22초 (macOS arm 5분 11초, macOS x64 8분 40초, Windows 11분 10초).
- 아티팩트 6종 생성 확인, 내려받아 실제 번들임을 확인: `Confluence Studio_0.1.0_aarch64.dmg`(4.5 MB), `Confluence Studio_0.1.0_x64-setup.exe`(3.6 MB).
- `gh api repos/developjik/confluence-studio/releases` → 0건: 수동 실행이 릴리스를 만들지 않는다는 설계대로입니다.
- 같은 커밋의 `ci`(#44281547451)·`deploy-demo`(#44281547441)도 성공 — 기존 워크플로는 깨지지 않았습니다.

## 아직 안 한 것

- **태그 경로(`v*` → 초안 릴리스)는 실제로 돌려보지 않았습니다.** 공개 저장소에 태그와 초안 릴리스가 생기는 일이라 검증 실행은 수동 경로로만 했습니다. 태그를 밀면 바로 검증됩니다.
- **코드 서명·공증 없음.** macOS는 Gatekeeper 경고가 뜨므로(우클릭 → 열기, 또는 `xattr -dr com.apple.quarantine`) Apple Developer ID·공증, Windows는 코드 서명 인증서가 있어야 경고가 사라집니다. 필요하면 `APPLE_CERTIFICATE`·`APPLE_ID`·`WINDOWS_CERTIFICATE` 계열 시크릿을 추가하고 `tauri-action` env에 연결합니다.
- 자동 업데이터(`tauri-plugin-updater`) 미연동 — 지금은 새 버전을 직접 내려받아 설치합니다.
