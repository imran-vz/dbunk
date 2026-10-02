# Error correction verification

2026-10-02. Imran selected error placement A and compact hover. This is an
uncommitted correction to Plan 026, not a completion SHA or human listening pass.

The source range helper preserves the existing `select_sql` result contract.
Only the native host consumes the new range API. Tauri execution, policy,
auditing and Query Session event/ACK/credit contracts are unchanged.
The stage 01 accessible editor adapter is unchanged.

## Repository checks

`pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt` and `just lint` pass.
The first parallel `just test` failed in
`connections::tests::duplicate_copies_credential_in_encrypted_mode` with
"Credential storage is locked". The complete `RUST_TEST_THREADS=1 just test`
passes: core 634 passed / 70 ignored; Tauri 658 passed / 85 ignored. Both logs
are preserved. This correction does not change credential storage code.

The prior human review window PID 77723 was closed through its native close
control. Its owning launcher confirmed exit zero and fixture connections
returned to zero; its temporary profile was cleaned. The isolated PostgreSQL
instance remains available. No daily-driver profile or database was opened.

## Review fixes

Independent source review found and corrected two timing problems before UI
verification. Terminal UI now updates only for an admitted ExecutionCompleted;
a reconnect's SessionState cannot recreate a retained old error. Each failure
has a fresh announcement node identity, so identical fast failures do not rely
on rendering an empty intermediate value. Close also cancels pending hover.


The first release AX run caught toolbar Tab navigation leaving the controls
for the result grid before reaching layout radios. The corrected route cycles
only visible, enabled controls in display order, with reverse Shift-Tab and
explicit Escape/F6 exits. A new run moves focus to SQL when it removes a
focused result tab or error control. The complete debug native AX workflow
then passed, including original editor geometry, Unicode and undo, selected
error positions, fast repeated announcement identity, stale in-flight source,
connection loss after failure, reconnect without error resurrection,
cancellation and actual active-query window closure. Exit was zero and fixture
connections returned 0 -> 0. This is automated evidence, not a listening pass.


## Native checks and visual evidence

Native debug/release Clippy and unit tests pass on the correction: 30 passed,
nine live tests ignored. The aggregate also passes opt-in core checks (642
passed, 70 ignored and two compile-fail docs), default Tauri facade tests
(eight passed), ten fixture helper tests and the pinned dependency graph check.
No GPUI revision or package version was upgraded. Release build passes.

Screenshots under `visual/` show the native compact hover opened with Cmd-K,
Cmd-I, its Escape-dismissed state, and the same popup opened by moving the
pointer over the token. Text and code are plain white on black, with a red
underline/border, and the explicit error surface stays above results. The first
capture selected a hidden auxiliary window; the retained screenshots instead
capture the full-size native window matching the AX geometry. Pointer input
used freshly queried AX token bounds after bringing the owned window forward.
The review window then closed cleanly with backend baseline 0 -> 0.

One release probe overlapped this separate visual driver and failed a focus
assertion while the visual driver activated its window. That run is not a pass;
its log is retained. The final release probe runs by itself with no competing
native window or input driver.


To reproduce the review state, launch `just dev-native`, then use its printed
PID and marked profile with the Swift probe:

```sh
swiftc tools/measure/editor-accessibility.swift -o /tmp/dbunk-editor-ax
/tmp/dbunk-editor-ax --native-fixture PID PROFILE --prepare-error-review
```

This option verifies fixture identity, exercises the existing geometry and
query checks up to the selected Unicode-offset error, opens the keyboard
hover, prints its token's AX screen bounds and leaves the window open. It does
not claim a full teardown or human listening pass. Press Escape, then move the
pointer over the underlined identifier to inspect pointer hover. F8 enters
controls; Tab/Shift-Tab cycle, Enter activates, Escape returns to content and
F6 moves between SQL and results. Cmd-Shift-Enter reruns the buffer for a fresh
error announcement. Close the window when finished; the launcher checks exit
and database connection cleanup.


## Foreground probe diagnosis

An isolated release probe intermittently failed its repeated-error assertion.
A reduced statement/hover/script loop reproduced this on cycle 1. Its captured
state was `active=false`, with both Query status and Run absent from AX
traversal, while the cached SQL field remained readable. Bringing the same
validated window forward, without running SQL again, showed `Failed · 759 ms`,
the expected 42P01 error and Run enabled. This ruled out an unfinished query
or terminal ACK in that reproduction.

The probe now foregrounds only its validated fixture before AX polling when
macOS hides that app's tree, and disables that behavior before testing closure.
It does not change application focus behavior or replay SQL. The reduced loop
then passed 20 statement-error / keyboard-hover / script-error cycles and
returned fixture connections 0 -> 0. The reduced probe source and before/after
outputs are retained here; it is a stress reproduction, not the full scenario
matrix. The full native release probe is run separately after this correction.


## Final release result

`just test-native-e2e` passes with the foreground probe correction. Its launch
log, complete AX output and teardown JSON are saved here. The release workflow
covers the original editor geometry/Unicode/undo checks, exact read-only values,
all three layouts, F8/Tab/Escape, result tabs, selected and repeated errors,
in-flight edits, guarded fixture connection loss, explicit reconnect,
cancellation and active-query window closure. The native process exits zero
and PostgreSQL connections return from baseline zero to zero.

The separate final human-review preparation additionally runs `SELECT 1;`,
`SELECT 2;` and a missing-table statement in one script, checks both retained
result tabs plus the error, and opens the compact hover. That review window is
left open deliberately for Imran, who confirmed “Clear now” on 2026-10-02.
The human VoiceOver listening gate is PASS; see [review](../voiceover-review.md).
At this correction handoff, the wider Plan 026 performance and combined
window-race evidence was partial. The subsequent
[final verification](../implementation-review.md#final-verification) closes
those gates. No completion SHA
exists and the plan is not DONE.


The corrected human-review window was PID 63138, with profile/fixture identity
recorded in `voiceover/identity.json`. Its preparation checks passed, including
two retained result tabs and the failed third statement. After Imran confirmed
the controls and error announcement were clear, the native close control closed
the window. Its owning launcher recorded exit 0 and backend connections 0 -> 0
in `voiceover/teardown.json`, and removed only the marked temporary profile.
The isolated PostgreSQL fixture remains running.
