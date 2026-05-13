import { create } from "zustand";

import { createConnectionsSlice } from "./connections";
import { createCredentialsSlice } from "./credentials";
import { createKeyValuePubSubSlice } from "./keyvalue-pubsub";
import { createKeyValueWorkspaceSlice } from "./keyvalue-workspace";
import { createRelationalQueriesSlice } from "./relational-queries";
import { createRelationalTablesSlice } from "./relational-tables";
import type { AppStoreState } from "./types";
import { createWorkspaceTabsSlice } from "./workspace-tabs";

// Public surface re-exports — every external `import … from "@/lib/store"`
// resolves through this barrel.
export { schemaRelationshipsKey } from "@/lib/schema-graph";
export type {
  ClickHouseStoredConnection,
  ColumnInfo,
  Connection,
  ConstraintInfo,
  CredentialStorageMode,
  DatabaseEngine,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  DDLOutcome,
  EditOutcome,
  ForeignKeyInfo,
  IndexInfo,
  MySqlConnection,
  MySqlStoredConnection,
  OverviewTabId,
  PgStoredConnection,
  QueryHistoryEntry,
  QueryOutcome,
  QueryPreviewData,
  QueryStatus,
  RedisConnection,
  RedisStoredConnection,
  SavedQuery,
  SchemaExplorer,
  SqliteStoredConnection,
  StorageClass,
  StoredConnection,
  StructureCapabilities,
  StructureCommitStatus,
  TableDataState,
  TableEditsCommitStatus,
  TableLoadStatus,
  TablePreviewData,
  TableStructure,
  TableStructureStatus,
  WorkspaceTab,
} from "./types";
export { tableDataKey, tableStructureKey } from "./types";

/**
 * The workspace Zustand store — composed of seven domain-concept
 * slices (see `./README.md`). The slice files own their state and
 * actions; this file is the wiring plus the public hook export.
 *
 * Each slice is typed against `AppStoreState` so cross-slice
 * `get()` / `set()` calls typecheck. Slices write their own state
 * during normal usage; cross-slice cascade cleanup goes through
 * named cleanup methods (e.g. `closeTabsForConnection`,
 * `dropRelationalCachesForConnection`) per the entity-owner pattern.
 */
export const useAppStore = create<AppStoreState>()((set, get, store) => ({
  ...createCredentialsSlice(set, get, store),
  ...createConnectionsSlice(set, get, store),
  ...createWorkspaceTabsSlice(set, get, store),
  ...createRelationalTablesSlice(set, get, store),
  ...createRelationalQueriesSlice(set, get, store),
  ...createKeyValueWorkspaceSlice(set, get, store),
  ...createKeyValuePubSubSlice(set, get, store),
}));
