import { create } from "zustand";

import { createBastionsSlice } from "./bastions";
import { createConnectionsSlice } from "./connections";
import { createCredentialsSlice } from "./credentials";
import { createKeyValuePubSubSlice } from "./keyvalue-pubsub";
import { createKeyValueWorkspaceSlice } from "./keyvalue-workspace";
import { createManagedServersSlice } from "./managed-servers";
import { createQuerySessionsSlice } from "./query-sessions";
import { createRelationalQueriesSlice } from "./relational-queries";
import { createRelationalTablesSlice } from "./relational-tables";
import type { AppStoreState } from "./types";
import { createWorkspaceTabsSlice } from "./workspace-tabs";

// Public surface re-exports — every external `import … from "@/lib/store"`
// resolves through this barrel.
export { schemaRelationshipsKey } from "@/lib/schema-graph";
export type {
  BastionAuthMethod,
  BastionServer,
  BastionStatus,
  ClickHouseStoredConnection,
  ColumnInfo,
  Connection,
  ConstraintInfo,
  CredentialStorageMode,
  DatabaseEngine,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  DDLOutcome,
  DockerStatus,
  EditOutcome,
  ForeignKeyInfo,
  IndexInfo,
  LoadingStatus,
  ManagedServer,
  ManagedServerStatus,
  ManagedServersStatus,
  ManagedServerWithStatus,
  MySqlConnection,
  MySqlStoredConnection,
  OverviewTabId,
  PgDriverOptions,
  PgExtension,
  PgSetting,
  PgStoredConnection,
  ProvisionManagedServerInput,
  ProvisionManagedServerResult,
  QueryHistoryEntry,
  QueryExecution,
  QueryExecutionStatus,
  QueryExecutionTerminalStatus,
  QueryResultSet,
  QuerySessionError,
  QuerySessionState,
  QueryTransactionIsolation,
  QueryTransactionMode,
  QueryTransactionSnapshot,
  QueryTransactionStatus,
  QueryOutcome,
  QueryPreviewData,
  QueryStatus,
  RedisConnection,
  RedisStoredConnection,
  RelationInfo,
  RelationStatsStatus,
  SaveBastionServerInput,
  SavedQuery,
  SchemaExplorer,
  SchemaMapAttrMode,
  SchemaMapPosition,
  SchemaMapPrefs,
  SchemaMapRouting,
  SchemaRelationshipsStatus,
  SecretChange,
  ServerDetails,
  ServerDetailsStatus,
  SettingsTab,
  SqliteStoredConnection,
  SshTunnelConfig,
  StorageClass,
  StoredConnection,
  StructureCapabilities,
  StructureCommitStatus,
  TableDataState,
  TableEditsCommitStatus,
  TableLoadStatus,
  TablePreviewData,
  TableRef,
  TableSessionSnapshot,
  TableStructure,
  TableStructureStatus,
  WorkspaceTab,
} from "./types";
export { tableDataKey, tableSessionKey, tableStructureKey } from "./types";

/**
 * The workspace Zustand store — composed of domain-concept slices
 * (see `./README.md`). The slice files own their state and
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
  ...createBastionsSlice(set, get, store),
  ...createManagedServersSlice(set, get, store),
  ...createConnectionsSlice(set, get, store),
  ...createWorkspaceTabsSlice(set, get, store),
  ...createRelationalTablesSlice(set, get, store),
  ...createQuerySessionsSlice(set, get, store),
  ...createRelationalQueriesSlice(set, get, store),
  ...createKeyValueWorkspaceSlice(set, get, store),
  ...createKeyValuePubSubSlice(set, get, store),
}));
