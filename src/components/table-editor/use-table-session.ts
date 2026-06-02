import { useEffect, useMemo } from "react";
import type { InsertRowPayloadEntry } from "@/lib/insert-row-form";
import {
  type EditOutcome,
  type TableRef,
  tableSessionKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
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

  const data = useAppStore((state) =>
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
  const refreshTableSession = useAppStore((state) => state.refreshTableSession);
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

  useEffect(() => {
    if (!ref) return;
    void openTableSession(ref);
  }, [ref, openTableSession]);

  const pagination = useTablePagination({
    tab,
    data,
    loadTableData,
  });

  const session = useMemo(
    () =>
      ref
        ? buildTableSessionSnapshot({
            ref,
            data,
            structure,
            loadStatus: status,
            structureStatus,
            writeStatus: commitStatus,
            edits: currentEdits,
          })
        : null,
    [ref, data, structure, status, structureStatus, commitStatus, currentEdits],
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
    pagination,
    refresh: async () => {
      if (!ref) return;
      await refreshTableSession(ref);
    },
    setCellEdit: withRef(setTableCellEdit),
    discardEdits: withRef(discardTableCellEdits),
    commitEdits: withOutcomeRef(commitTableCellEdits),
    addRow: withOutcomeRef((tableRef, values: InsertRowPayloadEntry[]) =>
      insertTableRow(tableRef, values),
    ),
    deleteRows: withOutcomeRef(deleteTableRows),
  };
}
