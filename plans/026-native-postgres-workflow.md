# Plan 026: First working native PostgreSQL workflow

- Priority: P1. Effort: L. Risk: HIGH at lifecycle and runtime integration.
- Migration stage 03. macOS Apple Silicon development host only.
- Planned against `102568b8003461c9821fb84fc0957ccd3e4d0b13` plus the
  inspected, uncommitted Plan 024/025 implementation, 2026-10-02.
- Depends on stage 01's **CLOSED: PASS / continue** gate and Plan 025's
  implemented host-neutral seam. Neither dependency is marked DONE without
  its separately authorized completion SHA.
- Execution status: [README.md](./README.md), the single source of truth.
- Source: [working-version task](./gpui-working-version-task.html),
  [ADR-0032](../docs/adr/0032-host-neutral-backend-seam.md),
  [stage 01 gate](./evidence/024/stage01-gate.md).
- Review: [Plan 025 review](./evidence/025/stage03-review.md).
- Decision artifact: [plan and layouts](./mocks/native-postgres/index.html).
- Selected by Imran on 2026-10-02: **A + B + C**, available through a layout
  switcher. [Switchable preview](./mocks/native-postgres/workspace.html).

> Read this plan completely. Preserve the existing working tree. Inspect
> implementation before changing it; do not replay Plan 024 or 025. Imran has
> selected all three layouts with a user-facing switcher; the layout-selection
> gate is satisfied. No commit, push, PR,
> daily-driver profile, keychain entry or published application channel is
> authorized. Update this plan's README row after each implemented step.
> READY FOR REVIEW requires all gates; DONE requires a completion SHA.

## Outcome and exclusions

One reproducible command opens a separate GPUI application, uses an explicitly
isolated SQLite profile, connects to the disposable PostgreSQL fixture,
executes SQL, displays exact read-only results and errors, cancels work,
reconnects explicitly and releases all owned work on window closure.

Keep Tauri command names, event JSON, sequence numbering, ACK/credit behavior,
connection hydration, safety checks and success-only audit behavior unchanged.
The native host depends on the existing backend with `default-features = false`.
Do not extract other service families or move the backend into a new core crate.

Not included: connection management forms, saved workspaces, schema browser,
mutations/cell editors, schema maps, backup/restore, transfers, comparisons,
Redis, other engines, full editor parity, signed packaging or cutover. A
read-only result grid does not imply that all SQL is read-only: services enforce
the stored connection policy. This slice sends `confirmed = false`; it displays
Safe Mode refusals without adding an override-confirmation workflow.

## Selected layouts and switching

Imran selected all three layouts on 2026-10-02. Offer a compact, labelled
**Layout** selector in the workspace toolbar:

- **Stacked**: SQL above full-width results (A, initial default).
- **Side by side**: SQL left, results right (B).
- **Results first**: compact SQL above a larger results region (C).

One typed layout value changes pane geometry only. Reuse the same editor
entity, buffer, result model and backend session in every layout. Switching
must not reconnect, re-execute SQL, cancel work, reset ACK/credit or allocate
another result copy. Preserve SQL selection/undo, active result set/cell,
scroll anchors and the previously focused content region. The layout control
keeps keyboard focus while it is being operated; returning to editor/results
restores the prior content focus. Recompute accessible bounds after reflow so
no old highlight geometry is exposed.

Allow switching while idle, streaming, cancelling and disconnected. Remember
this UI preference in the explicitly isolated profile through a narrow settings
operation; do not expose raw storage or extract the whole settings family.
Invalid/missing values fall back to Stacked. Clamp pane sizes to usable minimums
on resize; keep the selected preference when constrained by a narrow window.
Results first permits expanding the editor without replacing its entity.
No animated pane transitions or extra decorative chrome.

## Review findings that shape this plan

Plan 025 preserves the service policy and event contracts. No extraction
regression requiring a source repair was found in this review. Its test evidence
is narrower than the native lifecycle required here:

1. `close_window` closes registered sessions but neither retires its owner nor
   fences an in-flight `open`. A connect finishing afterward can create a late
   session. Native admission must close before the view is released.
