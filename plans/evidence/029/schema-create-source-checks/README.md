# Native Create Schema and recovery, 2026-10-03

Status: implementation, focused and broad checks, owned live verification and separate packaging pass. All 374 frozen source hashes match after packaging. CUA discovery failed before interaction; actual-window acceptance remains pending. This is Create Schema with an optional comment on that newly created schema, not complete object lifecycle/designer parity.

Baseline and ownership gaps: [DDL reconnaissance](../ddl-ownership-recon.md), Tauri `102568b`, ADR-0026. Existing-object rename/drop, CASCADE and standalone/partial groups remain inactive.

## Native workflow

The Objects Tool tab exposes Create schema / Schema draft, using the approved bottom review. Schema name and optional comment remain local staging inputs until explicit Review. Review validates and saves the exact typed intent and generated SQL preview without opening a PostgreSQL socket. Name and comment preserve spelling and None-vs-empty; names cap at 63 UTF-8 bytes, comments at 4 KiB, and NUL/blank names refuse. The field histories are bounded and actual marked composition prevents review/apply/navigation actions. Actual IME verification is still pending.

Apply freezes the review, marks its attempt OutcomeUnknown in workspace recovery and waits for the exact persisted revision before dispatch. Cancellation before dispatch invalidates late save ACKs. Stored policy can return a single-use confirmation for the same attempt and intent; Confirm gets a fresh exact revision barrier and rechecks current policy. Read-only cannot be bypassed.

A terminal receipt must match attempt UUID, connection and exact intent. Applied clears only that journal; NotApplied retains staged intent; unknown/lost delivery retains unknown recovery and never automatically retries. A disappeared window/worker cannot fabricate rollback. Successful apply reports that catalog refresh is separate. Explicit reconciliation/discard removes only the local record and does not retry or undo SQL. Draft close/retarget/disconnect guards prevent silently losing recoverable intent; application quit can persist it for disconnected restore.

Restored journals do not restore runtime review/confirmation tokens. The bottom panel can inspect or reconcile an unknown journal without connecting. Unadmitted recovery remains owned by the workspace while memory refusal is displayed; the record is not discarded to make room for a view. Copy review verifies immediate exact clipboard readback.

## Backend write ownership

The narrow CreateSchemaIntent/review/confirmation facade regenerates SQL from typed operations and never accepts executable host SQL. Only one native DDL write per connection is admitted across the bounded document registry. A short mutex shared by cancellation, retirement and commit admission decides whether COMMIT may start. No mutex is held over I/O. Before admission, cancellation prevents COMMIT; afterward, known COMMIT success cannot become Cancelled. Lost/error COMMIT response conservatively yields OutcomeUnknown.

The dedicated runner executes one transaction, uses the configured statement timeout plus a 10-second lock timeout, and applies a 30-second asynchronous operation deadline. Post-admission cancellation waits up to one second on the same COMMIT future, never sends another COMMIT. Socket cancellation/close/abort and driver joins retain ownership through cleanup. Synchronous TLS-file work and cooperative joins are not a hard wall-clock guarantee. The existing pooled Tauri DDL path and read-only native reader are not reused as writers.

Stored profile authority/policy is checked before secret hydration. Known confirmed success records one successful override audit; audit failure cannot recast committed DDL as failed. The audit is not a durable execution journal. Legacy Tauri behavior remains unchanged.

## Persistence and bounds

Workspace format 4 adds strict schema_changes only to a bound Objects document. Formats 1–3 remain readable; old-version schema journals, malformed UUID/intent, mixed document kinds and future records refuse without overwriting bytes. The existing 448 KiB workspace and 16-document bounds remain. Review SQL is premeasured before generation, limited to two statements and 16 KiB encoded/retained preview. Heavily escaped comments can hit that preview cap even when their input fits 4 KiB.

Each open schema review reserves 1 MiB of the shared 128 MiB retained-payload budget before editor/history construction. Delivery uses the existing 16 MiB queue. Schema recovery waiting for view admission remains within the workspace's existing saved/restored payload allowance. These are bounded payload/work allowances, not process RSS claims.

