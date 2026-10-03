# Native DDL ownership reconnaissance

2026-10-03. Read-only baseline/source review; no fixture or UI access. This is an implementation handoff, not acceptance evidence. Root owns all writes. Baseline: Tauri `102568b`; relevant design: ADR-0026.

## First coherent activation

Create Schema, with an optional comment on that newly created schema, through Objects and the approved bottom review. This exercises typed preview, policy, commit ownership, audit and durable recovery in one transaction. It does not establish existing-object lifecycle or designer parity.

Baseline references: `components/object-ddl/create-object-dialogs.tsx::buildCreateSchemaOps`, `ddl-review-dialog.tsx`, `drop-impact-dialog.tsx`, `lib/object-ddl.ts::applyObjectDdlWithSafetyConfirmation` and `objectDdlRefreshScope`, `commands/pg_objects.rs` (now extracted into `postgres/object_service.rs`). The old review freezes operations/connection, fences epochs, refuses concurrent apply and distinguishes a successful apply from a failed refresh.

Reuse `generate_object_ddl`, CreateSchemaOp and SetCommentOp, but expose a narrow native intent instead of enabling the full PgObjectOp union. The opaque backend review owns the exact intent/document/preview; Apply regenerates SQL. Bound UTF-8 identifiers at 63 bytes, reject NUL and blank names, cap comments and escaped preview before allocation. Do not introduce IF NOT EXISTS or silent name truncation.

## Write ownership gap

Neither existing reader nor legacy DDL execution can safely activate native writes unchanged:

- `native_catalog::owned_read` gives cancellation priority and performs a final cancellation check. It can replace a successful completed write with Cancelled.
- `Backend::object_read` performs a post-operation document-closed check. A terminal committed write must survive retirement.
- `object_service::apply` uses a detached SQLx pool connection without native tracked driver ownership, cancellation and joined cleanup.

A dedicated native write runner must reuse DataDocument admission and dedicated::connect_tracked, with explicit commit admission patterned on result_mutation. Before commit admission, cancellation/retirement prevents COMMIT and rolls back or tears down. After admission, cancellation must not claim rollback. Loss during COMMIT is OutcomeUnknown and must never automatically retry. Use an absolute operation deadline plus joined cleanup; distinguish bounded asynchronous work from synchronous TLS-file work. Preserve configured statement timeout and disclose the legacy DDL lock timeout of 10 seconds or a deliberate native bound.

Backend policy generates confirmation: authorize with confirmed=false, mint a single-use opaque confirmation only for the policy-required case, then recheck stored policy/authority on confirmation. Read-only never becomes writable through confirmation. Use WriteIntent::Ddl. Audit only confirmed known success, once. Existing best-effort class-label audit is not an execution journal and cannot resolve unknown outcomes.

## Durable review and recovery

Before dispatch, persist the exact bounded typed intent and OutcomeUnknown recovery revision, then wait for that revision's ACK. Cancellation before dispatch invalidates late acknowledgements. Existing table apply_flow is the reference. Unknown outcomes retain intent until explicit reconciliation/discard; reopen and refresh never replay or infer this attempt's outcome. Workspace currently has row/query mutation recovery, so DDL requires its own bounded journal before native activation.

## Later lifecycle work

PgObjectRef names/kinds/routine arguments are not immutable object-instance identities. A name/OID precheck alone leaves a race before name-based DDL. Rename/comment/drop require a stale-target contract and supported lock-plus-recheck strategy. Dependency previews remain advisory/capped and cannot authorize a drop. Leave CASCADE, view replacement, sequence state changes, enum additions and standalone/concurrent-index groups inactive in this first slice.

## Focused checks

Pure preview without credential/socket access; exact quoting and bounds; foreign/retired owner and changed policy; single-use confirmation; cancellation on both sides of commit admission; dropped waiter and joined cleanup; ambiguous COMMIT; exact durable ACK refusal/late ACK; no recovery replay; success audit once; successful apply with failed refresh. Later explicitly owned live verification should exercise atomic create/comment rollback and backend-count cleanup. No such probe ran during this reconnaissance.
