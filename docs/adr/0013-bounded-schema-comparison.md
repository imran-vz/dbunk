# Bounded, versioned schema comparison

**Status:** Accepted (Plans 021 and 022, rewritten 2026-10-11)

Schema comparison is deliberately narrow and versioned
(`postgres16OrdinaryTableProjectionV1`, `backend/src/postgres/schema_compare/`).
Each side is captured in a repeatable-read snapshot after locking the tables
in scope. Expressions compare equal only inside a small scalar grammar;
anything outside it is reported as "not comparable", never as equal. Captured
values and result pages run under fixed memory budgets and are read through
leased readers.

## Considered options

- **Diffing generated `CREATE` statements.** Rejected as unsound.

## Consequences

- A schema name that fails to resolve is an error, not an empty schema.
- Wider object coverage, migration SQL and data comparison are separate later
  work.
