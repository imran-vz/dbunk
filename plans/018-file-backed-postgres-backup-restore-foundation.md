# Plan 018: File-backed PostgreSQL backup and restore foundation (dark)

> **Executor instructions**: Do not start until Plan 017 is `DONE` in
> `plans/README.md`. Read this plan completely before editing. This is a dark
> backend plan: do not add a backup/restore UI or file picker. Run the focused
> verification after each step and the complete repository gates in Step 5.
> Update this plan's README row after every completed step and mark `READY FOR
> REVIEW` only after the disposable-PostgreSQL round trip and every gate pass.
> A reviewer/operator records `DONE: <completion SHA>` after an authorized
> commit.
>
> **Fresh-start drift check**:
>
> ```sh
> git diff --stat 25d36f1..HEAD -- \
>   src-tauri/src/postgres/ddl.rs \
>   src-tauri/src/postgres/mod.rs \
>   src-tauri/src/commands/relational.rs \
>   src-tauri/src/commands/mod.rs \
>   src-tauri/src/dispatch.rs \
>   src-tauri/src/dispatch/relational.rs \
>   src-tauri/src/lib.rs \
>   src-tauri/src/socket_lifecycle.rs \
>   src-tauri/src/types.rs \
>   src-tauri/src/docker.rs \
>   src-tauri/src/postgres/tls.rs \
>   src-tauri/src/commands/safety.rs \
>   src-tauri/Cargo.toml \
>   src/components/safety-confirm-dialog.test.tsx \
>   CONTEXT.md ROADMAP.md plans/parity-gap-register.md plans/README.md
> git status --short -- \
>   src src-tauri CONTEXT.md ROADMAP.md plans/parity-gap-register.md plans/README.md
> rg -n "run_pg_dump|run_pg_restore|PgDumpPayload|PgRestorePayload|data_base64" \
>   src src-tauri
> rg -n "begin_connection_teardown|begin_global_teardown" \
>   src-tauri/src/socket_lifecycle.rs
> rg -n "close_all|ExitRequested" src-tauri/src/lib.rs
> rg -n "FALLBACK_PATHS|fn docker_binary" src-tauri/src/docker.rs
> ```
>
> Expected on a fresh run: the first command is empty, and status output is
> limited to this plan, `plans/README.md`, and any operator-owned planning
> artifacts. The first `rg` finds only the legacy Rust path and its tests; it
> must not find a live frontend caller. The last two `rg` commands must find
> the exit-time `close_all` join and the Docker binary discovery helper this
> plan mirrors. A load-bearing mismatch with the
> contracts or excerpts below is a **STOP** condition. Re-plan from current
> source rather than guessing.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH (native subprocesses handle credentials and user-selected
  files; restore is destructive; disconnect must not tear down an SSH tunnel
  while a child process still uses it; unbounded output or base64 buffering can
  exhaust memory)
- **Depends on**: Plan 017 complete
- **Category**: direction
- **Planned at**: commit `25d36f1`, 2026-09-05
- **Gap**: `PAR-010` in `plans/parity-gap-register.md`
- **Follow-ups**: "Plan 019" (UI activation) and "Plan 020" (bounded table
  import/export) are intended successors that have not been written. Every
  reference below names that intent, not an existing document.

## Why this matters

dbunk already contains PostgreSQL `pg_dump`, `psql`, and `pg_restore` code,
but it is not a usable product capability. The current command contract moves
the complete archive through Tauri as base64, runs blocking process and file
I/O on the command path, has no cancellation or bounded error capture, and is
not wired to a UI. A large database can therefore consume several copies of
the archive in memory, and disconnecting an SSH-backed connection can remove
the tunnel while a native tool is still running.

This plan replaces that inaccessible path with a typed, asynchronous,
file-backed PostgreSQL job foundation. It makes backup output crash-safe,
keeps restore behind the existing backend safety policy, bounds retained
process output and job history, and joins the existing connection teardown
fence. Plan 019 can then add the user-facing picker and progress UI without
inventing process or safety semantics in React.

## Current state verified at `25d36f1`

- `src-tauri/src/postgres/ddl.rs:1-73` builds a synchronous
  `std::process::Command`, applies the resolved libpq/TLS environment and
  `PGPASSWORD`, and pipes stdout/stderr. `command_error` turns the complete
  captured stderr into a string.
- `src-tauri/src/postgres/ddl.rs:353-454` calls `Command::output()` for backup,
  returns the complete stdout as base64, decodes the complete restore payload,
  writes it synchronously to `psql` stdin or a temporary file, and waits with
  no cancellation. Its plain restore maps `clean` to
  `--single-transaction`; that is not a clean-before-restore operation.
- `src-tauri/src/types.rs:1271-1283` exposes `PgDumpResult { data_base64,
  extension, runtime_ms }` and `PgRestoreResult { runtime_ms }`;
  `:1721-1741` exposes stringly scoped dump and base64 restore payloads.
- `src-tauri/src/commands/relational.rs:301-356` runs backup through
  `with_active_connection` and restore through
  `with_gated_active_connection(WriteIntent::Restore)`. That helper audits and
  touches the connection when its future returns, so it cannot simply wrap a
  new command that returns immediately after job admission.
- `src-tauri/src/dispatch.rs:209-236` and
  `src-tauri/src/dispatch/relational.rs:941-974` retain legacy wrappers. The
  repository-wide search finds no frontend invocation, so this plan may remove
  the unsafe base64 contract rather than preserve it.
- `src-tauri/src/lib.rs:41-47` owns the pool, query-session,
  result-mutation, and table-browse managers. The Tauri handler still
  registers `run_pg_dump` and `run_pg_restore`.
- `src-tauri/src/socket_lifecycle.rs` fences query sessions, table browsing,
  and result mutations before pool/tunnel invalidation. Native PostgreSQL jobs
  are absent from every connection/global fence.
- `src-tauri/src/commands/mod.rs:49-59` hydrates credentials and resolves SSH
  tunnels before returning a connection. The new runner must continue to use
  that resolved connection and the shared libpq TLS material.
- `src-tauri/Cargo.toml` already enables Tokio `process`; filesystem and
  async-I/O features may be added narrowly if the chosen implementation needs
  them. `tempfile` is already present.
- `src-tauri/src/table_browse/manager.rs` is the local model for admission,
  connection/global closing fences, cancellation, bounded teardown waits, and
  monitor cleanup. `src-tauri/src/query_session/mod.rs` is the local model for
  running a completion hook only after successful work.
- `src-tauri/src/socket_lifecycle.rs:12-83` fences are infallible: every
  `begin_*_teardown` returns `()`, and each manager swallows its own
  `CLOSE_TIMEOUT` (for example `table_browse/manager.rs:133`). There is no
  rollback path, and the seven callers in
  `commands/{connections,bastions,managed,settings}.rs` cannot observe a
  teardown failure. `save_connection_inner` fences every save, including
  cosmetic edits.
- `src-tauri/src/lib.rs:461-484` handles `RunEvent::ExitRequested` by joining
  `close_all` on the three managers under a three-second timeout. App exit is
  a normal shutdown path separate from the global fence.
- `src-tauri/src/docker.rs:15-36` resolves the `docker` binary by scanning
  `PATH` and then fixed fallback directories, because Finder-launched macOS
  apps receive a minimal `PATH`. The PostgreSQL client tools have the same
  problem and no equivalent resolver today.
- `src-tauri/src/postgres/tls.rs:605` `apply_to_command` takes
  `&mut std::process::Command`, not a Tokio command.
- `src-tauri/src/commands/relational.rs:903` `LegacyCommand::ALL` enumerates
  twelve string-tag gated commands including `RunPgRestore`, and
  `src-tauri/src/commands/safety.rs:113` lists `run_pg_restore` in the legacy
  `write_intents` table. Both prove the string-tag contract that the new typed
  command will not use. `commands/safety.rs:14-28` exposes
  `assert_legacy_permitted` and `record_override`; there is no separate
  audit-entry writer.
- `src-tauri/src/commands/relational.rs:505` and `redis/pubsub.rs:131` emit
  Tauri events. The query-session, table-browse, and result-mutation managers
  are polled through typed commands instead.

## Decided architecture

### 1. One PostgreSQL-specific job boundary

Add `src-tauri/src/postgres/backup/` with:

- `protocol.rs` for serde payloads, snapshots, phases, and tagged errors;
- `manager.rs` for admission, state transitions, cancellation, retention, and
  teardown fences;
- `runner.rs` for the native-tool preflight and invocation; and
- `mod.rs` for the narrow module surface.

The implementation type is `PgToolJobManager` because one lifecycle owns both
backup and restore. Do not extract a generic background-job framework in this
plan. Plan 020 may reuse the pattern for import/export; only extract shared
machinery after the second concrete use proves the boundary.

Add `src-tauri/src/commands/pg_backup.rs` with exactly these commands:

- `start_pg_backup`
- `start_pg_restore`
- `get_pg_tool_job`
- `list_pg_tool_jobs`
- `cancel_pg_tool_job`
- `release_pg_tool_job`

The two start commands return the admitted job snapshot immediately. Get
returns the latest snapshot. List returns every retained snapshot, optionally
filtered by `connection_id`, so a caller that lost its job ids (or wants to
warn before editing a connection) can recover state. Cancel is idempotent for
terminal jobs and moves an active job toward `cancelled`. Release is idempotent
and removes only a terminal record; releasing an active job returns
`jobActive`.

Progress is observed by polling `get_pg_tool_job` / `list_pg_tool_jobs`. This
plan emits no Tauri events; that is the contract Plan 019 consumes, and it
must not add ad hoc events on top of it.

### 2. Typed protocol and state machine

`StartPgBackupPayload` contains `connection_id`, an absolute
`destination_path`, `format`, `clean`, and a tagged scope enum:

```text
PgBackupScope = database
              | schema { schema }
              | table { schema, table }