2. `close_all` does not reject later opens. `begin_global_teardown` does reject
   them, but logical closure is not a join of opens, executions and drivers.
   Existing three-second timeouts cannot be reported as successful cleanup.
3. Monitors and execution tasks currently detach. The native lifecycle must
   retain task ownership, then join or abort-and-join before runtime shutdown.
4. Stop does not release row credit. Continue consuming and ACKing during
   cancellation; a broken consumer instead retires its session explicitly.
5. The spike's raw-semicolon selection is synthetic-only. Do not copy it into
   real execution. Reuse core SQL lexing and statement spans.

These are inherited behaviors or native integration obligations, not reasons
to rewrite Plan 025 or alter Tauri's lifecycle in this slice.

## Architecture and invariants

### Narrow public backend API

Add `src-tauri/src/backend.rs` as an explicit export facade. An opaque backend
handle owns private `AppState`. Expose only profile startup, a sanitized fixture
connection summary/connect/disconnect, window owner registration/focus/heartbeat,
Query Session open/execute/ACK/cancel/close, and awaited native shutdown.
Re-export the exact request/event/error value types needed by those operations
from private modules, including nested DTO types. Never make an entire manager,
driver, storage, credential or policy module public. Do not accept a hydrated
connection, caller-supplied policy, pool, connect spec or audit callback.

Startup configures PlainSqlite credentials before saving the fixture through
the connection service. Query operations delegate to `query_session::service`.
The facade can expose a pure statement-selection helper over the existing Rust
lexer; it does not expose dispatch. Use compiler-checked downstream usage and
compile-fail documentation for forbidden raw access. Preserve the existing
Tauri adapter; no frontend protocol duplication or new JSON bridge.

### Native application and isolation

Create a separate `apps/native` Cargo package/workspace, binary `dbunk-native`,
`publish = false`, GPL-3.0-or-later for this Zed-linked target. Preserve the
current app's license and the stage 01 spike. Use Rust **1.98.1**, Zed revision
**506beb34de3f433707b7ebe8d8ad2d80f856af6c**, and the spike's required patches,
embedded assets and `runtime_shaders` configuration. Commit a native lockfile
only when separately authorized; during this task keep it uncommitted and use
`--locked` after initial resolution. The resolved native graph must contain
one GPUI revision and no Tauri/Wry/Tao/WebKit. Do not upgrade dependencies to
make the join compile without documenting and reviewing the conflict.

All GPUI state lives on the UI thread. A host-owned multi-thread Tokio runtime
with `enable_all` runs services and core monitors. Futures requiring Tokio are
spawned on that runtime, never directly polled by GPUI's executor. Retain
handles for command tasks, receiver/wake tasks and monitors. At most one
window, session and active execution in this slice; multiple result sets are
supported. Disable duplicate Run/connect requests while admission is pending.

The launch helper creates a fresh private directory with an ownership marker
and passes its absolute path explicitly. Backend startup refuses missing
markers, symlinks, non-empty foreign profiles and an unexpected credential
mode before reading credentials. It never calls the platform profile resolver
or falls back to a default directory. Reusing a profile requires its matching
fixture marker and loopback endpoint. No keychain migration or keyring access.

Use an owned disposable PostgreSQL fixture at `127.0.0.1:15432`, database
`dbunk_demo`, development-only `dbunk` credentials. Fixture setup verifies the
Compose project/service identity; refuse a foreign listener instead of
connecting to it. Prefer a dedicated stage03 Compose project with tmpfs and
loopback-only port binding; if another owned fixture occupies 15432, report it
instead of force-recreating it. Load `tools/measure/fixtures/postgres.sql` for
the existing many/wide/large views, plus a stage03 sentinel and policy fixtures.
Never run the broad `db:down` command. Cleanup removes only resources created
by this launch and its explicitly marked profile.

### Bounded delivery, ACK and retention

