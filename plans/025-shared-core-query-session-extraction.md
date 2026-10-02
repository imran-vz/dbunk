# Plan 025: Host-neutral core seam and Query Session service extraction

- Priority: P1. Effort: L. Risk: MEDIUM. Source: stage 02 of the
  [GPUI migration proposal](./gpui-migration.html), first slice.
- Planned against: `102568b`, 2026-10-02.
- Depends on: stage 00 of the proposal (decided 2026-10-02). Independent of
  Plan 024's result.
- Execution status: see [README.md](./README.md).
- Category: backend refactor. No user-visible behavior, wire contract or
  frontend file changes. The Tauri adapter keeps the sequence, ACK and credit
  mechanism exactly as it is.

> **Executor instructions**: read this plan completely before editing. Follow
> the steps in order and run each step's gate. Update the Plan 025 row in
> `plans/README.md` after each completed step. Stop on every STOP condition and
> report. Mark `READY FOR REVIEW` after all gates. Commits, pushes and PRs need
> separate authorization.
>
> **Drift check before Step 1**:
>
> ```sh
> git diff --stat 102568b -- src-tauri/src src-tauri/Cargo.toml \
>   src-tauri/build.rs justfile .github/workflows/ci.yml
> ```
>
> Expected on a fresh run: no output.

## Outcome and boundary

The Rust backend below the command layer no longer names Tauri, and a build
proves it. A host supplies three things instead: a Tokio runtime handle, an
event sink per stream, and lifecycle inputs (window identity, focus, document
replacement, teardown). The Query Session family is the first to have its
command-level logic (connection resolution, policy, audit) in a host-neutral
service that a second host can call.

This plan does not create `crates/dbunk-core/`, does not move files, does not
split any other family's commands, and adds no GPUI dependency.

## Why the proposal's stage 02 shape changes

The proposal expected families to move into `crates/dbunk-core` one at a time.
The code at `102568b` does not allow a crate boundary around one family:

- The modules below `commands/` form one strongly connected component.
  `query_session` uses `postgres` and `safety`; `postgres` uses
  `socket_lifecycle`, `safety`, `storage`, `seed` and `dispatch`;
  `storage` uses `table_browse` and `result_mutation` protocol types;
  `socket_lifecycle` uses `postgres`, `tunnel`, `table_browse`,
  `result_mutation` and `query_session`; `tunnel` uses `dispatch`; `dispatch`
  uses `redis`, `postgres` and `seed`.
- A Rust crate cannot depend back on the crate that depends on it, so moving
  one family means either moving the whole component or cutting it at
  submodule level and turning several hundred `pub(crate)` items `pub` for an
  interim state. The second widens the API that a native host could use to
  skip the safety checks, which the proposal forbids.
- The Tauri coupling below the command layer is small (about fifteen lines in
  fourteen files), so removing all of it is cheaper than cutting around it.

So the order is: decouple in place, prove it with a build that has no Tauri
dependency, split command logic into services family by family, and move the
directory once at the end as a mechanical step. This plan covers the
decoupling, the proof and the Query Session services. The remaining families
and the directory move are for a plan written after the stage 01 gate.

The proof is a Cargo feature. `tauri-host` (default) carries Tauri, its
plugins, the command wrappers and the entry point. `cargo check` and
`cargo test` with `--no-default-features` build everything else. A second host
can depend on the same library with `default-features = false` before the
directory move, so stage 03 is not blocked on it; it first has to choose which
services become public, because this plan exports nothing.

## Evidence

Read before implementation:

- `docs/adr/0021-dedicated-postgres-query-session-driver.md`,
  `docs/adr/0024-backend-enforced-production-safety-policy.md`,
  `docs/adr/0030-postgres-schema-comparison-foundation.md`,
  `docs/adr/0031-statement-scoped-query-session-execution.md`.
- `src-tauri/src/query_session/mod.rs`: `Outbox` (56-93) holds
  `Channel<QueryEventEnvelope>`; `start_monitor` (292-306) spawns on Tauri's
  runtime; `open` (375-448) takes the channel; the four send sites are
  `send_with_credit`, `send_terminal`, `send` and `close_session`
  (1160-1290). Tests build a recording `Channel` at 1837.
- `src-tauri/src/commands/query_session.rs`: `open_query_session`,
  `execute_query_session_inner` and `refresh_query_transaction_state` resolve
  the connection, build the policy and hook the audit. The other ten commands
  pass straight through to the manager.
- `src-tauri/src/commands/mod.rs`: `find_connection`,
  `with_active_connection`, `with_gated_active_connection` and
  `touch_connection_activity` take `&AppState` and are already Tauri-free.
