/**
 * Shared DTOs for the workspace store.
 *
 * Slice files import their types from here so they don't need to
 * import from each other (which the slice-isolation discipline
 * forbids — see `store/README.md`). External consumers continue to
 * import types from `@/lib/store` (the directory barrel re-exports).
 *
 * Keep this file types-only. Helpers / pure functions live with the
 * slice that uses them. Cross-slice helpers (`formatLatencyMs`,
 * `errorToMessage`) live in `@/lib/format` and `@/lib/tauri`.
 */

import type { ColumnChangeKind, PendingChange } from "@/lib/ddl";
import type { SchemaRelationships } from "@/lib/schema-graph";

// ---------------------------------------------------------------------------
// Engine + storage class
// ---------------------------------------------------------------------------

export type DatabaseEngine =
  | "PostgreSQL"
  | "MySQL"
  | "ClickHouse"
  | "SQLite"
  | "Redis";

/**
 * Top-level engine class. Relational engines share schemas/tables/
 * rows/SQL; keyvalue engines share a keyspace of typed keys. Derived
 * from `DatabaseEngine`; see ADR-0008 and `engine-policy.ts`.
 */
export type StorageClass = "relational" | "keyvalue";

// ---------------------------------------------------------------------------
// Redis-specific identity
// ---------------------------------------------------------------------------

export type RedisModuleInfo = {
  name: string;
  version: string;
};

/**
 * Connect-time pipeline result for Redis — surfaced in the
 * post-test-connection banner on the new-connection form. Every
 * field is optional because managed Redis (Upstash hobby tier,
 * locked-down ACLs) often restricts `INFO` sections or
 * `MODULE LIST` and we degrade per-field rather than failing.
 */
export type RedisCapabilities = {
  serverVersion?: string;
  /** `master` or `replica`. Drives auto-read-only (ADR-0009). */
  role?: string;
  connectedSlaves?: number;
  modules?: RedisModuleInfo[];
  dbSize?: number;
  maxmemoryPolicy?: string;
};

// ---------------------------------------------------------------------------
// Credentials + app settings
// ---------------------------------------------------------------------------

export type CredentialStorageMode =
  | "keychain"
  | "encrypted-sqlite"
  | "plain-sqlite";

export type CredentialState = "needs-onboarding" | "needs-unlock" | "ready";

export type AppSettingsSnapshot = {
  onboardingCompleted: boolean;
  credentialStorageMode: CredentialStorageMode | null;
  credentialState: CredentialState;
  configDir: string;
};

export type AppSettingsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready" }
  | { state: "error"; error: string };

// ---------------------------------------------------------------------------
// Connection records
// ---------------------------------------------------------------------------

/**
 * Connection records are per-engine tagged unions discriminated by
 * `engine`. The Rust backend serializes the same shape via internally-
 * tagged enums (ADR-0010); TypeScript narrowing makes engine-specific
 * fields unreachable on the wrong variant.
 *
 * See ADR-0011 for the rationale (strict per-engine, sits below the
 * storage-class fork from ADR-0008).
 */
type ConnectionCommon = {
  id: string;
  name: string;
  database: string;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
};

export type PgStoredConnection = ConnectionCommon & {
  engine: "PostgreSQL";
  /** TLS upgrade on the wire protocol. PG/MySQL only — distinct from
   *  ClickHouse `useHttps` and Redis `useTls`. */
  ssl: boolean;
};
export type MySqlStoredConnection = ConnectionCommon & {
  engine: "MySQL";
  ssl: boolean;
};
export type SqliteStoredConnection = ConnectionCommon & {
  engine: "SQLite";
};
export type ClickHouseStoredConnection = ConnectionCommon & {
  engine: "ClickHouse";
  /** Connect over HTTPS instead of HTTP. */
  useHttps: boolean;
  /** URL path prefix for proxied deployments (e.g. `/clickhouse`). */
  urlPath: string;
};
export type RedisStoredConnection = ConnectionCommon & {
  engine: "Redis";
  /** Which numbered DB (0–15 on standalone). */
  dbNumber: number;
  /** Connect over TLS (`rediss://`). */
  useTls: boolean;
  /** Verify the TLS certificate when useTls is on. */
  verifyTlsCert: boolean;
};

