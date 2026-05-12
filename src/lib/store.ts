import { create } from "zustand";

import { createConnectionsSlice } from "@/lib/store/connections";
import { createCredentialsSlice } from "@/lib/store/credentials";
import { createKeyValuePubSubSlice } from "@/lib/store/keyvalue-pubsub";
import { createKeyValueWorkspaceSlice } from "@/lib/store/keyvalue-workspace";
import { createRelationalQueriesSlice } from "@/lib/store/relational-queries";
import { createRelationalTablesSlice } from "@/lib/store/relational-tables";
import type { AppStoreState } from "@/lib/store/types";
import { createWorkspaceTabsSlice } from "@/lib/store/workspace-tabs";

// Re-exports — preserves the public surface of `@/lib/store` so every
// existing consumer (`import { Connection } from "@/lib/store"`) keeps
// resolving.
export type {
  ColumnChangeKind,
  NewColumn,
  PendingChange,
} from "@/lib/ddl";
export type {
  SchemaForeignKey,
  SchemaRelationships,
  SchemaTableNode,
} from "@/lib/schema-graph";
export { schemaRelationshipsKey } from "@/lib/schema-graph";
export type {
  ActiveView,
  AppSettingsSnapshot,
  AppSettingsStatus,
  ColumnInfo,
  Connection,
  ConstraintInfo,
  CredentialState,
  CredentialStorageMode,
  DatabaseEngine,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  ForeignKeyInfo,
  IndexInfo,
  QueryHistoryEntry,
  QueryPreviewData,
  QueryStatus,
  RedisCapabilities,
  RedisModuleInfo,
  SavedQueriesStatus,
  SavedQuery,
  SchemaExplorer,
  SchemaRelationshipsStatus,
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
  WorkspaceTabKind,
} from "@/lib/store/types";
export { tableDataKey, tableStructureKey } from "@/lib/store/types";

/**
 * The workspace Zustand store — composed of seven domain-concept
 * slices (see `store/README.md`). The slice files own their state
 * and actions; this file is the wiring + the public hook export.
 *
 * Each slice is typed against `AppStoreState` so cross-slice
 * `get()`/`set()` calls typecheck. Slices write their own state in
 * normal usage; cross-slice cascade cleanup goes through named
 * cleanup methods (e.g. `closeTabsForConnection`,
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