Keep `QueryEventEnvelope` unchanged. Capture native owner identity in the sink
closure because it is not a field in the envelope. Fence every message and
command completion by owner, session, connection generation and execution.
Sequences are monotonic per session, not per execution. Session-level events
have no execution id and must still be admitted against the current session.

Use a synchronous non-blocking `try_send` sink into a bounded native mailbox.
Initial limits: **64 envelopes and 8 MiB of encoded payload**, including
metadata/notices. Acquire byte budget before enqueue, release on consume/drop;
reject a single event over the budget. Count actual encoded bytes or a proven
upper bound without allocating an unbounded serialized copy. A count limit
alone is insufficient for wide cells. Keep the existing four-batch/4 MiB row
credit window. GPUI consumes at most eight envelopes or about 2 ms per turn,
then schedules another one-shot wake if necessary. No frame polling/spinners.

Use a sticky first-failure state plus a coalesced wake independent of the data
queue. On full/closed/oversize delivery, set a typed local transport failure,
return `SinkClosed`, fence this stream, and request service cleanup. Do not
enqueue a synthetic core terminal event or silently drop accepted events.
The UI can display a failed stream even when no further core event can arrive.
The terminal status is chosen once per execution. A later cancel reply or
stale completion cannot replace it. Cancellation/close commands use a separate
bounded control path, never the full event queue; close is a latched signal.

ACK cumulatively only after consuming the event into the bounded result model
(including an explicit retention decision). Preserve `requires_ack`, terminal
ACK and `retain_more_rows`. Do not ACK at enqueue or keep an unbounded ACK-task
backlog; coalesce pending cumulative ACKs per current execution and serialize
their sends. Run remains unavailable until the terminal ACK succeeds or the
session is retired. Stop keeps this consumer and ACK path alive. ACK failure
retires the session instead of leaving the interface ready on a busy backend.

Send owner heartbeats every ten seconds while focused, using a single owned
task; deliver focus changes explicitly. The existing lease is 120 seconds and
its monitor runs every ten seconds. Preserve background lease semantics. Test
ACK expiry with continuing foreground heartbeats so the failure is genuinely
`ackTimeout`, not `ownerTimeout`.

Retain core limits (10,000 rows/result set, 32 MiB row payload/execution, cell,
row, result-set and metadata caps). A native accounting limit of 48 MiB encoded
data per retained execution includes columns, notices and errors; on a budget
boundary send `retain_more_rows = false`, drain/ACK, and show omissions. Keep
only the current execution; clear prior data on a new Run. Count memory owned
by the queue and retained model separately and measure actual process memory;
encoded payload budgets are not a promise about allocator overhead. Render
visible cells only. Keep `Option<String>` values, including empty strings,
NULLs, numeric precision, Unicode and backend truncation metadata. Copy uses
retained text, not visual ellipses or numeric parsing.

### Lifecycle and reconnect

Model connection states explicitly: connecting, ready, running, cancelling,
disconnected, closing, closed. Database errors can finish an execution while
the session stays usable; session/transport loss requires explicit Reconnect.
Reconnect first retires the old sink/session, drains cleanup, then obtains a
fresh owner/session identity through services. Never replay SQL automatically.
Late replies, old generations and terminal events cannot affect the new run.
The existing heartbeat only renews liveness; it is not a database health probe.
Have the native lifecycle observe its tracked socket's closure and report an
idle connection loss through local host state, without changing core event
JSON. A silent network partition may remain unknown until a bounded operation
fails; document that limit rather than displaying a successful health check.

Add an opt-in native lifecycle owner behind the facade. Close its admission
before cancelling tasks. Retire window ownership before awaiting any open;
guard cancellation of opens so opening reservations and observers are released.
Track Query Session execution, producer and dedicated-driver joins for native
sessions, including an observer created during connect. Reuse the private
`postgres::dedicated::DriverJoins` pattern where appropriate. Keep the default
Tauri path unchanged and covered by the same tests. Do not expose those joins
or manager internals to GPUI.