export type StoredConnection =
  | PgStoredConnection
  | MySqlStoredConnection
  | SqliteStoredConnection
  | ClickHouseStoredConnection
  | RedisStoredConnection;

type ConnectionRuntimeFields = {
  status: "Connected" | "Read only" | "Disconnected";
  latency: string;
  lastSync: string;
  errorMessage?: string;
};

export type PgConnection = PgStoredConnection & ConnectionRuntimeFields;
export type MySqlConnection = MySqlStoredConnection & ConnectionRuntimeFields;
export type SqliteConnection = SqliteStoredConnection & ConnectionRuntimeFields;
export type ClickHouseConnection = ClickHouseStoredConnection &
  ConnectionRuntimeFields;
export type RedisConnection = RedisStoredConnection & ConnectionRuntimeFields;

export type Connection =
  | PgConnection
  | MySqlConnection
  | SqliteConnection
  | ClickHouseConnection
  | RedisConnection;

// ---------------------------------------------------------------------------
// Per-tab status shapes
// ---------------------------------------------------------------------------

export type QueryStatus =
  | { state: "idle" }
  | { state: "running" }
  | { state: "success"; runtimeMs?: number }
  | { state: "error"; error: string };

export type TableLoadStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

// ---------------------------------------------------------------------------
// Table data + structure
// ---------------------------------------------------------------------------

export type TableDataState = {
  connectionId: string;
  schema: string;
  table: string;
  columns: string[];
  rows: string[][];
  page: number;
  pageSize: number;
  totalRows?: number;
  runtimeMs: number;
};

export const tableDataKey = (
  connectionId: string,
  schema: string,
  table: string,
) => `${connectionId}::${schema}::${table}`;

export const tableStructureKey = (
  connectionId: string,
  schema: string,
  table: string,
) => `${connectionId}::${schema}::${table}`;

export type ColumnInfo = {
  name: string;
  dataType: string;
  nullable: boolean;
  defaultValue: string | null;
  isPrimaryKey: boolean;
  ordinalPosition: number;
};

export type ForeignKeyInfo = {
  name: string;
  columns: string[];
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate: string | null;
  onDelete: string | null;
};

export type IndexInfo = {
  name: string;
  columns: string[];
  isUnique: boolean;
  isPrimary: boolean;
  method: string | null;
};

export type ConstraintInfo = {
  name: string;
  kind: string;
  definition: string;
};

export type StructureCapabilities = {
  columns: boolean;
  primaryKey: boolean;
  foreignKeys: boolean;
  indexes: boolean;
  constraints: boolean;
  /** Whether new rows can be inserted via the row editor. */
  canInsertRows: boolean;
  /** Whether existing cells can be updated via the row editor. */
  canUpdateRows: boolean;
  /** Whether rows can be deleted via the row editor. */
  canDeleteRows: boolean;
  /** Whether ALTER TABLE-style schema edits are supported. */
  canAlterSchema: boolean;
  /**
   * "exact" — identity columns guarantee at most one matching row.
   * "best-effort" — identity may match multiple rows (ClickHouse).
   */
  uniquenessGuarantee: "exact" | "best-effort";
};

export type TableStructure = {
  columns: ColumnInfo[];
  primaryKey: string[] | null;
  foreignKeys: ForeignKeyInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  capabilities: StructureCapabilities;
  /** Engine-specific extension fields. Populated only for ClickHouse. */
  tableEngine?: string;
  partitionBy?: string;
  sampleBy?: string;
};

export type TableStructureStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

