# Plan 021 capture gate

**Gate passed for the conservative scalar subset, 2026-09-06.** This is a
disposable feasibility harness, not an integrated comparison backend.
All reproduction assertions passed on PostgreSQL **16.15
(Debian 16.15-1.pgdg13+2), aarch64**. A second minor has not been tested.

```sh
python3 infrastructure/test-db/schema-compare/gate.py
```

Requires Docker, Python 3.9+ and rustc (edition 2021). The first build downloads compiler/server
headers into a dedicated Docker image, `dbunk-schema-compare-gate:local`.
The harness creates and prints a random `dbunk-schema-compare-gate-*` container
name, uses no network or published port, keeps data in tmpfs, and removes its
container during cleanup. It accepts no connection string or existing container.
It does not start the shared compose fixture. The build image/cache remains for
subsequent runs. The fixture's C support function must never be installed on an
application connection.

Container creation and startup are separate. Cleanup removes the container only
after this process successfully created it, so a create/start failure cannot
delete a pre-existing container with the printed name. Cleanup failures are
reported separately and do not replace an earlier reproduction failure.

Exit zero means the documented observations and conservative-subset assertions
passed. The JSON labels this `captureGate: conservativeSubset`, not general
expression coverage. The original failed dependency-only strategy remains below. SQL errors
include SQLSTATE; expected failures are asserted and rolled back. Statements
have a five-second ceiling and lock waits a 500 ms ceiling. Client command waits
have a 15-second deadline. The special support-code timeout uses 100 ms. These
are experiment settings, not changes to the plan's proposed production limits.

## What was tested

Two persistent `psql` sessions provide deterministic ordering: the reader
acknowledges snapshot establishment, the writer acknowledges the DDL commit,
then the reader captures facts. No sleeps schedule DDL races. stdout and stderr
are merged **inside** the container before Docker transports them, so a client
echo cannot overtake a server error on a separate stream. Startup alone polls
readiness. Setup/DDL runs only in the fixture; the capture session uses catalog
SELECTs, session settings, explicit locks and read-only transactions. It never
selects table rows or executes the captured expressions as SQL.

| Experiment | Observed result |
| --- | --- |
| Integer default, literal text, ordinary CHECK, expression/partial/INCLUDE index | Stable baseline, including quotes inside literals |
| Snapshot, column rename, default/CHECK/index deparse | Defaults stay at their snapshot values; catalog column is `quantity`, while CHECK and index expressions render `amount`; successful mixed reads |
| Snapshot, table rename, default/CHECK/index deparse | Defaults and CHECK stay at their snapshot values; old catalog index identity remains while generated index text names `source.renamed_orders` |
| Snapshot, rename, then ACCESS SHARE | Same mixed column/CHECK result; a late lock cannot repair it |
| ACCESS SHARE on both endpoints, then snapshot | Both table/column renames time out with `55P03`; one reader session captures source and target CHECK facts from the same repeatable-read transaction |
| Locked tables, then function/sequence/enum-label rename | Defaults contain current names/label within the old snapshot |
| Locked table, index rename | Old catalog index name, new name from `pg_get_indexdef` |
| Concurrent index drop under old snapshot | Drop invalidates index then times out; old snapshot says valid while the current catalog says invalid |
| Prepared concurrent index replacement during capture | A replacement is built with `CREATE INDEX CONCURRENTLY` before capture. While the locked repeatable-read snapshot is open, one writer transaction renames the old index aside and the replacement into its name. The reader captures the old OID and `(quantity + 2)` expression while a current writer query captures the replacement OID and `(quantity + 3)` expression |
| SHARE UPDATE EXCLUSIVE on parent table | Allowed in a primary's read-only transaction, but index rename still succeeds |
| SELECT-only role | ACCESS SHARE succeeds; stronger lock fails with `42501` |
| DROP/recreate after snapshot | Old CHECK row is present, `pg_get_expr` returns NULL successfully; this is unavailable rendering, not absence |
| Custom constant type output | Emits notice from the C output function during deparsing; controlled `22000` error and `57014` timeout propagate |
| Oversized output | A materialized server projection renders once, measures UTF-8 bytes, and sends only a limit flag and NULL, not the oversized value |