- `src-tauri/src/lib.rs`: `AppState` (43-52), `test_app_state` (62-80),
  `setup` (326-360), `on_page_load` (361-365), `on_window_event` (366-387),
  exit cleanup (272-305, 586-631).
- Below-command coupling, complete list at `102568b`:
  - `tauri::async_runtime::spawn` in `start_monitor` of `query_session/mod.rs`,
    `table_browse/manager.rs`, `result_mutation/mod.rs`,
    `postgres/transfer/manager.rs`, `postgres/backup/manager.rs` and
    `postgres/schema_compare/manager/mod.rs`.
  - `tauri::ipc::Channel` in `query_session/mod.rs` and the test module
    `safety/live.rs`.
  - `AppHandle` and `Emitter` in `redis/pubsub.rs`, passed through
    `dispatch/keyvalue.rs`.
  - `AppHandle` path resolution in `storage.rs` (`Paths::from_app`,
    `resolve_config_dir`).
  - `tauri::webview::PageLoadEvent` in
    `postgres/schema_compare/manager/reads.rs`, `tests.rs` and
    `validation.rs`.
  - `tauri::ipc::Response` and two command attributes in `xlsx.rs`.
- Tests below the command layer that call command-level functions:
  `safety/live.rs`, `result_mutation/live.rs`,
  `postgres/transfer/runner_tests_live.rs`, `postgres/backup/runner.rs`,
  `postgres/schema_compare/manager/tests.rs` and `validation.rs`, and one test
  in `query_session/mod.rs`.

## Design

### Host seam (`src-tauri/src/host.rs`)

- `EventSink<T>`: `fn send(&self, event: T) -> Result<(), SinkClosed>`,
  `Send + Sync + 'static`, implemented for closures. Delivery is synchronous
  and must not block; a sink that cannot accept an event returns
  `SinkClosed`. Core code treats that exactly as it treats a failed
  `Channel::send` today.
- `SharedSink<T> = Arc<dyn EventSink<T>>`.
- The runtime is a `tokio::runtime::Handle` passed to each `start_monitor`.
  No global. The Tauri host passes `tauri::async_runtime::handle().inner()`.
- `DocumentLoad { Started, Finished }` replaces `PageLoadEvent` in the
  schema comparison manager. The Tauri host maps one to the other. A host
  with no document reload never sends `Started` after the first.

The seam carries no serialization. The Tauri adapter wraps a `Channel` and
serializes there, so the bytes on the wire do not change.

### Query Session service (`src-tauri/src/query_session/service.rs`)

Host-neutral functions over `&AppState` and a window label:
`open`, `execute`, `refresh_transaction_state`, `describe_parameters`.
They own connection resolution, the engine check, policy resolution, the audit
hook and the activity touch. The shared helpers they need move from
`commands/mod.rs` and `commands/safety.rs` to a Tauri-free module and are
re-exported so other command modules compile unchanged. The Tauri commands
become one-line adapters.

### Feature gate

`tauri`, `tauri-build`, the three `tauri-plugin-*` crates and `objc2-app-kit`
become optional behind `tauri-host`, which is a default feature. `commands`,
`run`, the logger plugin, exit lifecycle wiring and the traffic-light command
are `cfg(feature = "tauri-host")`. Tests below the command layer that call
command functions outside the Query Session family are gated on the same
feature until their family is split.

## Steps

### Step 1: Host seam and Query Session manager

1. Add `host.rs` with `EventSink`, `SinkClosed`, `SharedSink` and unit tests
   for the closure implementation.
2. `Outbox` holds `SharedSink<QueryEventEnvelope>`. `open` takes the sink.
   The four send sites map `SinkClosed` to the existing `Err(())`.
3. `start_monitor(&self, runtime: &tokio::runtime::Handle)`. Keep the
   regression test that it is callable with no ambient runtime.
4. `commands/query_session.rs` wraps the `Channel` in a sink.
5. Port the test helpers (`recording_channel`, `safety/live.rs`) to a
   recording sink that keeps the same JSON view of each envelope.

Gate: `just fmt`, `just lint`, `just test` pass with the same test count as
the baseline run. `grep -n "tauri" src-tauri/src/query_session/*.rs` shows
comments only.

### Step 2: Query Session service

1. Move the shared connection helpers below the command layer.
2. Add `query_session/service.rs` and move the logic out of the three
   commands that have any. Point `safety/live.rs` and the describe test at the
   service.
3. Keep `execute_query_session_inner`'s behavior: policy is resolved from the
   hydrated record, the audit runs only after success and only when the
   authorization requires it.

