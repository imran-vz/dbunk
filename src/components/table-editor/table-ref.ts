import type { TableRef, WorkspaceTab } from "@/lib/store";

export type { TableRef };

export function getTableRef(tab: WorkspaceTab): TableRef | null {
  if (tab.kind !== "table" || !tab.table) return null;
  return {
    connectionId: tab.connectionId,
    schema: tab.schema,
    table: tab.table,
  };
}