/**
 * Lifecycle state of an in-flight DDL commit — the **best-effort view**
 * the store keeps so the Commit button can stay disabled and labelled
 * "Committing..." across tab unmounts. Terminal state is NOT carried
 * here; callers get it from the awaited `DDLOutcome` returned by
 * `commitStructureChanges`. Absence from `structureCommitStatus` means
 * "nothing in flight." See CONTEXT.md — DDL Outcome.
 */
export type StructureCommitStatus = { state: "running" };

/**
 * Terminal result of one `commitStructureChanges` attempt — the
 * caller-facing outcome of executing a batch of DDL Statements. Always
 * synchronous today (no async tracking, unlike Edit Outcome). The
 * `noop` variant is returned when there were no pending changes —
 * distinct from `completed` so a future caller without a UI guard
 * doesn't render a "Committed in 0 ms" banner for nothing. See
 * CONTEXT.md — DDL Outcome.
 */
export type DDLOutcome =
  | { kind: "completed"; runtimeMs: number }
  | { kind: "failed"; reason: string }
  | { kind: "noop" };

/**
 * Lifecycle state of an in-flight Edit (cell-edit commit, row insert,
 * row delete) — the **best-effort view** the store keeps so the UI
 * badge can render across tab unmounts. Terminal state is NOT carried
 * here; callers get it from the awaited `EditOutcome` returned by the
 * action. Absence from `tableEditsCommitStatus` means "nothing in
 * flight." See CONTEXT.md — Edit Outcome.
 */
export type TableEditsCommitStatus =
  | { state: "running" }
  | {
      state: "queued";
      database: string;
      table: string;
      mutationIds: string[];
      runtimeMs: number;
    };

/**
 * Terminal result of one Edit attempt, returned by the store action
 * that initiated the write. Synchronous engines (PG/MySQL/SQLite)
 * resolve to `completed` or `failed`; `timeout` is reachable only on
 * async engines (ClickHouse, via Pending Mutation tracking). The
 * `noop` variant is returned when the action found nothing to do
 * (no edits, no rows, no payload after filtering) — distinct from
 * `completed` so callers don't render a misleading "saved 0 rows"
 * banner for a click that did no work. See CONTEXT.md — Edit Outcome.
 */
export type EditOutcome =
  | { kind: "completed"; runtimeMs: number; rowsAffected?: number }
  | { kind: "failed"; reason: string; mutationId?: string }
  | { kind: "timeout"; remaining: string[] }
  | { kind: "noop" };

export type SchemaRelationshipsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

export type DatabaseOverviewStats = {
  databaseSizeBytes: number;
  tableSizeBytes: number;
  indexSizeBytes: number;
  tableCount: number;
  schemaCount: number;
  rowCountEstimate: number;
  indexCount: number;
  connectionCount: number;
};

export type DatabaseOverviewStatsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

// Re-export domain types from their owning modules so the workspace
// state type (and slice files) can refer to them through one import
// path.
export type { ColumnChangeKind, PendingChange };
export type { SchemaRelationships };

// ---------------------------------------------------------------------------
// Schema explorer + saved queries + history
// ---------------------------------------------------------------------------

export type SchemaExplorer = {
  name: string;
  tables: string[];
  views?: string[];
};

export type SavedQuery = {
  id: string;
  name: string;
  body: string;
  /** `null` = saved query is not pinned to a specific connection. */
  connectionId: string | null;
  isFavorite: boolean;
  /** Reserved for future cloud-sync. Local writes leave this null. */
  ownerId?: string | null;
  createdAt: string;
  updatedAt: string;
};

export type SavedQueriesStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

export type QueryHistoryEntry = {
  id: string;
  sql: string;
  connectionId: string;
  connectionName: string;
  database: string;
  engine: DatabaseEngine;
  status: "success" | "error";
  errorMessage?: string;
  runtimeMs: number;
  rowCount?: number;
  startedAt: string;
};

// ---------------------------------------------------------------------------
// Workspace tabs
// ---------------------------------------------------------------------------

