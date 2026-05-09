import { create } from "zustand";
import { pickSqlToRun } from "@/lib/sql";
import { isTauri, tauriInvoke } from "@/lib/tauri";

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
  errorMessage?: string;
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
};

export type TableStructure = {
  columns: ColumnInfo[];
  primaryKey: string[] | null;
  foreignKeys: ForeignKeyInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  capabilities: StructureCapabilities;
};

export type TableStructureStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

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

export type SchemaFlow = {
  nodes: {
    id: string;
    position: { x: number; y: number };
    data: { label: string };
  }[];
  edges: {
    id: string;
    source: string;
    target: string;
    label: string;
    type: string;
  }[];
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
  queryEdits: Record<string, Record<number, Record<number, string>>>;
  tableEdits: Record<string, Record<number, Record<number, string>>>;
  schemaFlows: Record<string, SchemaFlow>;
  recentQueries: string[];
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
  loadConnections: () => Promise<void>;
  addConnection: (connection: Connection) => Promise<void>;
  updateConnection: (connection: Connection) => Promise<void>;
  deleteConnection: (connectionId: string) => Promise<void>;
  connectConnection: (connectionId: string) => Promise<void>;
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
const initialQueryEdits: Record<
  string,
  Record<number, Record<number, string>>
> = {};
const initialTableEdits: Record<
  string,
  Record<number, Record<number, string>>
> = {};
const initialSchemaFlows: Record<string, SchemaFlow> = {};
const initialRecentQueries: string[] = [];

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
  queryEdits: initialQueryEdits,
  tableEdits: initialTableEdits,
  schemaFlows: initialSchemaFlows,
  recentQueries: initialRecentQueries,
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
          ...(state.queryEdits[tabId] ?? {}),
          [rowIndex]: {
            ...(state.queryEdits[tabId]?.[rowIndex] ?? {}),
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
          ...(state.tableEdits[tableName] ?? {}),
          [rowIndex]: {
            ...(state.tableEdits[tableName]?.[rowIndex] ?? {}),
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
          latency: result.latencyMs ? `${result.latencyMs} ms` : "--",
          lastSync: "Just now",
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
    set((state) => ({
      queryStatus: {
        ...state.queryStatus,
        [tabId]: { state: "running" },
      },
    }));
    try {
      const result = await tauriInvoke<RunQueryResult>("run_query", {
        payload: { connectionId: tab.connectionId, query },
      });
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
        recentQueries: [query, ...state.recentQueries].slice(0, 10),
        workspaceTabs: state.workspaceTabs.map((item) =>
          item.id === tabId
            ? { ...item, lastRun: "Just now", isDirty: false }
            : item,
        ),
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to run query", error);
      set((state) => ({
        queryStatus: {
          ...state.queryStatus,
          [tabId]: { state: "error", error: message },
        },
        workspaceTabs: state.workspaceTabs.map((item) =>
          item.id === tabId
            ? { ...item, lastRun: "Failed", isDirty: false }
            : item,
        ),
      }));
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
