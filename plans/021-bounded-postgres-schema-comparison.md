# Plan 021: Bounded PostgreSQL schema comparison foundation (dark)

- Priority: P1. Effort: L. Gap: PAR-008.
- Planned against: `7745946efcc9f975b88f6468690923f2ec30cae3`, 2026-09-05.
- Depends on: Plans 013–017 (catalog and typed DDL), with Plan 020's completed
  lifecycle work as the current baseline.
- Execution status: see [README.md](./README.md). Implementation started 2026-09-05; through Step 5. The foundation through Step 4 is committed at `502674e`; Step 5 is verified and uncommitted. Step 6 is next. See the execution record below.
- Visual brief: [next-parity-item.html](./next-parity-item.html).

## Outcome

Provide a native, read-only comparison of ordinary PostgreSQL table definitions
between two explicit schema endpoints, on the same or different connections.
Return stable object identities, field-level differences and explicit coverage.
The future UI must be able to distinguish "equal within this scope" from
"not fully compared". Reading catalog names or comparing generated CREATE text
alone is insufficient.

This is the first slice of schema comparison, not full PAR-008 parity. It adds
no product surface, migration generation, apply command or row-data reads.
UI activation follows in a separately planned slice with several published
static mocks and Imran's selection before real component edits. Migration SQL
needs its own dependency, drift and safety design after comparison is reliable.

## Evidence and existing boundaries

- `plans/parity-gap-register.md`, PAR-008: the former Compare prototype was
  removed in `8411dbf`. No current comparison backend or surface exists.
- `src-tauri/src/postgres/objects.rs`: typed `PgObjectRef`, catalog batches,
  per-kind truncation and repeatable-read descriptions. Catalog listings are
  not complete definition snapshots, and independently calling describe once
  per object does not yield one schema-wide snapshot.
- `src-tauri/src/postgres/object_ddl/`: shared typed operation vocabulary and
  quoting. Its generator is not a comparison engine; do not diff its output as
  a substitute for normalized catalog facts.
- `src-tauri/src/commands/pg_objects.rs`: preview/apply boundary. Plan 021 must
  not call apply or turn observed database expressions into executable input.
- `src-tauri/src/postgres/dedicated.rs`, `connect_spec.rs`: established native
  connection, TLS, SSH, cancellation and driver cleanup paths.
- `src-tauri/src/postgres/transfer/manager.rs`, connection commands and
  `src-tauri/src/lib.rs`: admission and teardown precede resource invalidation.
  Reuse proven lifecycle primitives where they fit; do not build a generic
  task framework or couple read-only comparison to import finalization.

Read `docs/agents/domain.md` and ADRs 0001, 0025, 0026, 0027 and 0029 before
implementation. Add a comparison glossary term and ADR when the contract lands;
do not describe this draft as an accepted architecture.

## Scope and meaning of equality

One request supplies `{connectionId, schema}` for source and target. Source is
the reference; target is the compared endpoint. Names are case-sensitive exact
catalog identifiers. Schemas must exist and engines must be PostgreSQL. Never
silently compare a missing or unreadable schema as an empty one.

| Area | First-slice contract |
| --- | --- |
| Relations | Ordinary tables only. Detect and disclose partitioned, inherited, foreign and extension-owned tables as excluded. |
| Columns | Ordered names, qualified type identity and modifiers, catalog-declared array dimensions, nullability, default expression, generated kind/expression, identity mode, collation and comments. Ignore dropped physical slots; preserve visible column order. Identity sequence configuration is excluded. |
| Constraints | Named PK, unique, FK, CHECK and exclusion definitions; ordered keys, referenced identifiers, update/delete actions including delete-column subsets, match mode, deferrability, initial deferral, validation and NO INHERIT. Preserve equality/exclusion operator identities and constraint-owned index facts. |
| Indexes | Named indexes, access method, uniqueness including NULLS NOT DISTINCT, immediacy, ordered keys/expressions, included columns, sort/null order, opclasses/options, collations, predicate, relation options and valid/ready/live state. Fold constraint-owned indexes into their constraints without losing properties. |
| Table attributes | Persistence and comments. Unsupported storage, security, ownership or replication attributes are explicit coverage exclusions. |
| Other objects | Views, routines, sequences, types/domains, policies, grants, triggers, rules, extensions and database objects are excluded, with bounded inventory/counts where feasible. Definitions referenced by table fields do not establish coverage of those objects. |

### Version and field checklist

V1 supports PostgreSQL **16.x → 16.x only**, including different minor versions,
same-connection schemas and independent databases. Return `unsupportedVersion`
for either endpoint outside major 16, before version-dependent catalog queries.
This is a comparison capability boundary, not a new restriction on existing app
connections. The repo's PostgreSQL 16 fixture is the baseline; wider support
requires a separate compatibility pass. Record exact tested server versions.

The table above is the closed V1 field checklist. In particular:

- Unique constraints/indexes preserve `indnullsnotdistinct`; owning constraints
  also retain their index keys, INCLUDE, predicate, opclasses/options, collation,
  immediacy and valid/ready/live facts. Deduplication removes only duplicate
  presentation, never index facts. Index clustering, replica identity and
  tablespace placement are explicit exclusions.
- FKs preserve ordered local/referenced columns, match/update/delete actions,
  `confdelsetcols` (null means all referencing columns), deferral/validation and
  qualified equality operator identities. Exclusion operators retain qualified
  signatures, not cross-database OIDs. CHECK retains expression and NO INHERIT.
