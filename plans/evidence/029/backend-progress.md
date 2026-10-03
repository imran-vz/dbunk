# Plan 029 tools preparation

Status: selected Tool tabs A are implemented for history and saved queries,
with execution-history integration. Their frozen source checks pass in
`tool-tabs-source-checks/`; actual-window tools acceptance remains pending.
A later [verified source variant](../028/browse-inspector-source-checks/README.md)
adds EXPLAIN query drafts/tree inspection and owned catalog reads. The subsequent
[catalog/array slice](./catalog-array-source-checks/README.md) adds a native Objects
Tool tab and eight-kind descriptions, with an exact live probe passed. Other families
remain backend/model preparation or unimplemented.

## Implemented families

T02: six history/saved-query functions moved from Tauri commands into
`query_library`. The command adapters retain their payloads and results.
History keeps its 2,000-entry limit, start-time ordering, exact SQL and outcomes;
saved queries retain update ordering, creation timestamps and optional bindings
to removed connections. Three SQLite tests cover these semantics and failed
writes. That initial extraction did not integrate native execution capture.
The later Tool tabs slice records cancellation exclusion and truthful counts
without copying the baseline frontend's omitted-row double counting.

The native `Backend` now also exposes bounded profile-local history/saved-query
reads and writes. Calls use the owned control lane and shutdown fence without
hydrating a database connection. Pages contain at most 200 records and 8 MiB of
encoded response, with stable recency/ID cursors (favorites first for saved
queries). Unicode search scans at most 2,000 rows per call; an empty page may
carry a continuation. New SQL/body text is capped at 1 MiB, metadata at 8 KiB,
IDs at 256 bytes, and legacy stored rows over 2 MiB fail explicitly without
being rewritten. History append/trim is atomic; saved upsert returns the exact
committed record and preserves creation time. Storage errors do not expose SQL.
Six facade tests and isolated all-target Clippy pass, including worst-case
JSON escaping in continuation cursors. Final evidence is in
`../028/table-source-checks/query-library-final-tests.txt` and
`../028/table-source-checks/query-library-final-clippy.txt`; the earlier five-test
logs remain alongside this file. Native execution
history capture and the selected Tool tabs UI are now implemented in a newer
source variant, described below; the earlier six-test artifacts do not cover it.

T05/T06: catalog, object description, drop-impact, typed DDL preview and apply
now have a host-neutral `postgres::object_service`. The existing command inner
shims and wire layouts remain. Five existing tests now run without Tauri; a
sixth checks partial-prefix auditing/activity while preserving the returned
lock-timeout residue. Execution groups, regenerated DDL, refusal semantics,
applied-prefix disclosure and success-only/partial-success auditing remain.

T03: `backend::explain` parses a bounded JSON plan into flat preorder nodes for
native virtual rows. Timing/estimate calculations and insights follow the
baseline `plan-analysis.ts` (source SHA256
`b21c84bf55f662b5b92bd4ea72f9715d4edd5cf3105ba1809e18c51f18c2dd56`).
It retains exact JSON for inspection, costs, actual timing/rows/loops, buffers,
relationships, hottest node and typed insights. It refuses incomplete input,
more than 1 MiB, more than 4,096 nodes, depth above 32, malformed nodes and
overflowing metrics. Missing child timings retain the baseline inclusive-time
fallback, explicitly identified as derived attribution. Three focused tests
cover the baseline calculations and refusal boundaries. This is a parser/model,
not a new SQL execution route. EXPLAIN ANALYZE must still use query policy and
confirmation, and the caller must report truncation/failure honestly.

## Verification

- `pnpm format`, `pnpm lint`, `pnpm typecheck`: pass.
- `pnpm test`: 130 files, 1,488 tests pass.
- `just fmt`, `just lint`: pass.
- `RUST_TEST_THREADS=1 just test`: core 675 passed/71 ignored; Tauri 692
  passed/85 ignored. Ignored fixture tests were not run by these commands.
- Updated isolated/native backend checks pass; detailed counts and compile-fail
  documentation tests are in `native-backend.txt`.
