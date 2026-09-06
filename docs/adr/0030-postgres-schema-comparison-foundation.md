# ADR-0030: PostgreSQL schema comparison foundation

**Status**: Accepted for the Plan 021 foundation (2026-09-06). Native catalog
capture and structural diff are implemented; job commands are not yet integrated
and no comparison UI is active.

## Problem

PostgreSQL repeatable-read catalog queries and server deparsers can observe
different metadata. The PG16.15 fixture reproduced old column facts with new
CHECK/index names, NULL rendering after DROP/recreate, and names inside array
constants that change without an external `pg_depend` entry. Comparing generated
CREATE strings or assuming dependency listings certify expressions is unsound.

## Decision

The foundation compares the explicitly named
`postgres16OrdinaryTableProjectionV1`, normalization version 1. Portable identity
uses exact names and kinds, plus parent identity for children. Only structured
references to the selected schemas map to one namespace. Text is never rewritten,
casts removed, literals replaced, or renames inferred. Type and collation
references do not compare their underlying definitions.

Capture must discover a bounded inventory, acquire table ACCESS SHARE locks
before the repeatable-read snapshot, then validate schema existence and the
complete snapshot inventory against the actual held relation OIDs. Both schemas
on one stored connection share this transaction. A newly visible eligible table
without a lock requires a failed capture and at most one fresh attempt within
the original deadline. Name resolution failure is not an empty schema. Excluded
counterpart names remain in the inventory and cannot become directional absence.
The native reader implements this ordering with one dedicated resolved transport,
bound schema/OID queries, 32-table groups and guarded field pages. It checks
schema USAGE and table SELECT permissions, validates actual held locks and
performs at most one fresh retry. It rolls back every successful read transaction
before returning an immutable capture and closes its dedicated driver.
Admission, endpoint generations and global teardown belong to Step 5.

Rendered expressions are comparable only within a positive scalar grammar:

- scalar literal values, parentheses and safe column references;
- scalar casts to smallint, integer, bigint, numeric, text and boolean;
- arithmetic/comparison operators, Boolean operators and null/Boolean tests.

Safe referenced columns have catalog types `pg_catalog.bool`, `int2`, `int4`,
`int8`, `numeric` or `text`. Every recorded dependency other than the owning
locked table/columns is rejected, including non-pinned objects in `pg_catalog`.
Fixed PG16 session settings (including `quote_all_identifiers = off`) and
pre-snapshot locks remain prerequisites.
Expression comparison also requires matching `server_version_num` values.
Different PG16 minors remain supported for structured facts; rendered fields
are `notComparable` with `renderingVersionDifference` until that pair's rendering
compatibility is verified. Only PG16.15 has been exercised so far.
The recognizer does not establish safety in their absence. Function calls,
arrays, object-identifier alias types, custom types/operators/collations,
backslash string forms and unrecognized syntax are `notComparable`. Recognition
has byte, token and nesting limits; exceeding those also excludes comparison.
Captured raw text can still be retained for review, but cannot establish a
change or equality for an incomparable field. The comparison layer preserves
known differences alongside such fields.

This is a deliberately narrow capability rule, not a semantic SQL parser or
tree rewriter. Plain integer defaults, text literals and ordinary scalar
CHECK/index expressions remain useful. The prior hidden `regclass[]` example
is rejected both before and after a concurrent rename. Table column types and
other structured facts remain comparable even when an expression is excluded.

## Values and memory

Typed native fields and TypeScript discriminated unions describe endpoints,
references, field paths, coverage, errors and phases. Identity matching returns
`matched`, never `equal`; definition comparison must follow. Matching and field
comparison borrow immutable captures and sort their own bounded index arrays.
They must receive complete, validated facts from capture, not partial query rows.
Index keys use positional field paths so mixed column/expression keys and each
key's optional collation, opclass options and sort facts remain aligned without
sentinel values. Field/object summaries are tagged by difference kind, requiring
the valid observed sides and an incomparability reason. Status is also tagged by
phase: only completed states carry a result ID, and only failed states carry an
error. Capture metadata retains both the display version and numeric
`server_version_num` used by the rendering compatibility rule.