Gate: the checks above. The `safety::live` tests are `#[ignore]` without a
fixture; run them against the disposable PostgreSQL fixture on 15432 if it can
be started, and record whether they ran.

### Step 3: Remaining below-command coupling

1. Five other `start_monitor` functions take the runtime handle.
2. `Paths::from_app` leaves `storage.rs`. `storage.rs` keeps `Paths::from_dir`
   and a host-neutral `default_config_dir(home_or_appdata)` rule; the Tauri
   host resolves the base directory and applies the debug-only
   `DBUNK_DEV_CONFIG_DIR` override at the edge. The resolved paths must be
   byte-identical to today's on every platform.
3. `redis/pubsub.rs` takes a `SharedSink` for its envelope.
   `commands/keyvalue.rs` builds the sink from `AppHandle::emit` with the same
   event name.
4. `DocumentLoad` replaces `PageLoadEvent` in the comparison manager and its
   tests. `lib.rs` maps the Tauri event.
5. `xlsx.rs` returns bytes; the two commands and `ipc::Response` move to
   `commands/xlsx.rs`. The handler list keeps the same command names.

Gate: `grep -rn "tauri::" src-tauri/src | grep -v "^src-tauri/src/commands/" |
grep -v "^src-tauri/src/lib.rs"` prints nothing. Checks pass.

### Step 4: Feature gate and proof

1. Make the Tauri dependencies optional behind the default `tauri-host`
   feature. Gate `commands`, the entry point and the affected tests.
2. `AppState` construction becomes one host-neutral function used by both the
   Tauri setup and `test_app_state`.
3. Add `just lint-core` and `just test-core` (`--no-default-features`) and
   run them from `just lint` and `just test`. Add the same two commands to the
   Rust job in `.github/workflows/ci.yml`.

Gate: `cargo tree --manifest-path src-tauri/Cargo.toml --no-default-features
-e normal | grep -ci tauri` prints `0`. `just fmt`, `just lint`, `just test`
pass. `cargo build --manifest-path src-tauri/Cargo.toml --features
tauri/custom-protocol` succeeds, which is the feature set `tauri build` uses.

### Step 5: Failure-mode coverage against a non-Tauri sink

Add tests only where no existing test covers the case through the sink:

- Delivery failure: the sink returns `SinkClosed` on a row batch, on the
  terminal event and on the first `SessionState`. The credit window does not
  keep the failed entry; the session is removed; the connection is released.
- Backpressure: with a full credit window, a Stop is still accepted and
  seen at the driver's next checkpoint, and a close ends the parked execution
  task (the Plan 023 credit-loop repair, now asserted through the seam).
- Owner replacement closes the old owner's sessions and delivers nothing
  further to the old sink.
- Focus lease: an unfocused window's sessions are not expired by the monitor.
- Shutdown: `begin_global_teardown` returns only after sessions are closed
  and refuses later opens.

Gate: checks pass. New live tests are `#[ignore]` with the fixture named in
the attribute, like the existing ones.

### Step 6: Record

1. ADR-0032: the host seam, the feature gate as the interim proof, and the
   reason the crate move is deferred.
2. Update `CONTEXT.md` only if a new domain term is introduced.
3. `pnpm format`, `pnpm lint`, `pnpm typecheck` pass (no frontend change is
   expected; they are the repository gate).
4. Execution record at the end of this file; README row to `READY FOR
   REVIEW`.

## STOP conditions

- An ADR-0024 safety check would move without a test proving it still refuses
  before dispatch.
- Any Query Session event's JSON shape, sequence numbering or ACK behavior
  would change.
- `just test` loses tests compared with the baseline count, other than tests
  renamed by this plan.
- The config directory resolved by the Tauri host would differ from today's.
- A step needs a daily-driver profile, a keychain entry or a live database.
- The feature gate cannot be made to work with `tauri build`'s feature set.

## Deferred work

- Service extraction for every other family, and the 177 command wrappers
  becoming one-line adapters.
- The move to `crates/dbunk-core/` and the Cargo workspace.
- Amendments to ADR-0007, ADR-0011, ADR-0021 and ADR-0030 for native hosts.
- An in-process bounded sink for the native host (stage 03 decides).

## Execution record (2026-10-02)

Run against `102568b` plus this working tree. Nothing is committed. All six
steps are complete.

**Delivered**

- `host.rs`: `EventSink`, `SinkClosed`, `SharedSink`, `DocumentLoad`.
- `app.rs`: `AppState` with one constructor and `start_monitors`, the shared
  connection helpers, and exit cleanup, all moved out of `lib.rs` and
  `commands/mod.rs`.
- `query_session/service.rs`: all thirteen Query Session operations.
  `commands/query_session.rs` is thirteen one-line adapters.
