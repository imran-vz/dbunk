import { create } from "zustand";
import {
  type ColumnChangeKind,
  generateDdlForEngine,
  type PendingChange,
} from "@/lib/ddl";
import {
  type MutationOutcome,
  pendingMutationsFromResult,
  trackMutations,
} from "@/lib/pending-mutations";
import { pickRowIdentity } from "@/lib/row-identity";
import {
  type SchemaForeignKey,
  type SchemaRelationships,
  type SchemaTableNode,
  schemaRelationshipsKey,
} from "@/lib/schema-graph";
import { pickSqlToRun } from "@/lib/sql";
import { isTauri, tauriInvoke } from "@/lib/tauri";

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

export type DatabaseEngine = "PostgreSQL" | "MySQL" | "ClickHouse" | "SQLite";

export type StoredConnection = {
  id: string;
  name: string;
  database: string;
  engine: DatabaseEngine;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
  /** ClickHouse-only: connect over HTTPS instead of HTTP. */
  useHttps?: boolean;
  /** ClickHouse-only: URL path prefix for proxied deployments (e.g. /clickhouse). */
  urlPath?: string;
};

export type Connection = {
  id: string;
  name: string;
  database: string;
  status: "Connected" | "Read only" | "Disconnected";
  engine: DatabaseEngine;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  latency: string;
  lastSync: string;
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
  errorMessage?: string;
  /** ClickHouse-only: connect over HTTPS instead of HTTP. */
  useHttps?: boolean;
  /** ClickHouse-only: URL path prefix for proxied deployments. */
  urlPath?: string;
};

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

const errorToMessage = (error: unknown): string => {
  if (error instanceof Error) {
    return error.message;
  }
  if (typeof error === "string") {
    return error;
  }
  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") {
      return message;
    }
  }
  try {
    return JSON.stringify(error);
  } catch {
    return "Unknown error";
  }
};

type ConnectResult = {
  latencyMs: number;
};

const formatLatencyMs = (latencyMs: unknown): string =>
  typeof latencyMs === "number" && Number.isFinite(latencyMs)
    ? `${latencyMs} ms`
    : "--";

type RunQueryResult = {
  columns: string[];
  rows: string[][];
  runtimeMs: number;
  rowCount: number;
};

type TableDataResult = {
  columns: string[];
  rows: string[][];
  page: number;
  pageSize: number;
  totalRows?: number | null;
  runtimeMs: number;
};

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
   * "synchronous" — UPDATE/DELETE return after the change is applied.
   * "async" — UPDATE/DELETE return after the mutation is queued
   *   (ClickHouse). The frontend renders a "Queued" status and polls.
   */
  updateSemantics: "synchronous" | "async";
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

export type StructureCommitStatus =
  | { state: "idle" }
  | { state: "running" }
  | { state: "success"; runtimeMs?: number }
  | { state: "error"; error: string };

export type TableEditsCommitStatus =
  | { state: "idle" }
  | { state: "running" }
  | {
      state: "queued";
      database: string;
      table: string;
      mutationIds: string[];
      runtimeMs: number;
    }
  | { state: "success"; rowsAffected: number; runtimeMs: number }
  | { state: "error"; error: string };

type CellEditPayload = {
  rowIndex: number;
  identity: Array<{ column: string; value: string | null }>;
  set: Array<{ column: string; value: string | null }>;
};

/**
 * Backend-shape result for `commit_cell_edits` and `delete_rows`.
 *
 * `state === "committed"` (PG) means the row counts in `rowsAffected`
 * are authoritative. `state === "queued"` (CH) means the changes have
 * been accepted as `ALTER … UPDATE/DELETE` mutations that apply
 * asynchronously — see `@/lib/pending-mutations` for the lifecycle.
 */
type CommitCellEditsResult = {
  rowsAffected: number;
  runtimeMs: number;
  state?: "committed" | "queued";
  database?: string;
  table?: string;
  mutationIds?: string[];
};

/**
 * Translate a [`MutationOutcome`] into the workspace's per-table
 * commit-status shape. Lives here (not in `pending-mutations`) because
 * the status surface is workspace-specific — a future async DDL would
 * translate the same outcome into `structureCommitStatus` instead.
 */
const outcomeToTableEditsStatus = (
  outcome: MutationOutcome,
  ctx: { startedAt: number; database: string; table: string },
): TableEditsCommitStatus => {
  if (outcome.kind === "completed") {
    return {
      state: "success",
      // ALTER UPDATE/DELETE doesn't report a row count — the refresh
      // is the source of truth for the post-mutation state.
      rowsAffected: 0,
      runtimeMs: Date.now() - ctx.startedAt,
    };
  }
  if (outcome.kind === "failed") {
    return {
      state: "error",
      error: `ClickHouse mutation ${outcome.mutationId} failed: ${outcome.reason}`,
    };
  }
  return {
    state: "error",
    error:
      "Mutation did not complete in time. " +
      `Check system.mutations for ${ctx.database}.${ctx.table}.`,
  };
};

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

const generatePendingId = (): string => {
  if (
    typeof globalThis !== "undefined" &&
    typeof (globalThis as { crypto?: { randomUUID?: () => string } }).crypto
      ?.randomUUID === "function"
  ) {
    return (
      globalThis as { crypto: { randomUUID: () => string } }
    ).crypto.randomUUID();
  }
  return `pending-${Date.now()}-${Math.random().toString(36).slice(2)}`;
};

const hydrateConnection = (connection: StoredConnection): Connection => ({
  ...connection,
  status: "Disconnected",
  latency: "--",
  lastSync: "Never",
});

const toStoredConnection = (connection: Connection): StoredConnection => ({
  id: connection.id,
  name: connection.name,
  database: connection.database,
  engine: connection.engine,
  host: connection.host,
  port: connection.port,
  user: connection.user,
  password: connection.password,
  role: connection.role,
  lastActivityAt: connection.lastActivityAt,
  useHttps: connection.useHttps,
  urlPath: connection.urlPath,
});

const applyConnectionUpdate = (
  connections: Connection[],
  connectionId: string,
  updates: Partial<Connection>,
) =>
  connections.map((connection) =>
    connection.id === connectionId ? { ...connection, ...updates } : connection,
  );

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

const generateHistoryId = (): string => {
  if (
    typeof globalThis !== "undefined" &&
    typeof (globalThis as { crypto?: { randomUUID?: () => string } }).crypto
      ?.randomUUID === "function"
  ) {
    return (
      globalThis as { crypto: { randomUUID: () => string } }
    ).crypto.randomUUID();
  }
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
};