Large values have one immutable stored blob and small references. Strings retain
raw UTF-8. Other facts retain compact JSON with an explicit value kind. Source
and target values have independent keys; result identity is on the envelope.
Value reads validate identity, side, kind, byte length, TTL and UTF-8 byte offset.
One response contains at most 64 KiB of raw value text. Object/field pages contain
at most 100 summaries; a capped writer refuses before crossing 1 MiB of JSON,
including escaping and metadata. Status has no definition arrays.

Allocation reservations precede value copies and container allocations. The
value slot array has capacity for 50,000 values per endpoint; endpoint bytes are
capped at 16 MiB. A shared result scope limits the combined retained values,
inventory and diff allocations to 32 MiB. The future owner must pass the value
store's `result_budget()` to its inventory/diff work rather than creating an
independent result scope. Global admission has a 256 MiB ceiling, with 64 KiB
kept aside for bounded owner/control records and counters. Allocator and runtime
overhead remain part of the separate native RSS measurements in Step 6.

At most two serializers reserve 8 MiB each from the same global budget without
a wait queue. `EncodedPage` deliberately cannot be returned directly through
Tauri's automatic `IpcResponse` serialization. The integration must move the
body into Tauri while retaining its lease in `ResponseOwnership`, then release
only on an acknowledgement from the receiving webview or confirmed destruction
of that transport. Job cancellation, result expiry, an uncertain send outcome
or a timer cannot release in-flight scratch. Response identity is bound to the
encoded envelope; mismatched acknowledgements cannot release another response.

## IPC ownership evidence

The checked local dependencies are Tauri **2.11.2** and
`serialize-to-javascript` **0.1.2**. In Tauri's `ipc/mod.rs`, `IpcResponse::body`
returns an owned body before its responder runs. `ipc/protocol.rs` either moves
JSON into an HTTP response/channel queue or formats a JavaScript callback for
`webview.eval`. The callback path uses `format_callback::serialize_js_with`;
the renderer may therefore own queued buffers after a command returns.

The fixture calls the actual `serialize-to-javascript` dependency with
near-1-MiB envelopes containing single quotes, backslashes, control characters
and multibyte text. Original JSON plus escaped JavaScript and callback buffer
capacities remain within the 8 MiB reservation. The ownership tests retain that
reservation after handoff, reject a third serializer, and require the matching
session/response acknowledgement. This establishes the adapter contract and
Rust buffer allowance; it is not a measurement of WebView/driver RSS or a claim
that native dispatch is already integrated. Step 5 must wire this ownership
boundary, and Step 6 must validate it in the native runtime before activation.

## Consequences

Schema comparison can progress with explicit limited coverage. Unknown metadata
does not become equality, absence, executable SQL or an invented normalized
expression. Native capture propagates a supplied deadline and cancellation,
including lock waits. Retained result admission, endpoint invalidation,
resolution/teardown integration and live runtime measurements remain Plan 021 work.
The [capture fixture record](../../infrastructure/test-db/schema-compare/README.md)
preserves the failed original strategy and the conservative replacement proof.


## Native capture (Step 3, 2026-09-06)

The reader checks `server_version_num` before PG16-specific catalog SQL. Both
schemas on one stored connection share pre-snapshot locks and a transaction;
independent resolved connections use separate readers and capture times. A
required query, permission or render failure returns an error, never an empty
listing. Ordinary partition children, inherited/foreign/extension-owned tables
and other relation kinds remain in the complete counterpart inventory.

Direct catalog joins capture the closed column, constraint and index projection.
FK equality operators retain all three ordered groups (PK=FK, PK=PK, FK=FK).
Exclusion constraint key names omit zero expression placeholders; their complete
positions and expressions remain in the owning index keys. Multiple expression
keys retain the raw complete expression list on each expression position and
remain incomparable; no SQL-list parser is introduced. Option strings are sorted
as catalog option sets; key and INCLUDE order remains semantic. Built-in btree
flags become explicit sort/null-order names; unknown method/flag semantics remain
incomparable. Required NULL deparses abort capture.

