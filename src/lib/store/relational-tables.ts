/**
 * Relational Tables slice — owns every relational-class state map
 * keyed by `${connectionId}::${schema}::${table}` (or by connectionId
 * for overview stats), plus the schema-explorer tree and the cell-
 * edit / DDL / table-data mutation flows.
 *
 * Exposes `dropRelationalCachesForConnection(connectionId)` as its
 * piece of the delete-connection cleanup cascade. Today
 * `Connections.deleteConnection` clears `schemaExplorer` inline; once
 * this method is wired in (separate behaviour-fix PR) it'll clean up
 * every per-connection-keyed cache here.
 */

import type { StateCreator } from "zustand";

import { generateDdlForEngine, type PendingChange } from "@/lib/ddl";
import { pendingMutationsFromResult } from "@/lib/pending-mutations";
import {
  type SchemaForeignKey,
  type SchemaRelationships,
  type SchemaTableNode,
  schemaRelationshipsKey,
} from "@/lib/schema-graph";
import { clearLifecycleSlot } from "@/lib/store-lifecycle";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import {
  buildDeleteRowsPayload,
  buildEditPayload,
  pollMutationsToCompletion,
  resolveEditContext,
  resolveStructureCommitContext,
} from "./edit-strategies";
import type {
  AppStoreState,
  ColumnChangeKind,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  DDLOutcome,
  EditOutcome,
  QueryStatus,
  RelationInfo,
  RelationStatsStatus,
  SchemaExplorer,
  SchemaRelationshipsStatus,
  ServerDetails,
  ServerDetailsStatus,
  StructureCommitStatus,
  TableDataState,
  TableEditsCommitStatus,
  TableLoadStatus,
  TablePreviewData,
  TableStructure,
  TableStructureStatus,
} from "./types";
import { tableDataKey, tableStructureKey } from "./types";

// ---------------------------------------------------------------------------
// Private helpers and shapes
// ---------------------------------------------------------------------------

type TableDataResult = {
  columns: string[];
  rows: string[][];
  page: number;
  pageSize: number;
  totalRows?: number | null;
  runtimeMs: number;
};

type CommitCellEditsResult = {
  rowsAffected: number;
  runtimeMs: number;
  state?: "committed" | "queued";
  database?: string;
  table?: string;
  mutationIds?: string[];
};

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

// ---------------------------------------------------------------------------
// Slice
// ---------------------------------------------------------------------------

export type RelationalTablesSlice = {
  expandedSchemas: string[];
  schemaExplorer: Record<string, SchemaExplorer[]>;
  tablePreviews: Record<string, TablePreviewData>;
  tableData: Record<string, TableDataState>;
  tableStructure: Record<string, TableStructure>;
  tableLoadStatus: Record<string, TableLoadStatus>;
  tableStructureStatus: Record<string, TableStructureStatus>;
  pendingStructureChanges: Record<string, PendingChange[]>;
  structureCommitStatus: Record<string, StructureCommitStatus>;
  tableEdits: Record<string, Record<number, Record<number, string>>>;
  tableEditsCommitStatus: Record<string, TableEditsCommitStatus>;
  schemaRelationships: Record<string, SchemaRelationships>;
  schemaRelationshipsStatus: Record<string, SchemaRelationshipsStatus>;
  databaseOverviewStats: Record<string, DatabaseOverviewStats>;
  databaseOverviewStatsStatus: Record<string, DatabaseOverviewStatsStatus>;
  /**
   * Per-connection list of relations (tables, views, matviews) used by
   * the Tables and Schemas sub-tabs. Lazy on first activation of
   * either sub-tab. Invalidated on DDL commit and dropped on
   * disconnect/delete. Empty list on non-PG engines.
   */
  relationStats: Record<string, RelationInfo[]>;
  relationStatsStatus: Record<string, RelationStatsStatus>;
  /**
   * Per-connection server-info snapshot used by the Details sub-tab.
   * Lazy on first activation. Dropped on disconnect/delete. Postgres-
   * only — the Details sub-tab is gated to PG in the UI and never
   * invokes the loader on other engines.
   */
  serverDetails: Record<string, ServerDetails>;
  serverDetailsStatus: Record<string, ServerDetailsStatus>;

  setExpandedSchemas: (
    schemas: string[] | ((prev: string[]) => string[]),
  ) => void;
  toggleSchema: (schemaName: string) => void;
  focusTableInSchemaMap: (
    connectionId: string,
    schema: string,
    table: string,
  ) => void;

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
  loadRelationStats: (connectionId: string) => Promise<void>;
  loadServerDetails: (connectionId: string) => Promise<void>;

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

  addPendingStructureChange: (
    key: string,
    entry: { schema: string; table: string; change: ColumnChangeKind },
  ) => void;
  removePendingStructureChange: (key: string, id: string) => void;
  clearPendingStructureChanges: (key: string) => void;
  commitStructureChanges: (key: string) => Promise<DDLOutcome>;

  /**
   * Cascade cleanup — drops every per-connection cache entry. Not
   * called by `Connections.deleteConnection` today (the monolith only
   * cleaned `schemaExplorer`); wiring it up is a deliberate
   * behaviour-fix follow-up.
   */
  dropRelationalCachesForConnection: (connectionId: string) => void;
};

