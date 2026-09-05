# Plan 019: PostgreSQL backup and restore activation

- Priority: P1. Effort: L. Gap: PAR-010.
- Planned against: `de3272b`, 2026-09-05.
- Depends on: Plan 018, DONE at `de3272b`.
- Status: IN PROGRESS. A selected for connection-wide workspace; C selected for contextual table workflow. Table Restore opens the database-target review, as confirmed by Imran.

## Outcome

A PostgreSQL user can choose a local destination, back up a database, schema,
or table, restore a chosen plain SQL/custom archive into the connection's
database, and inspect or cancel the resulting PostgreSQL Tool Job. The UI
consumes Plan 018's process, file, safety, and teardown semantics.

## Evidence and boundaries

Read `docs/adr/0028-file-backed-postgres-tool-jobs.md`,
`src-tauri/src/postgres/backup/protocol.rs`, and
`src-tauri/src/commands/pg_backup.rs` before implementation. The protocol has
six commands, with camelCase payloads; start commands accept `{ payload }`,
get/cancel/release accept `{ jobId }`, list accepts `{ connectionId }`.
No frontend client or native file picker currently exists for this workflow.
Reuse existing `src/lib/safety-confirmation.ts` and typed object-DDL safety
patterns. The legacy string-tag invocation helper is not the typed job client.
Inspect connection form/store save, disconnect, deletion, credentials, and
bastion paths before wiring lifecycle warnings.

Scope is PostgreSQL only. No process runner changes, event transport, generic
job framework, job persistence, scheduling, notifications, owner/role switches,
archive browser, table transfer, or other engines. Plan 020 remains bounded
table import/export. Minimal native picker plumbing is in scope; Rust changes
require all Rust gates. Do not silently change ADR-0028 to accommodate UI.

## Design gate

Build three distinct static mocks using the html skill, publish a review URL,
and stop for Imran's selection before editing product components. Respect
black background, white primary text, dense rows, minimal copy, no decorative
cards/pills, no em dashes, and no repainting animations.

Compare A: a connection Backup/Restore workspace with inline job rows;
B: contextual launch dialog with a workspace job drawer;
C: a dedicated transfer tab with setup left and job detail right.
Show backup setup, restore review, running, cancellation requested, failed,
and completed states. Use fictional local fixture data only. Mock selection
chooses placement, not weaker safety or lifecycle behavior.

## Contract and implementation

1. Add a small typed client mirroring the Rust discriminated unions, including
   every refusal, nullable snapshot field, and phase. Validate unknown IPC
   errors before narrowing. Do not use `any`, raw casts of unknown errors,
   or a duplicated policy engine. Share existing statement summary types.
2. Add native open/save dialogs using the smallest existing-compatible Tauri
   integration, restricted to file selection. Return a path or cancellation;
   read/write/validate files only through backend jobs. A browser file input
   cannot supply the required native absolute path. Keep selected paths only
   in transient form state, never storage, logs, URLs, or retained job history.
   File extensions are suggestions only; format is an explicit selection.
3. Backup setup supports database/schema/table scope with exact catalog names,
   plain/custom format, and destination. Default to custom. Plain backup may
   embed clean/drop commands; explain that this changes future restore behavior.
   Custom backup must send clean=false. Never offer overwrite; destinationExists
   returns to selecting another name. Do not auto-delete an existing destination.
4. Restore review shows source filename, explicit format, target connection,
   database and environment. Custom restore may request clean; plain restore
   sends clean=false and explains that cleanup depends on the SQL file.
   Surface the trusted-source requirement: server-side SQL executes with the
   connection's privileges even with local psql commands restricted. Explain
   expected existing-object and missing-owner failures and lack of database
   creation/owner remapping. Require deliberate Restore submission.