export type WorkspaceTabKind = "table" | "query";

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

interface AppState {
  activeView: "workspace" | "connections";
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
  editorTheme: string;
  selectedRowIndex: number;

  setActiveView: (view: "workspace" | "connections") => void;
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
  commitTableEdits: (tableName: string) => Promise<void>;
  clearTableEditsCommitStatus: (tableName: string) => void;
  // The `tableEditsCommitStatus` map is intentionally shared between cell
  // edits, inserts, and deletes — there is one banner area in the UI per
  // table, so one status surface keeps things simple.
  addTableRow: (
    tableName: string,
    values: Array<{ column: string; value: string | null }>,
  ) => Promise<void>;
  deleteSelectedTableRows: (
    tableName: string,
    rowIndices: number[],
  ) => Promise<void>;
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
  commitStructureChanges: (key: string) => Promise<void>;
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
    connection: Pick<
      StoredConnection,
      | "id"
      | "name"
      | "database"
      | "engine"
      | "host"
      | "port"
      | "user"
      | "password"
      | "role"
    > &
      Partial<Pick<StoredConnection, "useHttps" | "urlPath">>,
  ) => Promise<{ ok: true; latencyMs: number } | { ok: false; error: string }>;
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

// Initial Empty State
const initialConnections: Connection[] = [];
const initialSchemaExplorer: Record<string, SchemaExplorer[]> = {};
const initialWorkspaceTabs: WorkspaceTab[] = [];
const initialTablePreviews: Record<string, TablePreviewData> = {};
const initialTableData: Record<string, TableDataState> = {};
const initialTableStructure: Record<string, TableStructure> = {};
const initialQueryPreviews: Record<string, QueryPreviewData> = {};
const initialQueryStatus: Record<string, QueryStatus> = {};
const initialTableLoadStatus: Record<string, TableLoadStatus> = {};
const initialTableStructureStatus: Record<string, TableStructureStatus> = {};
const initialPendingStructureChanges: Record<string, PendingChange[]> = {};
const initialStructureCommitStatus: Record<string, StructureCommitStatus> = {};
const initialQueryEdits: Record<
  string,
  Record<number, Record<number, string>>
> = {};
const initialTableEdits: Record<
  string,
  Record<number, Record<number, string>>
> = {};
const initialTableEditsCommitStatus: Record<string, TableEditsCommitStatus> =
  {};
const initialSchemaRelationships: Record<string, SchemaRelationships> = {};
const initialSchemaRelationshipsStatus: Record<
  string,
  SchemaRelationshipsStatus
> = {};
const initialDatabaseOverviewStats: Record<string, DatabaseOverviewStats> = {};
const initialDatabaseOverviewStatsStatus: Record<
  string,
  DatabaseOverviewStatsStatus
> = {};
const initialQueryHistory: QueryHistoryEntry[] = [];

let nextTabIndex = 1;
let nextQueryIndex = 1;

