# Plan 002: Activate PostgreSQL Query Sessions in the editor

> **Executor instructions**: Do not start until Plan 001 is `DONE` in
> `plans/README.md` and its completion SHA is recorded. Follow each step and
> gate exactly. Step 1 is a human design checkpoint and must finish before real
> TSX changes. Update this plan's README status after each step. Stop on every
> STOP condition; do not improvise. Mark `READY FOR REVIEW` after all gates; a
> reviewer/operator records `DONE: <completion SHA>` after an authorized commit.
>
> **Prerequisite and drift check**:
>
> ```sh
> rg -n '^\| \[001\].*DONE: [0-9a-f]+' plans/README.md
> git diff --stat <PLAN_001_COMPLETION_SHA>..HEAD -- src plans/README.md plans/mocks/query-session
> git status --short -- src plans/README.md plans/mocks/query-session
> ```
>
> Replace the placeholder with the SHA recorded by Plan 001. Expected on a
> fresh Plan 002 run: the prerequisite matches and no in-scope frontend files
> changed after that SHA. If resuming, follow "Resume protocol".

## Status

- **Priority**: P0
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: Plan 001 at its recorded completion SHA
- **Category**: direction
- **Planned at**: commit `24432fb`, 2026-08-18

## Why this matters

Plan 001 lands a dark PostgreSQL session backend. This plan makes each
PostgreSQL Query Tab own that session, reduces bounded Channel events into
tab-scoped Zustand state, releases old completed row payloads before global
memory becomes a cliff, and exposes Stop, transactions, notices, multiple
result sets, truncation, and recovery. Non-PostgreSQL engines and non-editor
SQLx callers remain on the legacy command.

## Required Plan 001 contract

Before work, confirm the implemented Rust DTOs and ADR match these facts. A
mismatch is a STOP condition, not permission to change backend behavior here.

- Commands: owner registration, open, execute, cumulative ACK, heartbeat,
  cancel, transaction Recheck/mode/isolation/Commit/Rollback, and close.
- Channel identity: session, tab, Connection, generation, sequence, optional
  execution, and `requiresAck`.
- Credit: four row batches and 4 MiB serialized unacknowledged rows per session.
- Liveness: 120 seconds of focused-time ACK/heartbeat stall; deadlines pause
  while the Tauri window is unfocused.
- Transactions: `idle | active | failed | unknown`; `manualIsolation` applies
  to the next manager-issued manual transaction only.
- Terminal omission counters cover rows, result sets, notices, metadata bytes,
  and column names.
- Backend commands remain unused until this plan enables PostgreSQL through one
  local capability switch.

## Current frontend state

- `src/lib/store/types.ts:342-369,748-779` models one Query Outcome and one
  `QueryPreviewData`; there is no Query Session state.
- `src/lib/store/relational-queries.ts:150-273` invokes `run_query`, keys the
  preview by `tab.label`, and logs the entire error object with
  `console.error("Failed to run query", error)`. Labels can collide and database
  error detail can contain sensitive values.
- `src/lib/store/workspace-tabs.ts:113-126,273-287` removes or retargets tabs
  without closing backend resources.
- `src/lib/store/connections.ts:507-548` clears frontend state and does not await
  backend disconnect before reconnect can race it.
- `src/components/query-editor/toolbar.tsx:24-40,177-216` has Run only.
- `src/components/query-editor/results-view.tsx:16,60-63,106-111` assumes one
  result and Results/Explain only.
- `src/components/query-editor/status-items.ts` has no session/transaction state.
- Store slices cannot import one another. Cross-slice cleanup calls named owner
  actions through Zustand `get()`; follow `src/lib/store/README.md`.
- UI must remain true black, white primary text, dense, minimal, and free of
  decorative card/pill chrome, gray eyebrow subtitles, em dashes, and
  continuously repainting animation.

## Decided frontend architecture

### Channel reduction and execution settlement

1. `src/lib/query-session-channel.ts` is the only owner of Tauri Channel
   construction, the process-lifetime owner UUID, registration, heartbeat, and
   event dispatch.
2. Create/register the owner before opening any session. A single module timer
   sends all open session IDs every 30 seconds; React components never own the
   timer. Send an immediate heartbeat on `visibilitychange` to visible and
   window focus.
3. Validate `sessionId`, `executionId`, Connection generation, and exact next
   sequence. Reduce events synchronously before returning credit.
4. Coalesce ACKs in a microtask and send cumulative `ackThroughSequence`.
   Account retained bytes before ACK. Never acknowledge a sequence not fully
   committed to state.
