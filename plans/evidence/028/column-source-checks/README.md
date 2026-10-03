# Column controls and SQL review AX, 2026-10-03

This source increment adds table-column sizing, ordering, visibility and saved
preferences, plus semantic SQL/bound-value text in mutation review. It preserves
the existing dirty migration tree. `source-sha256.json` identifies this variant;
subsequent Tool tab work is a different variant and needs separate checks.

| Check | Result |
| --- | --- |
| `pnpm format`, `pnpm lint`, `pnpm typecheck` | PASS |
| `just fmt`, `just lint`, serialized `just test` | PASS; core 675, Tauri 692; fixture tests ignored |
| Native debug fmt, all-target and fixture-harness Clippy, tests/build | PASS; 79 passed, 12 ignored |
| Native release all-target Clippy, tests/build/package | PASS; 79 passed, 12 ignored |
| Native dependency proof | PASS, one pinned GPUI revision; no Tauri/Wry/Tao/WebKit |
| Pending live query confirmation rerun | PASS, one exact test; `../table-window-20261003/query-confirmation-rerun.txt` |
| Release-window column/recovery/AX checks | PASS for scenarios in `../table-window-verification-20261003.md` |

The custom-protocol Tauri and isolated backend checks from `../table-source-checks/`
remain evidence for the unchanged backend in this variant; they were not repeated
for native-only column/rendering changes. Later backend changes need new checks.

Package: `/private/tmp/dbunk-native-package-20261003-columns/dbunk Native Preflight.app`.
Executable SHA256: `edce1936c62c739d5dd6ab63fd61aa3349d2e81520c4f647cbf1ad6f39a765a4`.
No fixture-ignored test is counted as passed. Complete PostgreSQL parity is not
established by these checks.
