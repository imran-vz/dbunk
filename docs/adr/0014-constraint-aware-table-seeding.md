# Backend-owned, constraint-aware table seeding

**Status:** Accepted (2026-06-11, rewritten 2026-10-11)

The backend generates seed rows from the table's catalog
(`backend/src/seed.rs`, `backend/src/backend/table_seed/`): NOT NULL, foreign
keys and uniqueness are guaranteed, CHECK constraints are attempted, and a run
is all-or-nothing. A seed fails fast when a referenced parent table is empty.
The native app seeds PostgreSQL; other engines' generators remain in
`backend/src/dispatch/seed.rs`.

## Considered options

- **Parsing CHECK expressions.** Rejected.
- **Cascading seeds into parent tables.** Rejected.
