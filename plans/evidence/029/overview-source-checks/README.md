# Native overview statistics, 2026-10-03

Status: source integration and verification in progress. This slice implements
the baseline database overview metrics and relation statistics through the
Administration Tool tab. It does not close Plan 029 or full PostgreSQL parity.
Recent-query/favorite/health presentation and the saved-connection Settings
mirror remain separate work.

The backend preserves the eight metrics in baseline `102568b`:
database/table/index bytes, table/schema/index counts, estimated rows and
current-database connections. Its baseline database scope counts namespaces
containing catalog relations, not empty schemas, and includes partition children
in table aggregates. Relation lists exclude child partitions for database/schema
scope; an explicit relation scope can inspect a child. Sizes remain physical
per-relation measurements, not recursive partition totals.

Unknown estimates and permission-denied optional metrics remain distinct from
known zero. Views use Not applicable for row estimates and physical size. Only
SQLSTATE 42501 becomes Restricted; other failures retain the previous capture.
Required metadata errors do not become empty successful pages.

The facade uses one admitted document read, a joined owned socket, a read-only
transaction and a 30-second deadline. Database-first refresh includes the eight
metrics and the first relation page in that read. Pages contain at most 256 rows
and use an opaque connection/database/scope-bound keyset cursor. A continuation
opens a fresh capture, not a retained transaction. Scope totals cover the whole
scope. Concurrent changes may affect later pages; statistics and filesystem
sizes are not an atomic server-wide measurement.

Native integration shares the Administration cancellation/reply fences and
16 MiB delivery budget. Incoming snapshots waiting in inactive tabs are charged
before render. The typed capture, bounded fields and exact selected-details
editor use the shared 128 MiB retained allowance, including old/new overlap.
These are payload bounds, not RSS claims. Scope edits disable continuation of a
different captured scope. Disconnection/cancellation preserves stale inspection
without replaying a read.

Seven focused backend tests and two owned live probes pass. The final live probes
cover exact 256+1 pagination, unchanged totals/order/OIDs, expected-identity
refusal, a different document attempting to consume a cursor, closed-document
refusal and joined cleanup. Both temporary schema/view OID sets are independently
absent with zero fixture backends. Restricted-role denial is classification-tested,
not live-tested. See [review corrections](./review.md), [final live log](./backend-live-final.txt)
and [independent teardown](./backend-teardown.json).

Required pnpm format/lint/typecheck and just fmt/lint/serialized test pass. Default
backend configurations pass 677/71 ignored and 694/85 ignored. Isolated all-target
Clippy passes; isolated tests pass 1,038 with 94 ignored and two doctests. Native
debug/release Clippy and tests pass (322 passed, 13 ignored), with debug build and
fixture-harness Clippy passing. A final human-readable refusal-message refinement
was separately rechecked before packaging. No ignored fixture test is counted as
a pass. The first package matches 594 frozen source hashes. Its
[actual-window checks](./window/README.md) pass scoped paging, estimates,
keyboard navigation, cancellation, stale inspection and repeated same-name OID
refusal with explicit capture reset. All 13 persisted documents are unchanged;
the 260 recorded fixture relations and schema are absent, with zero backends on
both owned fixtures. A clipped row-label defect was fixed and its
[separate package/window recheck](./row-window/README.md) passed. Complete keyboard
acceptance and real IME in this Tool tab remain pending.
VoiceOver remains deferred and non-blocking.