PgBackupFormat = plain | custom
```

`clean` on backup is accepted only for plain format and adds
`--clean --if-exists` to `pg_dump`, so the SQL archive drops and re-creates
objects when restored. Custom-format backup plus `clean: true` is rejected as
`invalidRequest`, because clean is a restore-time option for custom archives.

`StartPgRestorePayload` contains `connection_id`, an absolute `source_path`,
`format`, `clean`, and `confirmed`. `clean` is accepted only for custom-format
restore in v1. Plain SQL plus `clean: true` is rejected as `invalidRequest`;
do not silently reinterpret it. The two rules are symmetric by design: a plain
archive carries its cleanup inside the SQL when the backup asked for it, and a
custom archive gets it at restore time. Without either, restoring into a
database that already holds the objects fails under `ON_ERROR_STOP`; that is
expected, and ADR-0028 must say so.

`PgToolJobSnapshot` includes `job_id`, `connection_id`, `kind` (`backup` or
`restore`), `format`, `file_name` (not the full local path), `phase`,
`started_at`, `finished_at`, `bytes_processed`, `total_bytes`, `tool_version`
(the trimmed preflight `--version` line, absent until preflight succeeds), and
an optional tagged failure. Phases are:

```text
queued -> preflight -> running -> finalizing -> completed
   |          |           |           |
   +----------+-----------+-----------> cancelling -> cancelled
              +-----------+-----------> failed
                                                completed -> release/expiry
