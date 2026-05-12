import type { StatusBarItem } from "@/components/status-bar";

import type { TablePagination } from "./use-table-pagination";

interface BuildStatusItemsArgs {
  errorMessage: string | null;
  isLoading: boolean;
  rowCount: number;
  pagination: TablePagination;
  connectionStatus: string | undefined;
}

export function buildStatusItems({
  errorMessage,
  isLoading,
  rowCount,
  pagination,
  connectionStatus,
}: BuildStatusItemsArgs): StatusBarItem[] {
  return [
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
    },
    {
      id: "connection",
      label: "Connection",
      tone: "healthy",
      value: connectionStatus ?? "Healthy",
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
