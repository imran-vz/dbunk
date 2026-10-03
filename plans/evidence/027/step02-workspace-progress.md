# Plan 027: Native workspace implementation

2026-10-02, uncommitted continuation based on `3f987c9`. **IN PROGRESS**.
Workspace A, table-review A (Bottom review), and tool tabs A are selected.
The requested outcome remains complete PostgreSQL parity with Tauri.

## Implementation

- A persistent Navigator, connection forms, credential onboarding/unlock/recovery,
  native menus, and independently owned query tabs. Opening/restoring a tab does
  not connect or execute SQL. Password editors mask their value and suppress
  clipboard copy/cut and plaintext accessibility content.
- One workspace owner and heartbeat, at most four sessions and two executions,
  independent cancellation/ACK control, and a shared 16 MiB event queue budget.
  UI draining has one aggregate eight-envelope / approximately 2 ms turn budget.
  Retained results share 128 MiB; existing results are not silently evicted.
- One owned draft writer coalesces edits with a 500 ms debounce, retains only the
  latest pending snapshot and associates each acknowledgement with its revision.
  The 448 KiB encoded budget, compare-and-swap failures, explicit SQL export,
  corrupt-record export/reset, and save-or-confirm-discard close path preserve
  the last durable snapshot and current unsaved SQL separately.
- Native per-session task groups make a tab close join its own work. Shared
  observers remain until the last associated document closes. Default Tauri
  behavior retains its existing task configuration.
- TLS admission is explicitly opted into a newly marked profile. Its exact
  separate endpoint is `127.0.0.1:15433/dbunk_tls_demo`; old profile manifests
  cannot be widened on reopen. Certificate/server-name failures reach the TLS
  driver while the TCP destination remains pinned.

## Evidence so far

- [Scoped real OS Keychain CLI acceptance](./os-keychain-cli-18e8a1e5/environment.json):
  save, separate-process reopen, conversion and reset passed. Both disposable
  primary/rollback entries were absent after cleanup. Packaged GPUI coverage
  is recorded separately below.
- [Owned TLS fixture](./tls-fixture-15151cfc/environment.json): libpq and native
  facade matrices passed trusted/untrusted/hostname validation and encrypted
  native query execution. Both plain/TLS fixture activity counts returned 0 → 0.
  No global trust store changes; the separate fixture remains for window tests.
- Focused writer tests passed coalesced latest SQL, exact Unicode, failed oversize
  persistence, recovery, stale-writer refusal, final close flush and preserving
  an existing SQL export file.
- Focused runtime/task ownership tests passed admission, independent controls,
  pending-tab closure, per-session task teardown and shared observer ownership.
- [Release-window acceptance](./workspace-final-20261002/verification.md) passed
  connection Test/Save, two independent result streams, Unicode draft restoration,
  undo/focus, all three layouts and TLS validation/client-certificate fields.
  Both owned fixtures returned 0 → 0 after titlebar close and Cmd-Q on reopen.
- [Packaged credential acceptance](./workspace-package-20261002/agent-verification.md)
  passed Keychain onboarding, Test/Save, separate-process blank-password editing,
  encrypted SQLite conversion, wrong/correct unlock, conversion back to Keychain,
  and confirmed password reset with exact SQL and connection metadata retained.
  All four launches exited cleanly with fixture activity 0 → 0; guarded cleanup
  confirmed both entries in the announced namespace absent.
- [Recovery window acceptance](./workspace-recovery-20261002/agent-verification.md)
  passed normal quit from corrupt/future snapshots without changing their bytes
  or revision, explicit reset preserving connections/credentials, and oversize
  failed-save handling. The final oversize run blocked ordinary close, exported
  all current SQL exactly, then joined shutdown after confirmed discard while
  retaining the last durable draft. Every completed launcher returned to baseline.
- [Live workspace runtime checks](./workspace-runtime-live.json) passed independent
  streams/cancellation, four-session/two-execution admission, reconnect and joined
  teardown. Queue-pressure control checks used explicitly synthetic envelopes;
  they are not evidence of a live SQL stream saturating the full queue.
- [Performance diagnostics](./workspace-performance-b/summary.json) captured 900
  typing samples each with one and four sessions, zero missed inputs and 24 scroll
  runs without long frames. Median typing p95 was 35.10/35.34 ms; idle CPU was
  3.94/3.89% of one core. Twenty open/run/close cycles settled 15.8 MiB below the
  pre-cycle footprint; process lifetime peak was 350.7 MiB. Queue high-water was
  788,703 bytes and teardown retained zero bytes. The measured binary predates
  the subsequent recovery-only fixes; its exact source variant is recorded.
  AC power differs from Plan 026's battery capture, so these deltas do not
  establish an improvement. Idle is about 0.6 percentage points above Plan 026
  and below the original Tauri baseline; no continuous idle frame loop was added.
  Aggregate encoded-result peak was not directly instrumented in this run.
- Frontend format, lint, typecheck and all 130 files / 1,488 tests passed.
  Rust format and default/core Clippy passed; default/core tests passed 684/660
  respectively. Isolated-backend checks and the custom-protocol Tauri build pass.
  The recovery-fixed release built successfully. Final aggregate native checks
  must cover the latest source, including subsequent backend preparation.

## Build provenance and pending acceptance

The [packaged executable](./workspace-package-20261002/package.json) has SHA-256
`df2cb9a3141cd5c9ae83c38d28333e40fc843e56bbcd36c4f4bb8a1506803a02`.
Its [source manifest](./workspace-package-20261002/source-manifest.json) was
recorded after the recovery fixes and before Plan 028 backend preparation.
The package remains frozen as an isolated verification target; newer backend source changes
are not covered by its window checks. The performance capture used an earlier
binary, as recorded above.

The earlier [human handoff](./workspace-package-20261002/human-ready/status.md)
was superseded on 2026-10-03 when Imran asked the agent to run the checks.
VoiceOver is now deferred and non-blocking under the
[scope decision](./accessibility-scope-20261003.md). Later agent-driven real
Pinyin composition, commit/cancel, selection replacement and undo checks passed
in the SQL editor, connection form and table cell editor, including exact draft
reopen. See [scoped window evidence](../028/table-window-verification-20261003.md)
for the tested newer package identities and limits. The frozen package was not
changed. Broader/new-tool IME checks remain pending; VoiceOver is not passed.

Still unproven in the actual workspace window are three repeated overlapping
mode-change/close/reconnect races, forced-termination recovery of acknowledged
drafts, missing-connection restoration, and SQLite save-failure/retry. Focused
backend/runtime tests cover parts of these cases; they do not replace those
window gates. Aggregate encoded-result peak also remains unmeasured.

The first-wave evidence in `step01-services/` describes its own source hash set.
It does not establish acceptance of this subsequent UI implementation. Plan 027
remains in progress. Plan 028 and later PostgreSQL tools remain required for
full parity.
