# Workspace Store Slicing — Phased Refactor Plan

The Zustand store at `src/lib/store.ts` has grown to 2,551 lines / 47 action
methods covering ~7 distinct concerns interleaved with no internal grouping.
Adding any new action — Redis Pub/Sub auto-reconnect, sidebar DB switcher,
saved CLI commands — requires reading the whole file to know where it fits.
The interface (one `useAppStore` hook) is correctly cohesive; the
implementation has no internal seams.

This plan refactors the store into 7 domain-concept slices behind the same
outward interface. Pure implementation refactor — no behaviour change, no
consumer touch, no new actions.

Anchored on the grilling decisions:
- **Slicing axis** — hybrid (domain concepts; class-bundled where the concept
  is class-bound). 7 slices.
- **Composition** — Zustand slice pattern. One `useAppStore`, flat state,
  cross-slice access via `get()`.
- **Cross-slice ownership** — entity-owner pattern. Downstream slices expose
  named cleanup methods (`closeTabsForConnection`, etc.); the owning slice
  orchestrates the cascade.
- **Migration** — big-bang single PR, ~11 commits, tests green at every
  commit boundary.

No new ADR. The decisions are how-to choices about an accepted refactor,
documented in `store/README.md` (commit 11).

---

## Final layout

```
src/lib/store/
├── index.ts              ← AppStoreState type, useAppStore create()
├── types.ts              ← shared DTOs (Connection, WorkspaceTab, …)
├── README.md             ← slice taxonomy, cleanup convention
├── connections.ts        ← Connections + hydrate/toStored/applyUpdate
├── workspace-tabs.ts     ← Workspace Tabs + workspace UI (sidebar, theme)
├── credentials.ts        ← Credentials + App Settings
├── relational-tables.ts  ← Schema Explorer/Relationships, Table Structure,
│                           Cell Edits, DDL, Database Overview, Table Data
├── relational-queries.ts ← Query History, Saved Queries, editor buffers
├── keyvalue-workspace.ts ← Keyspace Browser/Key Inspector cache (placeholder)
└── keyvalue-pubsub.ts    ← Pub/Sub Session ring buffer (placeholder)

src/lib/format.ts         ← formatLatencyMs (promoted from store)
src/lib/tauri.ts          ← errorToMessage joins tauriInvoke (promoted)
```

`src/lib/store.ts` itself is deleted; existing `import … from "@/lib/store"`
resolves via the directory's `index.ts` barrel.

---

## Slice contracts

Each slice file exports a `StateCreator<AppStoreState, [], [], SliceShape>`
factory plus its `SliceShape` type. The root `index.ts` composes them via
spread. Cross-slice reads use `get()`; cross-slice writes happen through
cleanup methods named `<verb><Noun>ForConnection(id)` — the connection ID is
the only cascade dimension today (delete-a-connection is the canonical
cascade trigger).

### `connections.ts` — `ConnectionsSlice`

Owns: `connections`, `activeConnectionId`.
Actions: `loadConnections`, `addConnection`, `updateConnection`,
`deleteConnection` (orchestrator), `connectConnection`, `testConnection`,
`runHealthChecks`, `setActiveConnectionId`.
Helpers (private): `hydrateConnection`, `toStoredConnection`,
`applyConnectionUpdate`.
Cross-slice cascade: `deleteConnection` calls in order:
- `get().closeTabsForConnection(id)`
- `get().dropRelationalCachesForConnection(id)`
- `get().dropQueryStateForConnection(id)`
- `get().closeKeyTabsForConnection(id)`
- `get().closePubSubSessionsForConnection(id)`

### `workspace-tabs.ts` — `WorkspaceTabsSlice`

Owns: `workspaceTabs`, `activeTabId`, `activeView`, `isLeftSidebarOpen`,
`editorTheme`, `selectedRowIndex`.
Actions: `setActiveView`, `setActiveTabId`, `setWorkspaceTabs`,
`toggleLeftSidebar`, `setEditorTheme`, `setSelectedRowIndex`,
`openWorkspaceTab`, `openTableTab`, `openQueryForTable`, `openViewTab`,
`createNewQueryTab`, `createNewTableTab`, `closeTab`, `reopenHistoryEntry`.
Cleanup: `closeTabsForConnection(id)` — removes every tab for that
connection ID and updates `activeTabId` if affected.