- `connections.rs`: the connection service (moved from
  `commands/connections.rs`, which is now eight adapters).
- `safety/gate.rs`: the policy gate helpers and their tests, moved from
  `commands/safety.rs`.
- `tauri_host.rs`: window chrome, logger, exit lifecycle and `run`, moved
  from `lib.rs`. `commands/xlsx.rs`: the two XLSX commands.
- `tauri-host` (default) and `isolated-profile` features; `just lint-core`,
  `just test-core`; the same two commands in CI.
- ADR-0032; `CONTEXT.md` entries for Host, Event Sink and Service.

**Departures from the plan as written**

- The connection service was extracted too. The Query Session safety tests
  save their fixture connection through it, and gating them behind the Tauri
  feature would have left the family's safety proof out of the core build.
- The entry point moved to `tauri_host.rs` instead of being gated item by
  item in `lib.rs`.
- `Paths::resolve(host_default)` takes the host's default as a closure. The
  plan's `default_config_dir(home_or_appdata)` could not stay byte-identical:
  on Windows the directory is under Tauri's `AppData`, which includes the
  bundle identifier.
- `send` and `send_terminal` take `&Outbox` instead of `&Session`. They only
  ever used outbox fields; the change lets them be tested without a database.
- The build without Tauri allows `dead_code` and `unused_imports`
  (ADR-0032, Consequences).
- Tokio's `rt-multi-thread` is now requested explicitly. The build without
  Tauri did not compile without it: the SSH tunnel uses `block_in_place`, and
  Tauri had been enabling the feature through unification.

**Evidence**, macOS 27.0.1, Rust 1.97.1, disposable PostgreSQL 16 fixture
(`pnpm db:postgres`, port 15432):

- `just fmt`, `just lint`, `just test`: default build 657 passed, 85
  ignored; build without Tauri 633 passed, 70 ignored. Baseline: 651 passed,
  79 ignored (730). Now 742: the baseline plus twelve new tests, none lost.
- `pnpm format`, `pnpm lint`, `pnpm typecheck` pass. No frontend file changed.
- `cargo tree --no-default-features` lists 362 crates and none of Tauri, Wry,
  Tao or WebKit; the default graph has 555.
- `cargo build --features tauri/custom-protocol` succeeds.
- Live, build without Tauri: 17 `query_session_actor_live` tests and both
  `safety::live` tests pass, run in parallel.
- Step 1 and Step 3 greps print nothing: no `tauri::` outside `commands/`
  and `tauri_host.rs`.

**Independent review**, one pass on the finished diff, 2026-10-02. No
behavior drift found in the adapters, the event path, startup and shutdown,
the config directory or the feature gate. Its findings, all addressed:

- A staged rename left the index inconsistent. Unstaged; nothing is staged.
- The plan and ADR claimed a second host could already call the library.
  Corrected: nothing is exported yet.
- The global-teardown test could pass with an unawaited close. It now holds
  the session and asserts it is closed when the call returns.
- One assertion in the backpressure test could not fail, and a regression
  would have hung it. It now yields before asserting and bounds the Stop with
  a timeout.
- Delivery failure on the first event and on the terminal event were
  untested. Two live tests added.
- The storage override test assumed a debug build. It now asserts the
  optimized-build behavior as well.

**Not run**

- The `tauri` CLI itself (`tauri dev`, and `tauri build` with bundling).
  Plan 024 built two release binaries with plain `cargo build`, one with
  `tauri/custom-protocol` and the frontend embedded, and ran both: the app
  started on an isolated profile, connected to the fixture, and ran the three
  fixture reads (10,000, 400 and 10,000 rows) through a Query Session and
  the real IPC channel. That covers the refactored startup, the channel sink and the
  credit loop in a release build, but not the CLI's own build steps.
- Windows and Linux. CI covers Linux when this is pushed.
- The SSH-tunnel route (no fixture), as in Plan 023.

**Open for the reviewer**

Stage 03 follow-up review, 2026-10-02:
[implementation review and fresh checks](./evidence/025/stage03-review.md).
No extraction regression found. The global-teardown test proves sessions are
logically closed before return and database sessions disappear eventually;
it does not prove execution/driver tasks have joined at return. Window close
also does not fence in-flight opens. Plan 026 owns the stronger native-only
lifecycle barrier; this does not change the Tauri contract or mark Plan 025 DONE.

- Process exit uses `close_all`, which does not refuse later opens. That is
  how it was before this plan; noted in ADR-0032.
- `query_session_actor_live_an_unfocused_window_keeps_its_lease` needs the
  machine to have been up for more than two minutes.
