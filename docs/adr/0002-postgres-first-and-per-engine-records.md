# PostgreSQL first; engines as per-engine records and workspaces

**Status:** Accepted (2026-05-10, rewritten 2026-10-11)

PostgreSQL is the reference engine. A new feature may ship for PostgreSQL
only; other engines refuse with a clear "not yet implemented (PostgreSQL
only)" error (`backend/src/dispatch/relational.rs`) and catch up one feature
at a time, gated by capability flags such as `StructureCapabilities`
(`backend/src/types.rs`), not engine-name checks. This avoids building every
feature four times before it is useful once.

Engines are modelled per engine, not by a shared shape:

- **Stored records** are one tagged enum, `StoredConnection`
  (`#[serde(tag = "engine")]`, `backend/src/types.rs`), so reading a field that
  belongs to another engine is a compile error. PostgreSQL and MySQL stay
  separate variants even where their fields match.
- **Storage class** (`Relational` or `KeyValue`) is derived from the engine,
  never stored. The backend routes by class first (`backend/src/dispatch.rs`),
  and a relational-only operation on a key-value engine fails in one place.
- **Native workspaces** are per engine: PostgreSQL documents plus one
  `EngineSurface` variant each for SQLite, MySQL, ClickHouse and Redis
  (`apps/native/src/workspace_engines.rs`), each owning its own session, tree
  and tabs.
- **Engine is fixed at creation.** Editing a connection cannot change its
  engine; changing engine means a new record.

## Considered options

- **A shared `Engine` trait now.** Rejected: only acceptable once two or more
  engines implement the same operations the same way. A trait for one feature
  is not.
- **A two-level enum (class, then engine).** Rejected: routing needs the class,
  but records carry per-engine fields, and a nested enum ripples through every
  serialized type and `match` site.