5. Commit terminal state/history once, send and await the terminal ACK, then
   clear running state and resolve `runQuery`. A new run cannot overtake backend
   terminal settlement.
6. Ignore stale/duplicate envelopes. A gap closes the session. Channel loss or
   terminal-ACK failure rejects the pending promise and marks the session lost.

### Global retained-result budget without session eviction

7. The frontend owns a 128-MiB serialized Query Execution payload budget. It
   counts row bodies, column names, notices, structured error detail, and other
   retained result metadata. Session descriptors, transaction snapshots, Query
   History entries, and a compact execution tombstone are outside this budget
   and are never evicted to recover it.
8. Track execution-payload bytes and `lastViewedAt` by tab/execution. Update recency
   only on explicit tab or result selection, not renders.
9. Before retaining an envelope that exceeds the global budget, release the
   full display payload of the least-recently-viewed terminal execution in a
   background tab. Never release an active execution or the currently selected
   tab's visible result. Preserve only a compact tombstone: terminal status,
   result/row/affected/notice/omission counts, runtime, released byte count,
   timestamp, and `globalBudget` reason. Session and transaction state remain.
10. If eligible background payloads cannot make room, retain only bounded
    tombstone counters for the current execution, discard display bodies with
    explicit frontend omission counts, and ACK row batches with
    `retainMoreRows: false`. Store up to three largest budget owners as
    `{ tabId, label, retainedBytes }` for actionable UI copy. Do not close or
    evict a Query Session to recover display memory, and never rerun SQL
    automatically.
11. The UI shows a released-result message and a global-budget message naming
    the relevant tabs with Switch and Release-results actions. Releasing means
    the complete display payload, not the session or compact tombstone. Copy must not
    imply discarded results can be recovered without rerunning the query.

### State, compatibility, and errors

12. Key all new state by Query Tab `id`, never label. Model nullable database
    cells as `string | null`; convert null only at the grid display boundary.
13. PostgreSQL uses persistent sessions. Other relational engines continue to
    invoke `run_query` and normalize the single response into one
    `QueryExecution` result set.
14. `supportsPersistentQuerySessions(engine)` is the only activation switch.
    Rollback means disabling PostgreSQL there. Never fall back to `run_query`
    after the backend accepted an execution.
15. Model command failures as the exact `QuerySessionError` tagged union from
    Plan 001. Derive disabled-reason copy from transaction/session snapshots:
    `transactionActive`, `transactionFailed`, `transactionUnknown`,
    `executionRunning`, and `connectionClosing`. Do not parse backend messages
    to choose behavior.
16. In transaction `unknown`, expose Recheck, Rollback, and Close. A successful
    Recheck restores allowed actions. Recheck failure leaves results visible and
    state unknown.
17. Remove the full error-object console log. Render structured database errors
    but log at most payload-free kind and execution ID.

## Commands you will need

| Purpose | Command | Expected |
|---|---|---|
| Focused store tests | `pnpm test -- src/lib/store.test.ts` | pass |
| Focused UI tests | `pnpm test -- src/components/query-editor-panel.test.tsx src/components/settings-view.test.tsx src/components/credential-onboarding.test.tsx` | pass |
| Slice boundary | `pnpm check:slice-isolation` | exit 0 |
| Format | `pnpm format` | exit 0, no changes |
| Lint | `pnpm lint` | exit 0, no warnings |
| Types | `pnpm typecheck` | exit 0 |
| Frontend suite | `pnpm test` | all pass |
| Rust regression | `just test` | all non-ignored pass |
| Diff hygiene | `git diff --check` | no output |

## Suggested executor toolkit

- Use the repository `html` skill for Step 1 static mocks.
- After editing multiple TSX files, use the React best-practices skill if
  available.

## Scope

**In scope**:

- `plans/mocks/query-session/variant-a.html` (create)
- `plans/mocks/query-session/variant-b.html` (create)
- `plans/mocks/query-session/variant-c.html` (create)
- `plans/README.md` status and selected-mock note only
- `src/lib/query-session-channel.ts` (create)
- `src/lib/store/types.ts`, `index.ts`, `README.md`
- `src/lib/store/relational-queries.ts`, `workspace-tabs.ts`
- `src/lib/store/connections.ts`, `credentials.ts`
- `src/lib/store.test.ts`
- `src/components/query-editor-panel.tsx`
- `src/components/query-editor/toolbar.tsx`, `results-view.tsx`
- `src/components/query-editor/status-items.ts`
- `src/components/query-editor/use-monaco-query-editor.ts`
- `src/components/query-editor/use-query-outcome.ts`
- `src/components/workbench/relational-workbench.tsx`
- `src/components/workbench/object-tab-row.tsx`
- `src/components/workspace-tabs.tsx`
- `src/components/settings-view.tsx`, `credential-onboarding.tsx`
- `src/components/settings-view.test.tsx` (create)
- `src/components/credential-onboarding.test.tsx` (create)
- `src/components/query-editor-panel.test.tsx`

