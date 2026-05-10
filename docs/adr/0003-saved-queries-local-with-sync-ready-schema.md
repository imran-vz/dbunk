# ADR-0003 — Saved queries are local-only, with a sync-ready schema

**Status**: Accepted (2026-05-10)

## Context

The Saved Queries panel (DESIGN.md §5.3) lets users name and reopen SQL
snippets. Three storage options were on the table:

1. **Local-only, minimal schema** — `{ id, name, body, connectionId,
   isFavorite }`. Smallest change, but a future cloud-sync feature would
   require a schema migration.
2. **Local-only, sync-ready schema** — adds `ownerId`, `createdAt`,
   `updatedAt`. No sync today; future sync layer slots in without migrating
   existing files.
3. **Cloud-sync from day one** — Supabase or custom backend. Multi-week
   scope, blocks every other Group A item.

## Decision

Option 2. Saved queries persist in `~/.config/dbunk/saved_queries.json`,
managed by the `load_saved_queries` / `save_saved_query` /
`delete_saved_query` Tauri commands. The schema includes `ownerId`
(reserved, always `null` today), `createdAt`, and `updatedAt` so a future
sync transport can compare clocks without any file format change.

## Consequences

- Saved queries work offline forever, even if cloud sync never lands.
- The `ownerId` and timestamp fields look unused today — they're
  intentional, not dead code.
- A future cloud-sync ADR would build on this one; it should not require
  rewriting the on-disk shape.
