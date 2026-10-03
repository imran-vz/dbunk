# ADR-0032: Host-neutral backend seam

**Status**: Accepted and implemented for the seam, the Query Session and
connection services, and the build proof (Plan 025, 2026-10-02). The other
service families and the move into `crates/dbunk-core` are not done. Part of
the GPUI migration (`plans/gpui-migration.html`, stage 02); it stands on its
own if the migration stops.

## Problem

The backend could only run inside Tauri. Below the command layer it named
Tauri in fourteen files: six managers spawned their monitors on Tauri's
runtime, Query Sessions delivered events through `tauri::ipc::Channel`, Redis
pub/sub emitted through `AppHandle`, storage resolved its directory from
`AppHandle`, schema comparison took `PageLoadEvent`, and XLSX export returned
`ipc::Response`. Connection resolution, the safety policy and the audit for a
Query Session execution lived inside `#[tauri::command]` functions.

A second desktop host needs the same backend with the same safety checks, and
the backend's tests should not need a UI framework to compile.

## Decision

### A host supplies three things

`src-tauri/src/host.rs` is the whole seam.

- **A Tokio runtime handle**, passed to `AppState::start_monitors`. No global
  runtime. The runtime must be multi-threaded: the SSH tunnel uses
  `block_in_place`. Tauri used to enable Tokio's `rt-multi-thread` for us
  through feature unification; the crate now asks for it itself.
- **An event sink per stream**: `EventSink<T>`, `send(&self, T) ->
  Result<(), SinkClosed>`. Delivery is synchronous and must not block. A sink
  that cannot take an event returns `SinkClosed`, which the backend treats as
  a lost stream. The sink carries typed events; a host that serializes does it
  inside the sink. Bounded retention stays with the sender (the Query Session
  credit window), not the sink.
- **Lifecycle inputs** through the managers' existing methods: window label,
  focus, teardown, and `DocumentLoad` for a host whose document can be
  replaced under a living window.

### Services are what a host calls

`query_session::service` and `connections` are host-neutral functions over
`&AppState` and a window label. They own connection resolution, the engine
check, policy resolution from the hydrated record, the audit hook and the
activity touch. The Tauri commands for those two families are one-line
adapters. The policy gate helpers moved to `safety::gate` with their tests.

A host calls the service, never the manager: `QuerySessionManager::execute`
takes the policy as an argument, and only the service guarantees it comes from
the stored connection.

Plan 028 backend preparation extracts `table_browse::service` and
`result_mutation::service` with the same rule. Table paging/counts, preferences,
analysis, preview/apply and virtual-key operations keep their Tauri signatures;
the prior browse execution module is now named `runner`. Successful activity,
required-override auditing and storage-error redaction remain in the services.
This extraction alone does not activate native data tools: owned document
admission and joined executor/socket lifecycle are separate gates.

### The Tauri adapter keeps the wire protocol

`commands::channel_sink` wraps a `Channel` and serializes there. Sequence
numbers, cumulative ACKs, the credit window and every event's JSON are
unchanged. Whether a native host keeps the ACK and credit mechanism or uses a
bounded in-process channel is decided when that host exists, and only after
the same failure-mode tests pass against it.

### Initial stage 04 settings extraction

Plan 027's initial Step 1 work also extracts `settings`: app settings,
credential configure/unlock/change/reset, and namespaced UI-state persistence.
The Tauri adapters keep their command names and JSON. Credential mutation
fences remain in the service and are tested without Tauri.

Credential consumers now carry an explicit profile context, including SSH,
managed-server rollback and schema-comparison workers. The context binds its
SQLite pool, encryption key, password cache, mutation lock and Keychain store.
The default Tauri context retains the ordinary single Keychain entry and its
legacy read-error policy. The stage03 fixture has no OS Keychain capability.
SQLite credential rewrites are transactional; SQLite-only profile lifecycle
changes commit credentials, verifier and settings together before publishing
the new key/cache. Injected failures preserve the prior working SQLite state.