Window close and app quit use the same idempotent async barrier: reject new
work, detach/fence the UI receiver, retire the owner, request cancellation,
close sessions, await all owned opens/workers/drivers, stop monitors, close
the SQLite pool, then terminate the runtime. Keep the process alive while
cleanup runs, without blocking GPUI's event loop. Give graceful cleanup three
seconds, then abort and join owned tasks/drivers within a further two seconds.
If cleanup still cannot be established, report a failed gate and a nonzero
verification outcome. Never label timeout alone as successful cleanup. Verify
PostgreSQL activity returns to its pre-launch baseline, including observers.

## Implementation sequence

### Step 1: Selected layouts and API contract

Selection is recorded as A + B + C in README. Add the facade and DTO exports,
isolated profile constructor and downstream compile checks. Bind query calls
to service functions. Reject unsafe fixture startup and public bypass routes.

Gate: both backend feature configurations; service safety tests prove a
read-only refusal before dispatch, Safe Mode refusal with `confirmed = false`,
and existing successful-override audit behavior remains unchanged through the
service. This does not add an override UI.

### Step 2: Native runtime and fixture launch

Create the separate native package, pinned graph, profile/fixture setup and
documented commands. Port the verified accessible SQL editor adapter with its
geometry and focus behavior; keep the spike independently runnable. Wire a
connection summary and a real open through the facade, without synthetic rows.
Add owned lifecycle and the shutdown barrier before enabling Run.

Gate: fresh isolated launch connects; absent/foreign profile and unavailable
fixture fail explicitly; no keychain calls; native dependency proof; close
during a delayed connect cannot create a late session.

### Step 3: Mailbox, result reducer and failure controls

Implement queue count/byte budgets, independent failure notification, bounded
control and ACK delivery, stale-message fencing and result retention. Add
deterministic reducer/bridge tests with injected tiny budgets and delayed
consumers. Test logical outcomes and released resources, not widget markup.

Gate: every saturation, receiver-loss, late completion and terminal-ACK case
in the matrix below; no silent event loss, growing task list or event-loop
blocking. Use barriers rather than timing guesses for race tests.

### Step 4: Working query controls and read-only inspection

Implement all three selected layouts over the same workspace state, with the
Layout selector described above. Show connection state, SQL editor, Run,
Run script, Stop, results, multiple result-set selection, notices and terminal
status. Run uses selected SQL or the statement at the caret; Run script uses
the full buffer. Use existing Rust statement lexing/spans and explicit cursor
boundary handling. Include end-of-statement and end-of-file positions,
trailing semicolons, strings, quoted identifiers, comments, nested comments,
dollar quotes and UTF-8/UTF-16 offsets. Empty/comment-only input does not run.

Display zero-row column metadata, command row counts, partial results, error
SQLSTATE/message/position, notices and all omission reasons. A syntax error
does not imply disconnect. Support keyboard result navigation and copy; cells
cannot stage or apply edits. Omit the synthetic editable cell probe from the
native host, retaining it unchanged in the stage01 spike. Reconnect opens a
new session and returns focus predictably without running the buffer.

Gate: complete the real GPUI -> facade -> services -> PostgreSQL -> mailbox ->
GPUI path. Reuse the stage01 adapter; verify focus/selection/undo survive
Run, Stop, result inspection, layout switching and reconnect. New controls expose AX names,
roles, enabled state and deterministic focus return.

### Step 5: End-to-end verification and evidence

Add fixture-only deterministic test hooks for queue capacity, delayed drain,
delayed open and view replacement. Keep them outside production defaults.
The native E2E harness drives the actual window and captures event/cleanup
evidence without logging credential values. Required scenarios follow.