### `credentials.ts` — `CredentialsSlice`

Owns: `appSettings`, `appSettingsStatus`, `credentialStorageStatus`.
Actions: `loadAppSettings`, `configureCredentialStorage`, `unlockCredentials`,
`changeCredentialStorage`, `resetCredentialStorage`.
No cleanup methods — credentials are app-wide, not per-connection.

### `relational-tables.ts` — `RelationalTablesSlice`

Owns: `schemaExplorer`, `schemaRelationships`, `schemaRelationshipsStatus`,
`tableStructure`, `tableStructureStatus`, `tableData`, `tableLoadStatus`,
`tablePreviews`, `tableEdits`, `tableEditsCommitStatus`,
`pendingStructureChanges`, `structureCommitStatus`, `databaseOverviewStats`,
`databaseOverviewStatsStatus`, `expandedSchemas`.
Actions: `loadTablePreview`, `loadTableData`, `refreshTableData`,
`loadTableStructure`, `loadSchemaRelationships`, `loadDatabaseOverviewStats`,
`toggleSchema`, `setExpandedSchemas`, `focusTableInSchemaMap`,
`addPendingStructureChange`, `removePendingStructureChange`,
`clearPendingStructureChanges`, `commitStructureChanges`, `setTableEdit`,
`discardTableEdits`, `commitTableEdits`, `addTableRow`,
`deleteSelectedTableRows`, `clearTableEditsCommitStatus`.
Helpers (private): `outcomeToTableEditsStatus`.
Cleanup: `dropRelationalCachesForConnection(id)` — drops every entry from
the per-connection-keyed maps (schemaExplorer, tableStructure, tableData,
etc.).

### `relational-queries.ts` — `RelationalQueriesSlice`

Owns: `queryHistory`, `savedQueries`, `savedQueriesStatus`, `queryEdits`,
`queryStatus`, `queryPreviews`.
Actions: `loadQueryHistory`, `loadSavedQueries`, `saveSavedQuery`,
`deleteSavedQuery`, `updateQuery`, `runQuery`, `setQueryEdit`,
`discardQueryEdits`.
Helpers (private): `generateHistoryId`.
Cleanup: `dropQueryStateForConnection(id)` — drops query-status entries for
that connection's open tabs and removes query-history rows pinned to it.

### `keyvalue-workspace.ts` — `KeyValueWorkspaceSlice`

Owns: nothing today (Key Inspector state lives in component state).
Placeholder slice for future Keyspace Browser cache + watched-keys.
Cleanup: `closeKeyTabsForConnection(id)` — called by the cascade; today
this is a no-op because `workspace-tabs.closeTabsForConnection` already
handles the `key` tabs (singletons + multis live in the same tab list).
Reason for separate cleanup method: makes the cascade contract explicit
even when the cleanup is currently empty.

### `keyvalue-pubsub.ts` — `KeyValuePubSubSlice`

Owns: nothing today (Pub/Sub Session state lives in component state with
the backend holding the ring buffer).
Placeholder slice for future client-side pubsub session metadata cache.
Cleanup: `closePubSubSessionsForConnection(id)` — calls into the
backend's `redis_pubsub_close` for any sessions tied to the connection.
Today the cleanup is a no-op (no session state to clean); future Pub/Sub
auto-reconnect would populate it.

---

## Commit plan

Each commit is independently testable. `bun typecheck && bun lint &&
bun run test && cargo test --lib` passes at every commit boundary. No
behaviour change in any commit.

### Commit 1 — Extract types to `store/types.ts`