export const useAppStore = create<AppState>((set, get) => ({
  activeView: "workspace",
  activeConnectionId: "",
  activeTabId: "",
  expandedSchemas: [],
  isLeftSidebarOpen: true,
  connections: initialConnections,
  workspaceTabs: initialWorkspaceTabs,
  schemaExplorer: initialSchemaExplorer,
  tablePreviews: initialTablePreviews,
  tableData: initialTableData,
  tableStructure: initialTableStructure,
  queryPreviews: initialQueryPreviews,
  queryStatus: initialQueryStatus,
  tableLoadStatus: initialTableLoadStatus,
  tableStructureStatus: initialTableStructureStatus,
  pendingStructureChanges: initialPendingStructureChanges,
  structureCommitStatus: initialStructureCommitStatus,
  queryEdits: initialQueryEdits,
  tableEdits: initialTableEdits,
  tableEditsCommitStatus: initialTableEditsCommitStatus,
  schemaRelationships: initialSchemaRelationships,
  schemaRelationshipsStatus: initialSchemaRelationshipsStatus,
  databaseOverviewStats: initialDatabaseOverviewStats,
  databaseOverviewStatsStatus: initialDatabaseOverviewStatsStatus,
  queryHistory: initialQueryHistory,
  savedQueries: [],
  savedQueriesStatus: { state: "idle" },
  editorTheme: "vs",
  selectedRowIndex: 0,

  setActiveView: (view) => set({ activeView: view }),
  setActiveConnectionId: (id) => set({ activeConnectionId: id }),
  setActiveTabId: (id) => set({ activeTabId: id, activeView: "workspace" }),
  setExpandedSchemas: (schemas) =>
    set((state) => ({
      expandedSchemas:
        typeof schemas === "function"
          ? schemas(state.expandedSchemas)
          : schemas,
    })),
  toggleLeftSidebar: () =>
    set((state) => ({ isLeftSidebarOpen: !state.isLeftSidebarOpen })),
  setWorkspaceTabs: (tabs) =>
    set((state) => ({
      workspaceTabs:
        typeof tabs === "function" ? tabs(state.workspaceTabs) : tabs,
    })),
  setEditorTheme: (theme) => set({ editorTheme: theme }),
  setSelectedRowIndex: (index) => set({ selectedRowIndex: index }),

  setQueryEdit: (tabId, rowIndex, colIndex, value) =>
    set((state) => ({
      queryEdits: {
        ...state.queryEdits,
        [tabId]: {
          ...state.queryEdits[tabId],
          [rowIndex]: {
            ...state.queryEdits[tabId]?.[rowIndex],
            [colIndex]: value,
          },
        },
      },
    })),

  discardQueryEdits: (tabId) =>
    set((state) => {
      const { [tabId]: _, ...rest } = state.queryEdits;
      return { queryEdits: rest };
    }),

  setTableEdit: (tableName, rowIndex, colIndex, value) =>
    set((state) => ({
      tableEdits: {
        ...state.tableEdits,
        [tableName]: {
          ...state.tableEdits[tableName],
          [rowIndex]: {
            ...state.tableEdits[tableName]?.[rowIndex],
            [colIndex]: value,
          },
        },
      },
    })),

  discardTableEdits: (tableName) =>
    set((state) => {
      const { [tableName]: _, ...rest } = state.tableEdits;
      return { tableEdits: rest };
    }),

  clearTableEditsCommitStatus: (tableName) =>
    set((state) => {
      if (!(tableName in state.tableEditsCommitStatus)) {
        return {};
      }
      const { [tableName]: _, ...rest } = state.tableEditsCommitStatus;
      return { tableEditsCommitStatus: rest };
    }),

  commitTableEdits: async (tableName) => {
    const state = get();
    const editsForTable = state.tableEdits[tableName];
    if (!editsForTable || Object.keys(editsForTable).length === 0) {
      return;
    }

    // Find the loaded table data for this table. The active table edits are
    // keyed by table name; the underlying data is keyed by
    // `${connectionId}::${schema}::${table}`. We accept the first match for
    // the table name — only one table tab can be the active edit target at a
    // time today.
    const dataEntry = Object.entries(state.tableData).find(
      ([, data]) => data.table === tableName,
    );
    if (!dataEntry) {
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "error",
            error: "Table data is not loaded; cannot commit edits.",
          },
        },
      }));
      return;
    }
    const [dataKeyForTable, data] = dataEntry;
    const structureKeyForTable = tableStructureKey(
      data.connectionId,
      data.schema,
      data.table,
    );
    const structure = state.tableStructure[structureKeyForTable];
    const identity = pickRowIdentity(structure);
    if (!identity) {
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "error",
            error:
              "This table has no primary key or non-null unique index — it is read-only.",
          },
        },
      }));
      return;
    }

    const connection = state.connections.find(
      (c) => c.id === data.connectionId,
    );
    if (!connection) {
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "error",
            error: "Connection not found for this table.",
          },
        },
      }));
      return;
    }
    if (!structure.capabilities.canUpdateRows) {
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "error",
            error: `This table does not support cell edits on ${connection.engine}.`,
          },
        },
      }));
      return;
    }

    // Build edits payload. Filter out any cell edits where the new value
    // matches the original — those are no-ops and should not appear in the
    // UPDATE. Filter out rows whose only edits were no-ops.
    const columnIndexByName = new Map<string, number>();
    data.columns.forEach((name, index) => {
      columnIndexByName.set(name, index);
    });

    const identityMissing: string[] = identity.columns.filter(
      (col) => !columnIndexByName.has(col),
    );
    if (identityMissing.length > 0) {
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "error",
            error: `Identity column(s) not present in loaded data: ${identityMissing.join(", ")}`,
          },
        },
      }));
      return;
    }

    const editsPayload: CellEditPayload[] = [];
    const sortedRowIndices = Object.keys(editsForTable)
      .map((k) => Number.parseInt(k, 10))
      .filter((n) => Number.isFinite(n))
      .sort((a, b) => a - b);

    for (const rowIndex of sortedRowIndices) {
      const row = data.rows[rowIndex];
      if (!row) {
        continue;
      }
      const colChanges = editsForTable[rowIndex] ?? {};
      const sortedColIndices = Object.keys(colChanges)
        .map((k) => Number.parseInt(k, 10))
        .filter((n) => Number.isFinite(n))
        .sort((a, b) => a - b);

      const setEntries: Array<{ column: string; value: string | null }> = [];
      for (const colIndex of sortedColIndices) {
        const newValue = colChanges[colIndex];
        if (newValue === undefined) {
          continue;
        }
        const original = row[colIndex];
        if (newValue === original) {
          continue;
        }
        const columnName = data.columns[colIndex];
        if (!columnName) {
          continue;
        }
        setEntries.push({ column: columnName, value: newValue });
      }
      if (setEntries.length === 0) {
        continue;
      }

      const identityEntries = identity.columns.map((col) => {
        const idx = columnIndexByName.get(col) as number;
        return { column: col, value: row[idx] ?? null };
      });

      editsPayload.push({
        rowIndex,
        identity: identityEntries,
        set: setEntries,
      });
    }

    if (editsPayload.length === 0) {
      // All edits were no-ops; clear them silently and report success.
      set((s) => {
        const { [tableName]: _, ...rest } = s.tableEdits;
        return {
          tableEdits: rest,
          tableEditsCommitStatus: {
            ...s.tableEditsCommitStatus,
            [tableName]: { state: "success", rowsAffected: 0, runtimeMs: 0 },
          },
        };
      });
      return;
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [tableName]: { state: "running" },
      },
    }));

    if (!isTauri()) {
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "error",
            error: "Backend is unavailable in this environment.",
          },
        },
      }));
      return;
    }

    try {
      const result = await tauriInvoke<CommitCellEditsResult>(
        "commit_cell_edits",
        {
          payload: {
            connectionId: data.connectionId,
            schema: data.schema,
            table: data.table,
            edits: editsPayload,
          },
        },
      );
      // ClickHouse returns `state: "queued"` with a list of mutation
      // IDs. We hand the batch to `trackMutations` and translate its
      // outcome into the workspace's commit-status surface. The edit
      // buffer clears immediately so the UI doesn't "stick"; the
      // backing data refresh fires only when the mutations complete.
      const pendingMutations = pendingMutationsFromResult(result, {
        connectionId: data.connectionId,
        database: data.schema,
        table: data.table,
      });
      if (pendingMutations.length > 0) {
        const queuedDatabase = pendingMutations[0].database;
        const queuedTable = pendingMutations[0].table;
        set((s) => {
          const { [tableName]: _, ...restEdits } = s.tableEdits;
          return {
            tableEdits: restEdits,
            tableEditsCommitStatus: {
              ...s.tableEditsCommitStatus,
              [tableName]: {
                state: "queued",
                database: queuedDatabase,
                table: queuedTable,
                mutationIds: pendingMutations.map((m) => m.id),
                runtimeMs: result.runtimeMs,
              },
            },
          };
        });
        const startedAt = Date.now();
        const outcome = await trackMutations(pendingMutations);
        set((s) => ({
          tableEditsCommitStatus: {
            ...s.tableEditsCommitStatus,
            [tableName]: outcomeToTableEditsStatus(outcome, {
              startedAt,
              database: queuedDatabase,
              table: queuedTable,
            }),
          },
        }));
        if (outcome.kind === "completed") {
          await get().refreshTableData(dataKeyForTable);
        }
        return;
      }

      set((s) => {
        const { [tableName]: _, ...restEdits } = s.tableEdits;
        return {
          tableEdits: restEdits,
          tableEditsCommitStatus: {
            ...s.tableEditsCommitStatus,
            [tableName]: {
              state: "success",
              rowsAffected: result.rowsAffected,
              runtimeMs: result.runtimeMs,
            },
          },
        };
      });
      await get().refreshTableData(dataKeyForTable);
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to commit cell edits", error);
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: { state: "error", error: message },
        },
      }));
    }
  },

  addTableRow: async (tableName, values) => {
    const setError = (error: string) =>
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: { state: "error", error },
        },
      }));

    if (values.length === 0) {
      setError("Provide at least one value (or default) to insert.");
      return;
    }

    const state = get();
    const dataEntry = Object.entries(state.tableData).find(
      ([, data]) => data.table === tableName,
    );
    if (!dataEntry) {
      setError("Table data is not loaded; cannot insert a row.");
      return;
    }
    const [dataKeyForTable, data] = dataEntry;

    const connection = state.connections.find(
      (c) => c.id === data.connectionId,
    );
    if (!connection) {
      setError("Connection not found for this table.");
      return;
    }
    const structureKeyForInsert = tableStructureKey(
      data.connectionId,
      data.schema,
      data.table,
    );
    const insertStructure = state.tableStructure[structureKeyForInsert];
    // When structure has been loaded, trust its capability flag. When it
    // hasn't, defer to the backend — it will surface a clear engine-
    // specific error rather than us pre-emptively guessing.
    if (insertStructure && !insertStructure.capabilities.canInsertRows) {
      setError(`This table does not support inserts on ${connection.engine}.`);
      return;
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [tableName]: { state: "running" },
      },
    }));

    if (!isTauri()) {
      setError("Backend is unavailable in this environment.");
      return;
    }

    try {
      const result = await tauriInvoke<{
        rowsAffected: number;
        runtimeMs: number;
      }>("insert_row", {
        payload: {
          connectionId: data.connectionId,
          schema: data.schema,
          table: data.table,
          values,
        },
      });
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "success",
            rowsAffected: result.rowsAffected,
            runtimeMs: result.runtimeMs,
          },
        },
      }));
      await get().refreshTableData(dataKeyForTable);
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to insert row", error);
      setError(message);
    }
  },

  deleteSelectedTableRows: async (tableName, rowIndices) => {
    if (rowIndices.length === 0) {
      return;
    }

    const setError = (error: string) =>
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: { state: "error", error },
        },
      }));

    const state = get();
    const dataEntry = Object.entries(state.tableData).find(
      ([, data]) => data.table === tableName,
    );
    if (!dataEntry) {
      setError("Table data is not loaded; cannot delete rows.");
      return;
    }
    const [dataKeyForTable, data] = dataEntry;
    const structureKeyForTable = tableStructureKey(
      data.connectionId,
      data.schema,
      data.table,
    );
    const structure = state.tableStructure[structureKeyForTable];
    const identity = pickRowIdentity(structure);
    if (!identity) {
      setError(
        "This table has no primary key or non-null unique index — it is read-only.",
      );
      return;
    }

    const connection = state.connections.find(
      (c) => c.id === data.connectionId,
    );
    if (!connection) {
      setError("Connection not found for this table.");
      return;
    }
    if (structure && !structure.capabilities.canDeleteRows) {
      setError(
        `This table does not support row deletes on ${connection.engine}.`,
      );
      return;
    }

    const columnIndexByName = new Map<string, number>();
    data.columns.forEach((name, index) => {
      columnIndexByName.set(name, index);
    });
    const identityMissing = identity.columns.filter(
      (col) => !columnIndexByName.has(col),
    );
    if (identityMissing.length > 0) {
      setError(
        `Identity column(s) not present in loaded data: ${identityMissing.join(", ")}`,
      );
      return;
    }

    const rowsPayload: Array<Array<{ column: string; value: string | null }>> =
      [];
    const sortedIndices = [...rowIndices].sort((a, b) => a - b);
    for (const rowIndex of sortedIndices) {
      const row = data.rows[rowIndex];
      if (!row) {
        continue;
      }
      rowsPayload.push(
        identity.columns.map((col) => {
          const idx = columnIndexByName.get(col) as number;
          return { column: col, value: row[idx] ?? null };
        }),
      );
    }

    if (rowsPayload.length === 0) {
      // Selection pointed at non-existent rows. Nothing to do, no banner.
      return;
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [tableName]: { state: "running" },
      },
    }));

    if (!isTauri()) {
      setError("Backend is unavailable in this environment.");
      return;
    }

    try {
      const result = await tauriInvoke<CommitCellEditsResult>("delete_rows", {
        payload: {
          connectionId: data.connectionId,
          schema: data.schema,
          table: data.table,
          rows: rowsPayload,
        },
      });
      const pendingMutations = pendingMutationsFromResult(result, {
        connectionId: data.connectionId,
        database: data.schema,
        table: data.table,
      });
      if (pendingMutations.length > 0) {
        const queuedDatabase = pendingMutations[0].database;
        const queuedTable = pendingMutations[0].table;
        set((s) => ({
          tableEditsCommitStatus: {
            ...s.tableEditsCommitStatus,
            [tableName]: {
              state: "queued",
              database: queuedDatabase,
              table: queuedTable,
              mutationIds: pendingMutations.map((m) => m.id),
              runtimeMs: result.runtimeMs,
            },
          },
        }));
        const startedAt = Date.now();
        const outcome = await trackMutations(pendingMutations);
        set((s) => ({
          tableEditsCommitStatus: {
            ...s.tableEditsCommitStatus,
            [tableName]: outcomeToTableEditsStatus(outcome, {
              startedAt,
              database: queuedDatabase,
              table: queuedTable,
            }),
          },
        }));
        if (outcome.kind === "completed") {
          await get().refreshTableData(dataKeyForTable);
        }
        return;
      }
      set((s) => ({
        tableEditsCommitStatus: {
          ...s.tableEditsCommitStatus,
          [tableName]: {
            state: "success",
            rowsAffected: result.rowsAffected,
            runtimeMs: result.runtimeMs,
          },
        },
      }));
      await get().refreshTableData(dataKeyForTable);
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to delete rows", error);
      setError(message);
    }
  },

  loadTablePreview: async (schemaName, tableName) => {
    const connectionId = get().activeConnectionId;
    if (!connectionId) {
      return;
    }
    await get().loadTableData(connectionId, schemaName, tableName);
  },

  loadTableData: async (
    connectionId,
    schema,
    table,
    page = 1,
    pageSize = 100,
  ) => {
    if (!connectionId) {
      return;
    }
    const key = tableDataKey(connectionId, schema, table);
    set((state) => ({
      tableLoadStatus: {
        ...state.tableLoadStatus,
        [table]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      // In non-Tauri environments (browser preview, tests without mocks)
      // we cannot fetch real data; mark idle and bail.
      set((state) => ({
        tableLoadStatus: {
          ...state.tableLoadStatus,
          [table]: { state: "idle" },
        },
      }));
      return;
    }
    try {
      const result = await tauriInvoke<TableDataResult>("load_table_data", {
        payload: {
          connectionId,
          schema,
          table,
          page,
          pageSize,
        },
      });
      const totalRows =
        result.totalRows === null || result.totalRows === undefined
          ? undefined
          : result.totalRows;
      set((state) => ({
        tableData: {
          ...state.tableData,
          [key]: {
            connectionId,
            schema,
            table,
            columns: result.columns,
            rows: result.rows,
            page: result.page,
            pageSize: result.pageSize,
            totalRows,
            runtimeMs: result.runtimeMs,
          },
        },
        tablePreviews: {
          ...state.tablePreviews,
          [table]: {
            columns: result.columns,
            rows: result.rows,
            rowCount:
              totalRows !== undefined
                ? totalRows.toString()
                : result.rows.length.toString(),
            primaryKey: "id",
            size: "--",
            lastVacuum: "--",
          },
        },
        tableLoadStatus: {
          ...state.tableLoadStatus,
          [table]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load table data", error);
      set((state) => ({
        tableLoadStatus: {
          ...state.tableLoadStatus,
          [table]: { state: "error", error: message },
        },
      }));
    }
  },

  refreshTableData: async (key) => {
    const existing = get().tableData[key];
    if (!existing) {
      return;
    }
    await get().loadTableData(
      existing.connectionId,
      existing.schema,
      existing.table,
      existing.page,
      existing.pageSize,
    );
  },

  loadTableStructure: async (connectionId, schema, table) => {
    if (!connectionId) {
      return;
    }
    const key = tableStructureKey(connectionId, schema, table);
    set((state) => ({
      tableStructureStatus: {
        ...state.tableStructureStatus,
        [key]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      // In non-Tauri environments (browser preview, tests without mocks)
      // we cannot fetch real metadata; mark idle and bail.
      set((state) => ({
        tableStructureStatus: {
          ...state.tableStructureStatus,
          [key]: { state: "idle" },
        },
      }));
      return;
    }
    try {
      const result = await tauriInvoke<TableStructure>("load_table_structure", {
        payload: {
          connectionId,
          schema,
          table,
        },
      });
      set((state) => ({
        tableStructure: {
          ...state.tableStructure,
          [key]: result,
        },
        tableStructureStatus: {
          ...state.tableStructureStatus,
          [key]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load table structure", error);
      set((state) => ({
        tableStructureStatus: {
          ...state.tableStructureStatus,
          [key]: { state: "error", error: message },
        },
      }));
    }
  },

  loadSchemaRelationships: async (connectionId, schema) => {
    if (!connectionId) {
      return;
    }
    const key = schemaRelationshipsKey(connectionId, schema);
    set((state) => ({
      schemaRelationshipsStatus: {
        ...state.schemaRelationshipsStatus,
        [key]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      // Non-Tauri environments (browser preview, unmocked tests) cannot
      // hit the backend; mark idle and bail.
      set((state) => ({
        schemaRelationshipsStatus: {
          ...state.schemaRelationshipsStatus,
          [key]: { state: "idle" },
        },
      }));
      return;
    }
    try {
      const result = await tauriInvoke<{
        tables: SchemaTableNode[];
        foreignKeys: SchemaForeignKey[];
      }>("load_schema_relationships", {
        payload: { connectionId, schema },
      });
      set((state) => ({
        schemaRelationships: {
          ...state.schemaRelationships,
          [key]: {
            tables: result.tables,
            foreignKeys: result.foreignKeys,
          },
        },
        schemaRelationshipsStatus: {
          ...state.schemaRelationshipsStatus,
          [key]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load schema relationships", error);
      set((state) => ({
        schemaRelationshipsStatus: {
          ...state.schemaRelationshipsStatus,
          [key]: { state: "error", error: message },
        },
      }));
    }
  },

  loadDatabaseOverviewStats: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    set((state) => ({
      databaseOverviewStatsStatus: {
        ...state.databaseOverviewStatsStatus,
        [connectionId]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      set((state) => ({
        databaseOverviewStatsStatus: {
          ...state.databaseOverviewStatsStatus,
          [connectionId]: { state: "idle" },
        },
      }));
      return;
    }
    try {
      const result = await tauriInvoke<DatabaseOverviewStats>(
        "load_database_overview_stats",
        {
          payload: { connectionId },
        },
      );
      set((state) => ({
        databaseOverviewStats: {
          ...state.databaseOverviewStats,
          [connectionId]: result,
        },
        databaseOverviewStatsStatus: {
          ...state.databaseOverviewStatsStatus,
          [connectionId]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load database overview stats", error);
      set((state) => ({
        databaseOverviewStatsStatus: {
          ...state.databaseOverviewStatsStatus,
          [connectionId]: { state: "error", error: message },
        },
      }));
    }
  },

  focusTableInSchemaMap: (connectionId, schema, table) => {
    // Use the existing tab-opening flow: it focuses an existing tab when
    // present and creates one otherwise. We pin activeConnectionId to
    // match the requested schema so the workspace shows the right context.
    set({ activeConnectionId: connectionId });
    get().openTableTab(schema, table);
  },

  addPendingStructureChange: (key, entry) =>
    set((state) => {
      const existing = state.pendingStructureChanges[key] ?? [];
      const next: PendingChange = {
        id: generatePendingId(),
        schema: entry.schema,
        table: entry.table,
        change: entry.change,
      };
      return {
        pendingStructureChanges: {
          ...state.pendingStructureChanges,
          [key]: [...existing, next],
        },
      };
    }),

  removePendingStructureChange: (key, id) =>
    set((state) => {
      const existing = state.pendingStructureChanges[key];
      if (!existing) {
        return {};
      }
      return {
        pendingStructureChanges: {
          ...state.pendingStructureChanges,
          [key]: existing.filter((entry) => entry.id !== id),
        },
      };
    }),

  clearPendingStructureChanges: (key) =>
    set((state) => {
      if (!(key in state.pendingStructureChanges)) {
        return {};
      }
      const { [key]: _, ...rest } = state.pendingStructureChanges;
      return { pendingStructureChanges: rest };
    }),

  commitStructureChanges: async (key) => {
    const state = get();
    const pending = state.pendingStructureChanges[key];
    if (!pending || pending.length === 0) {
      return;
    }
    // Pending entries are added as a unit per (schema, table) so we can
    // safely take schema/table from the first entry.
    const { schema, table } = pending[0];
    // The key is `${connectionId}::${schema}::${table}`; derive connection.
    const connectionId = key.split("::")[0] ?? "";
    const connection = state.connections.find((c) => c.id === connectionId);
    if (!connection) {
      set((state) => ({
        structureCommitStatus: {
          ...state.structureCommitStatus,
          [key]: {
            state: "error",
            error: "Connection not found for this table.",
          },
        },
      }));
      return;
    }
    const ddlStructure = state.tableStructure[key];
    // Defer to the backend when structure isn't loaded — it'll surface a
    // clear engine-specific error rather than us pre-emptively guessing.
    if (ddlStructure && !ddlStructure.capabilities.canAlterSchema) {
      set((state) => ({
        structureCommitStatus: {
          ...state.structureCommitStatus,
          [key]: {
            state: "error",
            error: `This table does not support schema edits on ${connection.engine}.`,
          },
        },
      }));
      return;
    }

    const sql = generateDdlForEngine(
      connection.engine,
      schema,
      table,
      pending.map((entry) => entry.change),
      ddlStructure?.columns,
    );

    set((state) => ({
      structureCommitStatus: {
        ...state.structureCommitStatus,
        [key]: { state: "running" },
      },
    }));

    if (!isTauri()) {
      // No backend in browser preview / unmocked tests: leave pending in
      // place and surface a clear status.
      set((state) => ({
        structureCommitStatus: {
          ...state.structureCommitStatus,
          [key]: {
            state: "error",
            error: "Backend is unavailable in this environment.",
          },
        },
      }));
      return;
    }

    try {
      const result = await tauriInvoke<{ runtimeMs: number }>("execute_ddl", {
        payload: { connectionId, sql },
      });
      // Clear pending on success and refresh structure metadata.
      set((state) => {
        const { [key]: _, ...rest } = state.pendingStructureChanges;
        return {
          pendingStructureChanges: rest,
          structureCommitStatus: {
            ...state.structureCommitStatus,
            [key]: { state: "success", runtimeMs: result.runtimeMs },
          },
        };
      });
      await get().loadTableStructure(connectionId, schema, table);
      // Best-effort: also refresh data if it was previously loaded.
      const dataKey = tableDataKey(connectionId, schema, table);
      if (get().tableData[dataKey]) {
        await get().refreshTableData(dataKey);
      }
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to commit structure changes", error);
      set((state) => ({
        structureCommitStatus: {
          ...state.structureCommitStatus,
          [key]: { state: "error", error: message },
        },
      }));
    }
  },

  loadConnections: async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("load_connections");
      const connections = stored.map(hydrateConnection);
      set((state) => ({
        connections,
        activeConnectionId: connections.some(
          (connection) => connection.id === state.activeConnectionId,
        )
          ? state.activeConnectionId
          : (connections[0]?.id ?? ""),
      }));
    } catch (error) {
      console.error("Failed to load connections", error);
    }
  },

  loadQueryHistory: async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const stored =
        await tauriInvoke<QueryHistoryEntry[]>("load_query_history");
      set({ queryHistory: stored });
    } catch (error) {
      console.error("Failed to load query history", error);
    }
  },

  loadSavedQueries: async () => {
    if (!isTauri()) {
      set({ savedQueriesStatus: { state: "success" } });
      return;
    }
    set({ savedQueriesStatus: { state: "loading" } });
    try {
      const stored = await tauriInvoke<SavedQuery[]>("load_saved_queries");
      set({ savedQueries: stored, savedQueriesStatus: { state: "success" } });
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load saved queries", error);
      set({
        savedQueriesStatus: { state: "error", error: message },
      });
    }
  },

  saveSavedQuery: async (input) => {
    const now = new Date().toISOString();
    // Backend rewrites updatedAt; we send a placeholder. createdAt is filled
    // server-side on insert.
    const payload: SavedQuery = {
      id: input.id,
      name: input.name,
      body: input.body,
      connectionId: input.connectionId,
      isFavorite: input.isFavorite,
      ownerId: input.ownerId ?? null,
      createdAt: input.createdAt ?? now,
      updatedAt: input.updatedAt ?? now,
    };
    if (!isTauri()) {
      set((state) => ({
        savedQueries: [
          payload,
          ...state.savedQueries.filter((q) => q.id !== payload.id),
        ],
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<SavedQuery[]>("save_saved_query", {
        query: payload,
      });
      set({ savedQueries: stored });
    } catch (error) {
      console.error("Failed to save saved query", error);
    }
  },

  deleteSavedQuery: async (id) => {
    if (!isTauri()) {
      set((state) => ({
        savedQueries: state.savedQueries.filter((q) => q.id !== id),
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<SavedQuery[]>("delete_saved_query", {
        payload: { id },
      });
      set({ savedQueries: stored });
    } catch (error) {
      console.error("Failed to delete saved query", error);
    }
  },

  reopenHistoryEntry: (entry) => {
    const state = get();
    set({
      activeView: "workspace",
      activeConnectionId: entry.connectionId,
    });
    const existing = state.workspaceTabs.find(
      (item) =>
        item.kind === "query" &&
        item.connectionId === entry.connectionId &&
        (item.query ?? "") === entry.sql,
    );
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const id = `tab-${nextTabIndex}`;
    nextTabIndex += 1;
    const label = `query_${nextQueryIndex}.sql`;
    nextQueryIndex += 1;
    set((state) => ({
      workspaceTabs: [
        ...state.workspaceTabs,
        {
          id,
          kind: "query",
          label,
          connectionId: entry.connectionId,
          schema: "",
          query: entry.sql,
        },
      ],
      activeTabId: id,
    }));
  },

  addConnection: async (connection) => {
    if (!isTauri()) {
      set((state) => ({
        connections: [...state.connections, connection],
        activeConnectionId: connection.id,
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("save_connection", {
        connection: toStoredConnection(connection),
      });
      const connections = stored.map(hydrateConnection);
      set({ connections, activeConnectionId: connection.id });
    } catch (error) {
      console.error("Failed to save connection", error);
      set((state) => ({
        connections: [...state.connections, connection],
        activeConnectionId: connection.id,
      }));
    }
  },

  updateConnection: async (connection) => {
    if (!isTauri()) {
      set((state) => ({
        connections: state.connections.map((c) =>
          c.id === connection.id ? connection : c,
        ),
      }));
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>("save_connection", {
        connection: toStoredConnection(connection),
      });
      const connections = stored.map(hydrateConnection);
      // Preserve the connection status for the updated connection
      const currentConnection = get().connections.find(
        (c) => c.id === connection.id,
      );
      if (currentConnection) {
        const updatedConnections = connections.map((c) =>
          c.id === connection.id
            ? {
                ...c,
                status: currentConnection.status,
                latency: currentConnection.latency,
                lastSync: currentConnection.lastSync,
              }
            : c,
        );
        set({ connections: updatedConnections });
      } else {
        set({ connections });
      }
    } catch (error) {
      console.error("Failed to update connection", error);
    }
  },

  deleteConnection: async (connectionId) => {
    if (!isTauri()) {
      set((state) => {
        const connections = state.connections.filter(
          (c) => c.id !== connectionId,
        );
        const newActiveId =
          state.activeConnectionId === connectionId
            ? (connections[0]?.id ?? "")
            : state.activeConnectionId;
        return {
          connections,
          activeConnectionId: newActiveId,
          schemaExplorer: Object.fromEntries(
            Object.entries(state.schemaExplorer).filter(
              ([key]) => key !== connectionId,
            ),
          ),
        };
      });
      return;
    }
    try {
      const stored = await tauriInvoke<StoredConnection[]>(
        "delete_connection",
        {
          payload: { connectionId },
        },
      );
      const connections = stored.map(hydrateConnection);
      set((state) => {
        const newActiveId =
          state.activeConnectionId === connectionId
            ? (connections[0]?.id ?? "")
            : state.activeConnectionId;
        return {
          connections,
          activeConnectionId: newActiveId,
          schemaExplorer: Object.fromEntries(
            Object.entries(state.schemaExplorer).filter(
              ([key]) => key !== connectionId,
            ),
          ),
        };
      });
    } catch (error) {
      console.error("Failed to delete connection", error);
    }
  },

  testConnection: async (connection) => {
    if (!isTauri()) {
      // In dev/storybook mode we can't actually connect — pretend it
      // succeeded so the UI flow stays exercised.
      return { ok: true, latencyMs: 0 };
    }
    try {
      const result = await tauriInvoke<ConnectResult>("test_connection", {
        payload: {
          connection: {
            id: connection.id,
            name: connection.name,
            database: connection.database,
            engine: connection.engine,
            host: connection.host,
            port: connection.port,
            user: connection.user,
            password: connection.password,
            role: connection.role,
            useHttps: connection.useHttps ?? false,
            urlPath: connection.urlPath ?? "",
          },
        },
      });
      return { ok: true, latencyMs: result.latencyMs };
    } catch (error) {
      return { ok: false, error: errorToMessage(error) };
    }
  },

  runHealthChecks: async () => {
    if (!isTauri()) {
      return;
    }
    const connectionIds = get().connections.map((c) => c.id);
    if (connectionIds.length === 0) {
      return;
    }
    // Fan out in parallel; per-connection failures are local and shouldn't
    // block siblings.
    const results = await Promise.all(
      connectionIds.map(async (connectionId) => {
        try {
          const result = await tauriInvoke<
            | { state: "healthy"; latencyMs: number }
            | { state: "error"; error: string }
          >("health_check_connection", {
            payload: { connectionId },
          });
          return { connectionId, result };
        } catch (error) {
          return {
            connectionId,
            result: {
              state: "error" as const,
              error: errorToMessage(error),
            },
          };
        }
      }),
    );
    set((state) => {
      const next = state.connections.map((connection) => {
        const found = results.find((r) => r.connectionId === connection.id);
        if (!found) return connection;
        if (found.result.state === "healthy") {
          // Don't downgrade an explicit "Read only" status; the health-check
          // only proves reachability, not write capability.
          const status: Connection["status"] =
            connection.status === "Read only" ? "Read only" : "Connected";
          return {
            ...connection,
            status,
            latency: formatLatencyMs(found.result.latencyMs),
            lastSync: new Date().toISOString(),
            errorMessage: undefined,
          };
        }
        return {
          ...connection,
          status: "Disconnected" as const,
          errorMessage: found.result.error,
        };
      });
      return { connections: next };
    });
  },

  connectConnection: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    set({ activeConnectionId: connectionId });
    if (!isTauri()) {
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Connected",
          lastSync: "Just now",
          errorMessage: undefined,
        }),
      }));
      return;
    }
    try {
      const result = await tauriInvoke<ConnectResult>("connect_connection", {
        payload: { connectionId },
      });
      const schema = await tauriInvoke<SchemaExplorer[]>(
        "load_schema_explorer",
        { payload: { connectionId } },
      );
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Connected",
          latency: formatLatencyMs(result.latencyMs),
          lastSync: "Just now",
          lastActivityAt: new Date().toISOString(),
          errorMessage: undefined,
        }),
        schemaExplorer: {
          ...state.schemaExplorer,
          [connectionId]: schema,
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to connect", error);
      set((state) => ({
        connections: applyConnectionUpdate(state.connections, connectionId, {
          status: "Disconnected",
          errorMessage: message,
        }),
      }));
    }
  },

  updateQuery: (tabId, query) =>
    set((state) => ({
      workspaceTabs: state.workspaceTabs.map((tab) =>
        tab.id === tabId ? { ...tab, query, isDirty: true } : tab,
      ),
    })),

  runQuery: async (tabId, options) => {
    const state = get();
    const tab = state.workspaceTabs.find((item) => item.id === tabId);
    if (!tab || tab.kind !== "query") {
      return;
    }
    if (state.queryStatus[tabId]?.state === "running") {
      return;
    }
    const fullText = tab.query ?? "";
    const overrideSql = options?.overrideSql ?? null;
    const query = pickSqlToRun(fullText, overrideSql).trim();
    if (!query) {
      return;
    }
    if (!isTauri()) {
      set((state) => ({
        workspaceTabs: state.workspaceTabs.map((item) =>
          item.id === tabId
            ? { ...item, lastRun: "Just now", isDirty: false }
            : item,
        ),
      }));
      return;
    }
    const connectionAtRun = state.connections.find(
      (c) => c.id === tab.connectionId,
    );
    const startedAt = new Date().toISOString();
    set((state) => ({
      queryStatus: {
        ...state.queryStatus,
        [tabId]: { state: "running" },
      },
    }));
    const buildEntry = (
      base: Pick<QueryHistoryEntry, "status" | "runtimeMs"> &
        Partial<Pick<QueryHistoryEntry, "rowCount" | "errorMessage">>,
    ): QueryHistoryEntry => ({
      id: generateHistoryId(),
      sql: query,
      connectionId: tab.connectionId,
      connectionName: connectionAtRun?.name ?? "",
      database: connectionAtRun?.database ?? "",
      engine: connectionAtRun?.engine ?? "PostgreSQL",
      status: base.status,
      errorMessage: base.errorMessage,
      runtimeMs: base.runtimeMs,
      rowCount: base.rowCount,
      startedAt,
    });
    const persistEntry = async (entry: QueryHistoryEntry) => {
      try {
        await tauriInvoke<QueryHistoryEntry[]>("append_query_history", {
          entry,
        });
      } catch (error) {
        // Persistence is best-effort: never block the UI on a write failure.
        console.error("Failed to persist query history entry", error);
      }
    };
    try {
      const result = await tauriInvoke<RunQueryResult>("run_query", {
        payload: { connectionId: tab.connectionId, query },
      });
      const entry = buildEntry({
        status: "success",
        runtimeMs: result.runtimeMs,
        rowCount: result.rowCount,
      });
      const nowIso = new Date().toISOString();
      set((state) => ({
        queryPreviews: {
          ...state.queryPreviews,
          [tab.label]: {
            columns: result.columns,
            rows: result.rows,
            runtime: `${result.runtimeMs} ms`,
            rowCount: result.rowCount.toString(),
            cache: "Cold",
          },
        },
        queryStatus: {
          ...state.queryStatus,
          [tabId]: { state: "success", runtimeMs: result.runtimeMs },
        },
        queryHistory: [entry, ...state.queryHistory].slice(0, 200),
        workspaceTabs: state.workspaceTabs.map((item) =>
          item.id === tabId
            ? { ...item, lastRun: "Just now", isDirty: false }
            : item,
        ),
        connections: applyConnectionUpdate(
          state.connections,
          tab.connectionId,
          { lastActivityAt: nowIso },
        ),
      }));
      await persistEntry(entry);
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to run query", error);
      const entry = buildEntry({
        status: "error",
        runtimeMs: Math.max(0, Date.now() - new Date(startedAt).getTime()),
        errorMessage: message,
      });
      set((state) => ({
        queryStatus: {
          ...state.queryStatus,
          [tabId]: { state: "error", error: message },
        },
        queryHistory: [entry, ...state.queryHistory].slice(0, 200),
        workspaceTabs: state.workspaceTabs.map((item) =>
          item.id === tabId
            ? { ...item, lastRun: "Failed", isDirty: false }
            : item,
        ),
      }));
      await persistEntry(entry);
    }
  },

  closeTab: (tabId) =>
    set((state) => {
      const index = state.workspaceTabs.findIndex((tab) => tab.id === tabId);
      if (index === -1) {
        return {};
      }
      const nextTabs = state.workspaceTabs.filter((tab) => tab.id !== tabId);
      let nextActiveTabId = state.activeTabId;

      if (tabId === state.activeTabId) {
        const nextTab = nextTabs[index] ?? nextTabs[index - 1];
        nextActiveTabId = nextTab?.id ?? "";
      }

      return { workspaceTabs: nextTabs, activeTabId: nextActiveTabId };
    }),

  openWorkspaceTab: (tab) => {
    const state = get();
    set({ activeView: "workspace", activeConnectionId: tab.connectionId });

    const existing = state.workspaceTabs.find(
      (item) =>
        item.kind === tab.kind &&
        item.label === tab.label &&
        item.connectionId === tab.connectionId,
    );

    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }

    const id = `tab-${nextTabIndex}`;
    nextTabIndex += 1;
    set((state) => ({
      workspaceTabs: [...state.workspaceTabs, { ...tab, id }],
      activeTabId: id,
    }));
  },

  openTableTab: (schemaName, tableName) => {
    const connectionId = get().activeConnectionId;
    get().openWorkspaceTab({
      kind: "table",
      label: tableName,
      connectionId,
      schema: schemaName,
      table: tableName,
    });
    if (connectionId) {
      void get().loadTableData(connectionId, schemaName, tableName);
    }
  },

  openViewTab: (schemaName, viewName) => {
    get().openWorkspaceTab({
      kind: "query",
      label: `${viewName}.sql`,
      connectionId: get().activeConnectionId,
      schema: schemaName,
      query: `select * from ${schemaName}.${viewName} limit 100;`,
    });
  },

  openQueryForTable: (schemaName, tableName) => {
    const state = get();
    const queryLabel = `query_${nextQueryIndex}.sql`;
    nextQueryIndex += 1;

    get().openWorkspaceTab({
      kind: "query",
      label: queryLabel,
      connectionId: state.activeConnectionId,
      schema: schemaName,
      query: `select * from ${schemaName}.${tableName} limit 100;`,
    });
  },

  createNewQueryTab: () => {
    const state = get();
    const explorerSchemas =
      state.schemaExplorer[state.activeConnectionId] ?? [];
    const schemaName = explorerSchemas[0]?.name ?? "public";
    const queryLabel = `query_${nextQueryIndex}.sql`;
    nextQueryIndex += 1;

    get().openWorkspaceTab({
      kind: "query",
      label: queryLabel,
      connectionId: state.activeConnectionId,
      schema: schemaName,
      query: `select * from ${schemaName}.users limit 50;`,
    });
  },

  createNewTableTab: () => {
    const state = get();
    const explorerSchemas =
      state.schemaExplorer[state.activeConnectionId] ?? [];
    const schemaName = explorerSchemas[0]?.name;
    const tableName = explorerSchemas[0]?.tables[0];
    if (!schemaName || !tableName) {
      return;
    }
    get().openTableTab(schemaName, tableName);
  },

  toggleSchema: (schemaName) => {
    const state = get();
    const schemaId = `${state.activeConnectionId}:${schemaName}`;
    set((state) => ({
      expandedSchemas: state.expandedSchemas.includes(schemaId)
        ? state.expandedSchemas.filter((item) => item !== schemaId)
        : [...state.expandedSchemas, schemaId],
    }));
  },
}));