| Scenario | Required observation |
| --- | --- |
| Fresh launch | Only marked temporary SQLite profile accessed; PlainSqlite configured before connection save; visible fixture identity and ready state. |
| SELECT + exact values | NULL differs from empty string; bigint/decimal text, Unicode and quotes survive display/copy; keyboard navigation works. |
| Scripts and boundaries | Multiple result sets, zero rows with columns, command-only result, notice, trailing semicolon and quoted semicolons; selected/caret/script behavior is explicit. |
| Wide/large/many | Existing fixture counts 10,000 / 400 / 10,000; virtualized scrolling; bounded queue/model; no unbounded per-cell or per-event tasks. |
| Truncation | Over-limit query reports core cell/row/result/metadata limits; native retention refusal drains and ACKs; counts/reasons visible. |
| SQL error | Invalid SQL yields backend code/message/position; one terminal state; next query can run when session remains healthy. |
| Read-only/Safe Mode | Forbidden SQL has no execution event or server-side effect; audit invariants match the service tests. |
| Stop | `SELECT pg_sleep(30)` and streaming cancellation each settle once; repeated Stop and late cancel reply cannot overwrite completion. |
| Slow consumer | Hold below ACK timeout, reach four-batch credit, request Stop, resume drain; bounded memory, cancellation accepted, terminal ACK completes. |
| Full/closed queue | Saturate data and metadata paths, reject initial/row/terminal event, drop receiver; independent local failure appears and session/byte permits release. |
| ACK timeout | Stall consumption beyond lease while foreground heartbeats continue; session loss visible, resources release; no ready state or unbounded backlog. |
| Stale execution | Retire owner/session or reconnect while events and command replies are queued; old messages cannot alter new rows/status or ACK the new session. |
| Connection loss | Cut an owned loopback proxy during a read and while idle; active loss uses existing core events, idle socket closure uses native lifecycle observation; explicit reconnect succeeds; no automatic SQL replay. |
| Close/quit | Close during connect, `pg_sleep`, streaming, full credit and reconnect; no late SessionState; all joins complete/abort-and-join within budget; fixture backend count returns to baseline. |
| Focus lease | Background longer than the lease remains usable; refocus renews it; foreground stalled consumer still expires correctly. |
| Layout switching | A -> B -> C -> A during streaming and cancellation keeps the same session/execution, SQL selection/undo, results and ACK progress; keyboard focus and AX bounds follow reflow; preference restores from the isolated profile. |
| Accessibility | Existing text/range/bounds/scroll/resize assertions, keyboard-only editor/results/control navigation, exact copied/AX values, and focus return pass on the native host. |

Adapt `tools/measure/editor-accessibility.swift` with an explicit native-fixture
mode that validates executable identity plus this launch's fixture marker;
never accept an arbitrary PID/name wildcard. Keep its existing spike mode and
cell-editor assertions. Native mode checks read-only results and toolbar focus
instead of asserting a mutation editor. Preserve F6/Shift-F6 editor/results
cycling and Tab indentation in SQL, with a keyboard route to toolbar controls.
Do not silently delete geometry or focus assertions. Record a manual VoiceOver
check for new control/error announcements separately; the earlier confirmation
does not validate controls introduced here. If permission or human listening
is unavailable, record that gate as blocked rather than claiming a pass.

## Commands to deliver and run

These commands are provided by the native fixture and launch tooling.

```sh
just native-fixture-up
just dev-native
just test-native-e2e
just native-fixture-down
```

`dev-native` owns the temporary profile and prints the exact fixture identity,
profile path and PID; it checks the fixture rather than starting/replacing a
database silently. `test-native-e2e` creates its own profile, runs the real
native window, captures evidence and cleans up only its resources. Record
prerequisites (Docker/owned fixture, Rust 1.98.1, macOS permissions), setup,
failure recovery and clean shutdown in `apps/native/README.md`.

Required repository checks, with both Rust backend configurations:

```sh
pnpm format
pnpm lint
pnpm typecheck
pnpm test
just fmt
just lint
just test
cargo build --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol
cargo tree --manifest-path src-tauri/Cargo.toml --no-default-features -e normal
```

