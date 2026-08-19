import { useMemo } from "react";

import type { TableDataState, WorkspaceTab } from "@/lib/store";

interface UseTablePaginationArgs {
  tab: WorkspaceTab;
  data: TableDataState | undefined;
  loadTableData: (
    connectionId: string,
    schema: string,
    table: string,
    page?: number,
    pageSize?: number,
    // oxlint-disable-next-line anti-slop/no-unknown-returns -- The value is handled at a typed library or domain boundary here.
  ) => unknown;
}

export interface TablePagination {
  page: number;
  pageSize: number;
  totalRows: number | undefined;
  totalPages: number | undefined;
  isLastPage: boolean;
  runtimeMs: number | undefined;
  rowCount: number;
  startRow: number;
  endRow: number;
  countLabel: string;
  canJump: boolean;
  countApproximate: boolean;
  counting: boolean;
  onCountRows?: () => void;
  goToPage: (next: number) => void;
  onPrevPage: () => void;
  onNextPage: () => void;
  onFirstPage: () => void;
  onLastPage: () => void;
}

export function useTablePagination({
  tab,
  data,
  loadTableData,
}: UseTablePaginationArgs): TablePagination {
  const page = data?.page ?? 1;
  const pageSize = data?.pageSize ?? 100;
  const totalRows = data?.totalRows;
  const runtimeMs = data?.runtimeMs;
  const rowCount = data?.rows.length ?? 0;

  const totalPages = useMemo(() => {
    if (totalRows === undefined || pageSize <= 0) return undefined;
    return Math.max(1, Math.ceil(totalRows / pageSize));
  }, [totalRows, pageSize]);

  const isLastPage =
    totalPages !== undefined ? page >= totalPages : rowCount < pageSize;

  const startRow =
    totalRows === undefined && rowCount === 0 ? 0 : (page - 1) * pageSize + 1;
  const endRow =
    totalRows !== undefined
      ? Math.min(totalRows, page * pageSize)
      : (page - 1) * pageSize + rowCount;

  const goToPage = (next: number) => {
    if (tab.kind !== "table" || !tab.table || !tab.connectionId) return;
    const target = Math.max(1, totalPages ? Math.min(totalPages, next) : next);
    if (target === page) return;
    void loadTableData(
      tab.connectionId,
      tab.schema,
      tab.table,
      target,
      pageSize,
    );
  };

  return {
    page,
    pageSize,
    totalRows,
    totalPages,
    isLastPage,
    runtimeMs,
    rowCount,
    startRow,
    endRow,
    countLabel:
      totalRows === undefined
        ? `${rowCount.toLocaleString()} rows`
        : `${totalRows.toLocaleString()} rows`,
    canJump: totalPages !== undefined,
    countApproximate: false,
    counting: false,
    goToPage,
    onPrevPage: () => goToPage(page - 1),
    onNextPage: () => goToPage(page + 1),
    onFirstPage: () => goToPage(1),
    onLastPage: () => {
      if (totalPages !== undefined) goToPage(totalPages);
    },
  };
}