The decisive counterexample is `source.hidden_dependency`:

```sql
value text DEFAULT ('{external.serial}'::regclass[])::text
```

After an ACCESS SHARE lock and snapshot establishment, its only `pg_depend`
entry is the automatic dependency on its owning column (`pg_class`, `a`).
Renaming the sequence on the other session still changes deparsing from
`('{external.serial}'::regclass[])::text` to
`('{external.renamed_serial}'::regclass[])::text` inside that same transaction.
Both the column and the expression use built-in types. Excluding explicit
external dependencies and user-defined column types therefore cannot certify
rendered-field consistency. An array constant is not recursively inspected for
its OID-alias elements by PostgreSQL's expression dependency recorder.

## Tested ordering and remaining proof

The candidate algorithm tested is:

1. Begin `REPEATABLE READ READ ONLY`; set `search_path = pg_catalog`, UTC,
   ISO/YMD dates, postgres intervals, `extra_float_digits = 3` and
   `standard_conforming_strings = on` without a SELECT.
2. Lock the known candidate tables in both same-database schemas with
   `LOCK TABLE ONLY ... IN ACCESS SHARE MODE`, before the first SELECT.
3. Establish the snapshot and read structured facts directly from catalog
   joins. Render expressions separately using `pg_get_expr(..., false)` and
   measure each rendered field on the server before returning it.
4. Roll back before releasing connections. Required NULL/error rendering must
   prevent a complete result, unless that field was explicitly declared outside
   the comparable capability in advance.

This fixes the demonstrated table-descriptor race, **not** the complete capture
contract. A real capture still needs bounded discovery outside the transaction,
pre-snapshot locking, then snapshot inventory validation that each eligible OID
is among the actually locked objects. Added, replaced, or excluded counterparts
cannot be silently omitted. The additional `discovery.py` fixture now tests schema existence, a bounded
complete inventory including inherited/view counterparts, pre-snapshot locking,
actual held-OID validation, refusal of newly added unlocked candidates and one
fresh retry. Its small combined-schema cap is a proof setting; production caps
remain per endpoint and are implemented in Step 3. No checksum or later old-snapshot reread is
being claimed as a substitute.

`pg_get_indexdef` is unsuitable as the definition source even with stronger
parent locks. Index identities and attributes must come from direct snapshot
catalog joins, with expression/predicate rendering assessed separately. Stronger
locks also add write privileges, interfere with maintenance and are restricted
on standby servers. They do not solve the hidden dependency counterexample.

The index replacement experiment uses this exact ordering: build a second index
with `CREATE INDEX CONCURRENTLY`; begin and lock the parent table in the reader;
establish its repeatable-read snapshot; commit a writer transaction that renames
the old index aside and the prepared index into the canonical name; query the
canonical index from both the old reader snapshot and the writer's current view;
then roll back the reader and concurrently drop the retired index. The differing
OIDs and expressions prove that replacement overlaps capture. This is not a
`REINDEX CONCURRENTLY` claim, and the concurrent build itself precedes capture.

## Field capability matrix

This matrix records the evidence and conservative fallback, not a delivered API.
"Direct snapshot candidate" means a field can be sourced without name-resolving
deparsers; the complete bounded capture query and decoding are still unbuilt.