Integrate `fmt-native`, `lint-native` and `test-native` into documented aggregate
checks and a macOS CI job; do not attempt GPUI/macOS runtime tests in Linux CI.
Native checks (pinned toolchain, run from `apps/native`):

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
cargo clippy --release --locked --all-targets -- -D warnings
cargo test --release --locked
cargo build --release --locked
cargo tree --locked -e normal
```

Run the 17 existing `query_session_actor_live` tests and both `safety::live`
tests on the owned fixture in the core-only configuration, plus new focused
native lifecycle/safety tests in both relevant configurations. Rerun the
external AX probe against the native fixture PID and retain the unchanged
spike probe as the comparison. Record release typing/idle/scroll measurements
under AX-active load for performance diagnostics; don't reuse synthetic
measurements as proof of the real database workflow. Investigate unexplained
regressions before claiming the workflow ready.

## Evidence, estimate and stopping rules

Store commands, toolchain/OS, exact revision/lockfile/source hashes (dirty-tree
identity), test counts, fixture identities, screenshots/AX output, queue and
retention high-water marks, shutdown durations and backend-count assertions
under `plans/evidence/026/`. Distinguish automated, manual, historical and
unrun evidence. Do not call a headless reducer test an end-to-end UI test.

Allowance: API/isolation 1–2 days; pinned host/runtime/lifecycle 3–5; event
bridge/reducer 2–3; three-layout query UI 3–5; live/AX verification 2–3. Total
11–18 person-days, with 2–4 additional days if dependency unification or
cleanup ownership needs repair. This includes roughly 1–2 days of stage01's
statement/diagnostic allowance and 2–3 days of its read-only grid/focus work;
the remaining editor/grid parity work stays in stage05. These are estimates.

Stop and report if the dependency pins require a new GPUI/Zed revision,
public API exposes a policy bypass, Tauri behavior or wire contracts must
change, bounded cleanup cannot be demonstrated, verified text geometry/focus
regresses, a required gate is blocked, or any step needs a daily-driver profile,
keychain, foreign database or published app channel. A layout selection does
not authorize cutover, commits, pushes or PRs.

Implementation was authorized on 2026-10-02. Steps 1–5 now have code and
verification evidence in the working tree. All gates pass, including the final
27 window-race runs, guarded real-result performance captures and release AX
workflow. See [final verification](./evidence/026/implementation-review.md#final-verification). The reviewed layouts and switchable HTML preview remain the
design record. No completion SHA exists and the plan is not DONE.

The fixture may use a dedicated local PostgreSQL runtime when Docker is not
available. This alternative must preserve the same loopback endpoint, instance
sentinel, disposable cluster, resource ownership checks and narrowly scoped
cleanup. It must not start a Homebrew service or an existing Docker daemon.
Imran authorized creating this isolated environment on 2026-10-02.


## VoiceOver correction gate, 2026-10-02

The first human stage 03 check did not pass. Imran found the toolbar keyboard
route unclear and the error too subtle, resembling a footer warning. The
Control-Option shortcuts overlap VoiceOver's default modifier; the status node
also lacks an explicit live announcement setting. Stage 01 remains closed.

Imran selected **error placement A**, above results, and requested an editor
hover over the problematic SQL segment. This retains all three workspace
layouts. [Two hover treatments](./mocks/native-postgres/error-hover-review.html)
were published for the new surface. Imran selected **compact**. Both visual
choices are settled and implemented. Repository/native checks, the release
AX fixture workflow and native hover screenshots pass; see
[correction evidence](./evidence/026/error-correction/review.md). The repeat
human VoiceOver listening check passed on 2026-10-02: Imran confirmed
“Clear now” for the corrected controls and Query failed announcement. See
[human review](./evidence/026/voiceover-review.md).

The correction replaces conflicting shortcuts, adds visible keyboard guidance
and the selected error surface, and emits one explicit spoken terminal-error
announcement without stealing focus. Map server character positions through
the exact executed source range, preserve Unicode boundaries, clear diagnostics
on edits/new runs/reconnect, and retain the error above results when a source
position is unavailable or stale. Provide keyboard access to the same hover.
The automated native/AX/fixture checks and separate human listening gate pass
after correction. The remaining performance and combined window-race gates
also pass. Full evidence is recorded in
[evidence/026/implementation-review.md](./evidence/026/implementation-review.md).
No completion SHA exists; the plan is not DONE.
