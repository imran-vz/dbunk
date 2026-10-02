# Stage 03 implementation and verification

2026-10-02. Uncommitted work against `102568b8003461c9821fb84fc0957ccd3e4d0b13`.
Plan state remains in `plans/README.md`. No completion SHA exists.

All five steps are now verified. The final window-race and real-result
performance gates pass; see [final verification](#final-verification).
The plan is ready for review, pending a separately authorized completion commit.

## Implementation

- Separate `apps/native` macOS host, pinned Rust 1.98.1 and Zed revision
  `506beb34de3f433707b7ebe8d8ad2d80f856af6c`, with a four-worker Tokio runtime.
- Opaque opt-in backend facade keeps connection hydration, policy and auditing
  in existing services. Native execution always sends `confirmed = false`.
- Marked, private, exclusively locked SQLite profiles use PlainSqlite without
  invoking credential migration or Keychain. Fixture resources have separate
  ownership markers and an instance sentinel.
- Count/byte-bounded mailbox, independent sticky transport failure, coalesced
  cumulative ACKs and bounded controls. Terminal ACK success enables another Run.
- Read-only results preserve NULL/empty strings/numeric/Unicode values and copy
  exact retained text. Row and column rendering is virtualized; retention is bounded.
- Stacked, Side by side and Results first reuse the same editor/result entities
  and session. Layout preference persists through a narrow backend operation,
  including while disconnected.
- Explicit reconnect retires and joins the previous owner. Task ownership remains
  intact if a waiter is cancelled. Window close and OS quit share a latched
  shutdown barrier with a three-second grace and five-second total budget.

The stage 01 accessible editor adapter is byte-identical to the spike. The
native host adds control/result accessibility around it, without a cell editor.

## Repository checks

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: pass.
- `pnpm test`: 130 files, 1,488 tests pass.
- `just fmt`, `just lint`, `just test`: pass. Core-only: 634 passed, 70 ignored.
  Default Tauri: 658 passed, 85 ignored.
- Opt-in core facade: 641 passed, 70 ignored, plus two compile-fail API docs.
  Tauri-enabled facade: seven tests pass. Isolated-profile Clippy passes.
- Tauri custom-protocol build: pass.
- Resolved native graph: one pinned GPUI revision, no Tauri/Wry/Tao/WebKit.
- Fixture tooling: ten focused ownership/fault-identity tests pass.
- Existing core live suite on the owned fixture: all 17 actor tests and both
  safety tests pass, backend count 0 before and 0 after.

The initial native live suite passes all eight scenarios in `native-live.txt`, including
135 seconds unfocused followed by refocus, 125 seconds without ACK while
foreground heartbeats continue, held four-batch credit plus Stop, full-queue
retirement/reconnect, idle and active socket loss, and joined shutdown during
`pg_sleep` and held streaming credit. Every case returns the owned fixture to
its initial backend count. A ninth focused live test in `native-retention-live.txt`
injects a 512-byte result-model budget, verifies `retain_more_rows = false`
suppresses later batches, drains to terminal ACK and runs another query on the
same session. Its queue peaks at 27,966 bytes and returns to zero, with fixture
backend count 0 -> 0. These are controller/service integrations, not UI E2E.

## Verification findings

The first debug-window probe identified two harness issues. Foundation's URL
normalization rewrote `/private/var` to `/var`; canonical identity checks now
use POSIX `realpath`, agreeing with Python and Rust, while still rejecting
aliases and symlinks. An AXClose call can return `cannotComplete` after the
application has already exited; the probe now accepts that specific reply only
when the validated process subsequently terminates. The launcher independently
requires zero exit status and the original PostgreSQL backend count.

The diagnostic debug run passed text/range/bounds/scroll/resize, focus, exact
value/copy, query/error/recovery, and layout reflow during execution/cancellation.
It closed the native process and released all connections, but its old final
AXClose assertion failed. It is not recorded as a complete successful E2E run.
The disposable debug profile was removed only after its process exited.

## Cold release startup repair

The first release window connected to PostgreSQL but stayed on Connecting and
exposed no editor AX node until keyboard input arrived. Process samples had no
CVDisplayLink thread before input and did afterward. Inspection of the pinned
GPUI macOS code showed native visibility precedes frame callback registration.
The host now opens hidden and unfocused, then activates the window after
`open_window` returns. It adds no repaint timer or dependency patch.

Three fresh release launches now expose readable SQL, exact Ready status and
an enabled Run before any input or explicit editor-focus change. Each then
quits normally with exit code zero and backend count 0 -> 0. See
`cold-start.txt` and `cold-start-{1,2,3}/`. The probe performs normal AX
activation/raise but no key, mouse, AXPress or editor-focus action before its
startup assertions.

Controls now use stable identities and native radio/tab/toggle roles, and
notices/diagnostics expose named AX text. Geometry and keyboard verification
uses the unchanged editor adapter.

## Native UI evidence

The built-release AX run in `native-e2e/` passes the original editor geometry,
Unicode/range/selection/undo/scroll/resize checks, exact six-cell AX/copy values,
keyboard statement/selection/script execution, SQL error recovery, keyboard
focus routes, layout reflow during streaming/running/cancellation, and actual
window closure during a second `pg_sleep(30)`. AXClose-to-process-termination
was 60.756 ms; exit code was zero and fixture backend count returned 0 -> 0.
The first cancellation and its terminal ACK complete before that separate
active-query closure scenario.

An earlier release probe failed its first Unicode keyboard replacement; it
had checked AX editor focus but not active keyboard-window ownership. The
probe now checks the exact validated app and keyboard window before input and
prints actual disposable probe text on failure. The next run passes without
changing editor or key handling. The original failure is preserved under
`startup-investigation/keyboard-startup.txt`; its precise cause was not
established, so it is not claimed as a diagnosed host defect.

## Coverage boundaries

The full/closed mailbox and stale-event cases are deterministic reducer/bridge
tests. Delayed-connect shutdown uses the backend against a controlled stalled
socket; reconnect cleanup and cancelled waiters use owned-task barriers. Live
Host tests cover full credit, saturation, idle/active socket loss and cleanup.
The final fixture-only window hooks now add actual-window closure with delayed
connect/reconnect replies, streaming and held credit, plus replacement of the
root view while its queue is saturated. These supplement the earlier unit and
live-service evidence; they do not turn those headless tests into UI E2E tests.
The open hook holds the caller's pending reply after the real facade operation
is admitted. The facade continues owning that operation. The separate backend
test exercises a stalled TCP handshake; the UI hook does not claim to simulate
a network partition.

Native retention discard counts include received rows only. The service can
suppress subsequent batches after refusal without reporting them as core
omitted rows. The diagnostic therefore says “at least”; result-set row counts
remain the server counts. The live test distinguishes delivered and retained
rows instead of asserting a fabricated complete discard count.

## Limits and human review

The human VoiceOver check found unclear control navigation and weak error
presentation. It did not pass. [Correction review](./voiceover-review.md) records
the findings. Imran selected error placement A and requested an editor hover.
Imran selected the compact hover; correction implementation is recorded in
[the correction review](./error-correction/review.md). The separate stage 03
human listening recheck passed on 2026-10-02: Imran confirmed “Clear now”. Abrupt owned
backend termination verifies connection loss; a silent network partition may
remain unknown until an operation fails. No automatic SQL replay is implemented.

No daily-driver profile, Keychain entry, production database, existing Docker
runtime or published application channel was touched. No commit, push or PR
was made. The disposable fixture uses a source-built private PostgreSQL runtime;
see `local-fixture.md` for exact ownership and provenance.


## Historical handoff before the selected correction

The expanded launcher probe passes zero-row metadata, command-only results,
a server warning in Notices, Safe Mode refusal and visible `cellBytes`
truncation. Its attempted SQL self-termination is correctly blocked by policy;
that run is not a successful full E2E. The revised harness obtains its own
backend PID/start time through a safe query and uses the separately guarded
fixture helper for loss injection. That revised UI reconnect scenario has been
compiled but not run while the human review window is in use. It remains an
outstanding check. The earlier successful full E2E remains separately saved.

At that handoff, debug/release Clippy and unit tests passed (25 passed, nine live tests
ignored); the ninth live retention test passed separately. Final release build
passes. Ten fixture ownership/fault-identity tests pass. The current source
manifest records this dirty-tree candidate; no completion commit exists.

Real-result release typing/idle/scroll diagnostics have not run. The human
feedback requires the new hover selection and native correction before the
next native UI verification pass. Error placement A is already selected. The owned PostgreSQL fixture is deliberately kept available
for Imran. No global Docker daemon or unrelated database was started.


## Compact correction update

Error placement A and compact hover are selected and implemented. Repository
checks and native debug/release checks pass; native unit tests now include five
diagnostic mapping tests, for 30 passed and nine ignored live cases. Source
range selection has a focused duplicate-statement test. The full debug AX
workflow passes the revised guarded loss/reconnect scenario and the new error,
announcement-identity, toolbar-cycle and stale-position checks. Native release
screenshots verify keyboard hover, Escape dismissal and pointer hover, with
an underline on the correct SQL segment and the error surface above results.
See [error-correction/review.md](./error-correction/review.md) for check logs,
visual evidence and the final release launcher result. Human VoiceOver passed
after this correction on 2026-10-02; the original negative review is preserved
in [the human review](./voiceover-review.md). The reviewed window exited zero,
its fixture connections returned to zero and its temporary profile was removed.
The performance and combined window-race gates were still open at this
correction handoff. The final verification below closes them.

## Final verification

2026-10-02, following Imran's request to finish Plan 026. All evidence below
uses the owned PostgreSQL 17.11 fixture and fresh marked SQLite profiles.
The default release build contains no verification hooks. The opt-in
`fixture-verification` feature adds barriers and keyboard actions only to the
separate native fixture target. Native Clippy also checks that feature.

### Actual-window races

`python3 tools/native/verify_window.py races --out NEW_DIRECTORY` repeats nine
scenarios three times. All **27 runs pass**:

- Close with an admitted connect reply held.
- Close with a reconnect reply held after the old owner's cleanup.
- Close and Cmd-Q with four row batches held without ACK.
- Repeated Stop while four batches are held, acceptance before drain resumes,
  terminal ACK, and a successful subsequent query.
- Close with a saturated three-slot queue.
- Replace the actual root view with a saturated queue, release the old queue,
  open a fresh session and expose only the new query's exact result.
- Saturate metadata delivery, then expose the independent queue failure.
- Close after real streaming row delivery has begun.

The window close callback resolves the current root so closing still works
after replacement. Each successful run exits zero, releases all mailbox byte
permits, and returns fixture connections **0 -> 0**. Maximum observed
close/quit-to-process-termination time was **175.6 ms**, within the five-second
budget. Maximum queue allocation across these runs was **90,342 bytes**.
Delayed-open runs expose no SessionState after closure begins.

[Run log](./final-verification/window-races.txt),
[per-run summary](./final-verification/window-summary.json), and
[raw identities, AX assertions and cleanup logs](./final-verification/window-races/)
retain each result. The logs distinguish offered events from accepted data;
saturation is established by the explicit queue rejection, not by counting
offered rows as retained rows.

### Real-result release performance

`python3 tools/native/verify_window.py performance --out NEW_DIRECTORY` builds
without hooks. The accepted run used a 120 Hz display on battery power,
thermal state 0, accessibility active, and a foreground guard throughout.
It retained the real 10,000-row many fixture, 10,000-row wide fixture and
400-row large-cell fixture with no exposed omissions. Typing used the shared
2,000-line SQL document over retained real results.

| Diagnostic | Observed result |
| --- | --- |
| Typing | 900 samples, zero missed inputs; median of run p50s 30.4 ms; median of run p95s 37.4 ms |
| Typing p95 range | 37.2–37.8 ms across three runs |
| Foreground idle | 3.33% of one core over 30 seconds; median footprint 289.5 MiB |
| Scrolling | 12 runs, 6,080 measured intervals, zero long frames; run p95s 9.67–10.25 ms |
| Fixture footprint medians | Many 270.1 MiB; wide 348.6 MiB; large 319.8 MiB |
| Encoded retained data before terminal metadata | Many 1,048,626 bytes; wide 6,915,863; large 26,224,498, all below 48 MiB |
| Queue high-water | 788,433 bytes, below 8 MiB; zero bytes after release |
| Teardown | Exit zero; fixture connections 0 -> 0 |

[Summary](./final-verification/performance-summary.json) and
[raw captures](./final-verification/performance-guarded/) include environment,
counts, frame samples and cleanup evidence. Physical footprint includes more
than encoded row payload and is reported separately. These are diagnostics on
one machine, not a claim of improvement over the synthetic spike. The earlier
AX-active spike ran on AC power and had a median run p95 of 32.8 ms; the present
4.6 ms difference is below the harness's 8.33 ms capture interval, with different
power and workload conditions. Idle CPU is comparable to its 3.39%. No
unexplained within-run slowdown remains in the guarded captures.

### Capture interruption and repair

The first performance attempt is retained under `final-verification/performance/`
and is **discarded**. Another application took foreground ownership and covered
the scroll point. The third typing run rose to a 55.2 ms median; the subsequent
scroll correctly refused to post input. This attempt used forced process
cleanup and is not native-shutdown evidence. Its marked profile was removed
only after its PID had exited.

A minimal two-second scroll probe reproduced foreground loss. All three typing
captures had targeted the same full-size window, ruling out selection of an
auxiliary window in that attempt. The external measurement tool now accepts
`--foreground`, enabled by the native runner, to discard latency, scroll and
footprint captures when the target loses foreground ownership or is obscured.
A focused test uses two owned calibration windows: the unguarded interrupted
run writes samples; the guarded run exits with the expected refusal and writes
none. See [guard test](./final-verification/capture-diagnosis/guard-test.txt).
After Imran provided uninterrupted foreground time, all three typing runs
remained stable even as the comment line grew. No native rendering change was
needed to resolve this measurement failure.

### Final checks and accessibility

- `pnpm format`, `pnpm lint`, `pnpm typecheck`, and `pnpm test` pass; 1,488
  frontend tests in 130 files.
- `just fmt`, `just lint`, and `RUST_TEST_THREADS=1 just test` pass; core-only
  634 passed / 70 ignored, Tauri 658 passed / 85 ignored. Serial execution
  retains the established workaround for the credential-storage test race.
- Opt-in facade checks pass: core 642 passed / 70 ignored plus two compile-fail
  docs; Tauri facade eight passed. Custom-protocol build and core dependency
  tree checks pass.
- Native debug/release Clippy, tests and builds pass; 30 unit tests in each
  profile, nine live tests ignored here. The earlier nine owned-fixture live
  results remain separately recorded above. Ten fixture helper tests pass.
- The pinned dependency graph remains unchanged, with one GPUI revision and
  no Tauri/Wry/Tao/WebKit. Swift measurement-tool build and the foreground
  regression probe pass.
- Final `just test-native-e2e` passes the full release AX workflow, including
  original text geometry/Unicode/undo, exact results, layouts, controls,
  diagnostics, guarded socket loss/reconnect, cancellation and actual-window
  closure during `pg_sleep`. Exit zero, connections 0 -> 0, AXClose-to-exit
  70.1 ms. [AX output and teardown](./final-verification/native-e2e/).
- The separately recorded human VoiceOver listening pass remains valid; no
  controls, error copy or announcement behavior changed in this final slice.

[Source and lockfile hashes](./final-verification/source-sha256.txt),
[environment](./final-verification/environment.json), and command logs in
`final-verification/` identify the uncommitted candidate. No production or
daily-driver channel was touched. The existing owned PostgreSQL fixture remains
available; successful temporary profiles were removed. No commit, push or PR
was made, so there is no completion SHA and the plan is not marked DONE.
