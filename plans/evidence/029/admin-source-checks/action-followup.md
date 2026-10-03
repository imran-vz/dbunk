# Administration action follow-up constraints

Read-only reconnaissance, 2026-10-03. This is not implemented behavior or a
relaxation of Plan 029's target/policy requirements.

The old `postgres/admin.rs` signals raw PIDs through SQLx. The new native reader
cannot simply call those functions. `owned_read` may replace a completed read
with Cancelled during cleanup, and `object_read` may replace it after retirement.
Side effects need dispatch tracking and acknowledged-result precedence, so a
sent signal cannot be reported as safely cancelled before dispatch.

A future target should be opaque and issued from a retained observation, bound
to its document owner, database, PID, backend start and expiry. Cancel also needs
observed query start and active state. Do not manufacture authority from mutable
public display DTOs. Policy/confirmation must be consumed and rechecked before
submission, including connection/document retirement and self-target refusal.

One fresh parameterized identity guard plus a PostgreSQL signal call reduces
stale-target risk but does not atomically pin a backend or its current query.
PostgreSQL's signal implementation retains a PID-reuse race; a query can also
finish and another begin. Never claim exact-query atomicity or that a true signal
return proves termination completed. Preserve SignalSent, SignalNotSent,
StaleTarget, CancelledBeforeDispatch and OutcomeUnknown distinctions; do not
retry uncertain outcomes automatically.

References: [PostgreSQL signaling](https://www.postgresql.org/docs/current/functions-admin.html#FUNCTIONS-ADMIN-SIGNAL)
and [signal implementation](https://raw.githubusercontent.com/postgres/postgres/master/src/backend/storage/ipc/signalfuncs.c).

Maintenance is a distinct typed review. Relation OID/kind and qualified identity
need revalidation. A transaction can retain locks while rechecking and performing
some ANALYZE/REINDEX operations. VACUUM and partitioned REINDEX cannot share that
transaction barrier; a separate prior check does not solve name replacement.
Notices need finite retention, since warning-only skips do not prove every object
was maintained. Read-only and Protected/Strict policy must match the baseline.
No native maintenance action is activated by this document.
