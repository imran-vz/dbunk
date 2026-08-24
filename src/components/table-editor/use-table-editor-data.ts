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
  /**
   * True when the tab's connection exists but is not connected — the
   * restored-tab case (Plan 010). No fetch fires until it connects;
   * the panel renders a connect affordance instead of a doomed load.
   */
  awaitingConnection: boolean;
}

export function useTableEditorData(tab: WorkspaceTab): TableEditorData {
  const ref = getTableRef(tab);
  const tableData = useAppStore((s) => s.tableData);
  const tableStructure = useAppStore((s) => s.tableStructure);
  const tableLoadStatus = useAppStore((s) => s.tableLoadStatus);
  const loadTableData = useAppStore((s) => s.loadTableData);
  const loadTableStructure = useAppStore((s) => s.loadTableStructure);
  const connectionStatus = useAppStore(
    (s) =>
      s.connections.find((connection) => connection.id === tab.connectionId)
        ?.status,
  );
  const isConnected =
    connectionStatus === "Connected" || connectionStatus === "Read only";
  const awaitingConnection = connectionStatus !== undefined && !isConnected;

  // Stable string used as the effect's only ref dep — composite primitives
  // re-fire the effect on identity change of the ref object.
  const refKey = ref ? `${ref.connectionId}::${ref.schema}::${ref.table}` : "";

  /* oxlint-disable react-hooks/exhaustive-deps -- refKey captures every primitive ref field */
  useEffect(() => {
    if (!ref) return;
    // Session-restored tabs mount against Disconnected connections;
    // fetching would only error. The effect re-fires when the user
    // connects (isConnected flips) and loads then.
    if (!isConnected) return;
    void loadTableData(ref.connectionId, ref.schema, ref.table);
    void loadTableStructure(ref.connectionId, ref.schema, ref.table);
  }, [refKey, isConnected, loadTableData, loadTableStructure]);
  /* oxlint-enable react-hooks/exhaustive-deps */

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
    awaitingConnection,
  };
}
