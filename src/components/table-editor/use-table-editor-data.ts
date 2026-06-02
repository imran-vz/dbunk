import { useEffect } from "react";

import {
  type TableDataState,
  type TableLoadStatus,
  type TableStructure,
  tableDataKey,
  tableSessionKey,
  tableStructureKey,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";

import { getTableRef, type TableRef } from "./table-ref";

export interface TableEditorData {
  ref: TableRef | null;
  tableName: string;
  dataKey: string;
  data: TableDataState | undefined;
  structure: TableStructure | undefined;
  status: TableLoadStatus | undefined;
}

export function useTableEditorData(tab: WorkspaceTab): TableEditorData {
  const ref = getTableRef(tab);
  const tableData = useAppStore((s) => s.tableData);
  const tableStructure = useAppStore((s) => s.tableStructure);
  const tableLoadStatus = useAppStore((s) => s.tableLoadStatus);
  const loadTableData = useAppStore((s) => s.loadTableData);
  const loadTableStructure = useAppStore((s) => s.loadTableStructure);

  // Stable string used as the effect's only ref dep — composite primitives
  // re-fire the effect on identity change of the ref object.
  const refKey = ref ? `${ref.connectionId}::${ref.schema}::${ref.table}` : "";

  // biome-ignore lint/correctness/useExhaustiveDependencies: refKey captures every primitive ref field
  useEffect(() => {
    if (!ref) return;
    void loadTableData(ref.connectionId, ref.schema, ref.table);
    void loadTableStructure(ref.connectionId, ref.schema, ref.table);
  }, [refKey, loadTableData, loadTableStructure]);

  const tableName = ref?.table ?? "";
  const dataKey = ref
    ? tableDataKey(ref.connectionId, ref.schema, ref.table)
    : "";
  const structureKey = ref
    ? tableStructureKey(ref.connectionId, ref.schema, ref.table)
    : "";

  return {
    ref,
    tableName,
    dataKey,
    data: dataKey ? tableData[dataKey] : undefined,
    structure: structureKey ? tableStructure[structureKey] : undefined,
    status: ref ? tableLoadStatus[tableSessionKey(ref)] : undefined,
  };
}