export type WorkspaceTabKind =
  | "table"
  | "query"
  | "key"
  | "cli"
  | "pubsub"
  | "server";

export type WorkspaceTab = {
  id: string;
  kind: WorkspaceTabKind;
  label: string;
  connectionId: string;
  schema: string;
  table?: string;
  query?: string;
  lastRun?: string;
  isDirty?: boolean;
  /** Redis `key` tab: the inspected key name (under `dbNumber`). */
  redisKey?: string;
  /** Redis `key` tab: scoped DB number at open time. */
  redisDbNumber?: number;
};

export type TablePreviewData = {
  columns: string[];
  rows: string[][];
  rowCount: string;
  primaryKey: string;
  size: string;
  lastVacuum: string;
};

export type QueryPreviewData = {
  columns: string[];
  rows: string[][];
  runtime: string;
  rowCount: string;
  cache: string;
};

export type ActiveView = "workspace" | "connections" | "settings";

// ---------------------------------------------------------------------------
// AppStoreState — the full store shape
// ---------------------------------------------------------------------------

/**
 * The complete shape of the Zustand store. During the slice migration
 * this is the single canonical type — every slice's `StateCreator`
 * factory types its `set`/`get` against this shape so cross-slice
 * `get()` calls typecheck.
 *
 * After the migration is complete (commit 10), this can be
 * re-expressed as the intersection of every slice's `SliceShape`. The
 * runtime behaviour is identical either way; the intersection form
 * just removes the need to maintain this explicit declaration as
 * slices grow.
 */
export interface AppStoreState {
  activeView: ActiveView;
  activeConnectionId: string;
  activeTabId: string;
  expandedSchemas: string[];
  isLeftSidebarOpen: boolean;
  connections: Connection[];
  workspaceTabs: WorkspaceTab[];
  schemaExplorer: Record<string, SchemaExplorer[]>;
  tablePreviews: Record<string, TablePreviewData>;
  tableData: Record<string, TableDataState>;
  tableStructure: Record<string, TableStructure>;
  queryPreviews: Record<string, QueryPreviewData>;
  queryStatus: Record<string, QueryStatus>;
  tableLoadStatus: Record<string, TableLoadStatus>;
  tableStructureStatus: Record<string, TableStructureStatus>;
  pendingStructureChanges: Record<string, PendingChange[]>;
  structureCommitStatus: Record<string, StructureCommitStatus>;
  queryEdits: Record<string, Record<number, Record<number, string>>>;
  tableEdits: Record<string, Record<number, Record<number, string>>>;
  tableEditsCommitStatus: Record<string, TableEditsCommitStatus>;
  schemaRelationships: Record<string, SchemaRelationships>;
  schemaRelationshipsStatus: Record<string, SchemaRelationshipsStatus>;
  databaseOverviewStats: Record<string, DatabaseOverviewStats>;
  databaseOverviewStatsStatus: Record<string, DatabaseOverviewStatsStatus>;
  queryHistory: QueryHistoryEntry[];
  savedQueries: SavedQuery[];
  savedQueriesStatus: SavedQueriesStatus;
  appSettings: AppSettingsSnapshot | null;
  appSettingsStatus: AppSettingsStatus;
  credentialStorageStatus:
    | { state: "idle" }
    | { state: "running" }
    | { state: "error"; error: string };
  editorTheme: string;
  selectedRowIndex: number;

