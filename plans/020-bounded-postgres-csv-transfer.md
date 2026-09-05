# Plan 020: Bounded PostgreSQL CSV import and export

- Priority: P1. Effort: L. Gap: PAR-010.
- Planned against: `ab3396816f9fff0dab3a4b682a56769ee03e8e2d`, 2026-09-05.
- Depends on: Plans 018 and 019, DONE at `de3272b` and `ab33968`.
- Status: IN PROGRESS: through Step 6. A implemented and reviewed; native manual gates pending.

## Outcome and scope

A PostgreSQL user imports a local CSV into an existing table or exports a whole
table to a local CSV without memory growing with the file or table. Native jobs
own execution beyond a tab's lifetime. The user reviews the target, mapping and
CSV settings, observes honest progress, and can request cancellation.

V1 is CSV, UTF-8, append-only import, one transaction, and fail on the first
error. Export includes the whole relation's committed data, including descendant
partitions, with no promise of row order. Grid filters, selection and staged
changes do not affect this export. Visible-row/selection exports keep their
existing bounded-by-loaded-data path. XLSX, non-CSV whole-table exports, DDL,
table-to-table copy, compression, split files, upsert, truncate-first, resumability,
reject files, skip-bad-row mode, transforms and additional engines remain follow-ups.
Do not imply that Plan 020 completes all transfer parity.

## Read before implementation

- `docs/agents/domain.md`, ADRs 0001, 0024, 0025 and 0028.
- `src/components/table-editor-panel.tsx`: `handleImportRows`,
  `exportWholeTable`, saved export tasks and `loadWholeTableForExport`.
- `src/components/table-editor/data-import-wizard.tsx`, `src/lib/import.ts`,
  `src/lib/export.ts`, `src/lib/export-tasks.ts` and their tests.
- `src-tauri/src/postgres/mutations.rs`: buffered `copy_import_rows`.
- `src-tauri/src/postgres/dedicated.rs`, `connect_spec.rs`, `backup/`,
  `src-tauri/src/commands/pg_backup.rs`, connection/global lifecycle fences.
- `src/lib/pg-tool-jobs/`: typed client, observer, lifecycle, restore review and
  completion refresh. Inspect native picker restrictions before reusing it.

The current wizard calls `file.text()` / `arrayBuffer()`, materializes every row,
then serializes rows through IPC. The backend builds another complete CSV string.
Whole-table export appends every page into one renderer array. These are the
specific paths being replaced for PostgreSQL CSV.

