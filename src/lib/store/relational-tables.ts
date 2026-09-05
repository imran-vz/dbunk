/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/require-safety-comment-for-type-assertion -- Persisted table state and database cell values are external boundaries validated here. */
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

import { generateDdlForEngine } from "@/lib/ddl";
import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import {
  applyObjectDdlWithSafetyConfirmation,
  formatObjectDdlError,
  objectDdlRefreshScope,
} from "@/lib/object-ddl";
import { pendingMutationsFromResult } from "@/lib/pending-mutations";
import {
  parseCanonicalPgObjectRefKey,
  pgObjectDdlApplyKey,
  pgObjectDescriptionKey,
} from "@/lib/pg-object-ref";
import {
  DEFAULT_SCHEMA_MAP_PREFS,
  type SchemaForeignKey,
  type SchemaMapPosition,
  type SchemaMapPrefs,
  type SchemaRelationships,
  type SchemaTableNode,
  schemaRelationshipsKey,
  tableSchemaMapScope,
} from "@/lib/schema-graph";
import { clearLifecycleSlot } from "@/lib/store-lifecycle";
import {
  assertStructureChangeCanAppend,
  pendingStructureBatch,
} from "@/lib/structure-changes";
import { browseCellsToGrid } from "@/lib/table-browse";
import {
  buildTableSessionSnapshot,
  resolveTableRefByName,
  tableSessionStructureKey,
} from "@/lib/table-session";
import {
  normalizeTableStructure,
  type TableStructurePayload,
} from "@/lib/table-structure-contract";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import {
  buildDeleteRowsPayload,
  buildEditPayload,
  type EditDataSource,
  pollMutationsToCompletion,
  resolveEditContext,
  resolveStructureCommitContext,
} from "./edit-strategies";
import type {
  AppStoreState,
  DatabaseOverviewStats,
  DatabaseOverviewStatsStatus,
  DDLOutcome,
  EditOutcome,
  LoadingStatus,
  PendingChange,
  QueryStatus,
  RelationInfo,
  RelationStatsStatus,
  SchemaExplorer,
  SchemaRelationshipsStatus,
  ServerDetails,
  ServerDetailsStatus,
  StructureChange,
  StructureCommitStatus,
  TableBrowseTabState,
  TableDataState,
  TableEditsCommitStatus,
  TableLoadStatus,
  TablePreviewData,
  TableRef,
  TableSessionSnapshot,
  PgObjectRef,
  TableStructure,
  TableStructureStatus,
} from "./types";
import {
  isConnectedStatus,
  tableDataKey,
  tableSessionKey,
  tableStructureKey,
} from "./types";

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

const refreshAfterWrite = async (
  get: () => AppStoreState,
  ref: TableRef,
  dataKey: string,
  countMayHaveChanged: boolean,
) => {
  const hasBrowse = Object.values(get().tableBrowses).some(
    (item) =>
      item.connectionId === ref.connectionId &&
      item.schema === ref.schema &&
      item.table === ref.table,
  );
  if (hasBrowse) {
    await get().refreshTableBrowsesForRelation(
      ref.connectionId,
      ref.schema,
      ref.table,
      { invalidateExactCount: countMayHaveChanged },
    );
    return;
  }
  await get().refreshTableData(dataKey);
};

const browseDataSourceFor = (
  state: AppStoreState,
  ref: TableRef,
  tabId?: string,
): EditDataSource | undefined => {
  const matchesTable = (item: TableBrowseTabState) =>
    item.connectionId === ref.connectionId &&
    item.schema === ref.schema &&
    item.table === ref.table &&
    item.result !== null;

  const preferred = tabId ? state.tableBrowses[tabId] : undefined;
  const tab = tabId
    ? preferred && matchesTable(preferred)
      ? preferred
      : undefined
    : Object.values(state.tableBrowses).find(matchesTable);
  if (!tab?.result) return undefined;
  return {
    connectionId: ref.connectionId,
    schema: ref.schema,
    table: ref.table,
    columns: tab.result.columns.map((column) => column.name),
    rows: browseCellsToGrid(tab.result.rows),
    identityKind: tab.result.identity.kind,
    identityColumns: tab.result.identity.columns,
  };
};

type PositionRow = {
  tableId: string;
  x: number;
  y: number;
};

type SchemaMapPrefsWire = {
  routing: SchemaMapPrefs["routing"];
  attrMode: SchemaMapPrefs["attrMode"];
  showTypes: boolean;
  showNulls: boolean;
  showComments: boolean;
};

export type NavigatorGroupKey =
  | "tables"
  | "views"
  | "materializedViews"
  | "foreignTables"
  | "sequences"
  | "functions"
  | "procedures"
  | "aggregates"
  | "types"
  | "domains"
  | "extensions"
  | "eventTriggers"
  | "roles"
  | "tablespaces";