Move the type definitions out of `store.ts` into a new `store/types.ts`:
`DatabaseEngine`, `StorageClass`, `RedisModuleInfo`, `RedisCapabilities`,
`CredentialStorageMode`, `CredentialState`, `AppSettingsSnapshot`,
`AppSettingsStatus`, `StoredConnection`, `Connection`, `QueryStatus`,
`TableLoadStatus`, `TableDataState`, `ColumnInfo`, `ForeignKeyInfo`,
`IndexInfo`, `ConstraintInfo`, `StructureCapabilities`, `TableStructure`,
`TableStructureStatus`, `StructureCommitStatus`, `TableEditsCommitStatus`,
`SchemaRelationshipsStatus`, `DatabaseOverviewStats`,
`DatabaseOverviewStatsStatus`, `SchemaExplorer`, `SavedQuery`,
`SavedQueriesStatus`, `QueryHistoryEntry`, `WorkspaceTabKind`,
`WorkspaceTab`, `TablePreviewData`, `QueryPreviewData`.

Also move the key helpers used cross-slice: `tableDataKey`,
`tableStructureKey`.

The original `store.ts` re-exports every moved type so external imports
(`import { Connection } from "@/lib/store"`) stay working.

**Files**: `src/lib/store.ts` (shrinks), `src/lib/store/types.ts` (new).

### Commit 2 — Promote `formatLatencyMs` + `errorToMessage` helpers

- `formatLatencyMs` → new `src/lib/format.ts`.
- `errorToMessage` → `src/lib/tauri.ts` (existing).
- `store.ts` re-exports both for backwards compatibility (no consumer touch
  yet).

**Files**: `src/lib/store.ts`, `src/lib/format.ts` (new), `src/lib/tauri.ts`.

### Commit 3 — Add slice scaffolding

Create the 7 empty slice files and the root `store/index.ts`. Each slice
file exports its `SliceShape` type + a `createXSlice` `StateCreator`
factory that returns `{}` (empty). `store/index.ts` defines
`AppStoreState` as the intersection of all slice shapes and calls
`create<AppStoreState>()((...args) => ({ ...createX(...args), … }))`.

Crucially: the *existing* `useAppStore` in `store.ts` stays the source of
truth. `store/index.ts` re-exports `useAppStore` from `store.ts` — slices
exist but contribute nothing yet. This commit is pure scaffolding so the
file structure is in place before the moves begin.

**Files**: `src/lib/store/index.ts`, `src/lib/store/connections.ts`,
`src/lib/store/workspace-tabs.ts`, `src/lib/store/credentials.ts`,
`src/lib/store/relational-tables.ts`, `src/lib/store/relational-queries.ts`,
`src/lib/store/keyvalue-workspace.ts`, `src/lib/store/keyvalue-pubsub.ts`.

### Commit 4 — Move Credentials slice

