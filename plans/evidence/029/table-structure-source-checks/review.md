# Structure source and live review

The first exact owned metadata probe failed `Catalog(InvalidResponse)` at its
initial capture. A minimal built-in-catalog read through the owned fixture returned
`{"relation_oid":"1259"}` for `to_jsonb` of an `oid`. The typed contract expects
numeric u32 identities. Exported OIDs now cast to bigint before JSON encoding;
join parameters and catalog function arguments retain PostgreSQL OID types.
This applies to optional owning-constraint and parent-trigger IDs too.

The next probe reached metadata assertions and exposed an incorrect test
expectation: `COMMENT ... IS ''` removes the comment. The fixture now separately
checks a Unicode comment and an absent comment. Pure DTO/presentation tests still
preserve NULL versus empty text when supplied. No production behavior was changed
for that test correction.

The final exact probe passed ordered composite FK pairs, dropped attribute-number
gaps, identity/generated columns, expression and INCLUDE index positions, trigger
UPDATE OF columns, RLS/policies, explicit relation grants, rules, partition bounds,
ordinary inheritance, expected-identity refusal and retired-document refusal.
Each attempt joined backend shutdown before captured OID/owner/marker-guarded
RESTRICT cleanup. Independent checks find all three schemas absent and both
fixtures at zero activity. No temporary debug instrumentation was added.

Structure requires PostgreSQL 13+, matching the baseline Structure reader's
explicit tgparentid assumption (`postgres/schema.rs`). Other connection/catalog
features retain their own supported versions. A version guard runs before
version-specific SQL; older-server execution is not live verified. Only SQLSTATE
42501 maps to permission refusal; actual restricted-catalog execution is not
claimed from an owner-role probe. Relation ACL inspection excludes column ACLs
and effective authorization. Children are direct partition/inheritance relatives,
not a transitive dependency list. Definitions are metadata, not DDL authority.

Independent native review found two pre-package issues. Disconnect/invalidation
now drops a deferred incoming capture, and currency is assigned on accepted
response rather than manufactured during rendering. Starting another read also
retires deferred incoming data. A later reconnect cannot make that capture current.

The initial eight-text-copy allowance missed newline-heavy accessibility runs.
Structure details now disable soft wrapping (including its editor toggle action)
and admission includes UTF-8 bytes and logical line-boundary overhead for the
selected editor, cached/published AX runs and replacement overlap. Oversized
captures visibly refuse before detail allocation; exact text is not truncated.
A focused newline-heavy admission test covers this path. This remains a payload
allowance, not a total process-memory guarantee.