```

Cancellation is observable: an accepted cancellation moves a non-terminal job
to `cancelling`, and only a reaped child plus completed cleanup moves it to
`cancelled`. If final rename already won the race, the job is `completed` and
cancel is an idempotent no-op. The manager must serialize that boundary so one
job cannot publish both outcomes.

`cancelling` is never a resting state. If the child is not reaped within the
five-second kill wait, the manager still transitions the job to `cancelled`,
attaches `timeout { operation: "reap" }` as its failure, frees the
per-connection and global admission slots, and detaches a reaper task that
keeps waiting on the child and removes the partial file once it exits. A job
must never hold a connection's single admission slot past that bound, because
nothing else can free it before app restart.

`bytes_processed` is the partial file size for backup. Restore sets
`total_bytes` to the source size and leaves `bytes_processed` absent because
the native tools do not expose trustworthy progress. The frontend must never
derive a fake percentage for restore.

Errors are a serde-tagged enum, not string matching. It must distinguish at
least: `unsupportedEngine`, `invalidRequest { field, reason }`,
`connectionClosing`, `jobLimitReached`, `jobNotFound`, `jobActive`,
`destinationExists`, `toolUnavailable { tool }`,
`toolFailed { tool, exitCode, message }`, `io { operation, message }`,
`timeout { operation }`, `policyBlocked { reason }`,
`policyNeedsConfirmation { statements }`, and `cancelled`. Reuse
`StatementClassSummary` for the safety statement details, matching the typed
query-session and result-mutation contracts; do not flatten policy failures to
text.

Allowed transitions are centralized in the manager and asserted in tests.
Terminal snapshots are kept for one hour, with at most 32 terminal records.
The manager evicts the oldest terminal record when over capacity and runs a
one-minute expiry monitor. It never evicts active work.

Admission is deliberately conservative: at most four active PostgreSQL tool
jobs process-wide and one active job per connection. Admission fails rather
than queues when either limit is reached. This makes file and subprocess use
predictable and avoids holding a resolved SSH tunnel for work that has not
started.

The hydrated/resolved connection and its credential material live only in the
active work item. On any terminal transition they are dropped and only the
credential-free snapshot remains in retention.

### 3. File and native-process contract

All production invocations use `tokio::process::Command` directly; never a
shell. Continue to apply the existing resolved host/port, credential, and
libpq TLS environment. Set `kill_on_drop(true)`. Never log the command
environment, password, complete argument vector, or absolute user path.

Resolve each tool binary the way `docker.rs` resolves `docker`: scan `PATH`,
then probe fixed fallback directories, and only then fall back to the bare
name. The fallback list must include `/opt/homebrew/bin`, `/usr/local/bin`,
`/usr/bin`, `/Applications/Postgres.app/Contents/Versions/latest/bin`, and on
Linux the `/usr/lib/postgresql/<version>/bin` directories with the highest
version first. Resolution is a pure, tested helper. The resolved tool path may
appear in logs; user-selected file paths may not.

Before the operation, run the resolved binary with `--version` as its **only**
argument under a 10-second deadline, and return `toolUnavailable` or a bounded
typed failure. The PostgreSQL client tools honour `--version` only as the
first argument, so preflight must not go through the connection-argument
builder. Preflight proves presence and executability, not connectivity. Store
the trimmed version line in the snapshot as `tool_version` so client/server
skew (for example the `\restrict` meta-commands that newer `pg_dump` releases
emit and older `psql` rejects) is diagnosable without a negotiation feature.

There is no overall backup/restore deadline: large databases are legitimate.
Cancellation calls Tokio's `start_kill`, then waits up to five seconds for the
child to be reaped. On timeout apply the §2 rule: the job becomes `cancelled`
with `timeout { operation: "reap" }`, the slots are freed, and a detached task
finishes the wait and the partial cleanup. Tokio's orphan reaper already
handles dropped children on Unix, so a dropped handle is not a zombie risk;
the detached task exists so the partial file is still removed.

Pipe stderr and drain it concurrently for the entire child lifetime so a full
pipe cannot deadlock the process. Retain at most the final 64 KiB for a
sanitized error message while continuing to drain and discard overflow.
Stdout is null for file-backed operations and preflight output that is not
needed. Do not place `PGPASSWORD`, TLS key material, or raw database output in
tracing fields or returned errors.

Backup validation and execution:

1. Require an absolute destination, an existing directory parent, and a final
   path that does not exist. V1 refuses overwrite with `destinationExists`.
2. Create a unique sibling `.<name>.dbunk-partial-<job-id>` with create-new
   semantics before spawning. Pass that path to `pg_dump --file` with
   `--format=plain|custom`, `--clean --if-exists` when a plain backup requested
   `clean`, and the typed `--schema` or `--table` selector.
   Serialize schema/table identifiers as literal pg_dump patterns: quote each
   identifier component and escape embedded quotes so names containing pattern
   metacharacters cannot expand to additional objects. Never concatenate raw
   identifiers into a pg_dump pattern.
3. Poll partial-file metadata while running to expose `bytes_processed`.
4. After exit success, flush/sync the partial file as supported, verify it is
   a non-empty regular file, then atomically rename it to the final path.
5. On spawn failure, tool failure, cancellation, panic cleanup, or finalization
   failure, remove the partial file. Never remove or truncate the final path.

An abrupt application/process crash can leave the clearly named sibling
partial because arbitrary destination directories cannot be swept safely at
startup. This contract is crash-safe for the final destination, not
orphan-free: never promote or auto-delete an old partial on a later run, and
record this limitation in ADR-0028 for Plan 019 to surface honestly.

Restore validation and execution:

1. Require an absolute, existing, non-empty regular source file.
2. For plain format, run `psql --single-transaction --set=ON_ERROR_STOP=on
   --file <source>`; reject `clean: true`.
3. For custom format, run `pg_restore --list <source>` as format validation,
   then `pg_restore --single-transaction`, adding `--clean --if-exists` only
   when requested.
4. Do not add parallel restore in v1; it conflicts with the single-transaction
   correctness contract.
5. V1 passes no `--no-owner`, `--no-privileges`, `--create`, or `--role`
   options. Restoring under a role that differs from the dump owner fails
   under `ON_ERROR_STOP` / single-transaction with `role ... does not exist`.
   That surfaces as `toolFailed` with the sanitized stderr tail and is
   expected v1 behaviour; ADR-0028 records it as a known gap for the
   follow-up plans, not a bug.

Paths are never inferred from extensions. The selected format is explicit and
validated by the appropriate tool.

### 4. Safety, audit, and connection teardown

Both start commands perform cheap path/payload validation, resolve the
connection, and then attempt admission; the runner rechecks filesystem
preconditions when it starts to close the time-of-check/time-of-use gap.
`start_pg_restore` also calls `safety::policy::assert_permitted` for
`WriteIntent::Restore` **before** admission and maps its refusal variants
directly to the typed `policyBlocked` / `policyNeedsConfirmation` errors rather
than parsing the legacy string tags. The first strict/protected attempt returns
the confirmation challenge; the retry carries `confirmed: true`. Store the
resulting `AuditDisposition` with the active work item. The manager receives
the resolved, already-authorized work item and must not re-resolve credentials
after admission.

Because admission returns before execution completes, do not wrap the start
command in `with_gated_active_connection`. Instead, split/reuse its policy
check and install a completion hook with these semantics:

- successful restore calls `record_override` when the stored
  `AuditDisposition` is `RequiredAfterSuccess`, then touches connection last
  activity;
- successful backup touches last activity;
- failed or cancelled work never calls `record_override`; it may log a
  bounded operational failure through the existing logging convention;
- admission failure has no success side effects.

Add `PgToolJobManager` to `AppState`, to every connection and global fence in
`socket_lifecycle.rs`, and to the `ExitRequested` join in `lib.rs` through a
`close_all` method. A connection edit, delete, disconnect, credential change,
bastion change, global teardown, or app exit must mark the relevant admission
scope closing, cancel its active PostgreSQL tool job, wait the bounded reap
time, and only then allow pool/tunnel invalidation to continue. New admission
during the fence returns `connectionClosing`. Do not drop the tunnel cache
before the kill has been sent and the bounded wait has elapsed.

The fence contract stays infallible, matching the three existing managers:
`begin_*_teardown` and `close_all` return `()`, the wait is bounded by the
manager's own five-second reap limit, and a child that outlives the wait is
handled by the §2 reap-timeout rule (job `cancelled` with
`timeout { operation: "reap" }`, slots freed, detached reaper). The fence then
proceeds. Do not change the `with_*_fence` signatures or add a rollback path;
that would touch all seven callers and is the wider lifecycle redesign the
STOP list forbids. `end_*_teardown` reopens admission exactly as the other
managers do.

Every `save_connection` fences, including cosmetic edits such as a rename or
colour change. This plan keeps that behaviour for consistency with query
sessions, table browse, and result mutations, and does not diff old and new
records. ADR-0028 records the consequence: saving any edit cancels a running
backup or restore on that connection. `list_pg_tool_jobs` exists so Plan 019
can warn before a save while a job is active.

### 5. Legacy removal and truth pass

Once the new contract and tests pass, remove the inaccessible base64 path:

- old `run_pg_dump` / `run_pg_restore` functions and exports;
- `PgDumpPayload`, `PgRestorePayload`, `PgDumpResult`, `PgRestoreResult`;
- legacy dispatch enum arms/wrappers and their tests; and
- old Tauri command registrations.

Remove `RunPgRestore` from `LegacyCommand::ALL` in `commands/relational.rs`
(twelve entries become eleven) and remove the `run_pg_restore` row from
`write_intents` in `commands/safety.rs`. The new command never calls
`assert_legacy_permitted`, so renaming those entries to `start_pg_restore`
would document a gate path that does not exist. Typed policy coverage for
`start_pg_restore` lives in the `commands/pg_backup.rs` tests. Change the
generic confirmation component test fixture from `run_pg_restore` to
`start_pg_restore` so it does not document a dead contract; no component/UI
behavior changes belong in this plan.

Add `docs/adr/0028-file-backed-postgres-tool-jobs.md` documenting the file,
job, policy, and teardown decisions above. Update `CONTEXT.md` with a
**PostgreSQL Tool Job** entry. Update `ROADMAP.md` and `PAR-010` in the parity
register to say the safe file-backed backend is complete but activation is
still missing and owned by Plan 019. Do not claim user-facing parity in this
plan.

## Scope

Expected implementation files:

- `src-tauri/src/postgres/backup/{mod,protocol,manager,runner}.rs` and focused
  tests;
- `src-tauri/src/postgres/mod.rs`, `src-tauri/src/postgres/ddl.rs`;
- `src-tauri/src/commands/pg_backup.rs`, `src-tauri/src/commands/mod.rs`, the
  legacy removal sites in `commands/relational.rs` (including
  `LegacyCommand::ALL`), and the `write_intents` row in `commands/safety.rs`;
- `src-tauri/src/lib.rs` (`AppState`, command registration, and the
  `ExitRequested` `close_all` join), `src-tauri/src/socket_lifecycle.rs`;
- `src-tauri/src/types.rs`, `src-tauri/src/dispatch.rs`, and
  `src-tauri/src/dispatch/relational.rs` for legacy removal;
- `src-tauri/Cargo.toml` only for narrowly required Tokio features;
- `src/components/safety-confirm-dialog.test.tsx` command-name fixture only;
- `docs/adr/0028-file-backed-postgres-tool-jobs.md`, `CONTEXT.md`,
  `ROADMAP.md`, `plans/parity-gap-register.md`, and `plans/README.md`.

Out of scope: any backup/restore UI, file picker, OS notification, scheduler,
recurring backup, job persistence across app restart, archive browser,
compression/encryption UX, cloud destination, MySQL/ClickHouse support,
CSV/JSON/Parquet/XML export, table import/export, generated DDL bundles,
parallel restore, or a generic job framework. Plan 019 owns UI activation;
Plan 020 owns bounded table import/export.

## Commands you will need

```sh
# Focused Rust checks during implementation
cargo test --manifest-path src-tauri/Cargo.toml postgres::backup
cargo test --manifest-path src-tauri/Cargo.toml socket_lifecycle
cargo test --manifest-path src-tauri/Cargo.toml commands::pg_backup

