# Query-result mutation source and native integration, 2026-10-03

Status: partial implementation. Required checks, native debug/release checks,
separate package and dependency proof pass. No actual-window, keyboard/AX, IME or live query-edit acceptance is
recorded for this increment. VoiceOver remains deferred, not passed.

## Execution source and recovery

The native query document retains the exact submitted SQL and planner-derived
single-statement SQL independently of the mutable editor and terminal ACK. Named
parameter names and rewrite order are retained; bound values are not. Successful
completion must match the connection, session and execution before this source can
back result edits. Failed/cancelled results, clipped cells, incomplete metadata and
unrecognized truncation reasons refuse editing. Retained first-result rows share
the grid's original strings; captures drop before the grid's reservation releases.

Query UPDATEs use the existing Bottom review, original keys/guards, include/remove,
Safe Mode confirmation and exact persisted-revision Apply barrier. Cancellation
before dispatch invalidates late save acknowledgments. Unknown outcomes retain
intent and require explicit reconciliation. Apply commits on the separate mutation
connection and never reruns SQL; successful application invalidates displayed rows.
Rerun, retarget, clear and close cannot silently discard staged intent. The data
worker uses the document's existing joined ownership and bounded delivery budget.

Workspace version 3 writes an optional query mutation journal with immutable
source and UPDATE-only intent, including multiple origin tables. Versions 1 and 2
remain readable; their records cannot contain query journals. Future/corrupt data
is preserved. Restored intent requires fresh analysis and review. Borrowed source
and draft measurements precede workspace snapshot copies; the 448 KiB save limit
and exact-save barrier apply to query changes too. The editor SQL may differ from
the saved executed source.

## Explicit remaining restrictions

This does **not** complete query-result parity. The query stream currently carries
headings and text cells, without original execution relation/type OIDs or rendering
context. Analysis runs on a separate connection. To avoid claiming that those
contexts are equivalent, the native path currently requires:

- A supported single SELECT with every FROM/JOIN target schema-qualified and no
  temporary schema. Unqualified names, unsupported CTE/subquery/function sources
  and ambiguous self-joins are refused.
- Exact builtin projected type OIDs: bool, bytea, int2/int4/int8, text, varchar,
  bpchar, numeric, uuid, tid, oid and jsonb. Actual catalog column OIDs are checked
  too, because row descriptions may flatten domains. Dates, times, intervals,
  money, floats, JSON, arrays, domains and custom types remain unsupported here.
  Ordinary table and Tauri Statement analysis are unchanged.
- Projected non-NULL identities. Query ctid/tableoid may use their projected
  values; table browsing keeps its hidden-identity path.
- ASCII captured identity/guard literals until execution encoding is proven.
  New replacement values may contain Unicode; uncertain recovered guard literals
  remain retained but cannot be reviewed. Use a table document for these edits.

The origin/context restrictions need replacement with actual execution metadata
before full query-edit parity can be accepted. Concurrent drop/recreate between
execution and analysis also lacks an original execution OID fence. This increment
must not be presented as proving that catalog race is solved.

First use of Edit selected cell starts analysis. Choose it again after analysis to
open the selected cell, so an asynchronous reply does not reuse a stale selection.
Native focus, keyboard, AX, composition, live conflict, cancellation and recovery
scenarios remain required. Existing native table acceptance does not transfer.

## Bounds and verification

Provenance accepts at most 1 MiB SQL, 16,384 lexer tokens and 256 parameter names.
The bounded lexer preflight happens before planning/classification. Native source
creation admits 16 MiB working allowance; retained accounting includes parameter
headers and a queued analysis statement copy. Immutable validated source is not
reparsed for each analysis. Review copies admit working allowance before cloning,
and both the retained and queued plans are charged. Existing shared 128 MiB
retention, 16 MiB delivery, 128 changes/4 MiB draft, 16 documents and 448 KiB
workspace limits remain. These are not process RSS claims.

Focused coverage includes execution identity, parameter rewrite, shared row
ownership, clipped/failed result refusal, budget refusal/release, joined origin
updates, projected ctid guards, immutable recovery, ASCII guard refusal, exact
journal measurement, bounded lexing, qualified targets and native OID refusal.

- `pnpm format`, `pnpm lint`, `pnpm typecheck` and diff checks pass.
- `just fmt`, `just lint` and serialized `just test` pass: core 677 passed/71
  ignored; Tauri 694 passed/85 ignored.
- Isolated backend Clippy/tests pass: 813 passed/80 ignored and two doc tests.
  Tauri facade selection passes 81 tests/9 ignored. Custom-protocol build passes.
- Native format, debug/release all-target Clippy, fixture-harness Clippy, debug
  build and debug/release tests pass: 157 passed/13 ignored in each test run.
- The final parameter-header accounting correction and its test were checked
  again. Initial missing-field/scroll-container compilation errors and Clippy's
  unused ownership field/manual-slice-size warnings are preserved in separate
  logs; they were corrected, not counted as passes.
- Separate package `/private/tmp/dbunk-native-package-20261003-query-edits/dbunk Native Preflight.app`
  builds. Executable SHA256
  `70d53224a07fd886ad6c10cebd961b1706abfff2647f90c0b3a37786c591458f`;
  bundle 122,590,940 bytes. Dependency proof and all 317 source hashes pass.
- The [owned window attempt](../query-edits-window-20261003/failed-discovery.json)
  used PID 54914 and a new intended profile
  `/private/tmp/dbunk-native-query-edits-20261003-review`, with verified stage03
  fixture UUID `2283820d-33ec-4c4c-ae03-7051092bd410`. CUA listed the app running,
  but both path and bundle-ID lookups returned `cgWindowNotFound`. A process sample
  records its event loop; no feature interaction, keyboard/AX/IME or live apply
  pass follows from it. The owned PID was stopped with SIGTERM after checking
  executable/profile arguments and SHA256. This is not a normal-quit pass.
  Fixture backend count returned zero. Daily-driver profiles and the frozen
  earlier packages were not used.
