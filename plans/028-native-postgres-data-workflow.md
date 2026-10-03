# Plan 028: Native PostgreSQL data workflow

- Verification scope updated by Imran on 2026-10-03: VoiceOver is deferred and is not a blocking gate. Keyboard/AX and real IME checks remain required; see [the scope decision](./evidence/027/accessibility-scope-20261003.md).

- Migration stage 05. macOS Apple Silicon, existing pinned GPUI/Zed graph.
- Depends on Plan 027 workspace and lifecycle gates.
- Status: IN PROGRESS, service/native-admission and selected native UI implementation. Imran’s subsequent requests to continue allow development to overlap the pending Plan 027 real-window and IME checks; acceptance and replacement remain gated. The frozen human-check package is unchanged. Implementation requested by Imran on 2026-10-02 as part of complete PostgreSQL parity; table-review A, Bottom review, selected by Imran on 2026-10-02.
- Latest source scope: [browse/inspector/virtual-key checks](./evidence/028/browse-inspector-source-checks/README.md) and [structured arrays/object tools](./evidence/029/catalog-array-source-checks/README.md). [Composite FK navigation](./evidence/029/metadata-fk-source-checks/README.md) now has source/live evidence. New controls remain distinct from earlier scoped window passes.
- Latest row-action scope: [duplicate/bulk/snippet and retention checks](./evidence/028/duplicate-bulk-source-checks/README.md). Source is implemented; new-control window acceptance is pending.
- Latest grid scope: [auto-fit](./evidence/028/auto-fit-progress.md), [retained-row navigation](./evidence/028/grid-navigation-progress.md), and [ordered column pins](./evidence/028/pinning-progress.md) have scoped checks. Pinning source/package, AX projection/copy and table reopen pass; visual wheel/resize acceptance remains open. [Keyboard endpoints and page movement](./evidence/028/grid-keyboard-progress.md) are under verification. Independent/noncontiguous row selection remains unimplemented.
- Baseline: `102568b`; current native query milestone: `3f987c9`.
- Wider target: [native PostgreSQL parity inventory](./native-postgres-parity-inventory.md).
- Design: [three table-review options and full PostgreSQL scope](./mocks/native-postgres-parity/index.html).

## Outcome

Use the native workspace to browse a PostgreSQL table, filter/sort/page its data,
inspect values, stage and review identity-safe changes, and explicitly apply them
through the existing policy-enforcing services. Use query tabs with truthful
transaction state, commit/rollback/recheck and reconnect without SQL replay.
This milestone does not complete the wider PostgreSQL parity target.

The current workspace shell A remains selected. Imran selected A (bottom review) on 2026-10-02. The table-review design gate
is satisfied; B and C remain unselected alternatives. Shared backend work and the previously approved Plan 027 shell remain
authorized. No live database, daily-driver profile, release or dependency upgrade.

## Existing contracts to preserve

Read ADR-0021 (Query Session), ADR-0022 (Table Browse), ADR-0023 (Result Mutation),
ADR-0024 (policy), ADR-0031 (parameters/row limits) and ADR-0032 (host boundary)
before changing their respective services. Port behavior from
`src/components/table-editor`, `data-grid`, `mutation-review` and
`query-editor/transaction-controls.tsx`; do not infer parity from a screenshot.
Use the baseline source when current React behavior differs from `102568b`.

Backend managers, pools, hydration and raw drivers stay private. Native service
entry points use the same stored policy, connection generation and ownership
checks. Native confirmation can only acknowledge a specific service challenge;
it cannot weaken read-only or Strict policy. Stale challenges cannot authorize a
different query or mutation. Bound parameters remain driver-bound.

## Sequence

1. **Expose typed data services.** Extract only necessary Table Browse and Result
   Mutation command logic to host-neutral services; preserve existing Tauri wire
   contracts. Include transaction mode/isolation/commit/rollback/recheck and query
   parameter description. Add native fixture admission before sockets or hydration.
   Close and join every dedicated query, browse and observer task.
2. **Complete query control behavior.** Show actual transaction status, selected
   mode/isolation and server-reported errors. Disable incompatible controls during
   execution. Preserve selection/current-statement/script behavior. Handle failed
   transactions, refused commits and uncertain outcomes without claiming success.
   Session loss clears runtime state but retains SQL; reconnect is explicit.
3. **Activate Table Browse.** Use bounded server paging/sort/filter/count; preserve
   row and column virtualization, column sizing/order/visibility and table grid
   preferences. Do not fetch the full table for a client-side sort. Preserve NULL,
   empty strings, Unicode, exact numeric and binary display/copy semantics.