# Legacy contract must be absent after Step 4
rg -n "run_pg_dump|run_pg_restore|PgDumpPayload|PgRestorePayload|PgDumpResult|PgRestoreResult|data_base64" \
  src src-tauri

# Required final gates
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm run check:ui-gates
pnpm run check:slice-isolation
just fmt
just lint
just test
```

## Resume protocol

Each step leaves its focused tests green. On resume, re-run the last completed
step's focused commands, inspect the README status, and continue with the next
step. Never infer an active child process from an old snapshot: jobs are
in-memory only and app restart intentionally loses terminal history.

## Git workflow

Work in the current tree. Preserve unrelated operator changes. No commits,
pushes, PRs, production changes, or use of a daily-driver database without
separate authorization.

## Steps

### Step 1: Typed protocol and bounded manager

Create `postgres/backup/protocol.rs` and `manager.rs` with the decided payload,
snapshot, error, state-transition, admission, cancellation, retention, and
closing-fence contracts. Use injectable runner/completion seams in tests;
production code must not need a fake global or shell script.

Tests must cover:

- serde names and round trips for every scope, format, phase, and error arm;
- legal transitions and rejection of an illegal/late transition;
- accepted cancellation exposes `cancelling`, and exactly one of cancellation
  or final rename wins the terminal-state race;
- a reap timeout moves the job to `cancelled` with `timeout { operation:
  "reap" }`, frees both admission slots, and the detached reaper still runs the
  partial cleanup when the fake child finally exits;
- `list_pg_tool_jobs` returns every retained snapshot, honours the
  `connection_id` filter, and never exposes credentials or absolute paths;
- one-active-per-connection and four-active-total admission limits;
- connection/global closing rejects admission;
- cancel is idempotent and release rejects active work but is idempotent after
  terminal removal;
- terminal cap of 32, one-hour expiry, and active jobs never being evicted;
- exactly-once completion hook behavior under success, failure, cancellation,
  and a racing teardown.

Run `cargo test --manifest-path src-tauri/Cargo.toml postgres::backup`. Update
the README row to `IN PROGRESS: through Step 1` only after it passes.

### Step 2: Async runner and crash-safe files

Implement `runner.rs` and move only the reusable libpq/TLS command setup out of
`ddl.rs`; the DDL module must not retain backup/restore execution.
`tls::apply_to_command` takes a `std::process::Command`: build the std command
with it and convert through `tokio::process::Command::from` rather than
duplicating the TLS environment logic. Add the tool-path resolver next to it.
Add Tokio features only when required. Keep argument construction in pure
helpers so tests assert exact arguments without spawning a real database tool.

Tests must cover:

- tool-path resolution prefers `PATH`, then each fallback directory in order,
  then the bare name, driven by an injected filesystem/`PATH` seam;
- preflight passes `--version` as the sole argument and records the trimmed
  line as `tool_version`;
- database/schema/table dump argument construction, plain/custom formats, and
  `--clean --if-exists` only for plain backups that requested `clean`;
- exact schema/table selection for mixed case, embedded quotes, dots, and
  pg_dump pattern metacharacters (`*`, `?`, and `[]`);
- `psql` single-transaction plus `ON_ERROR_STOP`, and custom `pg_restore`
  list/restore arguments including `--clean --if-exists`;
- absolute-path, parent-directory, regular-file, non-empty-source, and
  destination-exists validation;
- partial-file uniqueness, success rename, and partial cleanup on spawn
  failure, non-zero exit, cancellation, and finalization failure;
- capped 64 KiB stderr while the producer writes more than one pipe buffer;
- 10-second preflight timeout and typed missing-tool failure;
- password/TLS values and absolute paths absent from returned errors and
  captured tracing output;
- child cancellation is reaped and does not leave a final destination file.

Use a Rust test helper process or injected process seam; do not depend on the
developer machine having PostgreSQL tools for unit tests. Run the focused
backup test command and `just fmt`. Update the README row through Step 2.

### Step 3: Commands, safety completion, and lifecycle fence

Add `commands/pg_backup.rs`, `PgToolJobManager` to `AppState`, its expiry
monitor at setup, and all six Tauri commands. Wire restore admission through
the existing `WriteIntent::Restore` policy without using the old
return-means-success wrapper. Add the exactly-once success hook for
`record_override` and connection activity.

Join the manager to every connection/global socket-lifecycle fence and to the
`ExitRequested` `close_all` join before pool or tunnel invalidation. Keep the
fences infallible and their signatures unchanged, as §4 requires.

Tests must cover:

- unsupported engine and missing connection fail before admission;
- strict/protected restore returns a confirmation challenge, confirmed retry
  admits exactly one job, and blocked policy never admits;
- only completed restore records success audit/override/activity effects;
- backup completion touches activity without a restore override;
- disconnect and each edit/delete/credential/bastion invalidation path reject
  new admission, cancel and reap the child, then continue teardown;
- a cosmetic `save_connection` (rename only) also cancels the active job, and
  the ADR documents that;
- a reap timeout inside the fence does not block the lifecycle mutation: the
  fence proceeds, the job reads `cancelled` with a `reap` timeout failure, and
  `end_*_teardown` reopens admission;
- `close_all` on the `ExitRequested` path kills the child, removes the
  partial, and returns within the existing three-second exit budget;
- tunnel invalidation is observed after child termination, not before it;
- global teardown handles active jobs on multiple connections.

Run all three focused Rust commands from “Commands you will need,” then
`just lint`. Update the README row through Step 3.

### Step 4: Remove the base64 contract and document the boundary

Remove every legacy item listed in §5, update safety command-name coverage and
the confirmation test fixture, then run the legacy `rg`; expected output is
empty. Add ADR-0028, the `CONTEXT.md` glossary entry, and the honest roadmap
and `PAR-010` progress updates. The docs must explicitly say the backend is
dark until Plan 019.

Run `pnpm format`, `pnpm lint`, `pnpm typecheck`, the relevant Rust tests, and
the legacy `rg`. Update the README row through Step 4.

### Step 5: Disposable PostgreSQL round trip and full gates

Use only the repository's disposable PostgreSQL fixture, never a daily-driver
or production database. Through a focused Rust integration harness or a
temporary dev-only invocation of the command layer:

1. Create a fixture table with multiple rows and a distinctive value.
2. Back up the database in plain format and in custom format; verify each job
   reaches `completed`, the final file is non-empty, and no sibling partial
   remains.
3. Create a second, empty database on the same disposable server and a second
   stored connection pointing at it (the restore target is always the
   connection's own `--dbname`). Restore the plain archive through that
   connection and verify the distinctive row. Also restore a plain archive
   made with `clean: true` back into the source database and verify it
   replaces the fixture table.
4. Repeat with custom restore and `clean: true`; verify the pre-existing
   object is replaced as expected.
5. Start a sufficiently long backup, cancel it, and verify terminal
   `cancelled`, no final file, no partial file, and no child process.
6. Start a job through an SSH-resolved test connection if the repository's
   fixture supports it; otherwise exercise the injected tunnel-ordering test
   from Step 3 and record that the live SSH case is unavailable. Do not invent
   or use an external host.

Delete only the disposable archives created for this pass. Run every required
final gate from “Commands you will need.” Record the fixture commands and
results in this plan's execution record, then mark `READY FOR REVIEW`.

## STOP conditions

Stop and report rather than broadening or improvising if:

- a frontend caller of the old base64 commands exists at execution time;
- the safety layer cannot authorize now and record success later without
  weakening its backend authority or audit semantics;
- any relevant socket lifecycle path invalidates a tunnel/pool before manager
  teardown and cannot be reordered without a wider lifecycle redesign;
- the supported platform cannot reliably terminate and reap the child with
  Tokio's process API;
- crash-safe sibling creation plus atomic rename is unavailable on a supported
  destination filesystem (document the concrete filesystem/platform);
- a destructive restore would be required against non-disposable data for
  verification; or
- implementation needs UI, persistence, scheduling, another database engine,
  or a generic task framework to proceed.

## Reconciliation and maintenance

- Plan 019 should consume only the six typed commands and must not invoke
  native tools or reimplement policy/progress semantics in TypeScript. It
  should call `list_pg_tool_jobs` before saving a connection edit and warn
  that saving cancels the active job.
- Owner/privilege/role handling (`--no-owner`, `--no-privileges`, `--create`,
  `--role`) is a known v1 gap. Add it as explicit typed options in a follow-up;
  do not infer it from failures.
- Exempting cosmetic connection edits from the teardown fence would need an
  old-vs-new record diff shared by all four managers. Do it for all of them or
  none; this plan does neither.
- Plan 020 may copy this bounded lifecycle for table import/export. Extract a
  shared job primitive only if both implementations have the same proven
  ownership and teardown needs.
- If a future product requirement allows overwrite, add an explicit policy and
  recoverable replacement design; do not change V1's refusal implicitly.
- If PostgreSQL later exposes trustworthy restore progress through a supported
  tool contract, add it as optional progress without changing existing
  indeterminate snapshots.
- Keep native-tool argument compatibility aligned with PostgreSQL's supported
  client/server version policy. Version negotiation is a follow-up, not a
  reason to silently accept an unknown tool in this plan.

## Execution record

_Executor appends dated step completions, verification results, deviations,
and the disposable round-trip evidence here._

### 2026-09-05: Step 1

Implemented typed polling protocol, bounded admission/retention, observable cancellation,
serialized publication, success-only completion, and teardown generations. Focused
`cargo test --manifest-path src-tauri/Cargo.toml postgres::backup` passed (14 tests;
one subprocess-helper entry ignored in the parent suite and invoked by process tests).
Injected reap timeout proves terminal cancellation, slot release, and delayed partial cleanup.
The operator-owned Plan 017 deletion and `plans/next-parity-item.html` are preserved.

### 2026-09-05: Step 2

Async runner and sibling partial files implemented. The focused backup suite passed
(14 tests plus the child-helper entry); `just fmt` passed. Tests cover no-overwrite
publication, literal selectors, bounded pipes, native spawn/failure/cancel/timeout
cleanup, and the cancellation/publication race. Legacy execution was removed from
DDL while wiring the new command boundary.

Verification correction: the legacy search needs whole-word matching because
`StartPgRestorePayload` contains `PgRestorePayload`, and XLSX independently uses
`data_base64`. Preserve XLSX and check PostgreSQL legacy symbols with `rg -w`.

### 2026-09-05: Step 3

Six commands, typed restore policy, completion-only audit/activity, every socket
fence, and ExitRequested cleanup are wired. All three focused Rust suites and
`just lint` passed. Connection generations reject stale admission across edits.
The cosmetic-save test confirms cancellation; injected child/tunnel ordering
covers connection, multi-connection/bastion, and global fences. The disposable
fixture has no SSH service, so live SSH verification is unavailable.

### 2026-09-05: Step 4

Removed legacy commands, payload/results, dispatch wrappers, registrations, and
legacy safety coverage. Updated the confirmation fixture, ADR-0028, glossary,
roadmap, and PAR-010 with the dark-backend boundary. `pnpm format`, `pnpm lint`,
`pnpm typecheck`, all focused Rust suites, and `just lint` passed. The corrected
whole-word legacy symbol search and PostgreSQL-only `data_base64` search are empty.

Diagnostics deliberately normalize recognized failure categories from the bounded
stderr tail instead of returning arbitrary native text: raw PostgreSQL error
messages can echo SQL, row values, passwords, or paths. This privacy refinement
is documented in ADR-0028 and tested with secret/path-bearing child stderr.

### 2026-09-05: Step 5 verification

Disposable fixture commands:

```sh
docker compose -f infrastructure/test-db/compose.yml --profile postgres up -d postgres
HOMEBREW_NO_AUTO_UPDATE=1 brew install libpq
HOMEBREW_NO_AUTO_UPDATE=1 brew install libpq@16
PATH="/opt/homebrew/opt/libpq@16/bin:$PATH" cargo test \
  --manifest-path src-tauri/Cargo.toml \
  disposable_postgres_plain_custom_clean_and_cancellation_round_trip \
  -- --ignored --nocapture
