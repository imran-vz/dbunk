# Plan 028 implementation progress

Status: in progress. Shell A, Bottom review A and Tool tabs A are selected.
The selected query/table workflow is implemented in source. Scoped release-window
checks now pass for transactions, paging/filter/count, staged/recovered writes,
column preferences and SQL-review AX; see
[2026-10-03 window evidence](./table-window-verification-20261003.md).
The frozen Plan 027
human-check package is unchanged and predates these controls. Combined acceptance
and complete PostgreSQL parity remain pending.

## Backend and runtime

- Table Browse and Result Mutation commands delegate to host-neutral services,
  preserving Tauri payloads, policy and audit behavior.
- Opaque data documents bind ownership independently of visible tab IDs.
  Retired handles cannot reopen work or authorize old reviews. Connection and
  credential lifecycle operations retire affected handles.
- Eight long-running data requests have admission separate from the sixteen
  control slots used by cancellation and persistence. The backend owns admitted
  work if a UI future is dropped. Browse/count/analysis release startup admission
  before waiting for the database.
- Native browse, executor and socket tasks have connection-scoped joins. Local
  close first joins that document; an unsettled shared actor retires all data
  documents on its connection, aborts/joins actors within the shared deadline,
  and returns explicit `ConnectionDataClosed`. A missed join is an error.
- The native table runtime owns up to sixteen workers, bounded command/reply
  channels, cancellation, late-open cleanup and retryable close ownership.
  Cancellation before the first backend poll never dispatches the request;
  cancellation afterward waits for the actual backend outcome.
- Query confirmations bind exact SQL, bound parameters, row limit and session
  instance. Mutation tokens bind immutable plan/analysis to one data document.
  Stored policy is checked again on apply; tokens cannot be rerouted.

## Selected native controls

- Query tabs expose actual transaction state, mode/isolation, commit, rollback
  and recheck. Independent transaction control remains available without
  occupying the execution lane. Parameter fields preserve text versus NULL,
  absent versus empty bindings, and explicit row limits. Editing invalidates
  confirmation; late review responses cannot authorize changed SQL.
- Open table creates a typed document on the selected connection. Server paging,
  first/previous/next/last, exact count, cancellation, SQL filtering, sorting and
  page sizes use the owned runtime. Stale responses cannot settle a newer request.
  Restored typed filters remain visible and can be cleared.
- Table pages share one allocation between the model, virtual grid and change
  capture. Query/table data, analysis and review retention share the 128 MiB
  encoded-result allowance; delivery shares the 16 MiB queue allowance. These
  are encoded-payload limits, not a claim about total process RSS.
- Cell updates preserve NULL and exact text. Inserts accept a JSON object whose
  values are strings or null; omitted columns use defaults. Numeric JSON values
  are refused to avoid rounding. Deletes capture the original row identity.
  This initial editor does not replace the remaining specialized value editors.
- Bottom review shows included changes and exact SQL with bound values. Changes
  can be included/excluded or removed while offline; every such edit invalidates
  previous review. Applying or an uncertain previous outcome locks those edits.
- Apply waits for the exact SQLite revision containing its recovery marker.
  Cancelling before dispatch revokes the release even if a late save ACK arrives.
  A dispatched request settles only from its matching backend outcome. Strict
  confirmation uses a new save fence and the original one-use service token.
- Failed or unknown outcomes preserve changes. Restored drafts require fresh
  compatible analysis; an uninterpretable draft blocks new writes until explicit
  discard. Unknown writes require explicit reconciliation, with no automatic
  retry. Close protects staged intent; mixed draft export preserves it.
- Workspace format v2 stores table query/draft state and still reads SQL-only v1.
  Future/corrupt state is preserved. Snapshotting reads the current child model,
  and SQL/selection metadata is retained. Drafts have a separate limit of 128
  changes and 4 MiB encoded intent per table, with at most sixteen documents.
  Persistence retains its 448 KiB bound and refuses oversized saves visibly;
  apply cannot bypass that refusal. Export remains available.

## Verification

The owned local fixture is `dbunk-native-stage03`,
`127.0.0.1:15432/dbunk_demo`, instance
`2283820d-33ec-4c4c-ae03-7051092bd410`. Earlier live checks used new private
profiles and session-local tables or a generated schema. Ownership was checked
before connecting; cleanup restored activity to baseline. No daily-driver
profile or arbitrary endpoint was used.