- Identity means none / ALWAYS / BY DEFAULT. Sequence start/increment/min/max/
  cache/cycle and current values are excluded, including identity-owned sequences.
  Generated columns preserve kind and expression. Types and collations compare
  references only; their underlying definitions remain outside coverage.
- Array dimensions mean available catalog declaration metadata, not enforced
  array shape or recovered declared size bounds. Do not inspect rows to infer it.
- Built-in access methods use their documented option interpretation. Unknown
  access-method option semantics become `notComparable`, not invented sort flags.

Identity is kind plus exact name within the mapped schema, plus parent identity
for children. OIDs are local lookup keys only; never compare them across
connections or persist them as portable identities. Same named user-defined
types in different databases are not proof that their definitions match.

Map only structured references to the selected source/target schemas into the
same logical namespace. Preserve external schema identities. Do not replace
schema strings inside expression text, strip casts, collapse arbitrary
whitespace, or remove quotes. SQL literals and quoted identifiers must survive
unchanged. Use non-pretty server-deparsed expressions with fixed session settings
for reproducibility, retaining raw text for future review. If an expression or
dependency cannot be compared safely across schema mappings/server versions,
mark that field `notComparable` with a reason instead of asserting equivalence.

Before eligibility filtering, capture a bounded, complete schema-local relation
name/kind inventory, including views, materialized views, sequences, foreign,
partitioned/inherited and extension-owned relations. Mark eligibility separately.
The inventory has its own hard cap; a cap hit aborts comparison. Index/child
identities remain typed separately from candidate table counterparts.

Match counterpart names before comparing eligible definitions. An ordinary
source `orders` with a partitioned table or view named `orders` on the target
is `notComparable` with reason `excludedCounterpart` and both observed identities,
never `sourceOnly`. Reverse direction is symmetric. Directional absence requires
an eligible object and proven absence of its counterpart in the complete inventory.
Excluded pairs remain disclosed; unsupported-only schemas cannot claim full equality.

Results distinguish `sourceOnly`, `targetOnly`, `changed`, `equal`, and
`notComparable`. A changed object contains typed field paths and source/target
facts; do not flatten everything into a human sentence. Renames appear as
source-only plus target-only objects. There is no automatic rename inference.

Coverage and job success are separate. A successfully collected result can have
declared excluded categories; its equality claim is restricted to the supported
table projection. Missing permissions, cap hits, failed required queries or
unreadable required definitions prevent a complete comparison result. Never
infer source-only/target-only objects from partial listings. Equal empty schemas
are valid only after both schema-existence and complete-inventory checks.

## Snapshot and resource contract

Use fixed, bound catalog queries on native connections. Each endpoint is read
inside one repeatable-read, read-only transaction. If both endpoints are on the
same stored connection/database, read both schemas in one transaction. Different
connections have independent capture times; do not claim an atomic snapshot
across servers. Retain endpoint identity, server version, capture timestamps,
normalization version and coverage in the result.

Catalog SQL and server deparsers do not necessarily use identical metadata
visibility. A successful deparse can mix current relation/cache names with old
snapshot facts. Step 1 must prove a capture strategy before comparison is wired;
catching missing-object errors alone does not establish consistency. Failed
required capture is a typed retryable error, never false deletion. Unproven
rendered fields are `notComparable` and cannot establish equality or a change.

The read-only guarantee is deliberately narrow: the comparator never executes
captured default/CHECK/index expressions as SQL, invokes row queries, or issues
DDL/DML. Server metadata rendering can invoke installed type output functions or
access-method support code. A read-only transaction is not a sandbox for that
code or its external effects. This plan does not promise zero user-code execution
or bounded server heap inside extension code. Native transport/retention bounds
and timeouts still apply. Step 1 tests this boundary only in disposable fixtures.

Proposed initial hard limits, centralized and tested at their boundaries:

- Two active/resolving comparisons globally, at most one involving any given
  connection. Reserve both endpoints atomically, including same-connection input.
- 2,000 relation-name/kind inventory entries, 1,000 eligible table entries and
  50,000 child facts per endpoint; 16 MiB retained definition data per endpoint,
  256 KiB raw UTF-8 per definition field, 32 MiB retained result per job.
- Cap each batch before retention. Guard definition bytes server-side before
  they cross the driver boundary; a frontend byte check alone is insufficient.
- 100 object/field summaries and 1 MiB serialized payload per page. Large text
  is a value reference, fetched in UTF-8-safe chunks of at most 64 KiB raw text.
  Return one value chunk per response; source and target values are independent.
  Sixfold JSON escaping of a chunk remains below the page budget. Bound the
  envelope too; never inline the complete field or object to bypass paging.
- 60-second end-to-end deadline starts when admission reserves the endpoints.
  It includes credentials, tunnels, both connection handshakes, session setup,
  capture and diff. Every await uses the remaining budget and cancellation.
  Statement ceiling is 10 seconds and lock ceiling 2 seconds, reduced by the
  remaining job budget and any stricter configured timeout. CPU work checks
  cancellation/deadline between bounded units.
- Cleanup has a separate five-second grace period for cancellation, rollback and
  driver joins. On expiry, abort owned workers/drivers and join them; keep the
  reservation until they actually stop. Grace expiry cannot fabricate cancellation
  or release capacity for work still running. Test this against existing teardown.
