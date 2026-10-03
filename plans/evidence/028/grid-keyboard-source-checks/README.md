# Retained-grid keyboard source checks, 2026-10-03

The separate package is
`/private/tmp/dbunk-native-package-20261003-grid-keyboard/dbunk Native Preflight.app`.
Executable SHA256 `445459cf6dd601a0dc33830f51889956a46afe36d026e1da1cae676d1b30648d`,
bundle 129,677,215 bytes. All 468 source hashes and all bundle file hashes matched
after packaging. Only grid.rs and grid/keyboard.rs changed from the pinning
source manifest. This identifies dirty source, not a commit or cutover.

Required pnpm format/lint/typecheck and just fmt/lint/serialized test pass.
Native debug/release all-target Clippy, fixture-harness Clippy and tests pass:
**273 passed, 13 ignored** in each test profile. Package and dependency proof
pass. Ignored tests are not passes. Backend implementation and tooling were
unchanged for this increment; their earlier isolated/facade/custom-protocol
and Python evidence remains separate.

[Implementation scope](../grid-keyboard-progress.md) covers retained-cell
Home/End, Cmd/Ctrl Home/End, Page Up/Down, Shift extension and Escape. Actual
keyboard/AX verification of these commands remains pending. After packaging,
the active tool connection no longer exposed cua_repl/native desktop control;
the package was not launched. Browser automation cannot verify this GPUI app.
This tool availability limit does not imply that the user's desktop is locked.

Earlier [capture limitations](../pinning-capture-comparison-20261003/README.md)
remain unresolved. Broader real IME acceptance remains required, VoiceOver is
deferred, and Plans 027–030 remain IN PROGRESS.