PostgreSQL reference: [COPY documentation](https://www.postgresql.org/docs/current/sql-copy.html).
Use client STDIN/STDOUT, never server file paths or PROGRAM. Verify behavior on
the repository's supported PostgreSQL versions; do not rely on newer skip-error
options. COPY column order, quoted NULL handling, defaults, RLS limitations and
partition behavior must be explicit in implementation and tests.

## Step 1: Design gate

Compare three static designs with identical capabilities:

- A: table Transfer sub-tab, setup and mapping beside job details.
- B: staged import/export dialog, with persistent jobs below the table grid.
- C: connection Transfer workspace, target list and cross-table job history.

Show import mapping, export review, running, cancellation requested, failed,
completed and unknown commit outcome. Use fictional fixtures, true black,
white primary text, dense rows, minimal copy, no decorative cards/pills, no
em dashes and no continuously repainting animations. Publish with the html skill.
STOP after reporting the review URL. Wait for Imran's pick before product edits.
Selection chooses placement, not weaker safety, memory or lifecycle guarantees.

## Step 2: Native contract and lifecycle

Write a focused ADR for PostgreSQL CSV Transfer Jobs. Preserve ADR-0028's native
backup/restore contract. Prefer a small sibling transfer manager using existing
connection resolution, fence integration and proven file publication helpers;
do not turn every background operation into a generic job framework. Extract
only genuinely shared file/lifecycle primitives with unchanged backup tests.

Define typed inspect/start-import/start-export/get/list/cancel/release commands,
with shared camelCase payload conventions and discriminated refusals. Transfer
snapshots contain job ID, connection/relation identity, direction, filename,
phase, elapsed time, source bytes when known, bytes read/written, and truthful
row counts. Use safe serialized counters. No paths, values or credentials in
retained history, logs, URLs or persisted app state. Bounded samples are transient
inspect responses, not job history. IPC errors are validated at the boundary.

Admission must reserve pending inspection/resolution as well as execution.
Default limits: four transfer workers globally, one per connection, 32 terminal
records, one-hour terminal expiry. Document these separately from backup limits;
combined work remains bounded. Inspection samples and file handles also need
capacity, TTL and explicit release. A slow filesystem must not hold the manager
mutex or stall unrelated job reads. Release refuses active jobs and is idempotent.

Integrate transfer ownership into every existing connection/global resource
fence and job-impact warning. Cover full save, disconnect/delete, credentials,
bastions, reset and app exit. Organization-only updates stay outside the fence.
Close admission before invalidation, fence pending resolution and reject stale
configuration generations. A tab closing must not cancel a job.

## Step 3: File inspection and bounded CSV import

Native selection returns a path or cancellation. Backend inspection accepts only
an absolute regular file and returns an expiring token, filename, byte size,
indexed source columns, a bounded sample, and explicit truncation indicators.
Read at most 256 KiB and return at most 50 records / 64 KiB of sample values.
Never scan the entire file to offer a preview or row count. A sample cut mid-record
is incomplete, not malformed; a truly over-budget record gets an explicit error.
Do not infer or silently alter header/dialect choices. Settings changes re-inspect.

CSV settings: header on/off, single-byte delimiter, quote and escape, UTF-8,
and a NULL token (default `\N`). Validate combinations and bounded setting lengths.
Reject invalid UTF-8; handle an optional leading UTF-8 BOM explicitly. Parse
incrementally with hard limits: 1 MiB per field, 8 MiB per record, at most 1,600
source columns; check before growing buffers. Input and COPY output chunks are
at most 64 KiB each, with no unbounded queue between file, parser and socket.
Large records fail clearly instead of bypassing limits.

Mapping is by source index, never source label. Duplicate/blank headers remain
distinct. Permit skipping/reordering source columns and choosing exact catalog
target names. Reject duplicate targets, zero mapped columns, missing targets and
unsupported generated/identity writes. Unmapped target columns use defaults;
review required columns without defaults. Resolve target identity and validate
its current columns again immediately before COPY to detect DDL since review.
Keep that identity stable through execution using appropriate transaction locks.

Preserve quoted versus unquoted NULL tokens and empty strings while parsing and
re-encoding mapped rows. Validate record width. Quote identifiers and CSV options
through typed builders; accept no arbitrary SQL fragments. Revalidate source
identity/size/modification metadata against inspection at admission, use the same
opened handle throughout execution, and check for changes before commit. Refuse
stale files and re-review; this is not a snapshot guarantee against concurrent
in-place writes, and the review must tell users to keep the file unchanged.

Use the dedicated PostgreSQL transport for TLS, SSH, credentials and driver
options. BEGIN, stream COPY FROM STDIN with backpressure, finish COPY, then COMMIT.
No chunk commits. Handle read/parse/COPY failures by aborting and rolling back.
Refuse unsupported relation kinds and RLS-protected imports with an actionable
reason; do not weaken policy or silently use an unbounded fallback. Verify actual
privileges at execution; capability information is advisory.

Use a fixed documented date/time session interpretation (ISO dates and UTC) and
show it in review. Leave richer locale/timezone conversion for a separate plan.
Triggers, constraints and defaults retain server behavior. Transaction rollback
does not promise reversal of sequence increments or external trigger effects.

## Step 4: File-backed whole-table CSV export

Export only catalog-resolved columns of a selected relation via a generated
COPY (SELECT ... FROM qualified_relation) TO STDOUT. Include partitions and honor
server SELECT policies. Use one statement snapshot, not offset pagination or an
extra count scan. Supported relation kinds and unpopulated materialized views
must produce explicit capability/refusal states. Execute in a read-only transaction.

Write streamed chunks to a private, unique create-new sibling partial. Reuse the
non-overwriting publication guarantee from backup jobs: successful stream end,
file flush/sync, then no-replace publication. Existing paths and races creating
the destination must never overwrite. An empty relation is a valid export,
including a zero-byte file when no header is requested. Remove only the job's
partial on ordinary failure/cancellation. Explain hard-crash partial leftovers.

Export settings share the supported dialect, NULL and UTF-8 contract. Default to
headers. Do not send values or file bytes through IPC. Source size and row total
are unknown until trustworthy observations exist; display bytes written and
elapsed time rather than invented percentages. Report completed only after final
file publication. Once publication claims completion, cancellation cannot win.

## Step 5: Safety, cancellation and uncertain outcomes

Evaluate import through the existing backend insert-write policy using hydrated
connection state, before admission. Start unconfirmed, then consume typed
policyNeedsConfirmation with shared statement summaries. Retry only the identical
frozen request after approval if form, source token and connection revision are
still current. Read-only remains blocked. Invalidate pending reviews at the start
of lifecycle changes, even changes that later fail. Never automatically retry
start after transport uncertainty; reconcile jobs through list.

Define explicit phases and transitions for preparing, running, cancelling,
finalizing, completed, cancelled, failed and outcomeUnknown. Bytes consumed and
rows sent are not rows committed. Cancellation before commit begins aborts COPY
and rolls back; serialize the commit claim with cancellation. Once COMMIT is
sent, do not promise cancellation or rollback. A successful commit observed
while cancelling is success. Lost acknowledgement around COMMIT is outcomeUnknown,
requires checking the target, and must never auto-retry or fabricate success audit.
Retain warnings when shutdown cleanup times out. Release resources only after
owned work stops, with bounded cancellation/fence waits and explicit driver task
ownership. Verify that dropping a handle really stops the dedicated socket.

Record activity and required override audit only after observed successful
import commit (bounded best effort). Export activity follows successful publication.
Errors include a sanitized category, logical record/column when trustworthy, and
rollback/unknown status; never echo raw server context or row values. V1 stops
on the first error, with no claim to produce reject files or skip invalid rows.

## Step 6: Activate the selected design

Own observations outside dialog/tab lifetime. Reuse bounded non-overlapping
polling patterns: pause when hidden, reconcile on return/open, back off on
transport errors, fence late results and keep active jobs until terminal.
Observation failure is not job failure. Expiry is unavailable, not completed.
Show filename, exact target, environment, settings, mapping and append-only
transaction review. Keep paths transient. Render all terminal and race states.

Route desktop PostgreSQL CSV import and UTF-8/uncompressed whole-table CSV export
to native jobs. Never fall back to the old buffering path for those requests.
Saved CSV export tasks choose a fresh destination each run; explicitly explain
unsupported encoding/compression combinations. Preserve existing formats and
non-PostgreSQL paths with honest boundedness labels. Keep XLSX reachable.
Browser fixtures do not claim to perform native transfers.

On first observed committed import, invalidate affected table data and browse
sessions, preserving staged edits but requiring refresh/review before applying
stale drafts. Because triggers may affect other tables, conservatively invalidate
connection data caches; avoid silently discarding query results or pending edits.
For outcomeUnknown, mark data as needing refresh and warn against a blind retry.
Add transfer jobs to lifecycle warnings so closing a connection explains impact.

## Step 7: Validation and completion

Focused Rust tests must cover parser chunk boundaries, quoted newlines and NULLs,
BOM/UTF-8, duplicate headers, mapping/defaults, hard limits, malformed final rows,
slow producers/consumers, bounded inspection, stale tokens/files, SQL quoting,
atomic import failure, policy refusal, RLS refusal, cancellation versus commit,
unknown commit acknowledgement, export destination races, empty exports, cleanup,
TTL/capacity, and connection/global fences. Exercise failures after valid prefixes.

Use a disposable PostgreSQL fixture and temporary files only. Name the isolated
target before starting it. Integration tests cover partitions, constraints,
trigger side effects, defaults/identity, RLS, TLS and reconnect/teardown. Measure
renderer and native memory with increasing synthetic file sizes (e.g. 32 MiB and
512 MiB); demonstrate a bounded plateau apart from fixed driver/runtime overhead,
including an oversized-field rejection. Tests must not hide ignored live coverage.

Frontend tests cover stale safety reviews, uncertain admission reconciliation,
job lifetime independent of tabs, source-index mapping, native picker cancel,
legacy format routing, saved tasks, refresh preservation and every result state.
Native manual verification covers chosen layouts at wide/narrow widths, actual
file dialogs, export/import round trip, cancel, destination exists and reconnect.

Run `pnpm format`, `pnpm lint`, `pnpm typecheck`, `pnpm test`, `just fmt`,
`just lint`, and `just test`. Use a fresh read-only review agent after implementation
per implement-plan. Fix actionable findings and revalidate. Update README,
PAR-010, ROADMAP, glossary and ADR only to the extent delivered. Obtain separate
commit/push authorization as required by the plan register. No live database or
daily-driver build/preview changes are authorized by this plan.

## Execution record

- 2026-09-05: Plan 019 completion recorded from operator confirmation at `ab33968`.
- 2026-09-05: inspected current CSV import/export, native job lifecycle and safety
  contracts; wrote Plan 020 and three static placement options.
- 2026-09-05: Imran selected A, the table Transfer sub-tab. Design gate passed.
- Draft checks: `pnpm format`, `pnpm lint`, `pnpm typecheck` and `git diff --check` passed. Static HTML exercised all 30 layout/direction/job-state combinations with jsdom without script errors.
- Initial visual QA was blocked by unavailable browser tooling; resolved in the design revision below.
- At the initial planning gate, no product or Rust files had changed and product test suites were not rerun. Repository changes remain uncommitted.
- Private review published: https://dbunk-plan-020-review.imran-vz.chatgpt.site

### Design revision: app fidelity (2026-09-05)

- Replaced the wireframe presentation with the current app shell and control styles,
  checked against `src/styles.css`, Button, activity rail, table header, the
  Backup / Restore workspace, and its captured native-style app screen.
- Embedded JetBrains Mono and the existing Tabler SVGs. Used the current amber
  accent, 40px rail, object tabs, compact controls, navigator and status bar.
- A retains table context; B uses a mapping dialog and persistent grid job tray;
  C uses a connection workspace with a table selector. Product code is unchanged.
- Browser inspected at 1440×1000, 900×700 and 390×844. All 33 supported layout,
  direction and phase combinations rendered without page-width overflow.
  Sample start/cancel, close/reopen, filename selection and duplicate-target
  mapping validation passed. A fresh browser load reported zero page errors.
- `pnpm format`, `pnpm lint` and `pnpm typecheck` passed for the mocks. Product/native transfer verification remains required.

### Implementation validation (2026-09-05, ongoing)

- Selected A is implemented in the real table workspace. Browser fixture checks
  use the actual React components, styles and app shell, with mocked native IPC.
  Import review, mapping, start, cancel, dismiss and export setup were exercised.
  Wide (1440 × 1000) and narrow (900 × 700) layouts were inspected without page
  overflow or browser errors. A narrow sub-tab wrapping issue was fixed and
  rechecked. Screenshots: `/tmp/dbunk020-real-a-wide.png` and
  `/tmp/dbunk020-real-a-narrow.png`.
- All required automated gates passed after review fixes: `pnpm format`,
  `pnpm lint`, `pnpm typecheck`, `pnpm test` (123 files / 1,437 tests),
  `just fmt`, `just lint` and `just test` (535 passed / 49 opt-in tests ignored).
  The new CSV live cases below were run explicitly, not claimed from that
  default ignored count. Final fresh read-only review passed with no findings.
- Explicit live CSV integration passed against disposable PostgreSQL 16 on
  port 15432: CSV round trip, custom dialect, Unicode/NULLs, defaults, triggers,
  late constraint/parse rollback, partitions, RLS import refusal, source/catalog
  changes, destination collision, oversized export refusal and teardown cancel.
- Identity default generation and mapped-identity refusal also passed in the
  final live rerun after bounded filesystem worker changes.
- Verified TLS export and hostname mismatch refusal passed against a separate
  disposable PostgreSQL fixture on port 15433 with a temporary CA.
- The existing live Query Session plaintext and TLS cancellation tests also
  passed, exercising the shared dedicated-driver lifecycle change.
- Native memory measurement used the test executable directly under macOS
  `/usr/bin/time -l`, importing and exporting generated 32 MiB and 512 MiB files,
  then rejecting an oversized field. Final post-review peak RSS: 32,538,624
  bytes and 32,522,240 bytes respectively (31.03 MiB and 31.02 MiB).
  Wall time: 3.30 s and 31.85 s. Logs:
  `/tmp/dbunk020-memory-32-reviewed.log`, `/tmp/dbunk020-memory-512-reviewed.log`.
- An isolated native build launched with config directory
  `/tmp/dbunk-plan020-native/profile` and a separate Cargo target. Desktop
  automation returned `cgWindowNotFound`; screen capture exposed no app window.
  Actual OS file-dialog interaction and native renderer memory measurement
  remain unverified. Browser fixture checks do not substitute for these gates.
  No daily-driver profile, preview channel or live database was touched.
- The first fresh review found four issues: export guards sized a text cast
  rather than the actual UTF-8 type output; CSV error columns were displayed
  one too high; an incomplete first preview record produced an unusable mapping
  review; final file publication could block an async runtime worker. A fresh
  fix agent corrected all four. Export now shares a canonical UTF-8 projection
  between the guard and COPY, with dedicated limit errors; the other fixes
  preserve one-based diagnostics, explicitly refuse an unusable inspection,
  and publish through bounded filesystem workers. The new live regression
  covers custom casts, all-null composites, field/record limits and an unrelated
  database error. All three CSV live integration tests passed after these fixes.
  The second fresh review found that replacing an existing column default did
  not change the private catalog signature, and late picker/inspection responses
  could retain a review token after the table unmounted. Fresh fix agents added
  a bounded default-expression fingerprint and a hook lifetime fence, with
  passing live and deferred-response tests.
  It also found that forced teardown aborted transfer tasks without joining
  them before resource invalidation. A fresh fix agent corrected that ordering
  and added a passing resource-drop assertion at the fence boundary, including
  shared completion waits when a cancellation watchdog owns the join handle.
  Final full validation passed, as did all four explicit live CSV cases. A
  fresh follow-up review found no issues in these three fixes or their
  integration. The isolated preview is closed; disposable database fixtures
  are removed after validation. Changes remain uncommitted.
