import type { WorkspaceTab } from "@/lib/store";

export interface TableRef {
  connectionId: string;
  schema: string;
  table: string;
}

export function getTableRef(tab: WorkspaceTab): TableRef | null {
  if (tab.kind !== "table" || !tab.table) return null;
  return {
    connectionId: tab.connectionId,
    schema: tab.schema,
    table: tab.table,
  };
}
