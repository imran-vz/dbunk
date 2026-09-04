import { create } from "zustand";

import { createBastionsSlice } from "./bastions";
import { createConnectionsSlice } from "./connections";
import { createConsoleSlice } from "./console";
import { createCredentialsSlice } from "./credentials";
import { createKeyValuePubSubSlice } from "./keyvalue-pubsub";
import { createKeyValueWorkspaceSlice } from "./keyvalue-workspace";
import { createManagedServersSlice } from "./managed-servers";
import { createMutationDraftsSlice } from "./mutation-drafts";
import { createPgObjectsSlice } from "./pg-objects";
import { createQuerySessionsSlice } from "./query-sessions";
import { createRelationalQueriesSlice } from "./relational-queries";
import { createRelationalTablesSlice } from "./relational-tables";
import { createTableBrowseSlice } from "./table-browse";
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
  ColumnChangeKind,
  ColumnInfo,
  Connection,
  ConnectionEnvironment,
  ConstraintInfo,
  CredentialStorageMode,
  DatabaseEngine,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  DdlApplyResult,
  DdlPlanPreview,
  DdlResidue,
  DdlStatementSummary,
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
  PgCatalogEntry,
  PgCatalogTruncation,
  PgCommentTarget,
  PgDefaultValue,
  PgDropDependent,
  PgDropImpact,
  PgEnumPosition,
  PgExtension,
  PgIndexColumn,
  PgNewColumnSpec,
  PgObjectCatalog,
  PgObjectDescription,
  PgObjectError,
  PgObjectFacts,
  PgObjectKind,
  PgObjectOp,
  PgObjectRef,
  PgParallelSafety,
  PgPolicyCommand,
  PgPrivilege,
  PgReferentialAction,
  PgTriggerEvent,
  PgRoutineObjectKind,
  PgSchemaObjects,
  PgScopedObjectKind,
  PgSetting,
  PgStoredConnection,
  PgTypeAttribute,
  PgTypeClass,
  PgVolatility,
  PendingChange,
  PlannedStatement,
  PolicyInfo,
  PrivilegeInfo,
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
  RowSecurityInfo,
  SaveBastionServerInput,
  SavedQuery,
  SafeMode,
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
  StatementGroup,
  StructureCapabilities,
  StructureChange,
  StructureCommitStatus,
  TriggerInfo,
  TableDataState,
  TableEditsCommitStatus,
  TableLoadStatus,
  TablePreviewData,
  TableRef,
  TabCaret,
  TableBrowseLoadStatus,
  TableBrowseTabState,
  TableSessionSnapshot,
  TableStructure,
  TableStructureStatus,
  WorkspaceTab,
} from "./types";
export {
  buildMutationDraftPlan,
  mutationDraftPreviewsEqual,
  queryMutationDraftScope,
  rebindMutationDraftChanges,
  tableMutationDraftScope,
} from "./mutation-drafts";
export {
  canonicalPgObjectRefKey,
  catalogToSchemaExplorer,
  decodePgObjectError,
  formatPgCatalogError,
  pgObjectDdlApplyKey,
  pgObjectDescriptionKey,
} from "./pg-objects";
export type {
  PgObjectCatalogState,
  PgObjectDescriptionState,
  PgObjectLoadResult,
  PgObjectLoadStatus,
  PgObjectsSlice,
} from "./pg-objects";
export {
  isNavigatorGroupExpanded,
  navigatorGroupId,
} from "./relational-tables";
export type { NavigatorGroupKey } from "./relational-tables";
export type {
  MutationDraft,
  MutationDraftAnalysisRecovery,
  MutationDraftApplyRequest,
  MutationDraftApplyState,
  MutationDraftCell,
  MutationDraftChange,
  MutationDraftChangeFailure,
  MutationDraftDelete,
  MutationDraftHandle,
  MutationDraftInsert,
  MutationDraftLoadedRow,
  MutationDraftOwner,
  MutationDraftPlanBuild,
  MutationDraftPreviewRequest,
  MutationDraftPreviewState,
  MutationDraftScope,
  MutationDraftUpdate,
  MutationDraftsSlice,
  OpenMutationDraftInput,
  QueryMutationDraftScope,
  StageMutationDraftDeleteInput,
  StageMutationDraftInsertInput,
  StageMutationDraftUpdateInput,
  TableMutationDraftScope,
} from "./mutation-drafts";
export {
  isConnectedStatus,
  tableDataKey,
  tableSessionKey,
  tableStructureKey,
} from "./types";
export type { QueryTransactionCommand } from "./query-sessions";
export { consoleSeverityForNotice } from "./console";
export type { ConsoleEvent, ConsoleSeverity, ConsoleSource } from "./console";

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
  ...createPgObjectsSlice(set, get, store),
  ...createRelationalTablesSlice(set, get, store),
  ...createMutationDraftsSlice(set, get, store),
  ...createQuerySessionsSlice(set, get, store),
  ...createTableBrowseSlice(set, get, store),
  ...createRelationalQueriesSlice(set, get, store),
  ...createKeyValueWorkspaceSlice(set, get, store),
  ...createKeyValuePubSubSlice(set, get, store),
  ...createConsoleSlice(set, get, store),
}));