4. **Inspect and stage changes.** Port specialized value editors, insert/update/
   delete, virtual-key selection and foreign-key drill-down. Carry original row
   identity and concurrency checks. Staging never writes. Rows without a usable
   identity explain why edits are unavailable. Review is the selected design.
5. **Apply and recover.** Preview the exact change set, use existing transactional
   mutation semantics and policy confirmation, report per-operation failure,
   invalidate affected data only after accepted results, and preserve unresolved
   changes on error. A lost reply is not a successful write or an automatic retry.
6. **Validate the real window.** Drive query → transaction → table filter/page →
   inspect → stage → review → apply → conflict/refusal → reconnect → close/reopen.
   Run concurrent-tab and close/cancel races against owned fixtures. Verify AX
   labels, keyboard focus and real IME for new controls independently. VoiceOver is deferred.

## Focused acceptance

- Two query tabs on one connection have independent transactions. Switching tabs
  does not change ownership, leak a transaction or stop consumption/ACKs.
- Manual BEGIN/error/rollback, auto-commit writes, isolation changes and ambiguous
  connection failure preserve truthful transaction state.
- Sorting/filtering/page changes reject stale replies. Count and slow page reads
  can be cancelled without corrupting the visible page or starving another tab.
- Composite keys, nullable values, changed/deleted rows and explicit virtual keys
  cannot update a different record. Review does not leak stored credentials.
- Read-only and Strict policy refuse before dispatch; confirmed Protected writes
  remain audited. SQL/connection edits invalidate prior confirmations.
- Result retention, queued events, pending edits and worker counts remain bounded.
  Apply cannot silently shed a user's pending changes to fit a budget.
- Quit and tab close join browse/query/mutation work within the shared cleanup
  deadline, return PostgreSQL activity to baseline and leave recoverable drafts.

Use dedicated owned fixture schemas/rows for mutation tests. Never contact an
arbitrary saved endpoint to exercise a form. Fixture ownership and unique IDs
must be checked before seeding, destructive probes or cleanup. New TLS fixture
ownership belongs to Plan 027's direct-connection matrix.

Run required frontend and Rust checks, both backend feature configurations,
native debug/release Clippy/tests/build and native dependency proof. Add meaningful
model/service tests for the failure cases above; real-window and human results
are separate evidence. Record commands, source hashes, fixture/profile identity,
cleanup counts, memory/queue peaks and remaining gaps under `plans/evidence/028/`.

## Completion and subsequent PostgreSQL work

READY FOR REVIEW requires every scoped gate, including real-window evidence.
DONE requires reviewed committed evidence and a completion SHA. Full PostgreSQL
parity additionally requires the catalog/DDL/designer, SQL tooling/history,
schema map, jobs/transfers/comparison, SSH/managed connections, administration,
and profile/package compatibility work in the
[parity scope](./mocks/native-postgres-parity/index.html#scope). Stage 06 and the
PostgreSQL acceptance slice of stage 07 need their own executable plans before
implementation. Other engines and daily-driver cutover remain separate.

Stop for identity/safety bypass, data loss disguised as success, unbounded work,
unjoined cleanup, failed accessibility or missing fixture ownership. Keep
requested work pending rather than marking this milestone as complete parity.

## Latest query-result increment

[Executed-source and query-journal integration](./evidence/028/query-mutation-source-checks/README.md)
adds a restricted native UPDATE path with exact-save Apply, cancellation and
recovery. This increment introduced workspace version 3 with compatible version 1/2 reads. The later Create Schema increment writes version 4 and preserves versions 1–3.
This does not close full query-result parity: original execution origin/rendering
metadata and the recorded unsupported-source/type/encoding cases remain. Final
source checks and release-window acceptance are tracked separately in that evidence.

Current grid sizing increment: [content-derived widths and auto-fit](./evidence/028/auto-fit-progress.md). Query/table defaults sample retained rows; explicit auto-fit preserves per-result query geometry and the table preference acknowledgement barrier. Required/native checks and separate packaging pass against 440 source hashes. Actual-window acceptance remains pending.

Latest actual-window evidence: [auto-fit package and disconnected reopen](./evidence/028/auto-fit-window-20261003/README.md). Scoped formatting, table geometry and library/administration checks ran; full acceptance remains open. The restored-library budget failure is tracked in the [activation fix](./evidence/029/library-activation-source-checks/README.md). VoiceOver remains deferred.
