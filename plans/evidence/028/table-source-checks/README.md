# Native table source checks, 2026-10-03

This checks the dirty source tree based on `3f987c9`. Exact source hashes and
local toolchain identity are in `source-sha256.json` and `environment.json`.
It does not mark Plan 028 complete or replace actual-window acceptance.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | PASS |
| `just fmt`, `just lint` | PASS |
| `RUST_TEST_THREADS=1 just test` | PASS: core 675, Tauri 692; fixture tests ignored |
| Native debug fmt, all-target Clippy, fixture-harness Clippy, tests/build | PASS: 76 tests, 12 ignored |
| Native release all-target Clippy and tests | PASS: 76 tests, 12 ignored |
| Native release executable/package | PASS: release build and isolated package; see `package.txt` |
| `RUST_TEST_THREADS=1 just check-native-backend` | PASS: isolated 744 + 2 doctests; Tauri/isolated backend 51; fixture tests ignored |
| Final isolated query-library tests/Clippy | PASS: six tests, including escaped cursor boundary |
| `cargo build --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol` | PASS |
| Native Python tooling tests | PASS: 23 |
| Native dependency proof | PASS: one pinned GPUI revision; no Tauri/Wry/Tao/WebKit packages |
| Actual window, live fixtures and real IME | NOT RUN in this source increment |

The library cursor boundary correction landed while the integrated checks were
running. The Tauri/isolated backend run includes it. The subsequent six-test
isolated run and Clippy check validate the final source explicitly. Earlier
five-test library logs remain in Plan 029 evidence. An initial custom-protocol
command used the nonexistent package feature `custom-protocol`; the corrected
command above passed. The command error is retained separately and is not a
source failure or a passing check.

Review fixes include journal-before-dispatch cancellation, outgoing-editor focus
return, tab navigation leaving the table, blocked writes for unrestorable drafts,
review/analysis retention accounting, and removal of redundant staged-draft
copies. Offline selection/removal invalidates old reviews. New tests exercise
late/duplicate save acknowledgements, uncertain outcomes, exact SQLite barriers,
shared retention refusal and worst-case cursor escaping.

No foreground automation or live PostgreSQL fixture tests ran while the human
handoff was pending. `frozen-handoff.json` confirms the old package executable
is unchanged. Imran subsequently delegated the checks to the agent and deferred
VoiceOver from the blocking gate on 2026-10-03. Real IME remains pending; no
automated test substitutes for composition. The old package also predates the
new table controls, which require their own window, keyboard/AX and IME checks.