| Plan fields | Capture capability after this gate |
| --- | --- |
| Schema existence, complete relation name/kind inventory, eligibility and child identities | Direct snapshot candidate; schema-wide discovery/lock validation still required |
| Visible column order/name, qualified type reference, numeric typmod, array dimensions, nullability | Direct snapshot candidate using `pg_attribute`, `pg_type`, `pg_namespace`; avoid `format_type` as portable identity |
| Identity mode and generated kind | Direct snapshot candidate; sequence settings/values excluded |
| Column default and generated expression | Comparable only within the positive scalar subset and prerequisites below; otherwise `notComparable` |
| Column collation reference and comments | Direct snapshot candidate; collation/type definitions excluded |
| Constraint name/kind, ordered keys, FK referenced identifiers, actions/delete subset/match, deferral/validation/NO INHERIT | Direct snapshot candidate; resolve references via joins, not regclass textual output |
| FK equality and exclusion operator qualified signatures | Direct snapshot candidate using operator and type joins, never portable OIDs |
| CHECK expression | Comparable only within the positive scalar subset and prerequisites below; otherwise `notComparable` |
| Constraint-owned index facts and standalone index name, method, uniqueness/NULLS NOT DISTINCT, immediacy, keys/INCLUDE, sort/null options, opclasses/options, collation, relation options, valid/ready/live | Direct snapshot candidate; fold full facts into owner. Built-in access-method interpretation still needs implementation; unknown option semantics `notComparable` |
| Index expressions and predicate | Positive scalar subset only; do not use whole `pg_get_indexdef` text |
| Table persistence and comments | Direct snapshot candidate |
| Index clustering, replica identity, tablespaces; other table storage/security/ownership/replication | Explicitly excluded |
| Partitioned, inherited, foreign and extension-owned relations; views, materialized views and sequences | Excluded definitions; retain complete bounded counterpart inventory before claiming directional absence |
| Routines, types/domains, policies, grants, triggers, rules, extensions, database objects | Excluded definitions; bounded inventory/counts still unbuilt |

Marking every rendered expression `notComparable` is conservative, but does not
establish the useful default/CHECK/expression-index capability required by Step
1. A few passing integer examples cannot certify arbitrary expressions with
the same dependency shape. That was the reason for the original stop. The positive scalar recognizer now
supplies a conservative alternative without parsing arbitrary `pg_node_tree`
text or adding a dependency lock walker. Protocol/normalization work has begun;
native capture and the manager are still unintegrated.

## Bounds and evidence limits

The output probe returns 262,145 characters before SQL quoting/casting. The
guard materializes that one rendered value, returns `octet_length(value) >
262144`, and conditionally returns the value only within the limit. The harness
asserts one output-function notice and under 1 KiB for the entire observation.
This proves refusal before transporting the rendered value for this query; it
does not establish global job/driver/IPC allocation bounds or server heap bounds.
The full 256 KiB boundary, batch budgets, cap/timeout lifecycle, UTF-8 chunks,
serializer ownership and RSS measurements remain later implementation work.

The support-code loop checks PostgreSQL interrupts and times out in about 0.1 s.
This proves cooperative timeout propagation, not that arbitrary extension code
can be stopped within that time. The comparator's read-only promise must not
claim zero support-code execution or isolation from external effects.

Source checks used alongside the live reproduction:

- [PG16 LOCK ordering and permissions](https://www.postgresql.org/docs/16/sql-lock.html)
- [PG16 renderer, including constant output and current relation names](https://github.com/postgres/postgres/blob/REL_16_STABLE/src/backend/utils/adt/ruleutils.c)
- [PG16 dependency recorder, Const handling](https://github.com/postgres/postgres/blob/REL_16_STABLE/src/backend/catalog/dependency.c)
- [PG16 utility read-only classification](https://github.com/postgres/postgres/blob/REL_16_STABLE/src/backend/tcop/utility.c)

The moving source branch is explanatory; the exact live server version above
is the measured evidence. No other endpoint version, production database, TLS,
SSH, IPC path, product component or comparison capability was activated.


## Conservative subset follow-up (2026-09-06)

The harness compiles `expression_probe.rs` against the actual native
`schema_compare/expression.rs` recognizer in a temporary directory. It accepts
plain integer defaults, scalar CHECK/index expressions and literal text; both
versions of the hidden regclass-array expression are rejected. Only bool, int2,
int4, int8, numeric and text column references qualify. Every recorded dependency
except the owning locked table/columns must be rejected before recognition,
including non-pinned objects in pg_catalog. Fixed session settings and the lock
sequence remain prerequisites, not properties proved by syntax alone. Rendered
fields also require matching `server_version_num` values. Different PG16 minors
can compare structured facts, with expressions marked `renderingVersionDifference`
until rendering compatibility for that pair has been verified.

The current field matrix changes only the rendered-expression rows: defaults,
generated expressions, CHECKs, index expressions and predicates can now be
compared if the recognizer and its capture prerequisites pass; otherwise they
remain `notComparable`. A multi-expression index list is currently outside the
grammar and conservatively incomparable. Raw text is never normalized.

`discovery.py` demonstrates both schema-existence checks, excluded counterpart
inventory, rejection at a row cap, actual held-lock OIDs, a table created after
discovery but before the snapshot, and one fresh retry. Locks use quoted
identifiers and `ONLY`; no row query executes. PostgreSQL can expand locks on a
view if DDL replaces a discovered table before LOCK resolves it. Step 3 must
retain statement/lock deadlines and revalidate eligibility; this fixture does
not claim a server-heap or internal lock-footprint bound. Native transport and
retention caps are separate from PostgreSQL's internal metadata work.

See [ADR-0030](../../../docs/adr/0030-postgres-schema-comparison-foundation.md)
for the accepted foundation, allocation ownership and IPC acknowledgement
contract. The Step 2 native fixtures test UTF-8 chunks, byte/row/result caps,
worst-case JSON escaping, the actual Tauri JavaScript serializer dependency,
concurrent serializer admission, TTL and immutable value identity. Native
command wiring and runtime RSS remain later gates.


## Native Step 3 fixture

```sh
python3 infrastructure/test-db/schema-compare/native.py
```

This runner creates and prints a random `dbunk-schema-compare-native-*` PG16
container, publishes only a random loopback port and keeps database data in
tmpfs. It accepts no DSN or existing container and removes only the container it
successfully created. Native build output uses `/tmp/dbunk-plan021-target`.
Readiness checks TCP so the image's temporary initialization server cannot be
mistaken for the final server. It does not use the shared compose database.

The Rust opt-in tests exercise the actual resolved dedicated connection and
catalog reader on PostgreSQL **16.15 (Debian 16.15-1.pgdg13+2), aarch64**:

- Both schema endpoints share a read-only repeatable-read transaction; portable
  facts match despite different local OIDs and selected schema names.
- Dropped slots, visible column order, array dimensions, numeric typmods,
  identity/generated fields, scalar defaults/CHECKs, FK delete subsets and all
  equality-operator groups, INCLUDE, NULLS NOT DISTINCT and constraint-owned
  index facts survive decoding. Mixed expression/column keys retain positions.
  Exclusion constraints retain qualified operators, operand types and their
  owned GiST index keys, opclasses and sort facts.
- Unsupported expression lists remain incomparable and differing minor-version
  metadata cannot establish rendered-expression equality.
- Different connection defaults for `quote_all_identifiers` produce equal
  CHECK/index expressions and defaults after capture normalization; required
  identifier quotes and literal contents are preserved.
- A 256 KiB control-character comment is accepted; one extra byte is refused
  before transport. Full inventory/table counts pass at 2,000/1,000 and refuse
  the next entry. Pure capture tests exercise field-count/retained-byte refusal
  and reservation release.
- Missing schemas and missing SELECT privileges fail required capture.
- An acknowledged lock wait lets the writer add a new candidate before the
  reader's snapshot. Held-lock validation forces one fresh attempt, which
  includes the new table. Cancellation during an acknowledged lock wait closes
  the reader and releases its retained allocations.

The gate harness above remains the detailed deparser interleaving proof.
Cross-server/minor compatibility, full lifecycle fences, native IPC and RSS
measurements remain Steps 5–6; the native test is not evidence for those claims.


## Native Step 4 coverage

The same `native.py` runner also passes its two successful schema captures through
`schema_compare::diff::compare`. Identical supported facts produce no known
changes, while excluded relations and expressions remain incomparable. Changing
the target's integer default and column comment then produces exactly two known
changed fields on that table without hiding the incomparable fields. The result
retains the verified shared-transaction marker and releases its allocation after
reads complete. This passed on the PG16.15 version recorded above.