  setActiveView: (view: ActiveView) => void;
  setActiveConnectionId: (id: string) => void;
  setActiveTabId: (id: string) => void;
  setExpandedSchemas: (
    schemas: string[] | ((prev: string[]) => string[]),
  ) => void;
  toggleLeftSidebar: () => void;
  setWorkspaceTabs: (
    tabs: WorkspaceTab[] | ((prev: WorkspaceTab[]) => WorkspaceTab[]),
  ) => void;
  setEditorTheme: (theme: string) => void;
  setSelectedRowIndex: (index: number) => void;
  setQueryEdit: (
    tabId: string,
    rowIndex: number,
    colIndex: number,
    value: string,
  ) => void;
  discardQueryEdits: (tabId: string) => void;
  setTableEdit: (
    tableName: string,
    rowIndex: number,
    colIndex: number,
    value: string,
  ) => void;
  discardTableEdits: (tableName: string) => void;
  commitTableEdits: (tableName: string) => Promise<EditOutcome>;
  addTableRow: (
    tableName: string,
    values: Array<{ column: string; value: string | null }>,
  ) => Promise<EditOutcome>;
  deleteSelectedTableRows: (
    tableName: string,
    rowIndices: number[],
  ) => Promise<EditOutcome>;
  loadTablePreview: (schemaName: string, tableName: string) => Promise<void>;
  loadTableData: (
    connectionId: string,
    schema: string,
    table: string,
    page?: number,
    pageSize?: number,
  ) => Promise<void>;
  refreshTableData: (key: string) => Promise<void>;
  loadTableStructure: (
    connectionId: string,
    schema: string,
    table: string,
  ) => Promise<void>;
  loadSchemaRelationships: (
    connectionId: string,
    schema: string,
  ) => Promise<void>;
  loadDatabaseOverviewStats: (connectionId: string) => Promise<void>;
  focusTableInSchemaMap: (
    connectionId: string,
    schema: string,
    table: string,
  ) => void;
  addPendingStructureChange: (
    key: string,
    entry: { schema: string; table: string; change: ColumnChangeKind },
  ) => void;
  removePendingStructureChange: (key: string, id: string) => void;
  clearPendingStructureChanges: (key: string) => void;
  commitStructureChanges: (key: string) => Promise<DDLOutcome>;
  loadAppSettings: () => Promise<AppSettingsSnapshot | null>;
  configureCredentialStorage: (input: {
    mode: CredentialStorageMode;
    password?: string;
  }) => Promise<AppSettingsSnapshot | null>;
  unlockCredentials: (password: string) => Promise<AppSettingsSnapshot | null>;
  changeCredentialStorage: (input: {
    mode: CredentialStorageMode;
    password?: string;
  }) => Promise<AppSettingsSnapshot | null>;
  resetCredentialStorage: () => Promise<AppSettingsSnapshot | null>;
  loadConnections: () => Promise<void>;
  loadQueryHistory: () => Promise<void>;
  loadSavedQueries: () => Promise<void>;
  saveSavedQuery: (
    query: Omit<SavedQuery, "createdAt" | "updatedAt"> &
      Partial<Pick<SavedQuery, "createdAt" | "updatedAt">>,
  ) => Promise<void>;
  deleteSavedQuery: (id: string) => Promise<void>;
  reopenHistoryEntry: (entry: QueryHistoryEntry) => void;
  addConnection: (connection: Connection) => Promise<void>;
  updateConnection: (connection: Connection) => Promise<void>;
  deleteConnection: (connectionId: string) => Promise<void>;
  connectConnection: (connectionId: string) => Promise<void>;
  testConnection: (
    connection: StoredConnection,
  ) => Promise<
    | { ok: true; latencyMs: number; redisCapabilities?: RedisCapabilities }
    | { ok: false; error: string }
  >;
  runHealthChecks: () => Promise<void>;
  updateQuery: (tabId: string, query: string) => void;
  runQuery: (
    tabId: string,
    options?: { overrideSql?: string },
  ) => Promise<void>;
  closeTab: (tabId: string) => void;
  openWorkspaceTab: (tab: Omit<WorkspaceTab, "id">) => void;
  openTableTab: (schemaName: string, tableName: string) => void;
  openQueryForTable: (schemaName: string, tableName: string) => void;
  openViewTab: (schemaName: string, viewName: string) => void;
  createNewQueryTab: () => void;
  createNewTableTab: () => void;
  toggleSchema: (schemaName: string) => void;
}
