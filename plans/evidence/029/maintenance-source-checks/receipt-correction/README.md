# Maintenance receipt clarity correction, 2026-10-03

Only maintenance_view.rs and backend/maintenance/types.rs differ from the first
maintenance package's source manifest. The review now discloses the existing
10-second lock-wait cap. A terminal receipt contains the exact target/SQL/deadlines
and outcome without repeating the pre-dispatch recovery state or duplicating
all disclosures. Unknown and possible-partial-effects journals retain their
current durable state. Execution, ownership, persistence and cleanup are unchanged.

Required pnpm format/lint/typecheck and Rust fmt/lint/serialized tests pass.
Isolated all-target Clippy passes; focused maintenance/recovery tests pass 21 with
1 ignored (the separately verified live probe). Native debug/release and fixture-
harness Clippy pass; native debug/release tests pass 260 with 13 ignored. Ignored
tests are not passes. Initial maintenance full isolated/facade/custom-protocol
and live evidence remains separate; those broader suites were not repeated for
this display-only correction. The source manifest contains 463 hashes.

The separate package and dependency proof pass. Executable SHA256
`9a685ef478ebca83c8c01f2b8d32c7c8b7c52017a2eab6b94843e9c159a43fe6`,
129651279 bytes. [Narrow window verification](../../maintenance-receipts-window-20261003/README.md)
passed the lock-cap disclosure, exact terminal receipt without stale recovery
state, keyboard traversal/End scrolling and normal quit with both fixtures 0 → 0.
Subsequent column-pinning source work is outside this frozen manifest. Full PostgreSQL
parity and broader keyboard/AX/IME acceptance remain open. VoiceOver is deferred.