5. Start restore unconfirmed. Only policyNeedsConfirmation opens the shared
   safety dialog with backend statement summaries. Retry the identical frozen
   request with confirmed=true once approved and only if the connection and
   form generation are still current. Add a monotonic restore-review revision:
   existing connectionEpochs do not cover full saves or all credential/bastion
   changes. Invalidate pending reviews when an affected full save or lifecycle
   fence begins, including attempts that fail later; check the revision before
   initial submission and after confirmation. Cover same-ID host/database edits,
   credentials, bastions, disconnect/delete and global fences. Do not rely only
   on the existing object-DDL epoch predicate. A read-only/policyBlocked refusal stays
   blocked. Closing the dialog cancels the retry. Never mark admission as
   successful restore or record success audit/activity in the frontend.
6. Own jobs outside a dialog/tab lifetime. Use one bounded, non-overlapping
   poll loop for the visible app (about one second while active, capped backoff
   on transport failures). List on opening/resuming and after uncertain start
   responses; never automatically retry start. Key snapshots by job ID and
   connection ID. Fence late responses with a request epoch so release, switch,
   teardown, or newer responses cannot resurrect stale state. Stop timers when
   hidden and refresh immediately on visibility return. Closing a surface must
   not cancel or release jobs. No polling when no active jobs and no consumer;
   reopen always reconciles with list. Cap retained records to backend bounds.
7. Render backend phases verbatim with concise user labels, filename, elapsed
   time, available bytes and tool version. Restore totalBytes is source size,
   not completed work; no percentage or fake progress bar. Poll failures show
   observation unavailable with retry, never fabricated job failure/success.
   jobNotFound becomes expired/unavailable, never completed. Cancellation is a
   request: await snapshots, and allow cancelling -> finalizing/completed.
   Hide cancel once finalizing; it cannot interrupt archive publication. Show
   cleanup/reap timeout honestly and never promise rollback after cancellation.
   Release only explicit terminal dismissal; retain active jobs and tolerate
   expiry. History lasts this app session, up to one hour/32 terminal records.
8. Before a full connection save, list affected jobs and warn that saving
   cancels them, with Keep editing / Save and cancel jobs. If inspection fails,
   show that active work cannot be checked and require explicit continuation.
   Own the pre-save warning in the form before invoking save, or return a typed
   saved/cancelled/failed result from the store and consume it in the form.
   Current updateConnection swallows errors and the form calls onSaved
   unconditionally; fix that contract for this workflow. Never call onSaved on
   cancelled or failed saves. Organization-only updates do not need this warning. Audit other fencing
   actions: disconnect/delete, credential changes/global reset and bastion
   edits. Reuse existing confirmation surfaces with job impact; bastion/global
   operations can affect all connections. The warning is advisory; races still
   resolve via backend fencing. Preserve form state if the user backs out.
9. On first observation of a completed restore, invalidate the whole connection's
   catalog and object descriptions, advance pgObjectDdlVersion through the existing
   markPgObjectDdlApplied mechanism, invalidate relation statistics, and refresh
   open browse structures/counts/data safely without discarding drafts or silently
   rerunning query SQL. Recompute exact counts only for browses that had requested
   them; other browses retain estimated counts to avoid new full-table scans.
   Snapshots have no affected-object inventory, so scope
   invalidation database-wide. Handle completion once per job ID, including
   list/resume recovery and completed-after-cancel. Keep this bounded with retained
   job state; a harmless repeated invalidation after eviction is preferable to
   missing it. Refresh failures are separate from restore success. Fence old
   metadata replies, and preserve staged edits with explicit stale/review state
   instead of rebasing them silently onto refreshed rows.
   Disclose crash-left partial files, in-memory history, unsupported ownership
   options and client compatibility limits in compact help/error details.
   Present only backend-sanitized diagnostics; never expose raw stderr.

## Sequence and verification

Step 1: Critique this plan with a GPT-6 subagent, resolve material findings,
and record the review. Publish mocks and wait for selection. No product edits.

Step 2: Implement typed client and native pickers, then job ownership/polling.
Test actual IPC argument shapes and typed refusals; picker cancellation must
start nothing. Exercise uncertain admission response without duplicate start,
late list responses after cancellation/release, hidden/resumed reconciliation,
transport failure recovery, and terminal expiry with controlled timers.

