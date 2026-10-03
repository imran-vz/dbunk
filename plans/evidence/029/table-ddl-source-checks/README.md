# Existing-table DDL service, 2026-10-03

Status at this service-only increment: backend implemented. Subsequent
[native activation and verification](../table-ddl-activation-source-checks/README.md)
is implemented, with actual-window acceptance still pending.
This increment supports one table/column comment or rename on an ordinary,
non-partitioned, non-inherited permanent or unlogged table. It does not complete
the designer or other object lifecycle operations.

`Backend::observe_table_ddl` produces an opaque document-bound observation.
Review consumes that observation; apply regenerates the typed preview, checks
stored policy before credentials, and uses one connection write permit and one
COMMIT admission. The caller must persist the exact attempt, target, intent and
SQL and await that exact acknowledgement before calling apply. The later native
caller is documented in the activation evidence above. Deserialized descriptions are read-only recovery information,
not execution authority.

The dedicated runner verifies the expected relation lock and uses fresh READ
COMMITTED namespace OID/name/xmin/ctid checks before and after the operation.
Target replacement or namespace changes refuse or roll back. Receipts distinguish
NotDispatched, acknowledged RolledBack, Applied and OutcomeUnknown. Cancellation
and driver cleanup are joined. Rollback covers transactional changes; sequence
or external effects of server hooks are not promised to roll back. There is no
automatic retry or event-trigger sandbox claim.

Bounds are 4 KiB comment, 8 KiB description, 16 KiB preview, 32 KiB receipt and a
30-second operation deadline. Exact quoted and whitespace identifiers remain
valid. PostgreSQL treats `COMMENT ... IS ''` as comment removal; the preview keeps
the exact submitted empty literal and the postcondition expects NULL.

Nine focused tests and one explicit owned stage03 live probe pass. The live
probe covers comments, rename, empty/NULL comments, committed schema swap/ABA,
cancel-before-COMMIT rollback and replacement-OID refusal. All owned probe
objects were removed using exact identity guards and RESTRICT; PostgreSQL
activity returned from zero to zero. The final live log records the identities.
Ignored tests are not passes. A subsequent
[real COMMIT with suppressed runner acknowledgement](../table-ddl-activation-source-checks/commit-ack-loss/README.md)
passes. Controlled server-hook and wire-level lost-COMMIT-reply injection remain
open.

The isolated-profile all-target Clippy and focused formatting pass. The first
broader `just lint` found a missing `isolated-profile` cfg on the PostgreSQL module
declaration. That guard is corrected; integrated verification is recorded under
[whole-table export checks](../whole-table-export-source-checks/README.md).
Native exact-save recovery and UI source now have separate passing checks;
actual-window recovery, keyboard/AX and real Tool-tab IME remain pending.
VoiceOver remains deferred.
