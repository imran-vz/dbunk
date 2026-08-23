import { connectionStatusItem } from "@/components/connection-status";
import type { StatusBarItem } from "@/components/status-bar";
import type { Connection } from "@/lib/store";

import type { TablePagination } from "./use-table-pagination";

interface BuildStatusItemsArgs {
  errorMessage: string | null;
  isLoading: boolean;
  rowCount: number;
  rowCountLabel?: string;
  pagination: TablePagination;
  activeConnection: Connection | undefined;
  /** Staged mutation count — renders the pending badge (§3.1). */
  stagedChangeCount?: number;
  /** Opens the mutation review panel; wired to the badge click. */
  onOpenReview?: () => void;
}

export function buildStatusItems({
  errorMessage,
  isLoading,
  rowCount,
  rowCountLabel,
  pagination,
  activeConnection,
  stagedChangeCount = 0,
  onOpenReview,
}: BuildStatusItemsArgs): StatusBarItem[] {
  return [
    ...(stagedChangeCount > 0
      ? [
          {
            id: "pending",
            value: `${stagedChangeCount} staged`,
            tone: "warning",
            onClick: onOpenReview,
          } satisfies StatusBarItem,
        ]
      : []),
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
      value: rowCountLabel ?? `${rowCount.toLocaleString()} rows`,
    },
    {
      id: "page",
      label: "Page",
      value: pageValue(
        pagination.page,
        pagination.totalPages,
        pagination.countApproximate,
      ),
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

function pageValue(
  page: number,
  totalPages: number | undefined,
  approximate: boolean | undefined,
): string {
  if (totalPages) return `${page} of ${approximate ? "~" : ""}${totalPages}`;
  return `${page}`;
}
