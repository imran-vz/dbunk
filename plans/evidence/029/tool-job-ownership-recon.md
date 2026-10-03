# PostgreSQL Tool Job ownership reconnaissance

2026-10-03. Read-only comparison against Tauri `102568b`, ADR-0028 and current source. No backup/restore process, file job, fixture or credential operation was performed. This is activation reconnaissance, not implementation or acceptance evidence.

## Preserved baseline

ADR-0028 defines four admitted jobs globally, one per connection, plain/custom formats, database/schema/table backup, database-target restore, polling/cancel/release, and 32 terminal records retained for one hour. Baseline `src/components/pg-tool-jobs/{workspace,use-tool-form,job-list}` supplies the selected setup/history arrangement. Table-context restore still reviews the whole target database. `src/lib/pg-tool-jobs/observer.ts` owns jobs beyond setup-tab lifetime and reconciles lost start responses without automatic retry. `restore-refresh.ts` invalidates connection-wide metadata and staged mutation sources after restore while preserving draft intent.

## Concrete native activation gaps

- `src-tauri/src/commands/pg_backup.rs::start_with` remains Tauri-gated. Its manager and runner are reusable, but start orchestration needs a host-neutral service preserving the existing command/wire contract.
- `backend.rs::from_state` and `shutdown_with_deadlines` do not own PostgreSQL Tool Job monitors/teardown. Manager workers, cancellation watchdogs, publication work, reapers and runner I/O tasks need native ownership and joining under the host's absolute deadlines. A facade wrapper alone would not provide it.
- `postgres/backup/manager.rs::admission` holds pending preparation, while `list` exposes only started jobs. After a lost start reply, an empty list cannot prove preparation will not start later. A bounded stable attempt identity and visible pending state (or an equivalent ordered reconciliation barrier) are required. Never automatically retry start.
- `JobContext::spawn_reaper` and `runner::process` intentionally permit detached cleanup in the legacy path. Native must retain these joins, including cleanup of an unkillable child; deadline expiry must report failed cleanup instead of releasing ownership as success.
- Restore validation rejects a symlink at selection, but plain restore later opens the path again and custom restore separately reopens it for inspection and execution. Path replacement can change the inspected source. Native needs a pinned source or private owned snapshot used for both phases. A metadata stamp alone does not establish immutable contents.
- Existing protocol strings/snapshots do not supply native retained-capacity bounds. Bound paths, identities, errors and aggregate history before cloning into native state. Preserve the legacy command types for Tauri.

Reuse existing libpq TLS/environment rendering, which already removes PostgreSQL environment redirectors. Native admission must check explicit profile endpoint authority before hydration/process launch, then preserve stored policy and generation fences through preparation/start. Fixture manifests remain unchanged; a general profile is not permission to exercise an arbitrary endpoint.

## Required native contracts

A review/confirmation captures its backend owner, connection generation, exact typed intent and restore source. It is not a DataDocument lease: closing a setup tab must leave an admitted job running and discoverable. Paths remain transient and excluded from workspace recovery, Debug, snapshots and logs. The app owns polling/attempt reconciliation and shared-budget captures independently of any setup view.

Known successful restore exit must survive cancellation during reap. Cancelled or failed execution does not prove rollback; lost acknowledgements after dispatch remain uncertain until a terminal observation establishes the outcome. Completion invalidates affected metadata/mutation sources once without silently discarding drafts. Release is terminal-only. A native window must expose the database target of table-context restore, exact confirmation, progress phases and observed byte/version values without fabricated restore percentages.

The next service boundary is `backend/pg_tools` plus extracted `postgres/backup/service`, bounded native DTOs and narrow opt-in manager/runner lifecycle changes. Activation additionally requires an app-owned native observer, approved setup/history UI, native file selection, and completion integration. None is claimed implemented by this reconnaissance.

Focused evidence must cover lost waiters during blocked preparation, pending reconciliation, policy/endpoint changes before launch, tab-close survival, connection/global cancellation, publication races, source replacement, terminal release/history limits, success-only audit, observed restore success during reap and shared-deadline shutdown. Existing legacy manager tests do not substitute for these native ownership checks.
