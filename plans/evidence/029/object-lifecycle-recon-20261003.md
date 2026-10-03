# Existing-object lifecycle contract reconnaissance, 2026-10-03

Status: source investigation only. No backend activation, fixture operations, builds or window verification. This supplements [DDL ownership reconnaissance](ddl-ownership-recon.md) and [Create Schema evidence](schema-create-source-checks/README.md); it does not close Plan 029 lifecycle parity.

## Recommendation

Implement a narrow, single-transaction existing-table surface next: table/column comments and table/column rename on ordinary, non-partitioned, non-inherited PostgreSQL tables. Consume an opaque document-bound observation and exact typed review. Preserve stored policy before credential access, exact-save recovery, one connection write permit, dedicated joined socket and once-only COMMIT admission. Add other relation kinds, DROP RESTRICT and multi-group operations after their separate identity/dependency cases are covered.

Use expected database/relation OIDs and column attnums, acquire and verify the expected relation lock, and compare namespace `(oid, nspname, xmin, ctid)` before and after each typed statement using separate READ COMMITTED commands. Refuse and roll back on any unexpected identity/version change. Verify the expected post-operation relation/column identity, accounting explicitly for the reviewed rename. Do not silently refresh a failed guard or adopt a replacement object.

This is a practical **detect-and-roll-back contract**, not namespace serialization and not a guarantee that an unintended relation was never transiently reached. It is consistent with existing schema creation and arbitrary SQL execution, provided receipts distinguish transactional rollback from absence of all server-side effects. Implementation and deterministic race evidence are still required before activation.

## Pinned PostgreSQL evidence

All PostgreSQL paths below are relative to the local source root `tools/native/.state/tls-runtime/postgresql-17.11/`. These findings are specific to inspected PostgreSQL 17.11 source, not an unverified claim about every supported major version.

| Source and lines | Finding |
| --- | --- |
| `src/backend/commands/schemacmds.c:249–304` | `RenameSchema` takes `RowExclusiveLock` on the **pg_namespace catalog relation** at line 258, looks up the name at 260, and updates its tuple at 293–294. It does not take a namespace object lock that an ordinary relation lock would conflict with. |
| `src/backend/catalog/namespace.c:467–481,525–534,565–592` | `RangeVarGetRelidExtended` resolves the current schema/name, locks the resolved relation and retries on invalidation. This is not a check against the app's reviewed OID. The comment explicitly notes limitations when the requested relation lock is already held. |
| `src/backend/utils/time/snapmgr.c:253–282,352–405` | READ COMMITTED obtains fresh transaction snapshots; transaction-snapshot isolation reuses the earlier one. Catalog lookups use invalidation-managed catalog snapshots. Calling this old `SnapshotNow` behavior is inaccurate. Repeated SQL checks under REPEATABLE READ cannot be assumed to observe the same metadata freshness as utility name resolution. |
| `src/backend/access/common/heaptuple.c:734–753` | `ctid` is the tuple location and `xmin` is the raw inserting transaction ID. `xmin` alone is not an update counter: repeated updates in one transaction can share it. `cmin`/`cmax` can expose a combo command ID, so they are not a simple substitute. |
| `src/backend/tcop/utility.c:1106–1113,1943–1946` | DDL-command-start hooks run before utility dispatch; SQL-drop and DDL-command-end hooks run afterward. Pre/post client guards therefore surround server-configured code as well as the typed statement. |
| `src/backend/commands/lockcmds.c:280–298` | Strong `LOCK TABLE` modes require catalog-relation privileges such as MAINTAIN/UPDATE/DELETE/TRUNCATE. Owning a user schema does not grant these on pg_namespace. |
| `src/backend/utils/misc/guc_tables.c:2021–2027` | PG17's `event_triggers` setting is privileged (`PGC_SUSET`). Silently disabling it would change configured server behavior and is not an ordinary-role solution. |

The full first path, for example, is `tools/native/.state/tls-runtime/postgresql-17.11/src/backend/commands/schemacmds.c:249`. The local tree was inspected read-only; no server configuration was changed.

## What the guard can and cannot establish

An expected relation lock blocks conflicting changes to that relation, but does not freeze its schema name. Another session can commit a schema-name swap while the lock is held. Qualified SQL may then resolve to another relation. Rechecking names alone misses rename-away/rename-back; a namespace tuple fingerprint detects ordinary committed ABA updates even when the original name returns. Include `ctid` with `xmin` for repeated updates by one transaction, and refuse even harmless version changes rather than claiming they are authorized.

Acquire locks before effects and verify that the expected OIDs actually have the required granted locks. SQL `LOCK TABLE` itself resolves names, so a catalog query returning an expected OID is insufficient evidence that that OID was locked. Use `ONLY` for the proposed plain-table scope, bounded lock/operation deadlines and an explicit refusal for unsupported inheritance/partition cases. Guard queries must qualify pg_catalog objects; do not rely on an untrusted search_path. Establish the namespace fingerprint against the observed review and retain it throughout this transaction; do not rebaseline after each statement.

