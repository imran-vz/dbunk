# Plan 022: PostgreSQL schema comparison activation

- Priority: P1. Effort: L. Gap: PAR-008.
- Planned against: `9312b41ab2d2c92f48b54d2b3229332bf74641a2`, 2026-09-14.
- Depends on: Plan 021, completed at `9312b41`.
- Execution status: see [README.md](./README.md). Steps 1–4 and the frontend
  half of Step 5 were implemented and verified on 2026-09-14. The native/WebView
  fixture gate ran on 2026-10-01 and the plan is ready for review. See the
  execution records at the end of this plan.
- Visual reference: [Object inspector mock and plan](./mocks/schema-compare/index.html).
- Published review: [private brief and mocks](https://dbunk-schema-compare-plan-022.imran-vz.chatgpt.site).
- Selected mock: **A, Object inspector**, chosen by Imran on 2026-09-14.
  [Selected reference](https://dbunk-schema-compare-plan-022.imran-vz.chatgpt.site/#A).

## Outcome and boundary

Let users compare two explicit Connection/schema endpoints, inspect ordinary
PostgreSQL table definitions and understand both known differences and coverage
limits without leaving dbunk. Activate the existing native foundation through
its typed client. Preserve the named `postgres16OrdinaryTableProjectionV1` scope
and normalization version 1 in ADR-0030.

This slice is read-only. It does not generate or execute migration SQL, compare
row data, infer renames, support other PostgreSQL majors, add object kinds or
change normalization. Values shown in the inspector are captured facts, never
executable SQL. Two PG16 minor versions may compare structured facts while
rendered expressions remain not comparable because their versions differ.

## Evidence and design constraints

Read these before implementation:

- `docs/agents/domain.md`, CONTEXT.md's PostgreSQL Schema Comparison entry and
  `docs/adr/0030-postgres-schema-comparison-foundation.md`.
- `src/lib/pg-schema-compare/protocol.ts`: phase and difference unions, coverage,
  exact identities, paged object/field results and bounded value references.
- `src/lib/pg-schema-compare/client.ts`: request creation, validated reads,
  response ownership and acknowledgement. Use this boundary for every page.
- `src-tauri/src/postgres/schema_compare/manager/`: backend owns jobs, endpoint
  admission, cancellation, retention and teardown independently of views.
- `src/lib/pg-tool-jobs/observer.ts` and `src/components/app-shell.tsx`:
  established application-owned observation and visibility lifecycle patterns.
  Reuse the pattern, not the backup-specific state or completion effects.
- `src/components/workbench/relational-workbench.tsx`, `workbench-shell.tsx`,
  `src/components/app-shell/activity-rail.tsx`, and `workbench-header.tsx`:
  actual workbench integration and connection identity.
- `designs/DESIGN-SYSTEM.md`, `src/styles.css`, and
  `src/components/pg-tool-jobs/workspace.tsx`: established metrics and native-tool
  styling. 40px rail, default 36px header/toolbars, 26px controls, 24px status
  bar; system UI text and bundled JetBrains Mono for identifiers/values.

Keep the product's shell and primitives. Dark mock content uses #000 with white
primary text, amber selection, flat separators, minimal copy and no continuous
animation. Preserve normal light-theme behavior when implementing. Treat source
and target identities as explicit form state; the global connection switcher
must never silently retarget an accepted job or its result.

## Selected entry and layout

Add a PostgreSQL-only **Schema compare** activity-rail destination, adjacent to
Backup / Restore. It opens a full-width workspace under the existing header;
comparison owns its object list rather than duplicating the database navigator.
The active connection seeds Source on first open only. Both endpoint identities
remain visible, including environment and schema. Explicit endpoint controls
select any stored PostgreSQL connection; native admission determines availability.
Schema names can be typed exactly, with loaded schema suggestions when available.
Do not require a complete frontend catalog or silently turn an unreadable schema
into an empty result. No new query/table tab kind or durable result persistence
is needed for this slice.

Imran selected **A: Object inspector** on 2026-09-14. Implement its paged object
list on the left, the selected table's field matrix on the right, and the
source/target value inspector below the fields. Coverage and capture metadata
remain reachable above the result; session jobs remain below it.

Use the existing panel-pressure conventions: the object list starts at 260px,
yields width as space tightens and moves above the inspector when two useful
panes no longer fit. Keep its selected object and page when the layout changes.
The field matrix scrolls horizontally within its pane. Endpoint controls wrap;
source and target identities stay visible. The two value panes stack on narrow
windows. Density changes spacing through existing tokens, never text size.

The cleaned mock retains only Object inspector and its state examples, with
the current planning brief. This plan and README.md hold selection and execution
status. Fixture values are invented.

## Behavior contract

### Endpoint form and admission

- Source is the reference and Target is the compared endpoint. Use **Source
  only** and **Target only**, not inferred add/delete actions.
- Both connection and schema are explicit. Editing a connection clears that
  side's schema; validation never trims or rewrites a valid catalog identifier.
- Separate draft endpoints from accepted-job endpoints. Edits invalidate the
  draft's comparison view; retained jobs remain accessible by their own identity.
- Compare creates one request with `createSchemaCompareRequest`. Disable duplicate
  submits until admission is resolved. On transport uncertainty reconcile list/get
  by the original requestId; any replay uses the exact request, never a fresh ID.
  An observation failure is not proof of failed admission. Surface uncertainty
  with a retry-observation action; do not silently start another job.
- The native backend remains authoritative for version, engine, credentials,
  safety/read-only compatibility, missing schemas and per-endpoint/global busy
  limits. Browser-only execution states that native runtime is required.

### Observation and ownership

Use a small comparison-owned observer and result reader under
`src/lib/pg-schema-compare/`. Keep job status separate from selected result pages.
Infer types from the existing protocol. Avoid a general job framework or a new
application-wide cache abstraction.

One application-owned observer polls serially while visible with consumers or
active/uncertain work, starting at one second with bounded failure backoff to
15 seconds. No overlapping polls, fixed setInterval loops or idle endless polling.
Resume with an immediate reconciliation on visibility return. View unmount stops
view reads and drops their retained payloads; it does not cancel a native job.
Show up to the backend's four job records with endpoint identity and phase.

Cancel transitions through cancelling until the backend confirms a terminal
state. Explicit **Dismiss** releases a terminal job, then removes local pages.
Release failure retains a visible retry action. Rerun is a deliberate fresh
request after admission reconciliation, not an automatic reaction to rerender.
Connection disconnect/edit/delete, result expiry, app lock and transport loss
must invalidate further reads and discard stale pages. A completed result is an
immutable capture, never a live synchronized schema view. Do not persist job
IDs, result IDs, raw values or credentials. On document reload reconcile native
jobs; after process restart there are no durable comparison results to restore.

### Truthful result presentation

- Read metadata first. Always show supported scope and a route to exclusions,
  capture times, server versions and consistency. Independent captures are not
  a single cross-database snapshot.
- Render all existing difference kinds exhaustively. `equal` means **Equal
  within scope**. `notComparable` always has its reason. Known changes may coexist
  with incomparable fields; neither may hide the other.
- An excluded counterpart is not directional absence. Use eligibility reads to
  explain excluded objects. Excluded categories without complete counts show
  **Not compared** or **At least N**, never zero or an invented exact total.
- The current metadata has an object total, not global per-kind totals or a
  search/filter command. Initially use explicit server pages and label positions
  **1–100 of N**. Do not advertise global search, sorting or status counts by
  deriving them from the visible page. No eager scanning of every page.
- Render field paths with column/constraint/index identities and order intact.
  Constraint-owned indexes keep their owner visible; grouping must preserve
  every returned field. Identifiers and raw values render as text, never HTML.
- Default the table inspector to the current field page. Show source/target
  values lazily on selection; a missing side says **Absent**, distinct from an
  observed NULL, empty string or an excluded definition.
- Values use explicit chunk navigation. Show offset and byte total for partial
  values. Do not concatenate all chunks or parse truncated JSON. JSON facts can
  remain raw text until a complete bounded value is explicitly loaded.

### Frontend resource budget

A view retains one object page, one field page, metadata and one selected
source/target chunk pair. Object/field pages contain at most 100 summaries and
are bounded to 1 MiB each by the native contract. Each raw value chunk is at most
64 KiB. Replace pages and chunks rather than append to caches or history.

Serialize view reads through a reader with at most one outstanding native read;
queue only the latest user selection. Each navigation/result change increments
an epoch. A late response still completes the client's acknowledgement but never
updates a newer view. Changing jobs clears every prior result payload. The client
already acknowledges fulfilled responses even if parsing fails; do not bypass it
or release an uncertain native response lease through a timer.

Use page cursors/returned UTF-8 byte offsets, not JavaScript string length. Reuse
plain text rendering instead of mounting a Monaco editor for every value. Do not
add speculative prefetch. These logical limits do not establish JS heap/RSS
bounds; measure the real native/WebView path in the verification step.

### Empty, error and narrow-window states

Provide initial endpoint selection, resolving/reading/comparing, cancelling,
cancelled, failure, expired/unavailable result, empty supported projection,
equal-within-scope and mixed changed/incomparable states. Phase text and returned
object counts replace fake percentages and animated spinners.

Map known native errors to concise actionable text. `unsupportedVersion` names
the side/version and PG16 boundary; `captureChanged` offers a deliberate rerun;
`busy` points to active jobs; limits say no complete result was produced;
`unavailable` must not claim to distinguish expiry from every other cause.
Transport/validation/acknowledgement failures remain separate observation errors.

At narrow widths collapse the comparison object pane into a selector or stacked
region while keeping both endpoint labels and coverage reachable. Contain wide
field/value rows within their own scroll area. Preserve the type ramp and focus
order; do not shrink text to make a desktop layout fit. Use shared density tokens.

## Implementation sequence after authorization

1. **Design selected:** A, Object inspector. Entry point and narrow-window
   behavior are specified above. Preserve its layout and the resource contract.
2. **Observer and reader:** add typed comparison-owned state, request
   reconciliation, bounded polling, serialized paging, stale-response fencing
   and explicit terminal release. Focused tests cover lifecycle and uncertainty.
3. **Workbench activation:** add the rail destination, endpoint form, phase/job
   controls and application observer mount/cleanup. Keep existing backup/restore
   and other engines' routes intact. No native writes or catalog broadening.
4. **Results and coverage:** implement the selected layout, semantic field labels,
   lazy value chunks, exclusions, capture metadata and all empty/error states.
5. **Failure and boundedness validation:** run focused frontend tests and the
   native/WebView fixture scenarios below; repair actual findings within scope.
6. **Completion review:** run required checks, record exact evidence and limitations,
   update the register/roadmap and set READY FOR REVIEW. DONE requires a separately
   authorized completion commit; do not mark full PAR-008 complete.

## Expected file scope

- New: `src/lib/pg-schema-compare/observer.ts`, `reader.ts`, presentation helpers
  and focused tests; `src/components/pg-schema-compare/` workspace and panels.
- Integration: `src/components/app-shell.tsx`,
  `src/components/app-shell/activity-rail.tsx`,
  `src/components/workbench/relational-workbench.tsx`, relevant workbench policy
  and tests; `src/styles.css` only for the scoped native-tool surface if needed.
- Existing `client.ts`/`protocol.ts` only for demonstrated activation defects,
  retaining current native wire contracts and validation.
- Documentation: this plan, `plans/README.md`, `plans/parity-gap-register.md`,
  `ROADMAP.md`, and ADR-0030's activation/validation status when actually verified.
- Rust and connection lifecycle changes are not planned. Stop and revise scope
  if a native defect prevents truthful presentation or reliable cleanup.

## Focused verification and completion gates

Frontend tests must cover lost start response/reconciliation without duplicate
admission, out-of-order/late reads, switching results/endpoints during reads,
cancellation vs completion, terminal release failure, visibility pause/resume,
view unmount, unavailable results, corrupt response handling and valid chunk
byte offsets. Test behavior through the client seam; do not mirror every component.
Component checks cover mixed changed/incomparable results, directional absence,
excluded counterparts, incomplete counts, no-comparable-fields and equality labels.

In an isolated native build against owned disposable fixtures:

- Same-connection schemas, independent PG16 databases, different PG16 minors
  with rendered-field coverage reasons and a refused PG17 endpoint.
- Concurrent DDL, cancel during resolution/capture, endpoint disconnect/reconnect,
  tunnel teardown, document reload and view switching during late reads.
- Cap-sized object/field pages and large escaped/multibyte values. Inspect
  actual IPC acknowledgement, renderer allocations and UI responsiveness.
  Repeat at least 20 next/previous object/field/value selections and job switches;
  record native RSS and WebView heap/process memory before, peak and after cleanup.
  Verify retained payload counts stay fixed and comparable post-GC measurements
  do not grow monotonically. Record allocator noise and platform limitations;
  no fabricated absolute RSS threshold or claim of an unmeasured platform.
- Keyboard operation, narrow/default desktop widths, density settings, actual
  dark/true-black comparison styling and existing light-theme readability.

Run `pnpm format`, `pnpm lint`, `pnpm typecheck`, and focused Vitest suites.
Rust changes additionally require `just fmt`, `just lint`, `just test`.
Use no production/live databases or daily-driver channels. Name the isolated
validation target before starting it. A browser-only mock cannot establish native
IPC/memory correctness; record a blocked native gate honestly and leave the plan
incomplete if that validation cannot run.

## Stop conditions and deferred work

The layout-selection gate is satisfied by Imran's choice of A. Product
implementation remains a separate action from this planning request. Stop to
revise this plan if the selected design needs server-wide
filtering/counts, unsupported coverage, persistent jobs or a changed native wire
contract. Do not turn those gaps into frontend full-result scans.

Migration generation/review/apply, rename confirmation, schema synchronization,
row-data comparison, broad PostgreSQL object coverage, compatibility beyond
PG16 and other engines remain later PAR-008 slices. Plan 022's success is a
truthful, responsive read-only comparison surface.

## Execution record (2026-09-14)

Implemented against `9312b41` plus the uncommitted planning updates. No Rust,
wire-contract or connection-lifecycle changes were made; `protocol.ts` only
gained two type-only exports (`SchemaCompareRelationIdentity`,
`SchemaCompareObjectSummary`).

**Delivered**

- `src/lib/pg-schema-compare/observer.ts`: application-owned serial poller
  (1 s start, failure backoff to 15 s, no polling when hidden or idle), exact
  request reconciliation by `requestId` after a lost or unreadable start
  response, cancel through `cancelling`, explicit release with `unavailable`
  tolerated and every other failure rethrown.
- `src/lib/pg-schema-compare/reader.ts`: one bounded result view per mounted
  workspace. At most one outstanding native read, only the latest intent
  queued, epoch fencing so late responses complete the client acknowledgement
  but never update a newer view. Retains one metadata detail, one object page,
  one field page, per-side eligibility and one chunk per side; page cursors
  and UTF-8 byte offsets come from the native responses. `unavailable` drops
  every payload.
- `src/lib/pg-schema-compare/failure.ts` and `presentation.ts`: failure
  decoding that keeps transport and validation failures separate from native
  errors, phase/difference/reason/category labels, field-path rendering with
  constraint owners and key positions, and incomplete-count formatting.
- `src/components/pg-schema-compare/`: Object inspector workspace (endpoint
  form with exact schema input and loaded suggestions, phase and job controls,
  coverage and capture metadata, paged object list, field matrix, lazy value
  inspector with chunk navigation, session job list with Cancel/Dismiss and
  retained failure text). Draft endpoints and the selected job live in a
  non-persisted store; the active connection seeds Source on first open only.
- Workbench: `schema-compare` rail item next to Backup / Restore, hidden and
  normalized for non-PostgreSQL connections; `AppShell` mounts the observer
  with the other native observers; dark mode uses the true-black surface.

**Verification (frontend)**

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: pass.
- `pnpm vitest run`: 1484 tests pass, including the new suites
  `observer.test.ts` (7), `reader.test.ts` (8), `presentation.test.ts` (4),
  `panels.test.tsx` (6), `workspace.test.tsx` (4) and the extended
  `relational-workbench.test.tsx`. They cover lost start reconciliation
  without duplicate admission, late and out-of-order reads, switching results
  and selections during reads, cancellation reported as completion, release
  failure, visibility pause/resume, view close, unavailable results, corrupt
  pages with acknowledgement, UTF-8 chunk offsets, mixed changed/incomparable
  fields, directional absence, excluded counterparts, incomplete counts,
  no-comparable-fields and equality labels.

**Native and WebView validation (Step 5, second half)**

Not run on 2026-09-14: that session could not drive the desktop app against
fixture databases. It ran on 2026-10-01; see the record at the end of this plan.

**Review fixes (2026-09-14, two-axis code review against `9312b41`)**

- Standards: the workspace now uses the shared `EmptyState`, `ErrorState`,
  `LoadingState` and `LoadingBar` primitives instead of a local notice panel;
  endpoint identities use the shared `EnvironmentBadge`; the endpoint select
  matches the pg-tool-jobs control (strong border, one crisp accent ring).
- Spec: transport loss during a read drops every retained page and Retry
  reopens the result; removing a connection used by the selected job drops
  its result at once and any connection change triggers an immediate
  observation refresh; "Run again" after `captureChanged` resubmits the failed
  job's own endpoints (and shows them in the draft) rather than an edited
  draft; a `notComparable` field row shows its reason inline; an excluded
  object shows its per-side eligibility above its fields, and a missing side of
  an excluded definition reads **Excluded (reason)**, never **Absent**.
- Duplication: one shared value-state helper for the field cells and the value
  panes, one phase-tone map for the job list, shared side labels, relation and
  field-path keys for identity, and a single `SchemaCompareSide` type.
- Not changed: the three native job observers stay separate, since this plan's
  observation section rules out a general job framework; the two type-only
  exports in `protocol.ts` remain; the documentation edits outside this plan's
  listed scope are left for Imran to confirm.
- Tests added: reader transport loss and reopen, workspace rerun endpoints and
  connection removal, field matrix excluded-side labels. The duplicate
  pg-tools rail normalization test was removed in favour of the `it.each`.

## Native and WebView validation record (2026-10-01)

Run against `3431c0e` plus this working tree. No Rust, wire-contract or
connection-lifecycle change was needed.

**Target.** An isolated debug native build (`DBUNK_DEV_CONFIG_DIR` under
`/tmp/dbunk-plan022-gate`, identifier `codes.imran.dbunk.plan022gate`, its own
cargo target) in the real WKWebView on macOS 27.0.1, Apple M4 Pro. Endpoints
were newly created, loopback-only, tmpfs containers removed afterwards:
PostgreSQL 16.15, 16.14 and 17.11 (Debian, aarch64) and an SSH bastion. No
production database, shared compose fixture or installed app data was used. The
harness is `infrastructure/test-db/schema-compare/webview-driver/`
(`fixtures.py`, `walkthrough.py`, `helpers.js`); it clicks and reads the
rendered workspace through an evaluation bridge.

**Result.** 117 scripted checks pass with the production frontend bundle. An
earlier pass against the Vite dev server covered the scenarios up to the
paging loops with the same outcomes.

| Scenario | Observed |
| --- | --- |
| Same-connection schemas | Completes in about 1 s; "one transaction on the same connection" is stated; changed, equal, source-only, target-only and not-comparable objects match the fixture; an incomparable `now()` default sits beside 25 known changes with its reason; a missing side reads **Absent**, an observed NULL reads **NULL**; an excluded counterpart shows per-side eligibility and never **Absent**; captured `<img onerror>` and `<script>` text renders as text. |
| Independent PG16 databases | Completes; "Independent captures. This is not a single cross-database snapshot." is shown; the changed default is found. |
| PG16.15 against PG16.14 | Completes; both versions shown; rendered expressions are not comparable with the version reason; structured facts compare. |
| PG17 endpoint | Refused on either side with the side, `17.11` and the PG16 boundary; no result. |
| Limits and absence | 1,001 tables: "The table count limit was exceeded. No complete result was produced." A missing schema, an empty pair and a views-only pair each read truthfully; none is called equal. A blank schema name is refused before native admission; three rapid Compare presses admit one job. |
| Cancel and busy | With a table locked, the job shows phase text and counts; a second start on the same endpoint is refused with the busy message; Cancel passes through **Cancelling…** to cancelled; the waiting backend is gone while the lock is still held. |
| Concurrent DDL | A rename, a new table and a drop committed inside the lock wait: the comparison completes from the post-commit state only. A table locked past both two-second lock waits fails as `captureChanged` after about 5 s; **Run again** resubmits the same endpoints and completes. |
| Disconnect, edit, delete | Disconnecting, editing or deleting an endpoint connection drops every retained page at once and shows the unavailable state; an active job ends and leaves no backend on either endpoint; a fresh comparison works after reconnect. |
| Tunnel teardown | Killing the SSH sessions under an active job fails it as unreadable with no result and no backend left. The earlier completed capture stays readable as captured. Disconnecting the tunnelled connection invalidates its result. |
| Document reload | Both native jobs (one completed, one waiting on a lock) are reconciled after reload with nothing selected automatically; the active job finishes; the earlier result reads with a new transport token. |
| View switching | Twenty leave-and-return rounds during object reads and thirty rapid selections: no error or stale page, only the last selection is read and shown, every delivered read acknowledged. |
| Visibility | Minimized: one reconciliation after start, then no observation for 5 s while the native job completes. On return the list is read within 5 ms and the result opens within about 30 ms. |
| Result expiry | After the ten-minute lifetime the job is gone from the native list, the view reads unavailable and holds no pages. |
| Cap-sized pages and values | 1,000 tables per side page as `1–100 of 1000`; a 400-column table pages 100 fields at a time. A 262,144-byte comment and pure three- and four-byte comments page in chunks of at most 65,536 bytes, cut on code point boundaries (65,535 and 65,533 bytes), with no replacement character and SHA-256 equal to the database value. |
| Layout, density, themes | 1200, 900 and 700 px wide: no page overflow, endpoints, Compare and the job list stay reachable, selection and page survive. The object list is 260 px beside the inspector and stacks above it at 700 px. Text stays 12 px at every width and density; rows are 22/24/28 px. Dark is `#000` with white text. All 121 visible controls are native, named and focusable. |

**Boundedness.** Two 24-cycle loops (object, field and value paging, and job
switches; about 15 interactions per cycle) made 934 reads. At most one read
was outstanding and one response unacknowledged at any moment; every read was
acknowledged; the largest response was 76,945 bytes. The rendered view held
100 object rows, 100 field rows and two value blocks at most, with identical
DOM node counts each cycle.

| Process footprint (RSS), MiB | Before | Two results | Paging loop | Value loop | After dismiss and close |
| --- | --- | --- | --- | --- | --- |
| Native | 37.7 (184) | 47.0 (193) | 46.4–47.2 (193) | 46.4–47.3 (193) | 37.0 (184) |
| WebContent | 275 (571) | 276 (572) | 306–324 (584–627) | 374–389 (632) | 176 (631) |

A separate 120-cycle value run moved the WebContent footprint 171, 409, 425,
418, 402, 337, 334 MiB at twenty-cycle marks and 162 MiB after cleanup, so it
plateaus and declines rather than growing with use. Interactions settled in
28–33 ms for paging, 84 ms (93 ms worst) for opening a 64 KiB value pair and
at most 70 ms for a chunk turn. No animation frame gap exceeded 35 ms.

**Findings**

- Repaired: `captureChanged` text. Native folds a catalog race and a lock wait
  that timed out twice into one kind, so "Definitions changed" alone was wrong
  for a table that was only locked. The message now names both.
- Repaired: `workspace.tsx` and `reader.ts` failed `pnpm format` at HEAD;
  whitespace only.
- Not a product defect: against the dev server the WebContent process grew by
  about 14 MiB per cycle (348 MiB to 2.0 GiB RSS in 120 cycles). React 19.2's
  development build keeps a User Timing entry per component render, and those
  for `FieldMatrix` and `ValuePane` carry 22–33 kB each. Clearing the entries
  returned the footprint to its baseline, and the production bundle shows no
  such growth. Memory and timing above are from the production bundle.
- Not a product defect: early stalls of 0.5–1.6 s on value turns came from the
  harness reading `innerText` of a 100,000-character pane on every poll.
- Observed, unchanged: native reports `excludedCounterpart` for an object
  excluded on both sides; the per-side eligibility lines state it correctly.
- Observed, unchanged: light-theme text from shared tokens measures 4.14:1
  (accent on selection), 4.16–4.63:1 (muted) and 3.17:1 (`text-warning`, the
  **Changed** badge). Names and values are 13.98:1; dark mode is 5.04:1 or
  better throughout. These tokens are app-wide.
- Observed, unchanged: `DBUNK_DEV_CONFIG_DIR` does not isolate the keychain
  entry or WebView storage. The harness guards the first and restores the
  second; a native fix is outside this plan.

**Limits of this evidence**

- One platform. Windows, Linux and a release-profile native build are
  unmeasured.
- macOS denied window capture and synthetic input. Styling was checked from
  computed styles and geometry, not pixels. Keyboard access was checked
  structurally (native controls, names, focusability, focus styles, document
  order), not by pressing keys.
- WKWebView exposes no JavaScript heap size and no way to force collection.
  The figures are process footprints with allocator noise; `vmmap` sampling
  briefly pauses the process and delayed a few observation calls.
- App lock was not exercised: the fixture profile uses plain SQLite credential
  storage.
- The last harness edit, restoring the theme and density found at start, was
  made after the final run and has not been executed.