The stage04 constructor now uses a separate versioned marker, canonical private
path, exclusive lock, app-generated namespace and launcher-verified fixture
manifest. A read-only SQLite identity check precedes migrations or credential
construction. The process cannot switch stage04 profiles. Native SQLite pools
use FULL synchronous commits; default Tauri pools retain their existing setting.
The bounded backend facade exposes redacted SQLite credential settings and
lifecycle operations. It rejects corrupt onboarding state and unsupported modes
before credential access; a confirmed reset preserves metadata and drafts.

Plan 030 adds a separate explicit general PostgreSQL profile constructor with
its own `.dbunk-native-profile` marker and SQLite identity. It reuses the native
credential lifecycle with an independent UUID namespace, private path and
exclusive lock. Endpoint authority is an enum: existing fixture profiles retain
their immutable manifests; general profiles admit supported direct PostgreSQL
connections only after the same credential, policy and document admission checks.
Opening never adopts an unmarked legacy profile or falls back to a default path.
The native entry point separates create from open and restores disconnected state.
Normal and stage04 profiles share the process-selection guard; stage03's original
constructor remains separate. This implements the explicit capability requested
by Plan 030, not production identity, profile import or daily-driver cutover.
See [current source evidence](../../plans/evidence/030/general-profile-source-checks/README.md).

Plan 027's subsequent Step 1 implementation adds native Keychain mode changes
with a secret-free SQLite recovery journal and namespace-scoped primary and
rollback entries. Failed cross-store changes retain recovery state rather than
claiming an empty store or successful conversion. Native admission serializes
credential/metadata changes with connection startup. Injected process tests
cover failed writes, recovery and namespace selection; SQLite-only lifecycle
paths never construct a Keychain entry. A disposable real OS Keychain CLI probe
also passes separate-process reopen, conversion and reset, with both owned
entries absent after cleanup. The subsequent
[packaged GPUI checks](../../plans/evidence/027/workspace-package-20261002/agent-verification.md)
also pass Keychain onboarding, blank-password edit preservation, separate-process
reopen, encrypted SQLite conversion/unlock, conversion back to Keychain, and
confirmed reset preserving SQL and connection metadata. All four launches join
shutdown and return fixture activity to baseline; scoped cleanup verifies both
entries absent.

[Recovery window checks](../../plans/evidence/027/workspace-recovery-20261002/agent-verification.md)
verify that corrupt/future records survive ordinary quit unchanged and that an
oversized current draft can be exported exactly before explicit discard without
replacing its last durable version. The
[workspace performance capture](../../plans/evidence/027/workspace-performance-b/summary.json)
records one/four-session diagnostics and 20 open/run/close cycles; it uses an
earlier binary than these recovery fixes.

These checks do not complete Plan 027 or establish full PostgreSQL parity.
VoiceOver is explicitly deferred and non-blocking under the current scope.
Keyboard/AX, real IME in remaining controls and actual-window race/recovery
gates retain their scoped pending status.
The [frozen package source manifest](../../plans/evidence/027/workspace-package-20261002/source-manifest.json)
predates Plan 028 backend preparation, so its window evidence does not validate
that newer source. Current scope and remaining gates are in the
[workspace progress record](../../plans/evidence/027/step02-workspace-progress.md).

### The proof is a build without Tauri

The `tauri-host` Cargo feature is on by default and carries Tauri, its
plugins, `commands` and `tauri_host` (window chrome, logger, exit lifecycle,
entry point). `just lint` and `just test` also run with
`--no-default-features`, where the dependency graph has no Tauri crate (362
crates against 555). CI runs both.

### Not a crate yet

The migration proposal expected one family at a time to move into
`crates/dbunk-core`. The modules below `commands` form one strongly connected
component (`query_session` → `postgres` → `socket_lifecycle` → `table_browse`,
`result_mutation`, `query_session`; `storage` → `table_browse`,
`result_mutation`; `tunnel` → `dispatch` → `redis`, `postgres`), so a crate
boundary around one family would have to cut it at submodule level and turn
several hundred `pub(crate)` items `pub`. That would hand a second host the
lower-level operations that skip the safety checks.

