# Existing-table DDL native activation, 2026-10-03

Status: native source and corrected package checks pass; actual-window
verification remains blocked by native automation startup. No native DDL window
or full parity pass claimed.
The [backend-only service](../table-ddl-source-checks/README.md) remains limited to
one table/column comment or rename on an ordinary, non-inherited table.

Workspace version 13 now adds a bounded descriptive TableDdl journal with exact
attempt, target (including namespace and optional column attnum), intent, SQL,
summary, both timeouts and staged/unknown state. Deserialization creates no
execution authority. Validation regenerates the exact typed preview. Older
versions 1–12 remain readable without rewriting; a new field, even null, in an
older envelope refuses. The 48 KiB journal allowance does not expand the 448 KiB
workspace cap. Other mutation journals cannot share this Objects document.

Six focused backend tests pass: exact/read-only restore including empty comments,
qualified/whitespace rename spelling, attempt/target/attnum/timeouts matching,
invalid-record preservation, capacity/envelope/mutual-exclusion refusal, and
legacy version compatibility. The recorded debug linker compact-unwind warning
is not a test failure. Required `just fmt`, `just lint` and serialized `just test` pass. Core tests
report 677 passed/71 ignored; Tauri tests report 694/85. Isolated-profile Clippy
and 1,085 backend tests plus two compile-fail doc tests pass, with 99 ignored.
The initial isolated Clippy failure found two omitted example constructors; both
are corrected and the failure log is retained. Ignored tests are not passes.

Native source is integrated from reviewed external proposals: Structure
supplies an exact OID/attnum selection; the existing document worker owns bounded
Observe/Review/Apply/Confirm; the approved Objects bottom review uses an exact
persisted recovery revision before dispatch. Cancellation before dispatch must
invalidate late save acknowledgements. NeedsConfirmation must match the exact
review and await its own new save acknowledgement. Unknown outcomes preserve
recovery and require explicit reconciliation, never retry. Matching outcomes
that may have changed the database invalidate connection-bound captures while
preserving SQL, mutation drafts and separately owned query sessions.

The runtime reserves 64 KiB of the existing 16 MiB delivery budget before backend
polling. This does not guarantee delivery after close; a lost write receipt stays
unknown. The view must separately admit its shared retention lease. Column
selection uses the observed PostgreSQL attnum, never display position or name
alone. The owned stage03 schema/table in `window/setup.json` was created for
acceptance and removed unused after automation failed. `window/teardown.json`
records exact OID/name/owner/tag/comment/row guards and RESTRICT cleanup. No
production, daily-driver or credential changes occurred. No full designer or
other object lifecycle parity claim.

Native debug Clippy/tests/build and release Clippy pass with 383 tests passed and
13 ignored. Four runtime tests cover identity/attnum and pre-dispatch delivery;
seven model tests cover exact receipts, Unknown retention, late save ACKs,
confirmation and admission; Structure/export tests cover typed selection and
recovery preservation. Initial compilation found three attempt IDs lacking
Display; their existing `as_str()` accessor fixes it. Two Clippy style findings
are fixed. All failure logs remain.

Source review found Tab/Shift-Tab action handlers consumed input despite the
composition-aware focus helper returning early. They now leave those actions
unconsumed during marked input. The initial release-package build was explicitly
interrupted for this correction, not a passing package or a spontaneous build
failure. `composition-correction/` records the corrected source and new checks.
This is source verification, not a real IME pass.

The corrected debug/release matrix and package pass: 383 native tests passed,
13 ignored in each configuration. `composition-correction/checks.json` records
commands, and `post-package-proof.json` matches all 689 frozen source files.
The package is
`/private/tmp/dbunk-native-package-20261003-table-ddl-correction/dbunk Native Preflight.app`,
executable SHA-256
`d8c05ed47469a44d6e0361ab021da69cd138ab19d4270feb9685875531838c7f`.

The ownership launcher started PID 84851 with the existing isolated review
profile. CUA could not inspect any window: `Sky Computer Use native pipe startup
failed`, repeated after resetting the automation runtime. This is a tool startup
failure, not an observation that the desktop is locked. No UI actions completed.
After checking exact process path, profile argument and UID, SIGTERM stopped
only this process. Launcher exit -15 is recorded; it is not normal-quit
acceptance. Both fixture backend counts were zero afterward.
Read-only profile inspection after shutdown records workspace version 13, 14
documents and no table-DDL journals. The launch saved the newer workspace
envelope despite no completed UI actions. Earlier packages that support only
version 12 cannot reopen this profile; use this corrected package or a later one.

Pending: native window comment/rename/cancel/recovery, keyboard/AX and real
Tool-tab IME. The backend-only live probe remains separate from these gates.
VoiceOver remains deferred.

The subsequent [real COMMIT/runner-acknowledgement loss probe](./commit-ack-loss/README.md)
passes: one real commit remains visible through an independent connection while
the runner reports Unknown with the exact receipt identity. This is test-only
boundary injection, not wire-level failure or native-window acceptance. Its
owned schemas were removed and fixture activity returned 0 → 0.