Step 3: Activate selected layout, restore safety review, lifecycle warnings,
and existing cache invalidation. Test frozen confirmation request and changed
connection refusal (including same-ID retargeting and global credential/bastion
changes), an open DDL preview and staged browse edits at completion, Keep editing
and declining after failed job inspection, read-only denial, format/clean combinations, no overwrite,
closing/reopening active jobs, and completed-after-cancel display. Tests should
cover behavior and failure boundaries, not mirror component implementation.

Step 4: Run native manual backup/restore only against explicitly identified
local disposable PostgreSQL fixtures and temporary files. Round-trip plain and
custom, schema/table selections, missing tool, existing destination, cancellation,
disconnect/save during running jobs, and dialog keyboard/focus handling. Verify
narrow and wide layouts. Mock/native runs must not touch daily-driver builds,
production, or live user databases. Use an isolated dev channel if needed.

Step 5: Run `pnpm format`, `pnpm lint`, `pnpm typecheck`, relevant Vitest tests,
and, if Rust/dependency/capability wiring changes, `just fmt`, `just lint`,
`just test`. Update PAR-010, ROADMAP, ADR activation wording, and plan status
honestly. Mark READY FOR REVIEW only after implementation and gates; completion
SHA is recorded after an authorized commit. Commit/push/PR are separate actions.

## Stop conditions

- No selected mock: finish design artifacts, report URL, and wait.
- UI requires backend semantic changes or broader engine/task scope: identify
  the concrete conflict and amend the plan before implementation.
- Required native tooling cannot be tested safely: report the unverified gate;
  do not claim completion.

## Review record

GPT-6 Astra independently reviewed this plan against the repository on 2026-09-05.
Three findings were incorporated before implementation:

- P1: Existing connectionEpochs do not fence same-ID saves and all credential/
  bastion changes. Step 5 now requires explicit restore-review revision fencing.
- P1: Restore scope is unknown and catalog refresh does not invalidate DDL
  previews. Step 9 now requires idempotent database-wide invalidation, DDL review
  revision advancement, safe browse refresh and separate refresh failures.
- P2: updateConnection returns void and swallows failures; the form unconditionally
  calls onSaved. Step 8 now specifies a warning/result contract preserving edits.

Evidence: src/lib/store/connections.ts:330,
src/lib/store/relational-tables.ts:1762, src/lib/store/pg-objects.ts:244,
src/components/object-ddl/ddl-review-dialog.tsx:280,
src/components/connection-form/use-connection-form.ts:78.

Follow-up GPT-6 check confirmed all three findings resolved, with no remaining
material issues. Planning gates passed: pnpm format, pnpm lint, pnpm typecheck.

Static mock comparison: plans/mocks/backup-restore/index.html.
Published private review: https://dbunk-plan-019-review.imran-vz.chatgpt.site
Desktop and narrow visual checks passed; all 18 layout/state combinations
fit a 390px viewport.
Imran selected A for the global workspace and C for contextual table tabs.


### Design fidelity revision

At Imran's request, the schematic comparison was replaced with full-window
screens grounded in the current app source: WorkbenchShell, WorkbenchHeader,
ActivityRail, DatabaseNavigator, ObjectTabRow, StatusBar, Button and Dialog,
plus styles.css density and typography tokens. Uses bundled JetBrains Mono and
Tabler icons; preserves the requested black app background and white text.
A proposes a connection-level rail destination; B uses a 560px dialog over the
existing table workspace plus a jobs drawer; C uses a workspace tab. The same
review URL hosts this revision. The fidelity revision is the selected implementation reference.


## Implementation verification (2026-09-05)

Implemented typed IPC/native path selection, app-owned job observation, A global
workspace and C table tabs, restore review fencing, lifecycle warnings, and
database-wide metadata/draft invalidation. Table Restore always targets the
connection database; no selective restore backend changes.

