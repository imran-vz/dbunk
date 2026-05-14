import { connectionStatusItem } from "@/components/connection-status";
import type { StatusBarItem } from "@/components/status-bar";
import type { Connection } from "@/lib/store";

import type { TablePagination } from "./use-table-pagination";

interface BuildStatusItemsArgs {
  errorMessage: string | null;
  isLoading: boolean;
  rowCount: number;
  pagination: TablePagination;
  activeConnection: Connection | undefined;
}

export function buildStatusItems({
  errorMessage,
  isLoading,
  rowCount,
  pagination,
  activeConnection,
}: BuildStatusItemsArgs): StatusBarItem[] {
  return [
    connectionStatusItem(activeConnection),
    {
      id: "query",
      label: "Query",
      tone: errorMessage ? "danger" : "healthy",
      value: queryStatusValue(pagination.runtimeMs, isLoading),
    },
    {
      id: "data",
      label: "Data",
      value: `${rowCount.toLocaleString()} rows`,
    },
    {
      id: "page",
      label: "Page",
      value: pageValue(pagination.page, pagination.totalPages),
      align: "right",
    },
  ];
}

function queryStatusValue(
  runtimeMs: number | undefined,
  isLoading: boolean,
): string {
  if (runtimeMs !== undefined) return `Completed · ${runtimeMs} ms`;
  if (isLoading) return "Loading…";
  return "Idle";
}

function pageValue(page: number, totalPages: number | undefined): string {
  if (totalPages) return `${page} of ${totalPages}`;
  return `${page}`;
}