const dropMatching = <T>(
  bag: Record<string, T>,
  matcher: (key: string) => boolean,
): Record<string, T> => {
  const next: Record<string, T> = {};
  for (const [key, value] of Object.entries(bag)) {
    if (!matcher(key)) {
      next[key] = value;
    }
  }
  return next;
};

export const createRelationalTablesSlice: StateCreator<
  AppStoreState,
  [],
  [],
  RelationalTablesSlice
> = (set, get) => ({
  expandedSchemas: [],
  schemaExplorer: {},
  tablePreviews: {},
  tableData: {},
  tableStructure: {},
  tableLoadStatus: {},
  tableStructureStatus: {},
  pendingStructureChanges: {},
  structureCommitStatus: {},
  tableEdits: {},
  tableEditsCommitStatus: {},
  schemaRelationships: {},
  schemaRelationshipsStatus: {},
  databaseOverviewStats: {},
  databaseOverviewStatsStatus: {},
  relationStats: {},
  relationStatsStatus: {},
  serverDetails: {},
  serverDetailsStatus: {},

  setExpandedSchemas: (schemas) =>
    set((state) => ({
      expandedSchemas:
        typeof schemas === "function"
          ? schemas(state.expandedSchemas)
          : schemas,
    })),

  toggleSchema: (schemaName) => {
    const state = get();
    const schemaId = `${state.activeConnectionId}:${schemaName}`;
    set((state) => ({
      expandedSchemas: state.expandedSchemas.includes(schemaId)
        ? state.expandedSchemas.filter((item) => item !== schemaId)
        : [...state.expandedSchemas, schemaId],
    }));
  },

  focusTableInSchemaMap: (connectionId, schema, table) => {
    // Cross-slice: also sets activeConnectionId (connections slice) and
    // routes through openTableTab (workspace-tabs slice).
    set({ activeConnectionId: connectionId });
    get().openTableTab(schema, table);
  },

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

  commitTableEdits: async (tableName): Promise<EditOutcome> => {
    const state = get();
    const editsForTable = state.tableEdits[tableName];
    if (!editsForTable || Object.keys(editsForTable).length === 0) {
      return { kind: "noop" };
    }

    const ctx = resolveEditContext({
      tableData: state.tableData,
      tableStructure: state.tableStructure,
      connections: state.connections,
      tableName,
      capability: "canUpdateRows",
      action: "cell edits",
    });
    if (!ctx.ok) {
      return { kind: "failed", reason: ctx.reason };
    }

    const editsPayload = buildEditPayload(
      editsForTable,
      ctx.data,
      ctx.identity,
      ctx.columnIndexByName,
    );
    if (editsPayload.length === 0) {
      set((s) => {
        const { [tableName]: _, ...rest } = s.tableEdits;
        return { tableEdits: rest };
      });
      return { kind: "noop" };
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [tableName]: { state: "running" },
      },
    }));
    const clearLifecycle = () =>
      clearLifecycleSlot(set, "tableEditsCommitStatus", tableName);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
    }

    try {
      const result = await tauriInvoke<CommitCellEditsResult>(
        "commit_cell_edits",
        {
          payload: {
            connectionId: ctx.data.connectionId,
            schema: ctx.data.schema,
            table: ctx.data.table,
            edits: editsPayload,
          },
        },
      );
      const pendingMutations = pendingMutationsFromResult(result, {
        connectionId: ctx.data.connectionId,
        database: ctx.data.schema,
        table: ctx.data.table,
      });
      if (pendingMutations.length > 0) {
        set((s) => {
          const { [tableName]: _, ...restEdits } = s.tableEdits;
          return {
            tableEdits: restEdits,
            tableEditsCommitStatus: {
              ...s.tableEditsCommitStatus,
              [tableName]: {
                state: "queued",
                database: pendingMutations[0].database,
                table: pendingMutations[0].table,
                mutationIds: pendingMutations.map((m) => m.id),
                runtimeMs: result.runtimeMs,
              },
            },
          };
        });
        const editOutcome = await pollMutationsToCompletion(pendingMutations);
        clearLifecycle();
        if (editOutcome.kind === "completed") {
          await get().refreshTableData(ctx.dataKey);
        }
        return editOutcome;
      }

      set((s) => {
        const { [tableName]: _edit, ...restEdits } = s.tableEdits;
        const { [tableName]: _status, ...restStatus } =
          s.tableEditsCommitStatus;
        return { tableEdits: restEdits, tableEditsCommitStatus: restStatus };
      });
      await get().refreshTableData(ctx.dataKey);
      return {
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: result.rowsAffected,
      };
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to commit cell edits", error);
      clearLifecycle();
      return { kind: "failed", reason: message };
    }
  },

  addTableRow: async (tableName, values): Promise<EditOutcome> => {
    if (values.length === 0) {
      return {
        kind: "failed",
        reason: "Provide at least one value (or default) to insert.",
      };
    }

    const state = get();
    const dataEntry = Object.entries(state.tableData).find(
      ([, data]) => data.table === tableName,
    );
    if (!dataEntry) {
      return {
        kind: "failed",
        reason: "Table data is not loaded; cannot insert a row.",
      };
    }
    const [dataKeyForTable, data] = dataEntry;

    const connection = state.connections.find(
      (c) => c.id === data.connectionId,
    );
    if (!connection) {
      return { kind: "failed", reason: "Connection not found for this table." };
    }
    const structureKeyForInsert = tableStructureKey(
      data.connectionId,
      data.schema,
      data.table,
    );
    const insertStructure = state.tableStructure[structureKeyForInsert];
    if (insertStructure && !insertStructure.capabilities.canInsertRows) {
      return {
        kind: "failed",
        reason: `This table does not support inserts on ${connection.engine}.`,
      };
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [tableName]: { state: "running" },
      },
    }));

    const clearLifecycle = () =>
      clearLifecycleSlot(set, "tableEditsCommitStatus", tableName);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
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
      clearLifecycle();
      await get().refreshTableData(dataKeyForTable);
      return {
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: result.rowsAffected,
      };
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to insert row", error);
      clearLifecycle();
      return { kind: "failed", reason: message };
    }
  },

  deleteSelectedTableRows: async (
    tableName,
    rowIndices,
  ): Promise<EditOutcome> => {
    if (rowIndices.length === 0) {
      return { kind: "noop" };
    }

    const state = get();
    const ctx = resolveEditContext({
      tableData: state.tableData,
      tableStructure: state.tableStructure,
      connections: state.connections,
      tableName,
      capability: "canDeleteRows",
      action: "row deletes",
    });
    if (!ctx.ok) {
      return { kind: "failed", reason: ctx.reason };
    }

    const rowsPayload = buildDeleteRowsPayload(
      rowIndices,
      ctx.data,
      ctx.identity,
      ctx.columnIndexByName,
    );
    if (rowsPayload.length === 0) {
      return { kind: "noop" };
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [tableName]: { state: "running" },
      },
    }));
    const clearLifecycle = () =>
      clearLifecycleSlot(set, "tableEditsCommitStatus", tableName);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
    }

    try {
      const result = await tauriInvoke<CommitCellEditsResult>("delete_rows", {
        payload: {
          connectionId: ctx.data.connectionId,
          schema: ctx.data.schema,
          table: ctx.data.table,
          rows: rowsPayload,
        },
      });
      const pendingMutations = pendingMutationsFromResult(result, {
        connectionId: ctx.data.connectionId,
        database: ctx.data.schema,
        table: ctx.data.table,
      });
      if (pendingMutations.length > 0) {
        set((s) => ({
          tableEditsCommitStatus: {
            ...s.tableEditsCommitStatus,
            [tableName]: {
              state: "queued",
              database: pendingMutations[0].database,
              table: pendingMutations[0].table,
              mutationIds: pendingMutations.map((m) => m.id),
              runtimeMs: result.runtimeMs,
            },
          },
        }));
        const editOutcome = await pollMutationsToCompletion(pendingMutations);
        clearLifecycle();
        if (editOutcome.kind === "completed") {
          await get().refreshTableData(ctx.dataKey);
        }
        return editOutcome;
      }
      clearLifecycle();
      await get().refreshTableData(ctx.dataKey);
      return {
        kind: "completed",
        runtimeMs: result.runtimeMs,
        rowsAffected: result.rowsAffected,
      };
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to delete rows", error);
      clearLifecycle();
      return { kind: "failed", reason: message };
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
        payload: { connectionId, schema, table, page, pageSize },
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
        payload: { connectionId, schema, table },
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
        { payload: { connectionId } },
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

  loadRelationStats: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    set((state) => ({
      relationStatsStatus: {
        ...state.relationStatsStatus,
        [connectionId]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      set((state) => ({
        relationStatsStatus: {
          ...state.relationStatsStatus,
          [connectionId]: { state: "idle" },
        },
      }));
      return;
    }
    try {
      const result = await tauriInvoke<RelationInfo[]>("load_relation_stats", {
        payload: { connectionId },
      });
      set((state) => ({
        relationStats: {
          ...state.relationStats,
          [connectionId]: result,
        },
        relationStatsStatus: {
          ...state.relationStatsStatus,
          [connectionId]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load relation stats", error);
      set((state) => ({
        relationStatsStatus: {
          ...state.relationStatsStatus,
          [connectionId]: { state: "error", error: message },
        },
      }));
    }
  },

  loadServerDetails: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    set((state) => ({
      serverDetailsStatus: {
        ...state.serverDetailsStatus,
        [connectionId]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      set((state) => ({
        serverDetailsStatus: {
          ...state.serverDetailsStatus,
          [connectionId]: { state: "idle" },
        },
      }));
      return;
    }
    try {
      const result = await tauriInvoke<ServerDetails>("load_server_details", {
        payload: { connectionId },
      });
      set((state) => ({
        serverDetails: {
          ...state.serverDetails,
          [connectionId]: result,
        },
        serverDetailsStatus: {
          ...state.serverDetailsStatus,
          [connectionId]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load server details", error);
      set((state) => ({
        serverDetailsStatus: {
          ...state.serverDetailsStatus,
          [connectionId]: { state: "error", error: message },
        },
      }));
    }
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

  commitStructureChanges: async (key): Promise<DDLOutcome> => {
    const state = get();
    const pending = state.pendingStructureChanges[key];
    if (!pending || pending.length === 0) {
      return { kind: "noop" };
    }
    const ctx = resolveStructureCommitContext({
      pending,
      key,
      connections: state.connections,
      tableStructure: state.tableStructure,
    });
    if (!ctx.ok) {
      return { kind: "failed", reason: ctx.reason };
    }
    const { connection, ddlStructure, schema, table, connectionId } = ctx;

    const sql = generateDdlForEngine(
      connection.engine,
      schema,
      table,
      pending.map((entry) => entry.change),
      ddlStructure?.columns,
    );

    set((s) => ({
      structureCommitStatus: {
        ...s.structureCommitStatus,
        [key]: { state: "running" },
      },
    }));

    const clearLifecycle = () =>
      clearLifecycleSlot(set, "structureCommitStatus", key);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
    }

    try {
      const result = await tauriInvoke<{ runtimeMs: number }>("execute_ddl", {
        payload: { connectionId, sql },
      });
      set((s) => {
        const { [key]: _pending, ...restPending } = s.pendingStructureChanges;
        const { [key]: _status, ...restStatus } = s.structureCommitStatus;
        // Invalidate the per-connection relation-stats cache so the
        // Tables / Schemas sub-tabs re-fetch on next activation. The
        // status slot is dropped alongside so the next call kicks
        // through its loading transition cleanly.
        const { [connectionId]: _stats, ...restRelationStats } =
          s.relationStats;
        const { [connectionId]: _statsStatus, ...restRelationStatsStatus } =
          s.relationStatsStatus;
        return {
          pendingStructureChanges: restPending,
          structureCommitStatus: restStatus,
          relationStats: restRelationStats,
          relationStatsStatus: restRelationStatsStatus,
        };
      });
      // Post-DDL refreshes are independent reads against the same
      // connection and write to disjoint store slices, so they run
      // concurrently — saves ~1 RTT on a remote DB. `loadTableData`
      // is only re-fetched when the data tab has already loaded it
      // once (preserved from the pre-parallel ordering).
      const dataKey = tableDataKey(connectionId, schema, table);
      const refreshes: Promise<unknown>[] = [
        get().loadTableStructure(connectionId, schema, table),
      ];
      if (get().tableData[dataKey]) {
        refreshes.push(get().refreshTableData(dataKey));
      }
      await Promise.all(refreshes);
      return { kind: "completed", runtimeMs: result.runtimeMs };
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to commit structure changes", error);
      clearLifecycle();
      return { kind: "failed", reason: message };
    }
  },

  dropRelationalCachesForConnection: (connectionId) => {
    const prefix = `${connectionId}::`;
    const matches = (key: string) =>
      key === connectionId || key.startsWith(prefix);
    set((state) => {
      const tableNamesForConnection = new Set(
        Object.values(state.tableData)
          .filter((data) => data.connectionId === connectionId)
          .map((data) => data.table),
      );
      const matchesTableName = (key: string) =>
        tableNamesForConnection.has(key);
      return {
        tableEdits: dropMatching(state.tableEdits, matchesTableName),
        tableEditsCommitStatus: dropMatching(
          state.tableEditsCommitStatus,
          matchesTableName,
        ),
        schemaExplorer: dropMatching(
          state.schemaExplorer,
          (k) => k === connectionId,
        ),
        tableData: dropMatching(state.tableData, matches),
        tableStructure: dropMatching(state.tableStructure, matches),
        tableLoadStatus: dropMatching(state.tableLoadStatus, matches),
        tableStructureStatus: dropMatching(state.tableStructureStatus, matches),
        pendingStructureChanges: dropMatching(
          state.pendingStructureChanges,
          matches,
        ),
        structureCommitStatus: dropMatching(
          state.structureCommitStatus,
          matches,
        ),
        schemaRelationships: dropMatching(state.schemaRelationships, matches),
        schemaRelationshipsStatus: dropMatching(
          state.schemaRelationshipsStatus,
          matches,
        ),
        databaseOverviewStats: dropMatching(
          state.databaseOverviewStats,
          (k) => k === connectionId,
        ),
        databaseOverviewStatsStatus: dropMatching(
          state.databaseOverviewStatsStatus,
          (k) => k === connectionId,
        ),
        relationStats: dropMatching(
          state.relationStats,
          (k) => k === connectionId,
        ),
        relationStatsStatus: dropMatching(
          state.relationStatsStatus,
          (k) => k === connectionId,
        ),
        serverDetails: dropMatching(
          state.serverDetails,
          (k) => k === connectionId,
        ),
        serverDetailsStatus: dropMatching(
          state.serverDetailsStatus,
          (k) => k === connectionId,
        ),
      };
    });
  },
});

// Re-export type used by the slice in case external consumers need it
// (today none do, but the type is exported from store/types.ts and
// already available there).
export type { QueryStatus };