With a fresh READ COMMITTED post-statement check, an external committed namespace version used to retarget the statement should become observable and cause rollback. A subsequent rename after the final check cannot change which relation an already executed statement affected, although it can make the success receipt's displayed name immediately stale. This is the rationale to test, not a substitute for race tests or a formal proof across server versions. `xmin`/`ctid` are short-lived refusal fingerprints, not durable object identity or executable recovery authority.

Rollback undoes transactional catalog effects, including unintended transactional effects detected afterward. It does not erase all possible side effects of event triggers, extension hooks, expressions or user functions. Sequence changes, for example, are explicitly non-rollbackable in [PostgreSQL's sequence documentation](https://www.postgresql.org/docs/17/functions-sequence.html). A configured hook can also perform external work. These are normal server semantics; the app must neither promise to sandbox them nor classify every installation containing an event trigger as malicious.

Ordinary external concurrent DDL is in scope for the refusal tests. A malicious server, superuser directly manipulating catalogs, or deliberately adversarial extension/hook that subverts identity reporting is outside this client safety guarantee. Known ordinary event-trigger behavior remains in scope for truthful receipts. In particular, report that transactional DDL was rolled back, not that nothing happened anywhere. Do not automatically retry a guard refusal, cancellation or unknown outcome.

## Consistency with existing execution

`src-tauri/src/postgres/native_schema_ddl.rs:153–173` executes BEGIN and the generated CREATE SCHEMA/optional COMMENT inside one transaction. It does not inspect, disable or sandbox event triggers. Lines 178–194 admit COMMIT once, prefer a known terminal reply and settle cancellation on the same COMMIT future. Lines 133–143 join socket cleanup and classify Applied, NotApplied or OutcomeUnknown. Its source comment correctly limits the pre-COMMIT guarantee to **transactional schema changes**.

`src-tauri/src/backend/schema_ddl.rs:193–209,220–259` requires the exact durable attempt, regenerates the preview, checks stored policy before credentials and acquires the write permit. `apps/native/src/schema_changes.rs:35–65` keeps unknown attempts until a matching receipt and retains the staged intent after NotApplied. None of these paths proves that a failed attempt produced no sequence or external hook effect. The next surface should use clearer terminal descriptions such as `NotDispatched`, `RolledBack { reason }`, `Applied`, and `OutcomeUnknown`, with rollback explicitly scoped to transactional changes.

The ordinary query runner also sends admitted SQL to PostgreSQL (`src-tauri/src/query_session/postgres.rs:499–542,894–905`) without an external-effect sandbox. That does not justify omitting native target guards: choosing an object in a native inspector carries an identity expectation beyond arbitrary SQL text. It does show why an absolute no-external-effect requirement would exceed the established app contract and block useful ordinary-role lifecycle work unnecessarily.

## Implementation boundary and focused evidence

Keep the first runner to one bounded typed operation and one transaction. A possible facade is opaque `ObservedTableTarget → review_table_change(intent) → apply/confirm(review) → exact-attempt receipt`; read-only recovery contains identities, typed intent and exact SQL but cannot be deserialized into authority. Reuse pure `object_ddl` rendering, not the pooled legacy writer. Exclude user SQL expressions, table rewrites, CASCADE, schema renames/moves, standalone statements and multi-commit groups from this first slice.

The connection slot in `src-tauri/src/backend/data_documents.rs:77–105` and one-way `WritePermit::admit_commit` at 345–365 fit one transaction. Cancellation/retirement at 207–225 is sticky before admission and wakes settlement afterward. Full grouped DDL needs a later operation-wide slot with indexed group boundaries, sticky cancellation across COMMIT, durable committed-prefix receipts and explicit partial/unknown outcomes. Dropping/reacquiring permits between groups would allow interleaving; blindly resetting the phase would forget cancellation received during COMMIT.

Before activation, add focused ownership/runner tests and an explicitly owned fixture probe covering:

- Exact relation lock verification, quoted identifiers, replaced OIDs, column attnums and unsupported relation kinds; stale policy or owner refuses before credentials.
- Two owned same-named tables in separate schemas, with externally committed schema swaps injected between guard, statement and postcheck. Require refusal and no committed transactional change to either table.
- Rename-back ABA in one and multiple transactions; fresh READ COMMITTED checks and a REPEATABLE READ negative control. Include a controlled same-session namespace-update hook to exercise `xmin` plus `ctid`, without treating that as general adversarial-server protection.
- A controlled hook using an owned sequence that demonstrates surviving sequence advancement after rollback. The expected result is an honest rollback receipt, never a claim of zero side effects.
- Cancellation before dispatch and before COMMIT, retirement, lost COMMIT reply, known success racing cancellation, dropped UI waiter and joined driver cleanup. Unknown recovery must block replay until explicit reconciliation.

If stronger external-concurrency serialization becomes a requirement, a privileged SHARE lock on pg_namespace would conflict with RenameSchema's catalog RowExclusive lock, at database-wide contention/privilege cost. Do not auto-grant those privileges. A cooperative advisory lock protects only participating writers; a server extension resolving and applying by OID would be a separate deployment requirement. Neither is needed merely to match the transactional-effect contract already used by Create Schema.
