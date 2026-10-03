# Native retained safety overrides, 2026-10-03

Status: source implemented; focused backend/native checks and independent review pass. Required and native debug/release checks, packaging and dependency proof pass. All 381 frozen source hashes match after packaging. Actual-window acceptance is pending. This is a local audit-list increment, not administration control or complete PostgreSQL parity.

## Scope

The Administration Tool tab adds Safety overrides in the approved Tool-tab arrangement. Refresh reads only the exact bound connection's local profile records and works disconnected. Older overrides continues a bounded page; Refresh starts over. Selecting a record exposes command kind, statement classes, timestamp and record ID. Copy uses the existing exact clipboard readback. No PostgreSQL socket, credential hydration, replay, policy modification or record deletion is involved.

These are retained successful safety overrides recorded by the app, not all database actions or a complete security audit. Storage retains the latest 1,000 records globally across connections. Record expiry while paging is disclosed. This does not change legacy Tauri readers/writers or their audit semantics.

## Ownership and bounds

`Backend::load_safety_audit` uses the owned profile call. Opaque in-memory cursors bind the backend owner, exact connection, last `(occurred_at,id)` and initial ID watermark. Foreign-owner/filter cursors refuse. Later inserts are excluded from a continued reading, while global pruning can remove older records. Pages have individual SQLite snapshots, not one historical whole-list snapshot.

The native-only reader streams at most 101 rows and returns at most 100 within 256 KiB encoded and checked retained capacities. SQLite CASE/typeof/UTF-8 byte-length guards run before persisted fields are returned. Command text caps at 256 bytes, timestamp at 64, classes JSON at 4 KiB and 256 known class labels. Invalid JSON, types, labels or timestamps refuse visibly instead of becoming an empty successful page. Redacted errors/debug do not print stored payloads. No SQL text or parameter values are added to the audit.

The native capture reserves 1 MiB of the shared 128 MiB allowance; old and replacement captures remain charged until replacement succeeds. Stable row IDs prevent selecting an unrelated row after replacement. The joined local-data worker reserves 512 KiB from the existing 16 MiB delivery budget before materialization and allows one outstanding reply. Cancel discards publication after the owned SQLite read settles; it does not claim to interrupt SQLite. Disconnect/close fences stale replies, and explicit refresh can reopen a closed local worker. Neither restore nor section selection performs remote work. These limits are payload allowances, not process RSS claims.

## Verification scope

Five native capture tests cover identity, ordering/expired selection, refusal without losing the old capture, capacities and scope disclosure. The local-worker integration test covers disconnected/deleted-binding reads, bounded response admission, queued-work fencing on close, release and explicit reopen, with error request identity preserved. Existing request-state tests cover cancellation and stale request IDs. Backend and native all-target Clippy pass. Five actual SQLite tests cover exact connection/profile cursors, insertion watermark, real global pruning and empty final continuation, encoded byte boundaries, corrupt/oversized persisted fields and capacity validation. The initial test-runtime setup refusal is preserved; corrected multi-thread test runtimes pass. Independent review found a missing worker-exit wake and then its channel-close ordering race. The exit guard now closes replies before waking; pending-state changes notify even for stale replies. A focused test keeps another sender alive and verifies that the exit wake observes a closed reply channel. The runtime close test also requires its exit notification.

No PostgreSQL fixture is required for these local SQLite checks. The preceding [schema package discovery attempt](../schema-create-window-20261003/failed-discovery.json) failed before interaction; it is not audit-window evidence. Keyboard/AX and real IME remain required for the migration. This audit list adds no editable input. VoiceOver remains explicitly deferred, not passed.


## Frozen verification and package

`pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt`, `just lint` and serialized `just test` pass. Core tests: 677 passed/71 ignored; Tauri: 694 passed/85 ignored. Isolated backend: 867 passed/84 ignored plus 2 doctests; Tauri facade: 112 passed/13 ignored. Native format, debug/release all-target Clippy, fixture-harness Clippy, debug build and debug/release tests pass: 207 passed/13 ignored in each run. Custom-protocol build and native dependency proof pass. Ignored tests are not passes. Python tooling is unchanged and its prior scoped tests were not repeated.

Package: `/private/tmp/dbunk-native-package-20261003-safety-audit/dbunk Native Preflight.app`.
Executable SHA256: `de8876f7a1262d5515f612b153b596db13c57151f4c707aeb8e8067b321d47a0`.
Bundle size: 123,971,596 bytes. See package-identity.json and post-package-source-proof.json. The first proof-collection script guessed the marker filename incorrectly after a successful package build; corrected collection uses the package module's actual MARKER. That tooling error is retained separately; no rebuild or source change was needed.

No window launch was attempted for this audit package after the preceding schema package failed CUA discovery. No audit workflow, keyboard/AX or normal-quit acceptance follows from the source checks. All profiles and the frozen Plan 027 package remain untouched by this local-only increment.