The existing generic save/dispatch/confirmation fence moved from table_changes/apply_flow.rs to the shared native apply_flow.rs and remains used by both paths. Source review caught an export gap: a schema-only workspace could select SQL export and omit its recovery. Export routing now chooses JSON for any structured intent; the SQL exporter independently refuses that lossy request. A focused file round-trip verifies the exact unknown journal and no partial SQL file creation.

## Focused verification

Backend schema tests: 9 pass. Document-fence tests: 4 pass. Workspace recovery tests: 26 pass. Native exact-attempt recovery, reservation and schema-export tests pass; both shared apply-flow tests and all 7 persistence tests pass. The broader schema_ name filter also ran two unrelated existing tests, which are not new schema coverage. Native all-target Clippy passes after correcting an unused import, a GPUI checkbox API mismatch and a collapsible-if lint; failed compile logs are retained separately.

Independent backend review found no remaining concrete defect in policy/admission, commit ordering, bounded generation or joined cleanup. Independent native review found the export omission, now fixed. No actual-window claim follows from these checks. The preceding [server-inspection package attempt](../server-details-window-20261003/failed-discovery.json) failed CUA discovery before interaction; that is not a schema-window test. Keyboard/AX and real IME remain required. VoiceOver remains deferred, not passed.

## Owned live scope

Owned stage03 only: 127.0.0.1:15432/dbunk_demo, UUID 2283820d-33ec-4c4c-ae03-7051092bd410. Unique native_schema_ddl_* names, private temporary profiles, exact identity/owner/comment/no-children checks before DROP RESTRICT cleanup. The first probe assumed confirmation was unnecessary; policy correctly returned NeedsConfirmation before any schema write. That failed test log is preserved. The corrected probe consumes the exact confirmation without relaxing policy and passes public receipt/comment checks, duplicate NotApplied, atomic rollback, cancellation/reuse, retired-owner refusal and joined cleanup. The final audit-enhanced run confirms exactly one apply_object_ddl audit with class ddl for the confirmed success; the failed duplicate adds none. The unique schema OID 16634 was removed with the recorded exact-identity guard; both test names are absent and fixture activity returned to zero. Earlier failed and successful logs/cleanup identities are preserved separately in backend-focused/. No production, personal profile, OS Keychain or daily-driver app was touched.

## Frozen verification

The source manifest contains 374 paths, expanding the prior server-inspection scope to all PostgreSQL Rust files. source-scope.json records the shared ApplyFlow move. Required frontend format/lint/typecheck and Rust fmt/lint/serialized tests pass: core 677 passed/71 ignored; Tauri 694 passed/85 ignored. Isolated backend 862 passed/84 ignored plus 2 doctests; Tauri facade 107 passed/13 ignored. Native format, debug/release all-target Clippy, fixture-harness Clippy, debug build and debug/release tests pass: 200 passed/13 ignored in each run. Custom-protocol build, separate package and native dependency proof pass. Python tooling is unchanged, so its prior scoped checks are not rerun for this increment. Ignored tests are not passes.


## Frozen package and window attempt

Package: `/private/tmp/dbunk-native-package-20261003-schema-create/dbunk Native Preflight.app`.
Executable SHA256: `996140dda1dd3df292039ff739163121e473de8cf7441bbbb5fab185fa30dbc7`.
Bundle size: 123,863,852 bytes. See package-identity.json and post-package-source-proof.json.

The guarded launcher created `/private/tmp/dbunk-native-schema-create-20261003-review` for owned stage03. PID 47592 was visible in CUA inventory, but exact-path and observed-bundle-ID selection each failed with `cgWindowNotFound` before any interaction. [Discovery evidence](../schema-create-window-20261003/failed-discovery.json) is a blocked window attempt, not Ready, keyboard/AX, IME or schema workflow acceptance. The exact process command, profile and executable hash were checked before SIGTERM; fixture activity returned to zero. [Forced cleanup](../schema-create-window-20261003/forced-cleanup.json) is not a successful normal quit gate. The profile is retained. VoiceOver remains deferred, not passed.
