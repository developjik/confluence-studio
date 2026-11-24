# Converter contract tests (DEV-85)

This standalone crate compiles the production converter directly with Rust `#[path]` modules. It does not copy or mock the converter and does not need a desktop window, Tauri build, credentials, or network access at test runtime. Existing inline converter unit tests run alongside the new contracts.

```sh
cargo test --manifest-path converter-tests/Cargo.toml --locked
cargo install cargo-llvm-cov --version 0.9.2 --locked
rustup component add llvm-tools-preview
cargo llvm-cov --manifest-path converter-tests/Cargo.toml --locked --lcov --output-path converter-coverage.lcov
cargo llvm-cov report --manifest-path converter-tests/Cargo.toml --json --output-path converter-coverage.json
python3 scripts/check-converter-coverage.py converter-coverage.lcov converter-coverage.json
```

## Fixtures and review

`fixtures/cases.json` contains 32 synthetic, Confluence-shaped storage/Markdown pairs. These are not exports from a real customer account. They cover headings, inline markup and escaping, nested lists, tables and spans, code fences/CDATA, panels, expand, tasks, links/attachments/images, properties, malformed macros and inert self/reciprocal references. URLs use the reserved `.invalid` domain; no real account data is stored.

Each expected string was inspected against the storage and the current converter's behavior. Tests compare exact Markdown bytes, including trailing newlines, and repeat conversions to detect instability. Golden changes require a human-readable input/output diff and an explanation. There is intentionally no automatic snapshot-accept command.

These are **characterization tests, not a declaration that every existing output is ideal**. The two products have different contracts. Studio currently flattens tasks, drops some inline styles, and shows unsupported macros as callouts; export-desktop emits task checkboxes and unsupported-macro comments. Missing targets and malformed input may lose content. The fixtures keep these limitations visible rather than silently changing production behavior in this test-only ticket.

Neither converter resolves includes recursively or fetches page content. Self/reciprocal reference tests verify bounded, deterministic conversion, not cycle detection in a recursive resolver. The XML entity-like case is a local safety regression; it makes no external request.

## Coverage gate

The CI workflow runs on every pull request and default-branch push. LLVM LCOV must name every production converter `.rs` source, and **each file** must have at least 80% line coverage. Missing, duplicate, empty, truncated, inconsistent LCOV/JSON totals, or below-threshold reports fail. Imported error/model support modules are outside the converter scope. The gate excludes lines in trailing `#[cfg(test)]` modules, so test assertions cannot inflate production coverage. The current converter sources put those modules last; preserve that layout or update the gate when refactoring. This is a production line gate, not a branch-coverage claim. CI uploads the raw LCOV and matching LLVM JSON for 7 days.

The harness dependencies have their own committed `Cargo.lock`, because it is an independent workspace. CI checks every harness registry dependency against the desktop lock version/source/checksum. To refresh, copy `src-tauri/Cargo.lock` to `converter-tests/Cargo.lock` and run `cargo check --manifest-path converter-tests/Cargo.toml` to prune unrelated desktop packages. A production dependency or source-layout change must update this harness as needed; compiling the actual sources makes API drift fail at compile time. Desktop/end-to-end behavior remains the responsibility of the main application's tests.
