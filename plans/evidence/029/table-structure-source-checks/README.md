# Native table Structure verification, 2026-10-03

Status: implementation and verification in progress. The bounded backend facade
passes nine focused tests and the corrected owned live probe. The first release
package has scoped actual-window evidence; two observed presentation/navigation
issues were corrected and rechecked in a separately hashed release package. No full PostgreSQL
parity, structured table editing or complete keyboard/AX/IME pass is claimed.
VoiceOver stays deferred.

The table-context Structure action opens the connection-bound Objects Tool tab.
The inspector uses typed sections with virtualized rows and one exact selected
read-only editor: overview, columns, primary key, outbound/inbound foreign keys,
indexes, constraints, triggers, policies, explicit relation grants, rules and
direct parents/children. JSON output is not used as the inspection UI. Definitions
are metadata text. Existing object SQL reconstruction remains a separate action.

Reads use the existing document-owned, serialized catalog lane with joined
cancellation and bounded 16 MiB delivery. The service captures one read-only
repeatable-read transaction with exact database/relation OIDs, requiring PG13+.
It refuses incomplete captures beyond 4096 aggregate components, 1600 columns,
4 MiB encoded/actual typed storage, 63-byte identifiers, 8 KiB metadata and 1 MiB
individual definitions. Native admission reserves actual typed capacity plus
bounded selected-editor overlap in the shared 128 MiB retained allowance. These
are payload bounds, not process RSS claims.

A refresh checks the original OIDs. Read failures preserve the previous capture
and mark it stale; navigation is disabled until a fresh capture succeeds. Related
relations inherit this Tool tab's connection and receive an expected-OID metadata
recheck before opening a fresh table read. Reconnection does not replay a prior
capture. A table-context request may await its own initial connection; cancelling
that request prevents late connection completion from dispatching it.

See [source/live findings](./review.md), [initial failure](./live-initial.txt),
[fixture expectation correction](./live-oid-fix.txt), [passing probe](./live-final.txt)
and [independent teardown](./live-teardown.json). Version refusal is source-tested;
only the current owned server was live-tested. Permission failure is classified
without collapsing it to empty metadata, but an owner-role probe is not evidence
of restricted-catalog behavior. Relation ACLs exclude effective authorization and
column ACLs. Direct children include both partitions and ordinary inheritance;
this is not a transitive dependency graph. Table DDL editing remains open.

Required pnpm format/lint/typecheck, just fmt/lint/serialized test pass. Backend
configurations pass 677/71 ignored and 694/85 ignored. Isolated all-target Clippy
passes; isolated tests pass 1,031 with 92 ignored and two doctests. Ignored probes
are not passes; the named one-test live probe above was separately opted in.
Native compilation initially found a test-only accounting accessor, now gated to
tests. Final initial native debug/release tests passed 312 with 13 ignored, with
Clippy and builds passing. The first package matched 578 source hashes. See
[window findings and correction scope](./window-corrections/README.md).