Move `appSettings`, `appSettingsStatus`, `credentialStorageStatus` state +
the five credential actions into `credentials.ts`. Switch `store.ts`'s
`useAppStore` to spread `createCredentialsSlice(...)` in — those fields
are now sourced from the slice. The same `useAppStore` hook still owns the
other state (we're going slice-by-slice).

This is the smallest slice (~6 actions, ~150 lines). Lands first as the
proof point.

**Files**: `src/lib/store/credentials.ts`, `src/lib/store.ts`.

### Commit 5 — Move Connections slice with cleanup-method coordination

Move the Connection records, `activeConnectionId`, and the seven
connection-level actions. Add `hydrateConnection`, `toStoredConnection`,
`applyConnectionUpdate` as slice-local helpers.

Crucially: at this point the cascade cleanup methods (`closeTabsForConnection`,
etc.) don't yet exist on their target slices — they're still part of the
monolith. To keep the migration mechanical, `deleteConnection` calls
`get().closeTabsForConnection(id)` etc. *and* the matching methods are
moved-but-stubbed on the destination slices in the same commit so the
cascade contract holds.

**Files**: `src/lib/store/connections.ts`, `src/lib/store.ts`, plus stub
cleanup methods on the workspace-tabs / relational-tables / keyvalue slices
(empty bodies; the actual cleanup logic moves in later commits when its
home state moves).

### Commit 6 — Move Workspace Tabs slice

Move tab state + tab actions + `closeTabsForConnection` (implementation
replaces the stub from commit 5). Includes workspace UI state
(`activeView`, `isLeftSidebarOpen`, `editorTheme`, `selectedRowIndex`).

**Files**: `src/lib/store/workspace-tabs.ts`, `src/lib/store.ts`.

### Commit 7 — Move Relational Tables slice

The largest slice. Moves all table-data / structure / cell-edit / DDL /
schema-explorer / database-overview state + actions +
`outcomeToTableEditsStatus` helper + `dropRelationalCachesForConnection`
(replaces stub).

Also moves `expandedSchemas` state + `setExpandedSchemas` /
`toggleSchema`.

**Files**: `src/lib/store/relational-tables.ts`, `src/lib/store.ts`.

### Commit 8 — Move Relational Queries slice

Moves query history, saved queries, query editor state +
`dropQueryStateForConnection` (replaces stub) + `generateHistoryId`.

**Files**: `src/lib/store/relational-queries.ts`, `src/lib/store.ts`.

### Commit 9 — Move KeyValue Workspace + KeyValue PubSub slices

These slices own no state today but exist for cascade-contract clarity
and as the home for future keyvalue store needs (watched keys, pubsub
session cache). Both `closeKeyTabsForConnection` and
`closePubSubSessionsForConnection` cleanup methods land as documented
no-ops (or call the backend in pubsub's case).

**Files**: `src/lib/store/keyvalue-workspace.ts`,
`src/lib/store/keyvalue-pubsub.ts`, `src/lib/store.ts`.

### Commit 10 — Collapse `store.ts` husk into `store/index.ts`

By this point `store.ts` contains only the `useAppStore = create<...>()`
call and re-exports of types/helpers. Move the `create` call into
`store/index.ts` (which becomes the real entry point); delete `store.ts`.
External `import … from "@/lib/store"` resolves through the directory
barrel.

**Files**: deletion of `src/lib/store.ts`, expansion of
`src/lib/store/index.ts`.

### Commit 11 — Add `store/README.md` and CI slice-isolation grep

- `store/README.md`: slice taxonomy table, entity-owner pattern contract,
  cleanup-method naming convention, cross-slice access rule.
- `package.json`: new script `check:slice-isolation` — a grep that fails
  CI if any slice file imports from a sibling slice file (the only
  permitted importer is `index.ts`).

**Files**: `src/lib/store/README.md` (new), `package.json`.

---

## Verification per commit

At every commit boundary:
```
bun run format     # 0 fixes applied
bun lint           # 0 errors
bun typecheck      # 0 errors
bun run test       # 262/262 frontend tests pass
cargo test --lib   # 75/75 backend tests pass
```

If any boundary fails, the commit is wrong. Bisect to the offending move.

---

## Cross-slice import discipline

After the refactor, the dependency graph between slice files looks like:

```
connections.ts ──┐
                 ├─→ workspace-tabs.ts
                 ├─→ relational-tables.ts
                 ├─→ relational-queries.ts
                 ├─→ keyvalue-workspace.ts
                 └─→ keyvalue-pubsub.ts
```

Connections orchestrates cleanup; everyone else exposes a cleanup method.
**Slice files do not `import` each other directly.** Cross-slice access
happens through `get()` (typed against `AppStoreState` from `index.ts`)
inside actions. `store/index.ts` is the only file that imports all slices
(to compose them).

The CI grep check from commit 11 enforces this:

```bash
# package.json
"check:slice-isolation": "! grep -rn 'from \"\\./\\(connections\\|workspace-tabs\\|credentials\\|relational-tables\\|relational-queries\\|keyvalue-workspace\\|keyvalue-pubsub\\)\"' src/lib/store/ | grep -v 'src/lib/store/index.ts'"
```

(A future Biome plugin could enforce this structurally; the grep is the
pragmatic floor.)

---

## Out of scope

- **Consumer-side selector hooks** (e.g. `useConnections()` instead of
  `useAppStore(s => s.connections)`). Add later if there's demand;
  pure-render-optimisation work.
- **Tests for individual slices.** Existing component-integration tests
  cover the action surface. Slice-level unit tests are a nice-to-have that
  unlock from this refactor but aren't required to land it.
- **The other architectural-review candidates** — viewer state-machine
  duplication, Tauri command boilerplate, Redis INFO parsers, policy
  narrowing. Each gets its own grilling + plan when prioritised.