**Out of scope**:

- Any `src-tauri` or infrastructure change; return to Plan 001 if its contract
  is incomplete
- MySQL, SQLite, ClickHouse, or Redis persistent sessions
- Table-preview, schema/admin, mutation, export, or `load_table_data` migration
- Fetch-more portals, disk spooling, or a virtualized million-row grid
- Configurable memory/session limits or persisted live sessions
- External mock hosting, production/daily-driver previews, commits, pushes, PRs

## Resume protocol

1. Read Plan 002 status and selected-mock note in `plans/README.md`.
2. Inspect scoped status/diff against the Plan 001 completion SHA.
3. Accept changes only through the last recorded completed step; compare them
   to that step's symbols and tests.
4. Unexplained or out-of-order changes are a STOP. Never discard user work.
5. Continue at the first incomplete step and update status after its gate.

## Git workflow

- Suggested branch: `feat/query-session-editor`, only if the operator asks.
- Do not commit unless authorized. If authorized, use a logical message such as
  `Activate PostgreSQL query sessions in the editor`.
- Never publish mock files externally, push, or open a PR without instruction.

## Steps

### Step 1: Produce and select local static UI mocks

Before real TSX edits, use the HTML skill to create three materially distinct
self-contained variants under `plans/mocks/query-session/`. Each must show:

- Run/Stop and active cancellation
- Autocommit/Manual, next-manual isolation, Commit/Rollback
- failed/unknown state with Recheck
- multi-result selector and command-only results
- Results/Explain/Output, structured notices, partial/truncated state
- released-result and global-budget ownership messages
- active-transaction close confirmation
- destructive credential-reset warning

Serve locally, for example from the repository root on an unused port, and
report localhost URLs. Do not upload or deploy. Verify wide and narrow
viewports, keyboard access, zero document overflow, and no console errors.

STOP and wait for the operator to choose A, B, or C. Record the selection in
`plans/README.md`, stop the local server, and only then continue.

### Step 2: Build the typed Channel reducer and retention manager

Add exact tagged unions matching Plan 001. Implement owner registration,
singleton heartbeat, focus/visibility kick, event identity/sequence validation,
four-batch cumulative ACK, terminal settlement, and promise cleanup.

Replace label-keyed preview state with tab-keyed sessions/executions. Implement
128-MiB accounting and deterministic background-terminal LRU payload release.
Preserve compact tombstones and expose budget owners when no eligible payload
can move. Release the prior execution payload when a new execution is accepted
for that tab.

Tests use fake timers and synthetic Channel events. Cover four-batch delivery,
coalesced cumulative ACK, terminal ACK before promise resolution, stale/gap
events, owner registration, heartbeat singleton, refocus heartbeat, exact byte
accounting, stable LRU ordering, protected current/running tabs, tombstone
preservation, frontend omission counters, and no-room drain.

**Verify**: focused store tests, slice check, and typecheck. Expected: all pass.

### Step 3: Integrate execution, compatibility, and lifecycle ownership

Refactor `runQuery`: PostgreSQL lazily opens and executes through the session;
other engines normalize the legacy result. History is appended once after
terminal ACK. `rowCount` is observed returned plus drained/omitted row-producing
rows, not affected rows. Extend Query Outcome with `cancelled` and update
EXPLAIN to use the first tabular result helper.

Remove sensitive error logging and add a console spy. Add named owner actions
for close/close-by-Connection/retarget/release-results. Workspace-tab actions
call them through `get()` before mutation. Await backend disconnect before
frontend disconnect or reconnect. Credential reset invokes named cleanup;
storage-mode migration does not.

**Verify**: focused store tests, slice check, typecheck. Tests cover same-label
tabs, close/retarget/disconnect races, late events, legacy engines, history once,
cancel/lost states, and payload-free console output.

### Step 4: Implement the selected query controls and output surfaces

Follow the selected mock exactly:

- Run becomes Stop during running/cancelling; no spinner or pulse.
- Isolation says it applies to the next manual transaction and is disabled in
  autocommit or non-idle manual state.
- Unknown shows Recheck, Rollback, Close, and concise disabled reasons.
- Result selector shows index, row/affected count, running/completed/partial,
  and truncation/release status.
- Output shows notices, timings, omission totals, and structured errors.
- Released payloads retain a compact summary and an honest rerun-required message.
- Budget blockage names owner tabs and offers Switch/Release results.
- Terminal/cancel status uses a live region; row batches do not.
- Narrow layouts wrap or use an accessible menu without document overflow.

Update workbench unions/callers. Null conversion remains at the grid boundary.

**Verify**: focused UI tests and typecheck. Expected: controls, accessible names,
notices, result selection, release/budget actions, close confirmation, reset
warning, and migration-copy preservation pass.

### Step 5: Activate PostgreSQL and exercise the complete flow

Keep `supportsPersistentQuerySessions("postgres")` false until Steps 2-4 pass.
Then enable it and run frontend plus Rust regressions. Exercise manually against
the test fixture:

- temp table persists across runs in one tab and is absent in another
- multi-result/notice/zero-row/partial output
- four batches arrive without sequential one-batch RTT blocking
- Stop settles exactly once
- raw BEGIN/error becomes failed or unknown according to observer truth
- observer failure shows Recheck rather than forcing Rollback
- background LRU release preserves the backend session, temp state, and compact summary
- four 32-MiB completed payloads do not starve a fifth tab when eligible
- active transaction close and destructive reset confirm rollback
- minimize/refocus does not prematurely close an execution or transaction

**Verify**: `pnpm format`, `pnpm lint`, `pnpm typecheck`, slice check, full
frontend tests, `just test`, and `git diff --check`. Expected: all pass and only
Scope files plus README status are modified.

## Test plan

- Store reducer tests carry most protocol/retention cases. Do not duplicate each
  through React.
- Component tests cover visible actions, disabled reasons, keyboard labels,
  notices, truncation, release/budget copy, and confirmations.
- Console spies prove query text, row/notice values, and database detail never
  reach frontend logging even though display rendering remains complete.
- Deferred-promise tests cover close, retarget, disconnect, terminal ACK, and
  late-envelope races.
- The manual fixture flow validates the boundary between Plan 001 and Plan 002.

## Done criteria

- [ ] A PostgreSQL Query Tab reuses one backend session; other engines stay legacy.
- [ ] State is keyed by tab ID and rejects stale generation/execution/sequence.
- [ ] Cumulative ACK permits four batches/4 MiB and terminal settles after ACK.
- [ ] One owner registration and heartbeat timer exist per renderer.
- [ ] Global retained execution payload never exceeds 128 MiB.
- [ ] Background terminal LRU release preserves session/transaction/tombstone.
- [ ] When release cannot help, UI identifies budget-owning tabs and drains rows.
- [ ] Stop, multi-result, notices, partial/truncated/released states render.
- [ ] Unknown exposes Recheck and never presents Rollback as the only recovery.
- [ ] Tab/retarget/disconnect/reset cleanup is awaited and stale-safe.
- [ ] No sensitive frontend logging remains.
- [ ] The selected local mock is recorded and implemented.
- [ ] All frontend, Rust regression, slice, format, lint, type, and diff gates pass.
- [ ] `plans/README.md` says `READY FOR REVIEW`; after an authorized commit,
      the reviewer/operator changes it to `DONE: <completion SHA>`.

## STOP conditions

Stop and report if:

- Plan 001 is not DONE or its implemented DTO/lifecycle/credit contract differs.
- No mock is explicitly selected.
- Cumulative ACK can return credit before state reduction or terminal promise
  settlement can precede terminal ACK.
- Global accounting cannot deterministically release only background terminal
  display payloads while preserving session state and compact tombstones.
- A legacy engine or non-editor SQLx caller would change behavior.
- A real TSX change is needed before Step 1 selection.
- A backend/source-infrastructure change is required; return to Plan 001 review.
- Any required verification fails twice or a change reaches outside Scope.

## Maintenance notes

- Row payload and session lifetime are intentionally separate. Future budget
  work may spill or paginate rows, but must never evict a transaction socket.
- The capability switch is the rollback seam. Do not add automatic fallback
  after accepted SQL.
- If Tauri gains a real renderer/channel closure signal, Plan 001 liveness can
  simplify without changing this reducer's identity checks.