- Plan 028's integrated native debug/release check/build and dependency proof
  pass. The log is `../028/native-check.txt`; newer backend preparation is also
  checked separately. Native model preparation added afterward needs its own
  focused check before activation.
- Python native tooling tests: 23 pass. New window harnesses were compiled and
  syntax checked; real-window execution remains pending.
- Custom-protocol build passed; see `custom-protocol.txt`.

## Before activation

History/saved-query Tool tabs now use one bounded, joined worker per document,
shared 16 MiB delivery admission and the workspace 128 MiB retained-page budget.
They replace pages, honor continuation on empty filtered scans, filter by
connection/search/outcome, and open exact SQL as durable drafts without running
it. Saved edits retain their saved ID and atomically preserve favorite/owner/
creation metadata. Execution capture excludes cancelled queries and uses
authoritative result-set completions without adding terminal omission counts;
incomplete sets leave counts unknown. Error text over 8 KiB explicitly reports
history-only truncation. Stored history acknowledgements join before backend
shutdown. New source checks are distinct from prior facade-only checks.

T04 has a bounded formatter and native retained-selection copy actions for TSV,
CSV, JSON, INSERT, Markdown, HTML and TXT. Display column projections borrow
source rows, so hidden/reordered columns cannot change copied source identity.
Input/output are capped at 8 MiB; partial copies remain disclosed. The pure
formatter supports UTF-16LE as well as UTF-8, and the [later file/impact slice](./export-impact-source-checks/README.md) adds
file publication/settings, gzip and bounded XLSX. Saved export tasks and
whole-table routing remain pending. Copy actions still require
actual-window acceptance.
The old object service still uses the existing process-global SQLx pools and a
detached DDL socket. Native pool/socket ownership, admission fences and explicit
cancellation/uncertain-COMMIT handling are prerequisites to exposing it through
the native facade. The service extraction does not solve those lifecycle gaps.

The newer EXPLAIN implementation opens durable drafts through Query Session,
binds plans to exact executed SQL and complete output, and adds native virtual
tree/detail/JSON/source controls under shared retention admission. Backend parser
tests now also reject present nonnumeric metrics. A read-only live probe passes
EXPLAIN and ANALYZE execution plus partial-output refusal. Window acceptance
remains pending.

Owned catalog reads now have DataDocument admission, cancellation and a dedicated
joined socket, read-only snapshot, deadline and payload limits. Their live probe
passes kind/identity/retirement/cleanup checks. A later native Objects Tool tab
shares this data worker, opens relations on its bound connection, and retains full
overload identities independently of clipped labels. Eight-kind descriptions now
use the same admitted dedicated socket and expose exact read-only JSON/definition
text; their live probe passes. The [later metadata/FK slice](./metadata-fk-source-checks/README.md) adds the
remaining table/foreign-table/type/domain kinds, guarded component streaming and
explicit reconstruction omissions. Exact live probes cover those descriptions
and composite FK navigation, with verified fixture cleanup. Cluster
administration is now available through the [read-only Administration slice](./admin-source-checks/README.md), with manual refresh, owned cancellation, bounded captures and scoped live checks. Backend control/maintenance and native-window acceptance remain absent. The later file/impact slice adds an owned bounded read-only downstream drop
impact facade and native viewer. DDL from the old pooled service remains inactive;
its lifecycle gaps remain. The remaining T01–T15 families,
real-window tests, keyboard/AX and real IME acceptance are still open.
VoiceOver is deferred and non-blocking under the
[2026-10-03 scope decision](../027/accessibility-scope-20261003.md). Complete PostgreSQL
parity is not yet achieved.

## SQL completion increment

The [completion provider and owned column reader](./completion-source-checks/README.md)
now share the existing query data worker. Metadata is connection-bound and
bounded; exact catalog names are quoted on insertion. Deferred editor menu tasks
are fenced during disconnect/retarget by a read-only reset barrier. Focused
source tests and a read-only table/view live probe pass. Release-window behavior,
keyboard/AX and real IME remain open. The [SQL layout formatter](./formatter-source-checks/README.md) adds a conservative gap-only command with no new dependencies; keyword normalization and complete formatter/window acceptance remain open.
