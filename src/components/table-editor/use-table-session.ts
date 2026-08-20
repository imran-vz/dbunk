import { useEffect, useMemo } from "react";

import type { ServerBrowseGridModel } from "@/components/data-grid/browse-model";
import type { InsertRowPayloadEntry } from "@/lib/insert-row-form";
import {
  type EditOutcome,
  type TableDataState,
  type TableRef,
  tableSessionKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import {
  browseCellsToGrid,
  browseIdentityReadOnlyCopy,
  cycleSort,
  identityIsEditable,
  supportsServerTableBrowse,
} from "@/lib/table-browse";
import { formatTableBrowseError } from "@/lib/table-browse-error";
import {
  buildTableSessionSnapshot,
  tableSessionStructureKey,
} from "@/lib/table-session";

import { useTablePagination } from "./use-table-pagination";

const NOOP_OUTCOME: EditOutcome = { kind: "noop" };

export function useTableSession(tab: WorkspaceTab) {
  const ref = useMemo(() => {
    if (tab.kind !== "table" || !tab.table) return null;
    return {
      connectionId: tab.connectionId,
      schema: tab.schema,
      table: tab.table,
    };
  }, [tab.kind, tab.connectionId, tab.schema, tab.table]);
  const refKey = ref ? tableSessionKey(ref) : "";
  const structureKey = ref ? tableSessionStructureKey(ref) : "";

  const engine = useAppStore(
    (state) =>
      state.connections.find((connection) => connection.id === tab.connectionId)
        ?.engine,
  );
  const browseEnabled = Boolean(engine && supportsServerTableBrowse(engine));
  const browseForTab = useAppStore((state) => state.tableBrowses[tab.id]);
  const browse =
    ref &&
    browseForTab?.connectionId === ref.connectionId &&
    browseForTab.schema === ref.schema &&
    browseForTab.table === ref.table
      ? browseForTab
      : undefined;

  const storeData = useAppStore((state) =>
    refKey ? state.tableData[refKey] : undefined,
  );
  const structure = useAppStore((state) =>
    structureKey ? state.tableStructure[structureKey] : undefined,
  );
  const status = useAppStore((state) =>
    refKey ? state.tableLoadStatus[refKey] : undefined,
  );
  const commitStatus = useAppStore((state) =>
    refKey ? state.tableEditsCommitStatus[refKey] : undefined,
  );
  const structureStatus = useAppStore((state) =>
    structureKey ? state.tableStructureStatus[structureKey] : undefined,
  );
  const currentEdits = useAppStore((state) =>
    refKey ? state.tableEdits[refKey] : undefined,
  );
  const openTableSession = useAppStore((state) => state.openTableSession);
  const openTableBrowse = useAppStore((state) => state.openTableBrowse);
  const loadTableStructure = useAppStore((state) => state.loadTableStructure);
  const refreshTableSession = useAppStore((state) => state.refreshTableSession);
  const refreshTableBrowse = useAppStore((state) => state.refreshTableBrowse);
  const loadTableData = useAppStore((state) => state.loadTableData);
  const setTableCellEdit = useAppStore((state) => state.setTableCellEdit);
  const discardTableCellEdits = useAppStore(
    (state) => state.discardTableCellEdits,
  );
  const commitTableCellEdits = useAppStore(
    (state) => state.commitTableCellEdits,
  );
  const insertTableRow = useAppStore((state) => state.insertTableRow);
  const deleteTableRows = useAppStore((state) => state.deleteTableRows);
  const setTableBrowseFilters = useAppStore(
    (state) => state.setTableBrowseFilters,
  );
  const clearTableBrowseFilters = useAppStore(
    (state) => state.clearTableBrowseFilters,
  );
  const setTableBrowseRawFilter = useAppStore(
    (state) => state.setTableBrowseRawFilter,
  );
  const setTableBrowseFilterMode = useAppStore(
    (state) => state.setTableBrowseFilterMode,
  );
  const setTableBrowseSort = useAppStore((state) => state.setTableBrowseSort);
  const setTableBrowsePageSize = useAppStore(
    (state) => state.setTableBrowsePageSize,
  );
  const goToTableBrowsePage = useAppStore((state) => state.goToTableBrowsePage);
  const goToTableBrowseNextPage = useAppStore(
    (state) => state.goToTableBrowseNextPage,
  );
  const goToTableBrowsePrevPage = useAppStore(
    (state) => state.goToTableBrowsePrevPage,
  );
  const goToTableBrowseFirstPage = useAppStore(
    (state) => state.goToTableBrowseFirstPage,
  );
  const goToTableBrowseLastPage = useAppStore(
    (state) => state.goToTableBrowseLastPage,
  );
  const countTableBrowseRows = useAppStore(
    (state) => state.countTableBrowseRows,
  );
  const cancelTableBrowse = useAppStore((state) => state.cancelTableBrowse);
  const applyTableBrowsePreset = useAppStore(
    (state) => state.applyTableBrowsePreset,
  );
  const saveTableBrowsePreset = useAppStore(
    (state) => state.saveTableBrowsePreset,
  );
  const applyTableBrowseHistory = useAppStore(
    (state) => state.applyTableBrowseHistory,
  );

  useEffect(() => {
    if (!ref) return;
    if (browseEnabled) {
      void openTableBrowse(tab.id, ref.connectionId, ref.schema, ref.table);
      void loadTableStructure(ref.connectionId, ref.schema, ref.table);
      return;
    }
    void openTableSession(ref);
  }, [
    browseEnabled,
    loadTableStructure,
    openTableBrowse,
    openTableSession,
    ref,
    tab.id,
  ]);

  const data: TableDataState | undefined = useMemo(() => {
    if (!browseEnabled || !ref || !browse?.result) return storeData;
    const count = browse.exactCount?.value ?? browse.result.count.value;
    return {
      connectionId: ref.connectionId,
      schema: ref.schema,
      table: ref.table,
      columns: browse.result.columns.map((column) => column.name),
      rows: browseCellsToGrid(browse.result.rows),
      page: browse.page,
      pageSize: browse.pageSize,
      totalRows: count ?? undefined,
      runtimeMs: browse.result.runtimeMs,
    };
  }, [browse, browseEnabled, ref, storeData]);

  const legacyPagination = useTablePagination({
    tab,
    data: storeData,
    loadTableData,
  });

  const countKind = browse?.exactCount?.kind ?? browse?.result?.count.kind;
  const countValue = browse?.exactCount?.value ?? browse?.result?.count.value;
  const browsePagination = useMemo(() => {
    const page = browse?.page ?? 1;
    const pageSize = browse?.pageSize ?? 100;
    const rowCount = data?.rows.length ?? 0;
    const totalRows = countValue ?? undefined;
    const canJump = countKind === "exact" || countKind === "estimated";
    const totalPages =
      canJump && totalRows !== undefined
        ? Math.max(1, Math.ceil(totalRows / pageSize))
        : undefined;
    const countLabel =
      countKind === "estimated" && totalRows !== undefined
        ? `~${totalRows.toLocaleString()} rows (estimated)`
        : countKind === "exact" && totalRows !== undefined
          ? `${totalRows.toLocaleString()} rows`
          : countKind === "unknown" || totalRows === undefined
            ? "unknown"
            : `${rowCount.toLocaleString()} rows`;
    return {
      page,
      pageSize,
      totalRows,
      totalPages,
      isLastPage: !(browse?.result?.pageInfo.hasMore ?? false),
      runtimeMs: browse?.result?.runtimeMs,
      rowCount,
      startRow: rowCount === 0 ? 0 : (page - 1) * pageSize + 1,
      endRow: (page - 1) * pageSize + rowCount,
      countLabel,
      canJump,
      countApproximate: countKind === "estimated",
      counting: browse?.countStatus.state === "loading",
      onCountRows: () => {
        void countTableBrowseRows(tab.id);
      },
      goToPage: (next: number) => {
        void goToTableBrowsePage(tab.id, next);
      },
      onPrevPage: () => {
        void goToTableBrowsePrevPage(tab.id);
      },
      onNextPage: () => {
        void goToTableBrowseNextPage(tab.id);
      },
      onFirstPage: () => {
        void goToTableBrowseFirstPage(tab.id);
      },
      onLastPage: () => {
        void goToTableBrowseLastPage(tab.id);
      },
    };
  }, [
    browse,
    countKind,
    countTableBrowseRows,
    countValue,
    data?.rows.length,
    goToTableBrowseFirstPage,
    goToTableBrowseLastPage,
    goToTableBrowseNextPage,
    goToTableBrowsePage,
    goToTableBrowsePrevPage,
    tab.id,
  ]);

  const session = useMemo(
    () =>
      ref
        ? buildTableSessionSnapshot({
            ref,
            data,
            structure,
            loadStatus: browseEnabled
              ? browse?.loadStatus.state === "error"
                ? {
                    state: "error",
                    error: formatTableBrowseError(browse.loadStatus.error),
                  }
                : browse?.loadStatus.state === "loading"
                  ? { state: "loading" }
                  : browse?.loadStatus.state === "success"
                    ? { state: "success" }
                    : { state: "idle" }
              : status,
            structureStatus,
            writeStatus: commitStatus,
            edits: currentEdits,
            browseIdentityKind: browse?.result?.identity.kind,
          })
        : null,
    [
      browse?.loadStatus,
      browse?.result?.identity.kind,
      browseEnabled,
      commitStatus,
      currentEdits,
      data,
      ref,
      status,
      structure,
      structureStatus,
    ],
  );

  const withRef =
    <T extends unknown[]>(fn: (ref: TableRef, ...args: T) => void) =>
    (...args: T) => {
      if (!ref) return;
      fn(ref, ...args);
    };

  const withOutcomeRef =
    <T extends unknown[]>(
      fn: (ref: TableRef, ...args: T) => Promise<EditOutcome>,
    ) =>
    async (...args: T): Promise<EditOutcome> => {
      if (!ref) return NOOP_OUTCOME;
      return fn(ref, ...args);
    };

  const serverBrowse: ServerBrowseGridModel | undefined =
    browseEnabled && browse
      ? {
          typedFilters: browse?.typedFilters ?? [],
          rawFilterText: browse?.rawFilterText ?? "",
          filterMode: browse?.filterMode ?? "typed",
          sort: browse?.sort ?? [],
          pageSize: browse?.pageSize ?? 100,
          loadStatus: browse?.loadStatus ?? { state: "idle" },
          error:
            browse?.loadStatus.state === "error"
              ? browse.loadStatus.error
              : null,
          inspection: browse?.result?.inspection ?? null,
          omittedRows: browse?.result?.omittedRows ?? 0,
          truncatedCells: browse?.result?.truncatedCells ?? 0,
          count: browse?.result?.count ?? { kind: "unknown", value: null },
          exactCount: browse?.exactCount ?? null,
          countStatus: browse?.countStatus ?? { state: "idle" },
          pageInfo: browse?.result?.pageInfo ?? null,
          history: browse?.prefs.filterHistory ?? [],
          presets: browse?.prefs.presets ?? [],
          onApplyTypedFilter: (filter) => {
            const next = [
              ...(browse?.typedFilters ?? []).filter((item) => {
                if (item.kind === "rawSql" || filter.kind === "rawSql")
                  return true;
                return item.column !== filter.column;
              }),
              filter,
            ];
            void setTableBrowseFilters(tab.id, next);
          },
          onRemoveTypedFilter: (column) => {
            void setTableBrowseFilters(
              tab.id,
              (browse?.typedFilters ?? []).filter(
                (item) => item.kind === "rawSql" || item.column !== column,
              ),
            );
          },
          onClearTypedFilters: () => {
            void clearTableBrowseFilters(tab.id);
          },
          onRawFilterApply: (text) => {
            void setTableBrowseRawFilter(tab.id, text);
          },
          onFilterModeChange: (mode) => {
            void setTableBrowseFilterMode(tab.id, mode);
          },
          onSortChange: (sort) => {
            void setTableBrowseSort(tab.id, sort);
          },
          onPageSizeChange: (pageSize) => {
            void setTableBrowsePageSize(tab.id, pageSize);
          },
          onHeaderSort: (column, append) => {
            void setTableBrowseSort(
              tab.id,
              cycleSort(browse?.sort ?? [], column, append),
            );
          },
          onCountRows: () => {
            void countTableBrowseRows(tab.id);
          },
          onCancel: () => {
            void cancelTableBrowse(tab.id);
          },
          onApplyPreset: (name) => {
            void applyTableBrowsePreset(tab.id, name);
          },
          onSavePreset: (name) => {
            void saveTableBrowsePreset(tab.id, name);
          },
          onApplyHistory: (index) => {
            void applyTableBrowseHistory(tab.id, index);
          },
        }
      : undefined;

  const identityKind = browse?.result?.identity.kind;
  const readOnlyCopy =
    browseEnabled && identityKind && !identityIsEditable(identityKind)
      ? browseIdentityReadOnlyCopy(identityKind)
      : undefined;

  return {
    ref,
    key: refKey,
    tableName: ref?.table ?? "",
    data: session?.data,
    structure: session?.structure,
    status: session?.loadStatus,
    commitStatus: session?.writeStatus,
    currentEdits: session?.edits,
    hasEdits: Object.keys(session?.edits ?? {}).length > 0,
    capabilities: session?.capabilities,
    pagination: browseEnabled ? browsePagination : legacyPagination,
    browseEnabled,
    serverBrowse,
    appliedBrowseRequestId: browse?.appliedRequestId ?? null,
    readOnlyCopy,
    refresh: async () => {
      if (!ref) return;
      if (browseEnabled) {
        await refreshTableBrowse(tab.id);
        return;
      }
      await refreshTableSession(ref);
    },
    // Data-only writes can change row counts without invalidating the
    // backend relation descriptor. Legacy sessions already refresh data only.
    refreshData: async () => {
      if (!ref) return;
      if (browseEnabled) {
        await refreshTableBrowse(tab.id, {
          refreshStructure: false,
          invalidateExactCount: true,
        });
        return;
      }
      await refreshTableSession(ref);
    },
    setCellEdit: withRef(setTableCellEdit),
    discardEdits: withRef(discardTableCellEdits),
    commitEdits: withOutcomeRef((tableRef) =>
      commitTableCellEdits(tableRef, tab.id),
    ),
    addRow: withOutcomeRef((tableRef, values: InsertRowPayloadEntry[]) =>
      insertTableRow(tableRef, values),
    ),
    deleteRows: withOutcomeRef((tableRef, indices: number[]) =>
      deleteTableRows(tableRef, indices, tab.id),
    ),
  };
}