export const navigatorGroupId = (
  connectionId: string,
  schema: string,
  group: NavigatorGroupKey,
): string => `${connectionId}:${schema}:${group}`;

/** Tables default open; every other navigator group defaults closed. */
export const isNavigatorGroupExpanded = (
  expandedGroups: readonly string[],
  groupId: string,
  group: NavigatorGroupKey,
): boolean =>
  group === "tables"
    ? !expandedGroups.includes(groupId)
    : expandedGroups.includes(groupId);

const schemaMapPrefsFromWire = (prefs: SchemaMapPrefsWire): SchemaMapPrefs => ({
  routing: prefs.routing,
  attrMode: prefs.attrMode,
  showTypes: prefs.showTypes,
  showNulls: prefs.showNulls,
  showComments: prefs.showComments,
});

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
  /**
   * Navigator group expansion exceptions. Tables are open when absent;
   * every other group is open when present, so the default serializes to [].
   */
  expandedNavigatorGroups: string[];
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
  schemaMapPositions: Record<
    string,
    Record<string, Record<string, SchemaMapPosition>>
  >;
  schemaMapPositionsStatus: Record<string, Record<string, LoadingStatus>>;
  schemaMapPrefs: Record<string, Record<string, SchemaMapPrefs>>;
  schemaMapPrefsStatus: Record<string, Record<string, LoadingStatus>>;
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
  setExpandedNavigatorGroups: (
    groups: string[] | ((prev: string[]) => string[]),
  ) => void;
  /**
   * Toggle a navigator schema row. Takes the full expansion id
   * (`"<connectionId>:<schemaName>"`) built by the caller — the store
   * must not prefix with `activeConnectionId`, which can differ from
   * the rendered connection (e.g. a keyvalue connection is active while
   * the relational workbench shows the first relational connection).
   */
  toggleSchema: (schemaId: string) => void;
  toggleNavigatorGroup: (groupId: string) => void;
  setSchemaExplorerForConnection: (
    connectionId: string,
    schemas: SchemaExplorer[],
  ) => void;
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
  readTableSession: (ref: TableRef) => TableSessionSnapshot;
  openTableSession: (
    ref: TableRef,
    options?: { page?: number; pageSize?: number },
  ) => Promise<void>;
  refreshTableSession: (ref: TableRef) => Promise<void>;
  loadSchemaRelationships: (
    connectionId: string,
    schema: string,
  ) => Promise<void>;
  loadTableSchemaRelationships: (
    connectionId: string,
    schema: string,
    table: string,
  ) => Promise<void>;
  loadSchemaMapPositions: (
    connectionId: string,
    schema: string,
  ) => Promise<void>;
  saveSchemaMapPosition: (
    connectionId: string,
    schema: string,
    tableId: string,
    x: number,
    y: number,
  ) => Promise<void>;
  resetSchemaMapPositions: (
    connectionId: string,
    schema: string,
  ) => Promise<void>;
  loadSchemaMapPrefs: (connectionId: string, schema: string) => Promise<void>;
  setSchemaMapPref: (
    connectionId: string,
    schema: string,
    patch: Partial<SchemaMapPrefs>,
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
  setTableCellEdit: (
    ref: TableRef,
    rowIndex: number,
    colIndex: number,
    value: string,
  ) => void;
  discardTableEdits: (tableName: string) => void;
  discardTableCellEdits: (ref: TableRef) => void;
  commitTableEdits: (tableName: string) => Promise<EditOutcome>;
  commitTableCellEdits: (ref: TableRef, tabId?: string) => Promise<EditOutcome>;
  addTableRow: (
    tableName: string,
    values: Array<{ column: string; value: string | null }>,
  ) => Promise<EditOutcome>;
  insertTableRow: (
    ref: TableRef,
    values: Array<{ column: string; value: string | null }>,
  ) => Promise<EditOutcome>;
  deleteSelectedTableRows: (
    tableName: string,
    rowIndices: number[],
  ) => Promise<EditOutcome>;
  deleteTableRows: (
    ref: TableRef,
    rowIndices: number[],
    tabId?: string,
  ) => Promise<EditOutcome>;

  addPendingStructureChange: (
    key: string,
    entry: {
      schema: string;
      table: string;
      change: StructureChange;
    },
  ) => void;
  removePendingStructureChange: (key: string, id: string) => void;
  clearPendingStructureChanges: (key: string) => void;
  /**
   * Commit a table's pending structure batch. PostgreSQL `pg-op`
   * batches route through the gated `apply_object_ddl` workflow;
   * `statements` carries the reviewed preview's summaries so typed
   * failures can name the statement they stopped at. `column` batches
   * stay on the legacy frontend-rendered `execute_ddl` path
   * (ClickHouse, plus the MySQL/SQLite dead end).
   */
  commitStructureChanges: (
    key: string,
    options?: { statements?: { summary: string }[] },
  ) => Promise<DDLOutcome>;

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
) => {
  const next: Record<string, T> = {};
  for (const [key, value] of Object.entries(bag)) {
    if (!matcher(key)) {
      next[key] = value;
    }
  }
  return next;
};