```

The initially missing client tools were installed; the live pass selected the
matching `pg_dump/psql/pg_restore` 16.15 clients with PostgreSQL server 16.15.
No shell profile was edited. The fixture is the repository Docker service at
127.0.0.1:15432, never a saved/daily-driver connection or external host.

The command-layer harness created two UUID-named disposable databases, stored
both connections only in temporary app-state SQLite, and inserted two rows,
including `plan018-distinctive-value`. Plain, plain-clean, and custom backups
all completed with non-empty final files and no sibling partials. Plain restore
into the empty target recovered both rows; plain-clean restored the changed
source table; custom-clean replaced the changed target table. Versions and
honest byte metadata were verified. A lock-blocked dump was cancelled, reached
`cancelled`, left no final or partial archive, and left no PostgreSQL tool
session. The native subprocess unit test separately exercises Tokio kill/reap.
The harness deleted its archives through TempDir and dropped both databases;
a follow-up catalog query found no `dbunk_backup_%` databases.

The repository fixture has no SSH service. Live SSH is unavailable; the injected
tunnel-ordering test covers all three canonical fences without an external host.

All final gates passed: `pnpm format`, `pnpm lint`, `pnpm typecheck`, `pnpm test`
(109 files, 1,361 tests), `check:ui-gates`, `check:slice-isolation`, `just fmt`,
`just lint`, and `just test` (476 passed, 44 opt-in/helper tests ignored in the
parent suite). The new live round-trip test was run explicitly and passed.
Vitest emits existing React `act` warnings in unrelated component tests but
reports no failures. `git diff --check` passed. Fresh code review is pending;
status remains through Step 4 until that review and any resulting fixes finish.

Tooling cleanup: removed the unused unversioned `libpq` package installed during
setup (`brew uninstall libpq`); retained only the matching `libpq@16` client
installation needed to reproduce the fixture pass.

### 2026-09-05: Fresh review and follow-up

The first fresh review found lifecycle gaps despite the green initial tests:
unbounded waits outside child reaping, publication under the global mutex,
terminal snapshots preceding admission release, a successful restore/cancel audit
race, unowned pending resolutions that could recreate a stale tunnel after a
fence, and unbounded resources across repeated detached reapers. It also requested
actual lifecycle-caller/exit tests. These findings are being fixed before the
plan is marked ready. No commit or publication is authorized by this execution.


### 2026-09-05: Review fixes and final verification

The review fixes now reserve capacity before connection preparation, cancel and
quiesce pending resolution before cache invalidation, and bound both ordinary
cancellation and teardown to five seconds. Detached native cleanup retains one
of four separate resource permits until child exit. Aborted workers are joined
before normal cancelled state; unfinished reap or cleanup reports its typed
timeout. Terminal snapshots and admission release now publish together after a
two-second best-effort success-effect attempt.

Publication uses an atomic per-job claim before the non-interruptible filesystem
operation, with no filesystem I/O under the global manager mutex. A claimed
publication remains finalizing and cancellation is a no-op, including when a
fence reaches its deadline. This narrow refinement avoids both a blocked manager
and a file appearing after cancelled state. Observed successful restore exit can
win a cancellation race and preserve its audit, including status returned by
post-kill reaping. Final cancellation freezes the outcome before worker abort.
ADR-0028 records these refinements and their bounds.

New tests cover pending-resolution ownership/capacity, stalled preflight and
finalization under direct cancellation and fences, cleanup before terminal state,
completion ordering and timeout, publication responsiveness, restore outcome
races, and bounded detached cleanup. Five caller-level tests exercise actual
disconnect, deletion, credential reset, bastion save/host-key reset, and the
app-exit join using isolated app state and explicit termination signals.

After all fixes, every final gate passed again: `pnpm format`, `pnpm lint`,
`pnpm typecheck`, `pnpm test` (109 files, 1,361 tests), `check:ui-gates`,
`check:slice-isolation`, `just fmt`, `just lint`, and `just test` (491 passed,
44 opt-in/helper tests ignored in the parent suite). The focused backup suite
passed 24 tests plus its ignored helper; command tests passed 3 plus the ignored
live test; socket lifecycle passed 1 and caller-level lifecycle passed 5.
The explicit PostgreSQL 16.15 round-trip command above passed again after all
code changes. Logs: `/tmp/dbunk-018-final-ts-after-review.log` and
`/tmp/dbunk-018-final-rust-after-review.log`. A new independent review is pending.


After the final round trip, a catalog check found no `dbunk_backup_%` databases.
The disposable PostgreSQL service was returned to its original stopped state
with `docker compose -f infrastructure/test-db/compose.yml --profile postgres
stop postgres`; no other fixture service or saved connection was touched.


### 2026-09-05: Second review follow-up

The second fresh review confirmed the revised process lifecycle but found two
additional gaps: unrestricted plain-file psql meta-commands could execute local
commands or change connections, and Bastion reference discovery could miss a
concurrent new reference before invalidation. Both are being fixed before ready
status. Bastion changes now close global admission before reference discovery;
this also covers deletion's zero-reference check. Credential configuration and
migration join the existing global fence, with caller coverage. The separate
organization-only metadata update does not invalidate resources and remains
unchanged; ADR wording now explicitly describes the full connection-save path.


Plain restore now enters native psql restricted mode with a fresh private key
before receiving file bytes. Bounded streaming preserves SQL/COPY data and
removes only canonical pg_dump outer guards. This refines the original direct
`--file <source>` invocation to `--file -` without introducing archive buffering
or frontend transfer. Feeder failures retain the pipe until kill/reap, preventing
premature EOF from committing a partial read. The minimum patched psql versions
and CVE-2026-18408 rationale are recorded in ADR-0028.

Native PostgreSQL 16.15 checks accepted valid SQL and rejected local shell,
program COPY, include, and connection-switch meta-commands, with no marker files.
The command-layer round trip now also verifies that prohibited meta-commands and
mismatched guards fail, roll back valid SQL earlier in the file, and do not add a
success audit. The full plain/custom/clean/cancellation round trip passed again.
Bastion save/reset coverage now adds a referencing connection during teardown;
its admission stays closed. Deletion checks references under the same global
fence. Credential configuration and migration have caller-level teardown tests.
The single-connection fence shares the existing batch-fence implementation,
keeping all fence signatures while avoiding redundant teardown waits.


Final verification after the security/topology fixes: all frontend gates passed
again (109 files, 1,361 tests), and `just fmt`, `just lint`, and `just test`
passed (497 tests, 44 opt-in/helper entries ignored in the parent suite). This
includes an explicit duplex-pipe test proving a source failure cannot signal
clean EOF before process cleanup. The seven actual lifecycle-caller tests pass.
Logs: `/tmp/dbunk-018-final-ts-security.log`,
`/tmp/dbunk-018-final-rust-security.log`, and
`/tmp/dbunk-018-final-rust-pipe-test.log`. The expanded live round trip was run
explicitly and passed (0.55 seconds). A fresh independent re-review is in progress.


The next fresh review found no remaining process, lifecycle, topology, or test
issues, but identified that pg_dump also needs the patched-client floor for
CVE-2026-19385. The same minimum-version gate is being shared across pg_dump,
psql, and pg_restore before connection/archive execution. ADR-0028 records both
security advisories and the consistent client policy.


The shared patched-client gate is complete for all three tools, with fixed,
vulnerable, and wrong-banner test cases. After that final production change,
`just fmt`, `just lint`, and `just test` passed again (498 passed, 44 opt-in/helper
entries ignored); focused backup coverage passed 29 tests plus its helper. The
expanded PostgreSQL 16.15 round trip passed again (0.67 seconds), including
security rejection/rollback and success-only auditing. Final Rust log:
`/tmp/dbunk-018-final-rust-client-floor.log`. Frontend gates remain green from
`/tmp/dbunk-018-final-ts-security.log`; no frontend source changed afterward.
A final catalog check found no temporary `dbunk_backup_%` databases, and only the
disposable PostgreSQL service was stopped, restoring its original state.


### 2026-09-05: Ready for review

The final fresh independent review reported **no findings**, confirmed the
shared client security floor closes the last issue, and found no correctness,
lifecycle, topology, scope, or test regression. Its targeted tests, format
check, and diff check passed. All required repository gates and the expanded
disposable PostgreSQL round trip are green. Plan 018 is `READY FOR REVIEW`.
No commit, push, PR, UI activation, or production change was made. Operator-owned
Plan 017 deletion and planning artifacts remain preserved.