So the code is decoupled in place and moves once, when every family's command
logic is a service. At the end of Plan 025, a second host could depend on this library with
`default-features = false`, but could not call it: every module remained
private and the seam and services were `pub(crate)`. Plan 026 adds the explicit
public surface described below; it does not expose the lower-level modules.

### Stage 03 opt-in native facade

Plan 026 adds `backend` behind the non-default `isolated-profile` feature.
Its opaque `Backend` accepts only a private, marked fixture profile and runs
Query Session operations through the existing services. Public event/request
DTOs preserve the existing protocol; managers, storage, hydrated connections,
credentials and policy internals remain private. Layout persistence and the
pure statement-selection helper are narrow operations on this same boundary.

The native facade opts into owned task/driver tracking, closes admission before
shutdown, retires owners and awaits cleanup before runtime termination. The
ordinary Tauri manager construction does not enable that lifecycle. Tauri's
wire format, sequence/ACK/credit behavior and service safety enforcement remain
unchanged. PlainSqlite fixture initialization does not call credential migration
or Keychain. This is an uncommitted stage 03 addition with separate verification
in `plans/evidence/026/`, not retroactive Plan 025 completion evidence.

Plan 027 found that fresh fixture seeding indirectly called ordinary
credential cleanup through `connections::save`, contradicting the no-Keychain
claim above. Fresh seeding now writes only its validated SQLite fixture
records. A recording-keyring test in a separate process proves no Keychain
entry is selected on fresh open or reopen. The public fixture constructor
and profile guard are unchanged. See
`plans/evidence/027/step01-progress.md` for the correction and red/green proof.

## Consequences

- `--no-default-features` builds allow `dead_code` and `unused_imports`:
  families whose logic still lives in `commands` leave their backend functions
  without a caller in that build. The default build keeps both lints. The
  allowance goes when the last family is extracted.
- `postgres::transfer::runner_tests_live` still requires `tauri-host`.
  Plan 028 moves `result_mutation::live::safety_live_apply_strict_confirmation_and_audit`
  to the shared service, so it also compiles without Tauri.
- `isolated-profile` is a non-default feature that honors
  `DBUNK_DEV_CONFIG_DIR` in optimized builds, for measurement against a
  disposable profile. Release artefacts are built without it and always use
  the normal directory. The override rule is in `storage.rs`; the platform
  default is resolved by the host (`tauri_host::default_config_dir`), exactly
  as before.
- A failed pub/sub delivery is logged by the host's sink. The message stays
  in the session buffer for `drain`, as before.

## Evidence

2026-10-02, macOS, Rust 1.97.1, disposable PostgreSQL 16 fixture on 15432.

- `just fmt`, `just lint`, `just test`: default build 657 passed, 85 ignored;
  build without Tauri 633 passed, 70 ignored. The baseline at `102568b` was
  651 passed, 79 ignored.
- Live, in the build without Tauri: 17 `query_session_actor_live` tests and
  both `safety::live` tests pass, including the six added here (a sink that
  refuses the first event, a row batch or the terminal event; owner
  replacement; the focus lease; global teardown).
- Three of the new tests were checked by breaking the behavior they guard
  (the credit entry of a refused batch, the cleanup after a refused first
  event, the awaited close in global teardown); each failed, and the code was
  restored.
- `cargo build --features tauri/custom-protocol` succeeds, which is the
  feature set `tauri build` uses.

- Plan 024 ran two release builds of the Tauri app (one with
  `tauri/custom-protocol`) on an isolated profile: startup, a fixture
  connection, and Query Session reads through the real IPC channel all
  worked.

Not run: the `tauri` CLI (`tauri dev`, `tauri build` with bundling), Windows
and Linux, the SSH-tunnel route.

Observed, unchanged: process exit calls `close_all`, which does not set the
global closing flag, so only `begin_global_teardown` refuses later opens.
The teardown evidence proves logical closure followed by eventual socket
release, not a join of every opening/execution/driver task before return.
Likewise, `close_window` does not retire the owner of an in-flight open.
Plan 026 must provide an opt-in native lifecycle barrier without changing
these Tauri behaviors. See `plans/evidence/025/stage03-review.md`.
