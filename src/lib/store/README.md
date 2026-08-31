# Workspace Store — Slice Architecture

The Zustand store at `src/lib/store/` is composed of domain-concept slices
behind a single `useAppStore` hook. This document is the contract every slice
file commits to.

If you're adding a new action: pick the slice that owns the
relevant entity (e.g. a new Pub/Sub Session action goes in
`keyvalue-pubsub.ts`). If you're adding new state: ditto.

## Slices

| Slice file              | Owns                                                                                                     | Class      |
| ----------------------- | -------------------------------------------------------------------------------------------------------- | ---------- |
| `connections.ts`        | Connection records, Active Connection ID, health checks                                                  | shared     |
| `workspace-tabs.ts`     | Workspace Tab list, active tab ID, sidebar/theme/UI flags                                                | shared     |
| `credentials.ts`        | App Settings, Credential Storage Mode lifecycle                                                          | shared     |
| `relational-tables.ts`  | Schema Explorer, Table Structure, Cell Edits, DDL, Database Overview, Table Data                         | relational |
| `pg-objects.ts`         | PostgreSQL Object Catalog, Object Viewer descriptions, and connection-generation guards                  | relational |
| `relational-queries.ts` | Query History, Saved Queries, query editor + run state                                                   | relational |
| `mutation-drafts.ts`    | Identity-keyed staged result mutations, review and apply lifecycle                                       | relational |
| `query-sessions.ts`     | Persistent PostgreSQL session lifecycle, streamed results, transactions (`applyQueryTransactionCommand`) | relational |
| `table-browse.ts`       | PostgreSQL Table Browse grid state keyed by Workspace Tab id                                             | relational |
| `keyvalue-workspace.ts` | (placeholder) Keyspace Browser + Key Inspector client cache                                              | keyvalue   |
| `keyvalue-pubsub.ts`    | (placeholder) Pub/Sub Session client metadata                                                            | keyvalue   |

The slice file names match the **domain glossary** entries from
`CONTEXT.md`. Searching for "where does Cell Edit live?" lands you
in `relational-tables.ts`. Searching for "where do Pub/Sub Sessions
live?" lands you in `keyvalue-pubsub.ts`.

## Composition

Each slice exports two things:

- A `SliceShape` TypeScript type describing the state and actions
  it contributes.
- A `createXSlice: StateCreator<AppStoreState, [], [], SliceShape>`
  factory that returns its initial state + actions.

`store/index.ts` composes them into one Zustand store:

```ts
export const useAppStore = create<AppStoreState>()((set, get, store) => ({
  ...createCredentialsSlice(set, get, store),
  ...createConnectionsSlice(set, get, store),
  // ...
}));
```

`AppStoreState` (in `store/types.ts`) is the full canonical store shape. Its
explicit slice intersection documents the entire surface and lets every slice
reach across via `get()` without narrowing ceremony.

## Cross-slice access — the entity-owner pattern

Every cross-cutting action lives in the slice that owns the
**primary entity** the action is named for. The canonical example
is `deleteConnection`:

- Lives in `connections.ts`.
- Cascades cleanup into other slices by calling cleanup methods on
  those slices via `get()`.

Each downstream slice exposes a named cleanup method:

| Slice                   | Cleanup method                                                                                                                                                  | What it does                                                                                                                                    |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| `workspace-tabs.ts`     | `closeTabsForConnection(id)`                                                                                                                                    | Drops every Workspace Tab pointing at that connection.                                                                                          |
| `query-sessions.ts`     | `closeQuerySessionForTab(id)`, `closeQuerySessionsForConnection(id)`                                                                                            | Closes persistent PostgreSQL editor sessions before tab, connection, or credential teardown.                                                    |
| `table-browse.ts`       | `closeTableBrowseForTab(id)`, `closeTableBrowsesForConnection(id)`                                                                                              | Closes PostgreSQL Table Browse executors before tab, connection, or credential teardown.                                                        |
| `relational-tables.ts`  | `dropRelationalCachesForConnection(id)`                                                                                                                         | Drops every per-connection cache entry (schema explorer, table structure, table data, overview stats, etc.).                                    |
| `pg-objects.ts`         | `dropPgObjectCachesForConnection(id)`                                                                                                                           | Invalidates in-flight catalog/description reads and drops cached PostgreSQL object metadata.                                                    |
| `relational-queries.ts` | `dropOpenQueryStateForConnection(id)`                                                                                                                           | Drops open-tab query status, edits, and previews without removing query history.                                                                |
| `relational-queries.ts` | `dropQueryStateForConnection(id)`                                                                                                                               | Drops query history rows pinned to the connection plus query status/edits for its open tabs.                                                    |
| `mutation-drafts.ts`    | `dropMutationDraftForScope(scope)`, `dropMutationDraftsForTab(id)`, `dropMutationDraftsForExecution(tabId, executionId)`, `dropMutationDraftsForConnection(id)` | Drops staged mutations at explicit lifecycle boundaries. Result-row budget release intentionally calls none of these because drafts survive it. |
| `keyvalue-workspace.ts` | `closeKeyTabsForConnection(id)`                                                                                                                                 | No-op today; reserved for future per-key cache.                                                                                                 |
| `keyvalue-pubsub.ts`    | `closePubSubSessionsForConnection(id)`                                                                                                                          | No-op today; reserved for future pub/sub auto-reconnect state.                                                                                  |

`deleteConnection` and `disconnectConnection` both use the canonical
`teardownConnectionWorkspace` coordinator in `connections.ts`. The owner marks
the connection disconnected, advances its lifetime epoch, and invalidates
PostgreSQL metadata before any await. It then closes query sessions, performs
the backend teardown, and drops the remaining slice-owned caches and tabs.
`closeTabsForConnection` owns tabs only; it does not trigger sibling cleanup
itself.

Naming convention for cleanup methods: **`<verb><Noun>ForConnection(id)`**.
Examples: `closeTabsForConnection`, `dropRelationalCachesForConnection`.
The grammatical shape makes the cascade list in `deleteConnection`
read as a sequence of "for this connection, clean up X."

## Slice isolation discipline

Slice files **must not import each other directly**. Cross-slice
access happens through `get()` (typed against `AppStoreState`)
inside actions. Only `store/index.ts` is allowed to import every
slice — that's where they're composed.

A CI grep check (`pnpm run check:slice-isolation` in `package.json`)
enforces this — any slice file importing from another slice file
fails the check. The check is intentionally a one-line grep rather
than a custom Biome rule because the maintenance cost is lower at
our scale.

If you genuinely need to share helpers between slices, lift them
out: `src/lib/format.ts`, `src/lib/tauri.ts`, etc. The store
directory is for slice-local code only.

## Adding a new slice

1. Create `src/lib/store/<concept>.ts`.
2. Export a `<Concept>Slice` type and a `create<Concept>Slice` factory.
3. Add the slice's fields to `AppStoreState` in `store/types.ts`.
4. Spread the factory into `useAppStore` in `store/index.ts`.
5. If the slice owns state keyed by connection, expose a
   `<verb><Noun>ForConnection(id)` cleanup method even if it's
   currently a no-op — keeps the cascade contract explicit.
6. Update the CI grep check in `package.json` to forbid imports of
   the new slice from siblings.
7. Update this README's slice table.