Field SQL measures UTF-8 bytes before returning text and guards each field at
256 KiB. A transport batch contains at most 64 fields and 2 MiB including
conservative row overhead. A running byte sum returns only a bounded prefix;
the next offset resumes the same immutable snapshot. Oversize values cross as
an error flag with NULL payload. A 32 MiB global scratch reservation precedes
native capture and covers bounded driver/decoder buffers and transfer into
retained storage. Inventory and field containers reserve their capacities,
retained strings/operator vectors charge their actual capacities, and each
endpoint refuses above 16 MiB or 50,000 field records. The manager must pass the
same result-budget scope to both readers, value storage and diff work.

Statement waits use the remaining original deadline, a 10-second ceiling and
any stricter configured statement timeout; lock waits additionally cap at two
seconds. Error cleanup allows three seconds for cancellation/rollback and the
existing dedicated driver up to two seconds to close, abort and join. Worker
abort/resolution fences and admission release are still the manager's task.
Routines, types, policies, triggers, rules and extensions have capped excluded
counts with an explicit completeness flag. Other exclusions remain categories.

The disposable native PG16.15 fixture exercises the real driver/SQL/decoder,
including same-database mappings, field details, exact limits, permission failure,
a candidate added during a lock wait and cancellation during a lock wait.
A second PG16 minor, cross-server integration and native RSS/IPC measurements
remain later validation. This is not a full runtime allocation measurement or
a promise about PostgreSQL's internal metadata/support-code heap.


## Structural diff (Step 4, 2026-09-06)

`schema_compare::diff::compare` consumes two successful captures and produces an
immutable `Comparison`. The readers and result must share the same allocation
scope; sharing only a global allocator is insufficient. A same-connection pair
also requires the private transaction marker installed by the native reader.
Matching timestamps alone cannot certify a shared snapshot. Independent stored
connections retain independent snapshot metadata.

Relations match exact names in the complete inventory before eligibility is
considered. Fields sort by typed paths within each table, with OIDs used only
for local lookup. A bounded first pass counts the exact field union; the second
retains one summary per path and stable source/target value identities. Shuffled
catalog rows or different local OIDs do not change serialized result ordering or
value IDs. Column positions and ordered key/INCLUDE/operator facts remain
semantic; raw expression text is never rewritten.

Known changed/source-only/target-only fields and incomparable fields have separate
counts. A table can be changed while retaining incomparable field summaries and
raw values. An excluded counterpart produces an incomparable object with no
invented field absences. Renames remain source-only plus target-only objects.
Each object also retains both sides' eligibility after captures are released,
including distinct inherited or extension-owned exclusion reasons. Native reads
bind that eligibility to the result, exact relation identity and requested side.
The overall result is changed if any supported change is known, incomparable if
only exclusions/unknown fields remain, and otherwise equal within the named
projection. Equal empty scopes still disclose the excluded categories.
Coverage's incomparable-field count counts merged result paths, including fields
excluded by the exact server-version comparison; it does not double-count the
two endpoints of one path.

Temporary field indexes reserve global scratch. Retained summary containers,
cloned identity/path strings, metadata and immutable Values blobs reserve the
same 32 MiB result scope as the still-live input captures. Inputs are released
before returning the completed result. Peak overlapping ownership can therefore
refuse a large comparison even when its eventual result alone would fit; limits
are never bypassed by starting a fresh scope. CPU checks use the original
capture/job deadline and cancellation between objects/fields and around bounded
sorts. The future manager must run this synchronous work on an owned worker and
hold admission through its join.

Object and detail reads project only the requested window, at most 100 summaries,
into the existing capped serializer. Definitions leave only as independently
addressed value chunks. Every page and chunk validates immutable result identity
and expiry; field reads also validate the exact observed relation identity.
Serializer leases retain the existing acknowledgement/handoff contract. Native
job admission/teardown and dispatch remain Step 5, and native IPC/RSS validation
remains Step 6.