- Two retained terminal results, 10-minute TTL; 256 MiB global comparison-owned
  allocation budget including active captures, diff output, retained results,
  containers and serialization scratch. Reserve before allocating; account owned
  buffer capacities and container overhead, not just text length. Native RSS
  measurements separately report allocator/driver/runtime overhead.
- At most two concurrent page serializers globally, each with an 8 MiB scratch
  reservation inside the global budget, held through IPC handoff. Excess requests
  return `busy`; no unbounded waiting queue. Step 2 must confirm the IPC handoff
  ownership boundary and bounded response duplication before claiming this limit.

These are explicit draft choices, not measured product limits. Adjust only with
fixture evidence and record the final values in the plan/ADR. Oversize schemas
return a named limit error and require a narrower future scope; never truncate
and call the remaining objects a complete diff.

## Implementation sequence

### Step 1: Capture and deparser feasibility gate

Before finalizing protocol or building a manager, use a small disposable PG16
fixture and capture harness to demonstrate the difficult contract. This is the
first implementation step, not work already performed during planning.

1. Reproduce snapshot establishment followed by a concurrent table/column rename,
   then CHECK/default/index deparsing. Also exercise DROP/recreate, concurrent
   index replacement and referenced-object rename. Include successful mixed reads,
   not just database errors, and both same-connection schema endpoints.
2. Try bounded relation locking with explicit lock/snapshot ordering. Determine
   which locks must precede the comparison snapshot, which required permissions
   they introduce, and which deparser dependencies remain unprotected. Do not
   assume acquiring locks after snapshot establishment repairs stale facts.
   Publish the tested query/lock sequence and its guarantees in the plan record.
3. For fields whose consistency the harness cannot establish, choose an explicit
   `notComparable` capability fallback. Do not use old-snapshot re-reads or a
   before/after checksum as proof against changes between reads. If structured
   capture itself cannot be made coherent, stop before Step 2. No unrestricted
   dependency lock walker or SQL-tree rewrite framework is in scope.
4. Demonstrate a custom constant type output function being reached during
   deparsing, using a controlled notice/error in a disposable fixture. Record
   the narrowed support-code guarantee above and verify timeout/error propagation.
   Check oversized rendered definitions are refused before transport allocation.

Gate passes only with the documented capture algorithm, deterministic interleaving
fixtures, and a field-by-field comparable/excluded matrix. If plain built-in
columns, simple defaults and ordinary CHECK/index expressions cannot be compared
reliably even without concurrent DDL, stop and revisit utility before a manager.
Any bounded retry is at most once and shares the original end-to-end deadline.

### Step 2: Protocol, normalization and byte-accounting fixtures

Create narrowly scoped comparison protocol, normalization and diff modules under
`src-tauri/src/postgres/schema_compare/`. Define typed endpoints, facts, field
differences, coverage, pages, phases and errors using existing serde conventions.
Mirror the IPC contract with discriminated TypeScript unions, without `any` or
casting-wrapper helpers. Start with deterministic pure fixtures for identity,
column order, defaults, constraints/indexes, exclusions and exact scope labels.

Define raw UTF-8 bytes, owned retained allocations and serialized JSON bytes as
separate budgets. Store immutable value blobs once; summaries carry job/result,
value identity and byte length. Value requests carry that identity and a validated
UTF-8 byte offset; responses carry text, next offset and completion. Reject offsets
inside a code point and mismatched/expired identities. Size-check using a capped
writer, including escaping and envelopes, rather than serializing then checking.
List/status responses contain metadata only, not definitions or diff value arrays.

Fixtures must include a 256 KiB control-character comment, multibyte boundaries,
source/target values, and simultaneous page requests. No response may exceed
1 MiB or trigger unreserved clones. Define typed `busy`, `limitExceeded`,
`unsupportedVersion`, `excludedCounterpart` and `unavailable` outcomes.

### Step 3: Bounded catalog capture

Implement Step 1's verified capture sequence, schema validation, transaction
settings and batched table/child-fact capture using the resolved native transport.
Scope SQL by retained identities, bound rows and bytes, and avoid one connection/query
per object.
Check permissions and record exclusions. Decode numeric quantities losslessly.
Finish or roll back each capture before publishing any completed result.

Enforce the PG16/PG16 capability matrix before querying version-sensitive facts.
Keep the name/kind inventory complete before eligibility filtering, then capture
only the closed supported field checklist. Fail required reads and record the
specific capability exclusions established in Step 1; never silently omit fields.

### Step 4: Deterministic structural diff

Compare normalized facts using stable sorted identities. Preserve source/target
direction, child order where semantic, and exact text where normalization is
unsafe. Report both known differences and incomparable fields without one hiding
the other. Bound CPU, retained result bytes and response serialization. Output
ordering must be independent of catalog row order and hash-map iteration.

### Step 5: Job ownership and lifecycle

Add a small comparison manager with start/list/status, bounded object/detail
pages, cancel and release commands. Register them in native dispatch; add a
typed client module without wiring product components or capability claims.
Use a caller request ID so an uncertain start can be reconciled without spawning
a second job. Bind pages to an immutable result ID and endpoint identity.

Reserve deadline and both endpoint generations before resolution. Propagate
admission cancellation through credentials, tunnel creation and both handshakes;
an absent configured connect timeout never disables the job deadline.

Phases: resolving, reading source/target (or both), comparing, completed,
cancelling, cancelled, failed. Report observed object counts and phases, not
invented percentages. Page expiry is `unavailable`, not equal or empty.