Automated gates: frontend full suite 1,402 passed across 117 files; Rust 498 passed with 44
opt-in tests ignored. The disposable native plain/custom round-trip test was
also run explicitly and passed. Formatting, lint, typecheck and UI gates passed.
All reported implementation review findings have been fixed and pass automated
gates. Final targeted re-review could not start because the agent tool returned
`agent thread limit reached` on repeated attempts. The subsequent review and fix
are recorded below.

Verification uses only container `dbunk-plan019-postgres` on localhost:15432,
temporary files and the isolated app profile under
`/tmp/dbunk-plan019-native`. A debug-only absolute
`DBUNK_DEV_CONFIG_DIR` override separates test settings from the daily-driver
profile; release builds do not honor it.

Native app startup succeeds, but the desktop session provides no accessible
application window. Native file-dialog selection, keyboard/focus, and full
interactive lifecycle checks remain unverified. Do not mark READY FOR REVIEW
until this gate and implementation review are complete. No commit/push/PR was
performed during that verification run.


Browser visual checks passed for global and contextual layouts at 1200px and
the native 900px minimum width, with no horizontal overflow. Contextual Restore
shows the database target and no single-table restore scope. These browser
checks use transient fixture state and do not substitute for native dialog checks.

First implementation review identified lifecycle epoch reuse, late mutation
completion after restore, stale-draft staging, polling form locks, native
extension filtering, contextual target/rail handling and additional behavior
coverage. Fresh fix agents addressed these with separate file ownership and focused tests.
Previously requested exact counts will be refreshed; other views remain
estimated to avoid new expensive scans.


Post-fix browser checks confirm the selected dark workspace uses an actual
`rgb(0, 0, 0)` content background with white primary text and existing app
chrome/control tokens. Global and contextual layouts pass visual inspection;
the contextual view remains within the 900px minimum width. No browser console
errors were reported. Local evidence: `/tmp/dbunk019-final-global.png`,
`/tmp/dbunk019-final-context.png`, `/tmp/dbunk019-final-narrow.png`.

Native pickers intentionally have no extension restrictions. Backups suggest
`archive.sql` or `archive.dump`; restore uses the explicit form format.
The installed rfd macOS backend forwards extension lists literally to
NSSavePanel, so a wildcard filter is not used as a substitute for unrestricted
selection.

Temporary native app processes, Vite server, browser session and disposable
PostgreSQL container were shut down after verification. Screenshots, logs, and
isolated profile files remain under /tmp for inspection.

A second independent review found that table/query surfaces still advertised
editing for restore-invalidated drafts. A fresh fix added explicit stale status,
read-only controls, mid-flow authoring guards and late-analysis rejection, while
keeping Review/Discard available. Four component behavior/race tests cover it.
A follow-up review found that empty drafts also need generation fencing. Restore
now removes empty drafts and advances their generation; retained edits remain
stale and available for review/discard. A further fix ties open authoring and
secondary virtual-key loads to the analysis generation so draft removal cannot
leave stale controls actionable. Replacement-generation tests also cover an old virtual-key lookup or save
settling after fresh authoring begins. Editor visibility is owned by the analysis
handle and save/clear busy state by a request token. All frontend gates and the
full suite pass. The last review also identified temporary result inactivity
being treated as invalidation; the fix preserves valid key state on a
switch-away/back while still rejecting invalid owners. Its focused behavior
test and the full suite pass. The agent thread limit initially blocked final
targeted re-review; the follow-up below completes that review.

### Review follow-up (2026-09-05)

The thermo-nuclear review identified one P2 issue: a lost start response followed
by failed reconciliation could stop observation after the job view closed.
The observer now retains uncertain admission until an accepted full list issued
after the failure succeeds, continuing retries with backoff without retrying start.
Focused tests first reproduced the failure, then passed for both a completed
restore and no admitted job, including a late list from before the failure.

Post-fix gates passed: pnpm format, pnpm lint, pnpm typecheck, UI gates, slice
isolation, all 1,404 frontend tests, just fmt, just lint and just test (498 passed,
44 opt-in tests ignored). The user authorized committing the implementation and
review fix. Native dialog, keyboard/focus and interactive lifecycle verification
remain open, so Plan 019 stays IN PROGRESS.
