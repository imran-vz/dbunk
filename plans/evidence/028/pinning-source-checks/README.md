# Column pinning source checks, 2026-10-03

The separate package is
`/private/tmp/dbunk-native-package-20261003-pinning/dbunk Native Preflight.app`.
Executable SHA256 `0ecfd79cf7db96da14130b7361a3f43b6d7473d3ee9ee3b673fd49e5d4819736`,
bundle 129,675,679 bytes. All 467 source hashes matched before launch. The manifest
identifies the final dirty source; no commit, push or cutover is implied.

Required pnpm format/lint/typecheck and just fmt/lint/serialized test pass.
Native final debug/release all-target Clippy, fixture-harness Clippy and tests
pass: **271 passed, 13 ignored** in each test profile. The package and dependency
proof pass. Ignored tests are not passes. Backend implementation and tooling did
not change; prior maintenance isolated/facade/custom-protocol/Python evidence is
separate and was not rerun for native-only pinning.

Initial visibility/test-helper compiler failures are retained in initial-clippy
and second-clippy logs. The initial 270-test runs preceded the independent
review's all-hidden recovery correction; final-* logs cover that correction and
the added result-local admission assertion. [Review](./review.md) records its
scope and resolution. [Implementation](../pinning-progress.md) describes limits.

[Scoped AX and copy evidence](../pinning-window-20261003/README.md) and
[table reopen](../pinning-reopen-20261003/README.md) pass their listed cases.
Captured pixels became stale or lost unchanged regions; the
[preceding-package comparison](../pinning-capture-comparison-20261003/README.md)
reproduced that symptom. Wheel input failed with noWindowsAvailable. Full visual
alignment, wheel/resize and broader keyboard/IME acceptance remain open.
Temporary Pinyin setup was restored to ABC only, hidden input menu and English
(United States) dictation; restoration AX files are included. VoiceOver remains
deferred. Plans 027–030 and complete PostgreSQL parity remain IN PROGRESS.