Cancellation must stop both readers and comparison work, close native drivers
and release reservations only after workers join. Teardown of either endpoint,
connection editing/deletion, credential/global resource invalidation and app
exit fence admission and invalidate that job's retained results. Cancellation
and terminal publication have one winner; stale workers cannot publish after
reconnect. A UI tab's future lifetime must not own the backend job.

### Step 6: Failure and boundedness validation

Focused unit/contract tests cover shuffled facts, exact mixed-case/quoted names,
equal empty schemas, direction reversal, OID independence, expression literals,
different schema names, external references, defaults, identity/generated fields,
constraint-owned index properties, NULLS NOT DISTINCT, FK delete-column subsets,
identity sequence exclusions, invalid indexes and excluded counterpart names.
Exercise every row/byte/page cap at and just beyond its boundary, including a
single oversized field/object, worst-case JSON escaping, UTF-8 chunk offsets,
concurrent serializers, TTL expiry and retry-safe admission.

Use only named disposable PostgreSQL fixtures and temporary files. Integration
coverage must include same-database schemas, independent PG16 databases, a
second PG16 minor where available (record any blocked coverage), refusal of
non-16 endpoints, permission failures, missing schema, Step 1's concurrent DDL,
unusual collations/types, expression/partial/include indexes, cancellation during
credentials/tunnel/handshake resolution as well as capture, failed TLS verification,
reconnect and teardown of either endpoint. Test deadline expiry before capture,
cleanup grace expiry and retained capacity until workers join.
Test that catalog failures cannot produce false absence or a complete result.

Measure native peak memory with increasing schema size and oversized metadata.
Show that jobs refuse at defined limits and that concurrent jobs, retained
results and page serialization obey the combined budget. Verify bounded database
connections and termination time; no user-table scanning or DDL execution.
Opt-in tests must be run explicitly and their results recorded.

### Step 7: Completion and handoff

Run `pnpm format`, `pnpm lint`, `pnpm typecheck`, focused frontend contract tests,
`pnpm test`, `just fmt`, `just lint` and `just test`. Inspect the final diff against
this scope. Record blocked checks honestly. Update README, PAR-008, the roadmap,
glossary and an accepted ADR only to describe delivered backend behavior. Keep
the roadmap's comparison UI marked absent until its activation actually ships.

The next UI planning pass compares several static app-faithful layouts for
endpoint selection, field differences, exclusions, cancellation and stale results.
Do not treat this implementation draft as approval of a product layout.

## Stop conditions

- Step 1 cannot prove the capture contract or its useful conservative fallback:
  stop before protocol/manager implementation and report the failed fixture.
- Required metadata is unavailable or normalization would need semantic SQL
  rewriting: return explicit coverage/error; stop scope expansion for a decision.
- Snapshot/result limits cannot be enforced before driver/IPC allocation: resolve
  the architecture before claiming boundedness or proceeding to activation.
- Migration generation, SQL execution, row comparison or another engine becomes
  necessary: split the work into a new plan instead of broadening Plan 021.
- Real UI changes require the user's published mock selection first.
- No production, live database, daily-driver preview/build, commit, push or PR
  is authorized by this planning request. Prepare isolated fixtures when later
  implementation is authorized; name them before touching them.

## References and planning record