- Earlier live data acceptance passed filtered/sorted paging, count, preferences,
  mutation analysis/review, Strict confirmation, stored read-only recheck,
  optimistic conflict and foreign/retired/replacement document handles.
- Two query live tests passed parameter/read-only refusal/auditing and independent
  transactions. The third initially failed in harness setup when reusing a retired
  ID. The corrected owner-retirement setup now passes its exact live rerun:
  `table-window-20261003/query-confirmation-rerun.txt`.
- Workspace v2 persistence tests passed. Native model tests cover stale pages,
  identity capture, budgets, recovery, offline selection and immutable review.
  Runtime tests cover owned close/cancel/delivery behavior; persistence tests
  cover exact-save barriers, refusal, lost writers and lossless mixed export.
- Native aggregate debug tests and all-target Clippy passed during development.
  Final integrated command evidence for this source increment is recorded in
  `table-source-checks/` once completed. Ignored fixture tests are not passes.
- Read-only review found and fixed an outgoing-editor focus bug. This source
  correction is included in the later scoped tab-switch and real Pinyin checks;
  broader focus traversal coverage remains pending.

Imran ended the human handoff by asking the agent to perform the checks.
VoiceOver is now deferred and non-blocking under the
[2026-10-03 scope decision](../027/accessibility-scope-20261003.md).
Keyboard/AX and real IME checks remain required. The later scoped real Pinyin
pass is recorded in [window evidence](./table-window-verification-20261003.md);
this does not establish every control or future tool.

## Remaining

Table column sizing/order/visibility and durable profile-local preferences are
implemented with focused tests and scoped actual-window/reopen evidence. Failed
or unsupported preference loads preserve storage and disable preference mutation;
saves publish only after acknowledgement. Source identity survives display order.

The later [browse/inspector source variant](./browse-inspector-source-checks/README.md)
implements typed/raw filter, sort NULL placement, history, named presets, exact
accepted SQL inspection, bounded value/JSON/UTF-8-hex inspection, typed JSON/array/
WKT literal editing, staged-cell reopening and schema-bound virtual-key controls.
Its Rust checks and two read-only fixture probes pass; new controls still lack
actual-window acceptance. The [later array/object slice](../029/catalog-array-source-checks/README.md) adds
per-element array add/remove and explicit NULL/text editing with raw fallback.
The [metadata/FK slice](../029/metadata-fk-source-checks/README.md) adds composite
foreign-key drill-down with exact connection/source selection and bound filters.
Its model and owned live probes pass; new-control window acceptance is pending.
Geometry map, complete query-result editing and remaining grid/copy/export actions remain.
The integrated real-window workflow, concurrent-tab/close/cancel races,
performance evidence and broader keyboard/AX coverage remain open. Real Pinyin
composition now passes for tested SQL/form/cell controls with exact restored SQL
and staged values; this does not cover every new/future tool control.
Plans 029 and 030 retain the wider tools, profile/package and full PostgreSQL
parity requirements. Service tests and source implementation do not establish
complete application parity.

## Duplicate/bulk rows and snapshot ownership

The [next native increment](./duplicate-bulk-source-checks/README.md) adds original-row
duplication and atomic literal/NULL bulk assignment for selected rows. Draft and
recovery payloads now have explicit retained accounting; candidate/editor work
requires shared admission. Raw/array editors bound retained text and undo history,
refuse oversized edits visibly, and preserve ordinary marked input. Real IME
acceptance for these guards remains pending.

Workspace saves measure borrowed draft payloads before cloning and retain exact
revision refusal semantics. The shared budget admits an 8 MiB persistence allowance.
Oversized workspace export snapshots require separate admission and use host-owned
joined file jobs. Query menu snippets append the three exact baseline templates
without execution. Required checks, native debug/release checks and a separate package pass, as
recorded in the increment's evidence. No new window or complete-parity pass is claimed.

## Guarded query-result UPDATEs

[The query-edit increment](./query-mutation-source-checks/README.md) retains submitted
SQL independently of editor changes and terminal ACK, routes a query document's
Bottom review through its owned data worker and exact journal revision, and stores
UPDATE-only recovery in workspace version 3. A successful apply invalidates rows
without rerunning SQL. Pending intent blocks silent rerun/clear/retarget/close.

This path deliberately refuses sources, types and captured guards whose separate
execution/analysis contexts cannot yet be proven equivalent. It is partial native
implementation, not full query-result parity or acceptance of new controls.
VoiceOver remains deferred. Actual-window keyboard/AX and real IME checks remain
required; prior table-window checks do not transfer.
