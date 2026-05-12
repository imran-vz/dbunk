import { useMemo } from "react";

import type { WorkspaceTab } from "@/lib/store";

import { getTableRef } from "./table-ref";

export function useTableExportFilename(tab: WorkspaceTab): string {
  return useMemo(() => {
    const ref = getTableRef(tab);
    if (!ref) return "export";
    const today = new Date().toISOString().slice(0, 10);
    return [ref.connectionId, ref.schema, ref.table, today]
      .map(slug)
      .filter(Boolean)
      .join("-");
  }, [tab]);
}

function slug(value: string): string {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
}