const withNestedValue = <T>(
  bag: Record<string, Record<string, T>>,
  connectionId: string,
  schema: string,
  value: T,
) => ({
  ...bag,
  [connectionId]: {
    ...bag[connectionId],
    [schema]: value,
  },
});

const withoutConnection = <T>(
  bag: Record<string, T>,
  connectionId: string,
): Record<string, T> => {
  const { [connectionId]: _dropped, ...rest } = bag;
  return rest;
};

export const createRelationalTablesSlice: StateCreator<
  AppStoreState,
  [],
  [],
  RelationalTablesSlice
> = (set, get) => ({
  expandedSchemas: [],
  expandedNavigatorGroups: [],
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
  schemaMapPositions: {},
  schemaMapPositionsStatus: {},
  schemaMapPrefs: {},
  schemaMapPrefsStatus: {},
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

  setExpandedNavigatorGroups: (groups) =>
    set((state) => ({
      expandedNavigatorGroups:
        typeof groups === "function"
          ? groups(state.expandedNavigatorGroups)
          : groups,
    })),

  toggleSchema: (schemaId) => {
    set((state) => ({
      expandedSchemas: state.expandedSchemas.includes(schemaId)
        ? state.expandedSchemas.filter((item) => item !== schemaId)
        : [...state.expandedSchemas, schemaId],
    }));
  },

  toggleNavigatorGroup: (groupId) => {
    set((state) => ({
      expandedNavigatorGroups: state.expandedNavigatorGroups.includes(groupId)
        ? state.expandedNavigatorGroups.filter((item) => item !== groupId)
        : [...state.expandedNavigatorGroups, groupId],
    }));
  },

  setSchemaExplorerForConnection: (connectionId, schemas) => {
    set((state) => ({
      schemaExplorer: { ...state.schemaExplorer, [connectionId]: schemas },
    }));
  },

  focusTableInSchemaMap: (connectionId, schema, table) => {
    // Cross-slice: also sets activeConnectionId (connections slice) and
    // routes through openTableTab (workspace-tabs slice).
    set({ activeConnectionId: connectionId });
    get().openTableTab(schema, table);
  },

  readTableSession: (ref) => {
    const state = get();
    const key = tableSessionKey(ref);
    const structureKey = tableSessionStructureKey(ref);
    return buildTableSessionSnapshot({
      ref,
      data: state.tableData[key],
      structure: state.tableStructure[structureKey],
      loadStatus: state.tableLoadStatus[key],
      structureStatus: state.tableStructureStatus[structureKey],
      writeStatus: state.tableEditsCommitStatus[key],
      edits: state.tableEdits[key],
    });
  },

  openTableSession: async (ref, options) => {
    await Promise.all([
      get().loadTableData(
        ref.connectionId,
        ref.schema,
        ref.table,
        options?.page,
        options?.pageSize,
      ),
      get().loadTableStructure(ref.connectionId, ref.schema, ref.table),
    ]);
  },

  refreshTableSession: async (ref) => {
    await get().refreshTableData(tableSessionKey(ref));
  },

  setTableEdit: (tableName, rowIndex, colIndex, value) => {
    const resolution = resolveTableRefByName(get().tableData, tableName);
    if (!resolution.ok) {
      console.warn(resolution.reason);
      return;
    }
    get().setTableCellEdit(resolution.ref, rowIndex, colIndex, value);
  },

  setTableCellEdit: (ref, rowIndex, colIndex, value) =>
    set((state) => ({
      tableEdits: {
        ...state.tableEdits,
        [tableSessionKey(ref)]: {
          ...state.tableEdits[tableSessionKey(ref)],
          [rowIndex]: {
            ...state.tableEdits[tableSessionKey(ref)]?.[rowIndex],
            [colIndex]: value,
          },
        },
      },
    })),

  discardTableEdits: (tableName) => {
    const resolution = resolveTableRefByName(get().tableData, tableName);
    if (!resolution.ok) {
      console.warn(resolution.reason);
      return;
    }
    get().discardTableCellEdits(resolution.ref);
  },

  discardTableCellEdits: (ref) =>
    set((state) => {
      const { [tableSessionKey(ref)]: _, ...rest } = state.tableEdits;
      return { tableEdits: rest };
    }),

  commitTableEdits: async (tableName): Promise<EditOutcome> => {
    const resolution = resolveTableRefByName(get().tableData, tableName);
    if (!resolution.ok) {
      if (!resolution.missing) {
        return { kind: "failed", reason: resolution.reason };
      }
      return { kind: "noop" };
    }
    return get().commitTableCellEdits(resolution.ref);
  },

  commitTableCellEdits: async (ref, tabId): Promise<EditOutcome> => {
    const state = get();
    const key = tableSessionKey(ref);
    const editsForTable = state.tableEdits[key];
    if (!editsForTable || Object.keys(editsForTable).length === 0) {
      return { kind: "noop" };
    }

    const ctx = resolveEditContext({
      tableData: state.tableData,
      tableStructure: state.tableStructure,
      connections: state.connections,
      ref,
      dataSource: browseDataSourceFor(state, ref, tabId),
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
        const { [key]: _, ...rest } = s.tableEdits;
        return { tableEdits: rest };
      });
      return { kind: "noop" };
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [key]: { state: "running" },
      },
    }));
    const clearLifecycle = () =>
      clearLifecycleSlot(set, "tableEditsCommitStatus", key);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
    }

    try {
      const result = await invokeWithSafetyConfirmation<CommitCellEditsResult>({
        command: "commit_cell_edits",
        connection: ctx.connection,
        payload: {
          connectionId: ctx.data.connectionId,
          schema: ctx.data.schema,
          table: ctx.data.table,
          edits: editsPayload,
        },
      });
      const pendingMutations = pendingMutationsFromResult(result, {
        connectionId: ctx.data.connectionId,
        database: ctx.data.schema,
        table: ctx.data.table,
      });
      if (pendingMutations.length > 0) {
        set((s) => {
          const { [key]: _, ...restEdits } = s.tableEdits;
          return {
            tableEdits: restEdits,
            tableEditsCommitStatus: {
              ...s.tableEditsCommitStatus,
              [key]: {
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
          await refreshAfterWrite(get, ref, ctx.dataKey, false);
        }
        return editOutcome;
      }

      set((s) => {
        const { [key]: _edit, ...restEdits } = s.tableEdits;
        const { [key]: _status, ...restStatus } = s.tableEditsCommitStatus;
        return { tableEdits: restEdits, tableEditsCommitStatus: restStatus };
      });
      await refreshAfterWrite(get, ref, ctx.dataKey, false);
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
    const resolution = resolveTableRefByName(get().tableData, tableName);
    if (!resolution.ok) {
      return {
        kind: "failed",
        reason: resolution.missing
          ? "Table data is not loaded; cannot insert a row."
          : resolution.reason,
      };
    }
    return get().insertTableRow(resolution.ref, values);
  },

  insertTableRow: async (ref, values): Promise<EditOutcome> => {
    if (values.length === 0) {
      return {
        kind: "failed",
        reason: "Provide at least one value (or default) to insert.",
      };
    }

    const state = get();
    const key = tableSessionKey(ref);
    const data = state.tableData[key] ?? {
      connectionId: ref.connectionId,
      schema: ref.schema,
      table: ref.table,
      columns: [],
      rows: [],
      page: 1,
      pageSize: 0,
      runtimeMs: 0,
    };
    if (!state.tableData[key] && !browseDataSourceFor(state, ref)) {
      return {
        kind: "failed",
        reason: "Table data is not loaded; cannot insert a row.",
      };
    }

    const connection = state.connections.find(
      (c) => c.id === data.connectionId,
    );
    if (!connection) {
      return { kind: "failed", reason: "Connection not found for this table." };
    }
    const insertStructure = state.tableStructure[tableSessionStructureKey(ref)];
    if (insertStructure && !insertStructure.capabilities.canInsertRows) {
      return {
        kind: "failed",
        reason: `This table does not support inserts on ${connection.engine}.`,
      };
    }

    set((s) => ({
      tableEditsCommitStatus: {
        ...s.tableEditsCommitStatus,
        [key]: { state: "running" },
      },
    }));

    const clearLifecycle = () =>
      clearLifecycleSlot(set, "tableEditsCommitStatus", key);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
    }

    try {
      const result = await invokeWithSafetyConfirmation<{
        rowsAffected: number;
        runtimeMs: number;
      }>({
        command: "insert_row",
        connection,
        payload: {
          connectionId: data.connectionId,
          schema: data.schema,
          table: data.table,
          values,
        },
      });
      clearLifecycle();
      await refreshAfterWrite(get, ref, key, true);
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
    const resolution = resolveTableRefByName(get().tableData, tableName);
    if (!resolution.ok) {
      if (!resolution.missing) {
        return { kind: "failed", reason: resolution.reason };
      }
      return { kind: "noop" };
    }
    return get().deleteTableRows(resolution.ref, rowIndices);
  },

  deleteTableRows: async (ref, rowIndices, tabId): Promise<EditOutcome> => {
    if (rowIndices.length === 0) {
      return { kind: "noop" };
    }

    const state = get();
    const key = tableSessionKey(ref);
    const ctx = resolveEditContext({
      tableData: state.tableData,
      tableStructure: state.tableStructure,
      connections: state.connections,
      ref,
      dataSource: browseDataSourceFor(state, ref, tabId),
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
        [key]: { state: "running" },
      },
    }));
    const clearLifecycle = () =>
      clearLifecycleSlot(set, "tableEditsCommitStatus", key);

    if (!isTauri()) {
      clearLifecycle();
      return {
        kind: "failed",
        reason: "Backend is unavailable in this environment.",
      };
    }

    try {
      const result = await invokeWithSafetyConfirmation<CommitCellEditsResult>({
        command: "delete_rows",
        connection: ctx.connection,
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
            [key]: {
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
          await refreshAfterWrite(get, ref, ctx.dataKey, true);
        }
        return editOutcome;
      }
      clearLifecycle();
      await refreshAfterWrite(get, ref, ctx.dataKey, true);
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
        [key]: { state: "loading" },
      },
    }));
    if (!isTauri()) {
      set((state) => ({
        tableLoadStatus: {
          ...state.tableLoadStatus,
          [key]: { state: "idle" },
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
          [key]: {
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
          [key]: { state: "success" },
        },
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load table data", error);
      set((state) => ({
        tableLoadStatus: {
          ...state.tableLoadStatus,
          [key]: { state: "error", error: message },
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
    const catalogGeneration =
      get().pgObjectCatalog[connectionId]?.generation ?? 0;
    const isCurrent = () =>
      (get().pgObjectCatalog[connectionId]?.generation ?? 0) ===
      catalogGeneration;
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
      const result = normalizeTableStructure(
        await tauriInvoke<TableStructurePayload>("load_table_structure", {
          payload: { connectionId, schema, table },
        }),
      );
      if (!isCurrent()) return;
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
      if (!isCurrent()) return;
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
    const catalogGeneration =
      get().pgObjectCatalog[connectionId]?.generation ?? 0;
    const isCurrent = () =>
      (get().pgObjectCatalog[connectionId]?.generation ?? 0) ===
      catalogGeneration;
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
      if (!isCurrent()) return;
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
      if (!isCurrent()) return;
      set((state) => ({
        schemaRelationshipsStatus: {
          ...state.schemaRelationshipsStatus,
          [key]: { state: "error", error: message },
        },
      }));
    }
  },

  loadTableSchemaRelationships: async (connectionId, schema, table) => {
    if (!connectionId) {
      return;
    }
    const generation = get().pgObjectCatalog[connectionId]?.generation ?? 0;
    const isCurrent = () =>
      (get().pgObjectCatalog[connectionId]?.generation ?? 0) === generation;
    const scope = tableSchemaMapScope(schema, table);
    const key = schemaRelationshipsKey(connectionId, scope);
    const current = get().schemaRelationshipsStatus[key];
    if (current?.state === "loading") {
      return;
    }
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
      }>("load_table_schema_relationships", {
        payload: { connectionId, schema, table },
      });
      if (!isCurrent()) return;
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
      if (!isCurrent()) return;
      const message = errorToMessage(error);
      console.error("Failed to load table schema relationships", error);
      set((state) => ({
        schemaRelationshipsStatus: {
          ...state.schemaRelationshipsStatus,
          [key]: { state: "error", error: message },
        },
      }));
    }
  },

  loadSchemaMapPositions: async (connectionId, schema) => {
    if (!connectionId || !schema) {
      return;
    }
    if (get().schemaMapPositions[connectionId]?.[schema]) {
      return;
    }
    const current = get().schemaMapPositionsStatus[connectionId]?.[schema];
    if (current?.state === "loading" || current?.state === "success") {
      return;
    }
    set((state) => ({
      schemaMapPositionsStatus: withNestedValue(
        state.schemaMapPositionsStatus,
        connectionId,
        schema,
        { state: "loading" },
      ),
    }));
    if (!isTauri()) {
      set((state) => ({
        schemaMapPositionsStatus: withNestedValue(
          state.schemaMapPositionsStatus,
          connectionId,
          schema,
          { state: "idle" },
        ),
      }));
      return;
    }
    try {
      const rows = await tauriInvoke<PositionRow[]>(
        "load_schema_map_positions",
        {
          payload: { connectionId, schema },
        },
      );
      const positions = Object.fromEntries(
        rows.map((row) => [row.tableId, { x: row.x, y: row.y }]),
      );
      set((state) => ({
        schemaMapPositions: withNestedValue(
          state.schemaMapPositions,
          connectionId,
          schema,
          positions,
        ),
        schemaMapPositionsStatus: withNestedValue(
          state.schemaMapPositionsStatus,
          connectionId,
          schema,
          { state: "success" },
        ),
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load schema map positions", error);
      set((state) => ({
        schemaMapPositionsStatus: withNestedValue(
          state.schemaMapPositionsStatus,
          connectionId,
          schema,
          { state: "error", error: message },
        ),
      }));
    }
  },

  saveSchemaMapPosition: async (connectionId, schema, tableId, x, y) => {
    if (!connectionId || !schema || !tableId) {
      return;
    }
    set((state) => ({
      schemaMapPositions: withNestedValue(
        state.schemaMapPositions,
        connectionId,
        schema,
        {
          ...state.schemaMapPositions[connectionId]?.[schema],
          [tableId]: { x, y },
        },
      ),
    }));
    if (!isTauri()) {
      return;
    }
    try {
      await tauriInvoke("save_schema_map_position", {
        payload: { connectionId, schema, tableId, x, y },
      });
    } catch (error) {
      console.error("Failed to save schema map position", error);
    }
  },

  resetSchemaMapPositions: async (connectionId, schema) => {
    if (!connectionId || !schema) {
      return;
    }
    set((state) => ({
      schemaMapPositions: withNestedValue(
        state.schemaMapPositions,
        connectionId,
        schema,
        {},
      ),
      schemaMapPositionsStatus: withNestedValue(
        state.schemaMapPositionsStatus,
        connectionId,
        schema,
        { state: "success" },
      ),
    }));
    if (!isTauri()) {
      return;
    }
    try {
      await tauriInvoke("reset_schema_map_positions", {
        payload: { connectionId, schema },
      });
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to reset schema map positions", error);
      set((state) => ({
        schemaMapPositionsStatus: withNestedValue(
          state.schemaMapPositionsStatus,
          connectionId,
          schema,
          { state: "error", error: message },
        ),
      }));
    }
  },

  loadSchemaMapPrefs: async (connectionId, schema) => {
    if (!connectionId || !schema) {
      return;
    }
    if (get().schemaMapPrefs[connectionId]?.[schema]) {
      return;
    }
    const current = get().schemaMapPrefsStatus[connectionId]?.[schema];
    if (current?.state === "loading" || current?.state === "success") {
      return;
    }
    set((state) => ({
      schemaMapPrefsStatus: withNestedValue(
        state.schemaMapPrefsStatus,
        connectionId,
        schema,
        { state: "loading" },
      ),
    }));
    if (!isTauri()) {
      set((state) => ({
        schemaMapPrefs: withNestedValue(
          state.schemaMapPrefs,
          connectionId,
          schema,
          DEFAULT_SCHEMA_MAP_PREFS,
        ),
        schemaMapPrefsStatus: withNestedValue(
          state.schemaMapPrefsStatus,
          connectionId,
          schema,
          { state: "idle" },
        ),
      }));
      return;
    }
    try {
      const prefs = await tauriInvoke<SchemaMapPrefsWire>(
        "load_schema_map_prefs",
        {
          payload: { connectionId, schema },
        },
      );
      set((state) => ({
        schemaMapPrefs: withNestedValue(
          state.schemaMapPrefs,
          connectionId,
          schema,
          schemaMapPrefsFromWire(prefs),
        ),
        schemaMapPrefsStatus: withNestedValue(
          state.schemaMapPrefsStatus,
          connectionId,
          schema,
          { state: "success" },
        ),
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to load schema map preferences", error);
      set((state) => ({
        schemaMapPrefsStatus: withNestedValue(
          state.schemaMapPrefsStatus,
          connectionId,
          schema,
          { state: "error", error: message },
        ),
      }));
    }
  },

  setSchemaMapPref: async (connectionId, schema, patch) => {
    if (!connectionId || !schema) {
      return;
    }
    const current =
      get().schemaMapPrefs[connectionId]?.[schema] ?? DEFAULT_SCHEMA_MAP_PREFS;
    const next: SchemaMapPrefs = { ...current, ...patch };
    set((state) => ({
      schemaMapPrefs: withNestedValue(
        state.schemaMapPrefs,
        connectionId,
        schema,
        next,
      ),
    }));
    if (!isTauri()) {
      return;
    }
    try {
      const saved = await tauriInvoke<SchemaMapPrefsWire>(
        "save_schema_map_prefs",
        {
          payload: { connectionId, schema, patch },
        },
      );
      set((state) => ({
        schemaMapPrefs: withNestedValue(
          state.schemaMapPrefs,
          connectionId,
          schema,
          schemaMapPrefsFromWire(saved),
        ),
        schemaMapPrefsStatus: withNestedValue(
          state.schemaMapPrefsStatus,
          connectionId,
          schema,
          { state: "success" },
        ),
      }));
    } catch (error) {
      const message = errorToMessage(error);
      console.error("Failed to save schema map preferences", error);
      set((state) => ({
        schemaMapPrefsStatus: withNestedValue(
          state.schemaMapPrefsStatus,
          connectionId,
          schema,
          { state: "error", error: message },
        ),
      }));
    }
  },

  loadDatabaseOverviewStats: async (connectionId) => {
    if (!connectionId) {
      return;
    }
    const catalogGeneration =
      get().pgObjectCatalog[connectionId]?.generation ?? 0;
    const isCurrent = () =>
      (get().pgObjectCatalog[connectionId]?.generation ?? 0) ===
      catalogGeneration;
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
      if (!isCurrent()) return;
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
      if (!isCurrent()) return;
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
    const catalogGeneration =
      get().pgObjectCatalog[connectionId]?.generation ?? 0;
    const isCurrent = () =>
      (get().pgObjectCatalog[connectionId]?.generation ?? 0) ===
      catalogGeneration;
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
      if (!isCurrent()) return;
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
      if (!isCurrent()) return;
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
    const catalogGeneration =
      get().pgObjectCatalog[connectionId]?.generation ?? 0;
    const isCurrent = () =>
      (get().pgObjectCatalog[connectionId]?.generation ?? 0) ===
      catalogGeneration;
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
      if (!isCurrent()) return;
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
      if (!isCurrent()) return;
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
      const connectionId = key.split("::")[0];
      const engine = state.connections.find(
        (connection) => connection.id === connectionId,
      )?.engine;
      assertStructureChangeCanAppend(existing, entry.change, engine);
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

  commitStructureChanges: async (key, options): Promise<DDLOutcome> => {
    const state = get();
    const pending = state.pendingStructureChanges[key];
    if (!pending || pending.length === 0) {
      return { kind: "noop" };
    }
    const batch = pendingStructureBatch(pending);
    if (batch.kind === "invalid") {
      return { kind: "failed", reason: batch.reason };
    }
    if (batch.kind === "empty") {
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
    // Only the entries that are part of this apply. Edits queued while
    // the request is in flight belong to the next commit and must
    // survive this one's cleanup.
    const committedIds = new Set(pending.map((entry) => entry.id));

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

    // Drops the running status and — because statements may have
    // applied — invalidates the per-connection relation-stats cache so
    // the Tables / Schemas sub-tabs re-fetch on next activation. Only
    // the committed entries are cleared, and only on full success; a
    // partial apply keeps the batch so the user can edit and retry.
    const finalizeApplied = (clearPending: boolean) =>
      set((s) => {
        const { [key]: _status, ...restStatus } = s.structureCommitStatus;
        const { [connectionId]: _stats, ...restRelationStats } =
          s.relationStats;
        const { [connectionId]: _statsStatus, ...restRelationStatsStatus } =
          s.relationStatsStatus;
        const next = {
          structureCommitStatus: restStatus,
          relationStats: restRelationStats,
          relationStatsStatus: restRelationStatsStatus,
        };
        if (!clearPending) {
          return next;
        }
        const remaining = (s.pendingStructureChanges[key] ?? []).filter(
          (entry) => !committedIds.has(entry.id),
        );
        if (remaining.length === 0) {
          const { [key]: _pending, ...restPending } = s.pendingStructureChanges;
          return { ...next, pendingStructureChanges: restPending };
        }
        return {
          ...next,
          pendingStructureChanges: {
            ...s.pendingStructureChanges,
            [key]: remaining,
          },
        };
      });

    // Post-DDL refreshes are independent reads against the same
    // connection and write to disjoint store slices, so they run
    // concurrently — saves ~1 RTT on a remote DB. `loadTableData`
    // is only re-fetched when the data tab has already loaded it
    // once (preserved from the pre-parallel ordering).
    const runPostDdlRefreshes = async () => {
      const dataKey = tableDataKey(connectionId, schema, table);
      const refreshes: Promise<unknown>[] = [
        get().loadTableStructure(connectionId, schema, table),
      ];
      if (get().tableData[dataKey]) {
        refreshes.push(get().refreshTableData(dataKey));
      }
      refreshes.push(
        get().refreshTableBrowsesForRelation(connectionId, schema, table, {
          refreshStructure: true,
          invalidateExactCount: true,
        }),
      );
      await Promise.all(refreshes);
    };

    if (batch.kind === "pg-op") {
      if (connection.engine !== "PostgreSQL") {
        clearLifecycle();
        return {
          kind: "failed",
          reason: "Typed structure operations require a PostgreSQL connection.",
        };
      }
      // Same fencing as the Plan 014 review dialog: a reconnect (epoch
      // bump), engine change, disconnect, or in-progress transition
      // invalidates the reviewed batch — including while the safety
      // confirmation dialog is open.
      const expectedConnectionEpoch = get().connectionEpochs[connectionId] ?? 0;
      const isConnectionCurrent = () => {
        const current = get();
        const liveConnection = current.connections.find(
          (candidate) => candidate.id === connectionId,
        );
        return (
          (current.connectionEpochs[connectionId] ?? 0) ===
            expectedConnectionEpoch &&
          liveConnection?.engine === "PostgreSQL" &&
          isConnectedStatus(liveConnection.status) &&
          !current.connectionTransitionIds.includes(connectionId)
        );
      };
      if (!isConnectionCurrent()) {
        clearLifecycle();
        return {
          kind: "failed",
          reason: "Connect to the PostgreSQL database before applying DDL.",
        };
      }
      // One apply per connection at a time, shared with the object-DDL
      // review dialog's gate.
      const applyKey = pgObjectDdlApplyKey(connectionId);
      if (!get().beginPgObjectDdlApply(applyKey)) {
        clearLifecycle();
        return {
          kind: "failed",
          reason: "Another DDL apply is already running on this connection.",
        };
      }
      const expectedGeneration =
        get().pgObjectCatalog[connectionId]?.generation ?? 0;
      const refreshScope = objectDdlRefreshScope(batch.ops);
      // Refresh every object cache named by the shared DDL scope. The table
      // is always included because `dropIndex` cannot carry its table ref.
      // Inline CREATE OR REPLACE FUNCTION additionally refreshes the catalog
      // and every description loaded for this connection: its identity args
      // cannot be reconstructed safely from the create operation.
      const refreshObjectCaches = async () => {
        const tableReference: PgObjectRef = {
          kind: "table",
          schema,
          name: table,
          identityArgs: null,
        };
        const references = new Map(
          [...refreshScope.references, tableReference].map((reference) => [
            pgObjectDescriptionKey(connectionId, reference),
            reference,
          ]),
        );
        const before = get();
        if (
          (before.pgObjectCatalog[connectionId]?.generation ?? 0) !==
          expectedGeneration
        ) {
          return;
        }
        const described = new Set(Object.keys(before.pgObjectDescriptions));
        if (refreshScope.revalidateAllDescriptions) {
          const prefix = `${connectionId}:`;
          for (const [descriptionKey, descriptionState] of Object.entries(
            before.pgObjectDescriptions,
          )) {
            if (!descriptionKey.startsWith(prefix)) continue;
            const reference =
              descriptionState.description?.reference ??
              (descriptionState.error?.kind === "objectNotFound"
                ? descriptionState.error.reference
                : parseCanonicalPgObjectRefKey(
                    descriptionKey.slice(prefix.length),
                  ));
            if (reference) references.set(descriptionKey, reference);
          }
        }
        if (refreshScope.catalog) {
          await before.loadPgObjectCatalog(connectionId, expectedGeneration);
          if (
            (get().pgObjectCatalog[connectionId]?.generation ?? 0) !==
            expectedGeneration
          ) {
            return;
          }
        }
        for (const [descriptionKey, reference] of references) {
          if (!described.has(descriptionKey)) continue;
          await get().loadPgObjectDescription(
            connectionId,
            reference,
            expectedGeneration,
          );
          if (
            (get().pgObjectCatalog[connectionId]?.generation ?? 0) !==
            expectedGeneration
          ) {
            return;
          }
        }
      };
      const refreshAfterApply = async () => {
        try {
          await Promise.all([runPostDdlRefreshes(), refreshObjectCaches()]);
        } catch (refreshError) {
          // The DDL outcome is already decided; a refresh failure must
          // not masquerade as an apply failure.
          console.error("Post-DDL refresh failed", refreshError);
        }
      };
      try {
        const result = await applyObjectDdlWithSafetyConfirmation(
          { connectionId, ops: batch.ops },
          connection,
          isConnectionCurrent,
        );
        if (result.kind === "cancelled") {
          clearLifecycle();
          return { kind: "noop" };
        }
        if (result.kind === "error") {
          const { error } = result;
          const partiallyApplied =
            (error.kind === "database" || error.kind === "lockTimeout") &&
            error.appliedStatements > 0;
          if (partiallyApplied) {
            get().markPgObjectDdlApplied();
            finalizeApplied(false);
            await refreshAfterApply();
          } else {
            clearLifecycle();
          }
          return {
            kind: "failed",
            reason: formatObjectDdlError(error, options?.statements ?? []),
          };
        }
        get().markPgObjectDdlApplied();
        finalizeApplied(true);
        await refreshAfterApply();
        return { kind: "completed", runtimeMs: result.value.runtimeMs };
      } finally {
        get().endPgObjectDdlApply(applyKey);
      }
    }

    const sql = generateDdlForEngine(
      connection.engine,
      schema,
      table,
      batch.changes,
      ddlStructure?.columns,
    );

    try {
      const result = await invokeWithSafetyConfirmation<{ runtimeMs: number }>({
        command: "execute_ddl",
        connection,
        payload: { connectionId, sql },
      });
      finalizeApplied(true);
      await runPostDdlRefreshes();
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
      return {
        tableEdits: dropMatching(state.tableEdits, matches),
        tableEditsCommitStatus: dropMatching(
          state.tableEditsCommitStatus,
          matches,
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
        schemaMapPositions: withoutConnection(
          state.schemaMapPositions,
          connectionId,
        ),
        schemaMapPositionsStatus: withoutConnection(
          state.schemaMapPositionsStatus,
          connectionId,
        ),
        schemaMapPrefs: withoutConnection(state.schemaMapPrefs, connectionId),
        schemaMapPrefsStatus: withoutConnection(
          state.schemaMapPrefsStatus,
          connectionId,
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