- PostgreSQL [transaction isolation](https://www.postgresql.org/docs/16/transaction-iso.html)
  and [SET TRANSACTION](https://www.postgresql.org/docs/16/sql-set-transaction.html):
  repeatable-read snapshot semantics and read-only transactions. These define
  transaction behavior, not a cross-server atomicity guarantee.
- 2026-09-05: Imran confirmed Plan 020 complete at `7745946`. Reviewed the plan
  register, roadmap, phase claims, current catalog/DDL and lifecycle code. Chose
  PAR-008 as the proposed next arc, following the prior planning brief. Corrected
  stale shipped comparison claims; no product implementation changed.
- Draft verification: `pnpm format`, `pnpm lint`, `pnpm typecheck` and
  `git diff --check` passed. HTML structure, local links, anchors and disclosure
  markup passed a jsdom check. Browser visual QA remains unverified: computer
  use reported no browser and the collaborative preview failed to load the
  isolated local server. No product/Rust tests were rerun for documentation-only
  changes. The planning changes remain uncommitted.

### Critique revision (2026-09-05)

All six GPT-6 findings are incorporated in this planning revision:

1. Chose the narrower metadata-rendering guarantee; server support code may run.
2. Added a capture/deparser proof gate before protocol and manager work.
3. Extended the deadline to admission/resolution, with separate cleanup grace.
4. Specified value chunks, distinct byte budgets and bounded IPC serialization.
5. Pinned V1 to PG16/PG16 and closed the correctness-sensitive field checklist.
6. Required complete counterpart inventory before relation eligibility filtering.

Reference evidence: PostgreSQL 16 [deparser source](https://raw.githubusercontent.com/postgres/postgres/REL_16_STABLE/src/backend/utils/adt/ruleutils.c)
(`get_const_expr`, `pg_get_indexdef_worker`, `set_relation_column_names`),
[index facts](https://www.postgresql.org/docs/16/catalog-pg-index.html),
[constraint facts](https://www.postgresql.org/docs/16/catalog-pg-constraint.html),
and [array declarations](https://www.postgresql.org/docs/16/arrays.html#ARRAYS-DECLARATION).
The deparser consistency scenario is a source-backed risk awaiting the Step 1
fixture, not a claimed live reproduction. No comparison implementation or database
verification is performed by this revision.

Revision verification: `pnpm format`, `pnpm lint`, `pnpm typecheck` and
`git diff --check` passed. Checked the seven-step sequence and the HTML brief's
local links, anchors, disclosure markup and revised contract summaries with
jsdom. Visual QA remains unverified; the previous browser attempts failed and
the newly available agent-browser skill has no installed CLI in this environment.
Only the plan, visual brief and register were edited in this revision. No product
or Rust code changed, and no implementation tests or databases were run.


### Step 1 implementation record (2026-09-05)

Implementation authorized by Imran's request to start this plan. Added and ran
the [disposable capture harness](../infrastructure/test-db/schema-compare/README.md)
on PostgreSQL 16.15 (Debian 16.15-1.pgdg13+2), aarch64. The linked record contains
the tested lock/query ordering, exact reproductions, field capability matrix,
permissions and remaining proof obligations. All reproduction assertions passed;
the architectural gate did **not** pass.

Snapshot-then-rename yields successful mixed CHECK/index definitions. Locking
before the snapshot protects table descriptors but not index names or expression
dependencies. Stronger parent locks still permit index renaming. DROP/recreate
can return NULL from a required renderer without an SQL error. The controlled
constant type output function emits a notice, propagates errors and cooperative
timeouts, and demonstrates a server-side oversized-value transport guard.

The failed conservative capability rule is "built-in types plus no external
pg_depend references". A text default containing a regclass[] constant has only
its owning-column dependency; a concurrent sequence rename still changes its
rendered text within the same locked snapshot. Blanket expression exclusion is
safe but does not prove the useful default/CHECK/index-expression capability.
Per the Step 1 stop condition, no protocol, manager, product UI, accepted ADR or
shipped glossary contract is introduced. A sound positive expression capability
strategy or explicit scope reduction must precede Step 2. This is a failed proof
for the tested strategies, not a claim that comparison is impossible.

The plan's proposed product limits are unchanged. Fixture timing and memory
limits are isolated test settings. No production, existing fixture database,
daily-driver channel, commit, push or PR was touched.

Validation: the opt-in Docker/C/Python harness passed its reproduction assertions;
`pnpm format`, `pnpm lint`, `pnpm typecheck`, `pnpm test` (123 files, 1,439 tests)
and `git diff --check` passed. No Rust or product code changed, so native Rust
checks and later protocol/lifecycle/memory acceptance tests were not run. The
full seven-step plan remains incomplete.

Review completion (2026-09-06): fixed the first review's harness findings:
optimized-Python checks, absolute client deadlines, cleanup error/ownership
handling, index replacement overlapping capture, both endpoint reads in one
transaction, and default/CHECK/index reads after table/column renames. Normal
and `python -O` live harness runs passed on PG16.15; cleanup failure/ownership
probes passed and no gate containers remained. The frontend checks and all
1,439 tests passed again after the fixes. A fresh final reviewer reported no
actionable findings and agreed the documented gate blocker is supported.


### Conservative capture and Step 2 (2026-09-06)

Imran authorized proceeding to the next step. The positive scalar expression
recognizer resolves the earlier gate for a useful conservative subset; it does
not rehabilitate dependency-only certification. The live PG16.15 fixture accepts
plain integer/text defaults and scalar CHECK/index expressions, rejects both
versions of the hidden regclass-array dependency, and demonstrates bounded
discovery, schema existence, held-lock validation, changed-candidate refusal
and one fresh retry. Field coverage and prerequisites are documented in the
[fixture record](../infrastructure/test-db/schema-compare/README.md) and
[ADR-0030](../docs/adr/0030-postgres-schema-comparison-foundation.md).

Step 2 adds native typed endpoints, reference namespaces, field paths, facts,
coverage, phases and errors, mirrored by TypeScript discriminated schemas.
Pure matching preserves excluded counterparts and distinguishes identity
matching from definition equality. Field comparison preserves ordered facts
and reports known differences alongside incomparable fields. Raw expressions
and literals are never rewritten.

Value storage reserves immutable blobs and slot capacity, enforces per-endpoint
and shared result budgets, validates result/side/value identity and TTL, and
returns UTF-8-safe chunks. Capped serialization includes JSON escaping and page
envelopes. Two serializer leases reserve scratch without a queue and survive
transport handoff until the receiving session acknowledges. The actual Tauri
JavaScript serializer dependency is tested for bounded Rust buffer duplication.
A 64 KiB control-record allowance is kept within the unchanged 256 MiB global
ceiling. This is an ownership contract with tested primitives, not native IPC
integration or a claim about full runtime RSS.

Next is Step 3: implement the bounded native catalog capture against these
contracts. The native manager/dispatch, complete capture decoder, cancellation
and teardown integration, cross-server live coverage and runtime measurements
remain unbuilt. No comparison product surface is activated. No commit, push,
PR, shared database fixture or daily-driver channel is touched.

Validation on 2026-09-06:

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: passed.
- Full frontend suite after review fixes: 124 files, 1,446 tests passed.
- `just fmt`, `just lint`, `just test`: passed using the isolated
  `/tmp/dbunk-plan021-target` directory. Rust: 561 passed, 51 existing opt-in
  tests ignored; no new ignored tests.
- `python3 -O infrastructure/test-db/schema-compare/gate.py`: passed on PG16.15,
  including missing-schema refusal, discovery races and the actual Rust scalar
  recognizer. The owned disposable container was removed. A second PG16 minor
  remains untested; differing minor versions cannot establish rendered-expression
  equality until compatibility is verified.
- Independent review and fresh review-fix verification passed. Full native IPC/WebView allocation
  measurements remain Step 6; the current serializer fixture measures Rust
  buffer capacities and ownership only.

The first independent review identified result-scope reset, optional response
binding, contradictory summary variants, unrestricted generic page payloads,
missing numeric server-version metadata and missing positional index-key facts.
It also corrected the IPC evidence to the locked Tauri 2.11.2 dependency.
All findings were fixed by a fresh implementation agent and the required checks
passed again. Positional `IndexKey` paths preserve mixed keys and per-key facts;
tagged summaries/status prevent contradictory states; only typed summaries are
accepted by public page APIs. Result scopes retain their shared counters and
response handoff derives its identity from the encoded page. A second review
caught mismatched source/target value sides at the Rust page boundary; a fresh
fix agent added validation before serializer admission and tests for every
summary variant. The full Rust checks passed again. The second reviewer found
no further issues after inspecting the fix; a fresh focused review of that last
change also found no actionable issues. These fixes do not activate native capture or expand the
supported expression grammar.


### Native catalog capture, Step 3 (2026-09-06)

Implemented `schema_compare/capture/` using the resolved dedicated PostgreSQL
transport. The native reader validates the PG16 boundary and schema permissions,
discovers complete bounded relation inventories, acquires quoted `ONLY` table
locks before the snapshot, validates held OIDs and retries one changed capture
within the original deadline. Same-connection endpoints share a transaction.
Successful captures are rolled back and their dedicated drivers closed before
returning data; errors never produce partial completed captures.

The reader captures the closed typed column/constraint/index projection, including
all FK equality operators and delete-column subsets, full constraint-owned index
facts, positional mixed keys, array declarations, identity/generated fields and
comments. Expressions retain raw text, with explicit scalar/dependency/version
fallbacks. Excluded relation identities remain in the counterpart inventory;
additional excluded categories have bounded counts where implemented.

SQL guards raw UTF-8 fields before transport, caps pages at 64 fields/2 MiB and
scopes child queries to retained OID groups of at most 32 tables. Native scratch,
fixed inventory/field containers and owned string/vector capacities are reserved;
per-endpoint retained allocation and field-count caps refuse oversized captures.
The ADR records these transport/retention settings and the future manager's
shared result-budget responsibility. No comparison commands or UI are activated.

Validation:

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: passed.
- Full frontend suite: 124 files, 1,446 tests passed.
- `just fmt`, `just lint`, `just test`: passed with isolated
  `/tmp/dbunk-plan021-target`; Rust: 564 passed, 52 opt-in tests ignored by the
  default suite. The one new ignored test was explicitly run below.
- `python3 infrastructure/test-db/schema-compare/native.py`: passed on
  PostgreSQL 16.15 (Debian 16.15-1.pgdg13+2), aarch64. The real native reader passed
  field/mapping, exact field/inventory/table caps, missing-schema/permission,
  concurrent discovery retry and cancellation-during-lock-wait checks. Each
  disposable `dbunk-schema-compare-native-*` container was removed.
- Independent review found one coverage gap: the native fixture did not exercise
  exclusion constraints. A fresh fix agent added qualified operator signatures,
  operand types and constraint-owned mixed GiST index/key coverage. It also
  corrected plain multi-expression lists to report `expressionOutsideSubset`
  rather than a synthetic `externalDependency`. All required checks and both full
  suites passed again after the fixes; the explicit PG16.15 fixture passed.
  A fresh final reviewer found no actionable findings.

Next is Step 4's structural diff. Job admission, connection-generation fencing,
resolution/teardown integration, independent-server/minor live coverage and
native runtime allocation measurements remain Steps 5–6. Changes remain
uncommitted; no push, PR, shared fixture or daily-driver channel was touched.


### Deterministic structural diff, Step 4 (2026-09-06)

Implemented an immutable result builder connecting the native captures,
complete-inventory matcher, typed field comparison and bounded value/page storage.
Exact relation names and sorted field paths determine result order and value IDs;
OIDs remain local lookup keys. Known differences coexist with incomparable fields,
excluded counterparts never become directional absence, and raw expression text
is retained without rewriting. The result reports equality only within the named
projection and derives rendered-field exclusions from both exact server versions.

Both input captures and retained output use one result allocation scope. The native
reader marks the actual transaction, so same-connection captures cannot claim a
shared snapshot based on equal timestamps. Container/path/value reservations,
per-field cancellation/deadline checks and capped summary/value serialization
bound construction and reads. Input captures are dropped before a result escapes.
Peak input/output overlap is intentionally counted against the result cap.

Validation:

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: passed.
- Full frontend suite: 124 files, 1,446 tests passed.
- `just fmt`, `just lint`, `just test`: passed in the isolated
  `/tmp/dbunk-plan021-target`; Rust: 574 passed, 52 default opt-in tests ignored.
- Ten new focused diff tests passed, covering shuffled identities/OIDs/value
  IDs, changed-plus-incomparable output, key/column order, exact literals,
  directional renames, excluded counterparts, empty/excluded-only schemas,
  minor-version fallback, actual transaction/scope binding, result limits,
  cancellation between fields, paging, worst-case escaping, TTL and value sides.
  Side-specific inherited and extension-owned exclusion reasons survive capture
  release and reject reads for the wrong result, relation kind or side.
- `python3 infrastructure/test-db/schema-compare/native.py`: passed on PG16.15
  (Debian 16.15-1.pgdg13+2), aarch64. The expanded native fixture passes captures
  through the diff, then changes a target default and comment and verifies both
  known changes alongside the existing incomparable fields. Its owned disposable
  container was removed. No new ignored tests were added.
- Independent review found that per-relation exclusion reasons were lost when
  input captures were released. A fresh fix agent retained each side's eligibility
  in the bounded result and added exact identity/side-bound native reads and tests.
  All required checks, both full suites and the disposable PG16.15 fixture passed
  again after the fix. A fresh final reviewer found no actionable standards or
  specification findings.

Step 5 is next: native job ownership, admission, lifecycle and dispatch. UI,
production/shared databases and daily-driver channels remain untouched. No commit,
push or PR is authorized by this step; the changes remain uncommitted.

### Thermo-nuclear review follow-up (2026-09-06)

The foundation through Step 4 and the review fix are now committed at `502674e`.
The audit found one P2 correctness issue: inherited `quote_all_identifiers`
settings could make identical CHECK/index expressions appear changed and make
supported defaults incomparable. Capture now sets it to `off` locally within
the comparison transaction. Required identifier quotes and literal contents are
preserved. The ADR and native fixture documentation describe this behavior.

Validation after the fix:

- The new native quoting test failed before the fix and passed afterward. It
  captures with opposite connection settings and verifies an equal structural
  result, including supported defaults and CHECK/index expressions.
- `pnpm format`, `pnpm lint`, `pnpm typecheck`, `just fmt`, `just lint` and
  `just test`: passed. Rust used `/tmp/dbunk-plan021-target`; 574 tests passed
  and 53 opt-in tests were ignored by the default suite.
- `python3 infrastructure/test-db/schema-compare/native.py`: both native tests
  passed on PostgreSQL 16.15 (Debian 16.15-1.pgdg13+2), aarch64. The disposable
  container was removed.
- The full frontend suite passed during the audit: 124 files, 1,446 tests.
  It was not repeated for the subsequent Rust-only fix.

Plan 021 remains in progress through Step 4. Step 5 is next: native job
ownership, admission, lifecycle and dispatch. Runtime/IPC allocation validation
remains Step 6; comparison UI activation is a separate slice.


### Native job ownership and lifecycle, Step 5 (2026-09-06)

Implemented comparison admission, native execution, phase/count status, cancellation,
result release and endpoint-bound metadata/object/field/value/eligibility reads.
The native dispatch and typed client are connected without activating product UI.
Both endpoint reservations and a single deadline precede credential/tunnel setup;
same-connection schemas share capture and independent connections use sequential
readers. Canonical connection/global fences and app exit invalidate results and
wait for real worker and dedicated-driver joins.

Caller IDs contain a timestamp and random nonce. New IDs are accepted for one
minute; 64 request records retain reconciliation for ten minutes and return busy
when full. Released or evicted jobs cannot be recreated by a retained ID, and
expired IDs cannot become new work. Two terminal results retain the existing
10-minute TTL. Page serializers remain owned through native IPC handoff until
document acknowledgement, committed replacement document, or
window destruction, including after release.

Blocking OS setup cannot be forcibly stopped by Tokio. Grace expiry requests abort
and keeps the job cancelling with its reservations until its actual join completes.
Dedicated-driver joins remain tracked when a connect future is cancelled during
session setup; grace expiry aborts existing and late-registered drivers.
Comparison-owned ephemeral SSH routes use an OS-assigned local port. Publication
guards preserve shared and pending setup leases while rolling back new resources,
and owned tunnel workers are joined before admission is released. The tested setup/transport scratch reservation is 32 MiB per active
job in addition to existing capture/result accounting; the global ceiling remains
256 MiB. Full native runtime allocation evidence remains Step 6.

Validation before independent review:

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: passed.
- Full frontend suite: 125 files, 1,451 tests passed.
- `just fmt`, `just lint`, `just test`: passed using
  `/tmp/dbunk-plan021-target`; Rust: 583 passed, 54 opt-in tests ignored.
- `python3 infrastructure/test-db/schema-compare/native.py`: all three native
  tests passed on PostgreSQL 16.15 (Debian 16.15-1.pgdg13+2), aarch64. This
  explicitly ran the new opt-in manager test, exercising native start/capture/diff,
  metadata/field reads and result invalidation after a real connection edit.
  The owned `dbunk-schema-compare-native-15a0c11632f2` container was removed.
- Focused tests cover admission collisions and deduplication, cancellation versus
  late success, either-endpoint/global fences, deadline and cleanup-grace expiry,
  retained capacity until join, worker panic, history/TTL limits, page ownership,
  endpoint binding and tracked-driver cleanup.

Independent review found a Tauri monitor-startup regression, an SSH setup
cancellation ownership gap, missing driver abort at cleanup-grace expiry, a
restart-exit cleanup bypass and unbounded connection-fence records. Two fresh fix
agents resolved all five findings. New tests cover monitor startup outside Tokio,
late driver registration after abort, repeated exit/restart cleanup, SSH publication
rollback and concurrent setup leases, plus the four-record fence limit and the
64 KiB control-memory bound. Forward workers also reap completed stream handles
and check cancellation while opening direct channels.

Post-fix verification passed again: all frontend format/lint/type checks and the
full 125-file, 1,451-test suite; `just fmt`, `just lint`, and `just test` with
591 Rust tests passed and 54 default opt-in tests ignored. All three native
PG16.15 tests passed explicitly again; the owned
`dbunk-schema-compare-native-8373ab4780fe` container was removed. A fresh final
review found two further issues: duplicate SSH-session cleanup held the global
tunnel lock, and repeated ordinary exit requests could bypass pending cleanup.
A fresh fix agent moved duplicate cleanup outside the lock and added a pending/
finished exit decision used by the actual event callback. Deterministic regression
tests cover both. All required checks passed again: 593 Rust tests, 1,451 frontend
tests, and all three disposable PG16.15 native tests. The final owned fixture,
`dbunk-schema-compare-native-2dc40f7c9759`, was removed. The corrected tree is
reviewed by a fresh final reviewer, who found no actionable findings. Step 5 is
complete; Step 6 is next.
Step 6 remains native failure/load/IPC validation;
comparison UI activation remains a separate slice. No commit, push, PR, production
or daily-driver channel was touched.

### Thermo-nuclear review fixes, Step 5 (2026-09-06)

The requested GPT-6 audit found two issues: ProxyCommand shutdown could block on
a descendant retaining a pipe, and a document reload could abandon both global
response serializer slots. Proxy pipe pumps now use cancellable reads and writes,
including backpressure, before joining all owned workers. Unix uses nonblocking
descriptors and bounded polling; Windows uses synchronous-I/O cancellation on
owned thread handles. Fallible proxy setup also retains ownership of the child.

Native response ownership now includes a bounded document token registry. A
committed replacement document retires old replies and creates a new token.
The locked Wry desktop implementations expose this through page-load `Started`
(macOS `didCommitNavigation`, Linux `Committed`, Windows `ContentLoading`).
Failed or cancelled provisional navigation preserves the live document's token
and replies; `Finished` is ignored, including unmatched or duplicate completions.
The registry is managed before configured windows are created, so their first
commit cannot be missed while application setup is pending.
Every read obtains the current token,
and native reads/acknowledgements reject stale tokens under the handoff lock.
Window destruction removes the transport. No product UI was activated.

Verification passed: `pnpm format`, `pnpm lint`, `pnpm typecheck`, all 1,452 frontend
tests, and isolated `just fmt`, `just lint`, `just test` with 597 Rust tests passed
and 54 default opt-in tests ignored. All three native PG16.15 fixtures passed
explicitly; `dbunk-schema-compare-native-d5e30ccfeea0` was removed. Focused tests
cover descendant-held stdout, backpressured stdin, reload reclamation, stale
read/acknowledgement rejection, bounded transports, and current-token client reads.
An unrelated DDL-dialog timing assertion failed on the first full frontend run;
its isolated rerun and the subsequent full suite passed without changes to it.
Windows-specific compilation/execution remains unverified on this macOS host.
The isolated build initially ran out of disk space; removing only its disposable
incremental cache allowed verification to complete. Changes remain uncommitted.

Fresh follow-up review corrected the document lifecycle boundary: completion is
not evidence of replacement after a cancelled or failed navigation. The shared
native page-load handler now retires replies only on the verified desktop commit
callbacks described above. Its focused test preserves both live leases and a
legitimate acknowledgement through unmatched/duplicate completion, reclaims both
leases on replacement commit, and rejects late old-document reads and
acknowledgements. Delayed completion cannot retire a new-document lease or recreate
a destroyed transport. The manager is registered before initial window creation.
Follow-up `just fmt`, `git diff --check`, and the isolated manager suite passed
(13 passed, one owned PostgreSQL fixture ignored). Final `just fmt`, `just lint`,
and `just test` passed again with 597 Rust tests and 54 default opt-in tests
ignored. All three native PG16.15 tests passed again, and the owned
`dbunk-schema-compare-native-185d3027c94d` container was removed. The frontend
remained unchanged after its passing format/lint/typecheck and 1,452-test run.

A subsequent fresh review found an avoidable cancellation wait in nested SSH
resolution: an exhausted local SQLite pool could hold bastion loading or post-join
fingerprint persistence until its acquisition timeout. Bastion reads and pool
acquisition now observe the same cancellation/deadline check while pending.
Focused review demonstrated that dropping a dispatched fingerprint UPDATE can
leave SQLx's SQLite worker running after setup returns and overwrite a subsequent
host-key reset. The write is therefore awaited unconditionally after acquisition,
with cancellation checked again before session publication. The spawned SSH
worker is also still joined unconditionally.

The focused tunnel suite now covers cancellation and deadline expiry with all five
local pool connections held, cancellation before fingerprint connection
acquisition, and a dispatched UPDATE blocked by a real temporary SQLite write
transaction. The latter keeps setup pending through cancellation until the write
finishes, rejects session publication, then verifies a subsequent host-key reset
persists after both SQLite workers drain. It fails against the previous
cancellable-UPDATE implementation at the expected ownership assertion and passes
with the fix. All eight focused tunnel tests pass.

Final verification after the write-ownership correction passed: `just fmt`,
`just lint`, and `just test` with 601 Rust tests passed and 54 default opt-in
tests ignored. All three native PG16.15 tests passed explicitly, and
`dbunk-schema-compare-native-0f74a33263cb` was removed. The unchanged frontend
retains its passing format/lint/typecheck and 1,452-test results.
The fresh final GPT-6 review found no actionable findings in the corrected
ownership paths and their callers. Both original review findings and all
follow-up findings are resolved. Step 6 remains next; changes are uncommitted.
