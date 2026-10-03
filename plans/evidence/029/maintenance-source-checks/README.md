# Maintenance and materialized-view refresh, 2026-10-03

The approved Objects Tool tab and bottom review now expose VACUUM, ANALYZE,
REINDEX TABLE, REFRESH and REFRESH CONCURRENTLY. The implementation and limits
are described in [maintenance progress](../maintenance-progress.md).

The backend owns observations, single-use document-bound review/confirmation,
policy admission before credential access, bounded notices and joined dedicated
connections. Native apply and confirmation wait for the exact persisted recovery
revision. Workspace version 9 holds a read-only description, with distinct unknown
and possible-partial-effects states. Reopening never restores execution authority.
The review uses 128 KiB of the shared retained allowance; dispatch reserves 64 KiB
of the delivery queue. Limits remain 128 MiB retained, 16 MiB delivery and 448 KiB
workspace persistence. These are not process RSS claims.

Independent review found stale timeout validation after credential access and a
hidden pending review with no route back to Cancel. Both were corrected. The
backend focused suite passes 19 tests. Native focused recovery/export tests pass
3 tests. Required pnpm format/lint/typecheck and Rust fmt/lint/serialized tests
pass: core 677 passed/71 ignored and Tauri 694 passed/85 ignored. Native debug and
release Clippy/tests, including fixture-harness Clippy, pass: 260 passed/13 ignored.
Isolated tests initially found three old version-8 assertions; only those test
expectations were corrected, and the failed log is preserved. Final isolated Clippy/tests pass: 954 passed/89 ignored plus 2 doc tests. Facade
tests pass 179/18 ignored; Tauri custom-protocol build, native package and dependency
proof pass. Ignored tests are not passes. Python tooling was unchanged and was
not rerun. [Window/reopen evidence](../maintenance-reopen-20261003/README.md)
verifies scoped review/recovery, ANALYZE/REFRESH completion, pending-review reopen,
keyboard cancellation and the earlier grid AX label correction. Its second
launcher teardown check failed on the external test blocker; guarded post-helper
cleanup subsequently verified both fixtures at 0. The initial package is frozen
at executable SHA256 `d36ebbdc250287ef1f6a9d3d94b9e8cbe5db8aee8c9bda8aac05cae08517852e`,
129651071 bytes. Follow-up receipt/disclosure source corrections have separate
checks under receipt-correction and are not in that binary.

The [owned live probe](./live-final.log) passed on stage03
`127.0.0.1:15432/dbunk_demo`, UUID `2283820d-33ec-4c4c-ae03-7051092bd410`.
It checked quoted identifiers, the three maintenance commands, both refresh
modes and prerequisite refusal, stale OID replacement, exact Strict confirmation
and six successful override audits. REINDEX cancellation rolled back; VACUUM
cancellation retained an unknown outcome; server timeout reported possible
partial effects. It does not measure those effects or eliminate concurrent-DDL
name resolution races. The initial probe cleanup refused string-encoded OIDs;
its parser was corrected and recorded residue was removed with OID/owner/comment
guards before the passing rerun. Both logs and independent cleanup evidence are
preserved here. Both run schemas and named blocker clients are absent, temporary
profiles removed and fixture activity returned 0 → 0.

The source manifest contains 463 hashes. Foreign-table maintenance, broader
administration, full PostgreSQL parity and remaining keyboard/AX/IME acceptance
remain open. VoiceOver is deferred; its checklist remains intact.
