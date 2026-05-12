/**
 * List viewer — offset-paginated `LRANGE` with a direction toggle
 * and page-size selector.
 */

import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  fetchList,
  formatValueOneLine,
  type SerializedValue,
} from "@/lib/redis/api";

interface ListValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

const PAGE_SIZES = [50, 100, 200, 500, 1000] as const;

export function ListValueView({
  connectionId,
  keyName,
  elementCount,
}: ListValueViewProps) {
  const [page, setPage] = useState(0);
  const [pageSize, setPageSize] = useState<number>(200);
  const [reverse, setReverse] = useState(false);
  const [items, setItems] = useState<SerializedValue[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const start = page * pageSize;
  const stop = start + pageSize - 1;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchList({ connectionId, key: keyName, start, stop, reverse })
      .then((result) => {
        if (!cancelled) setItems(result.items);
      })
      .catch((err) => {
        if (!cancelled)
          setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId, keyName, start, stop, reverse]);

  const hasMore = elementCount === undefined || start + pageSize < elementCount;

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <div className="flex flex-wrap items-center gap-2 text-[0.65rem] text-text-muted">
        <label className="flex items-center gap-1">
          Page size
          <select
            value={pageSize}
            onChange={(e) => {
              setPage(0);
              setPageSize(Number(e.target.value));
            }}
            className="rounded border border-border-subtle bg-surface-panel px-1 py-0.5 text-xs"
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                {size}
              </option>
            ))}
          </select>
        </label>
        <Button
          variant={reverse ? "default" : "outline"}
          size="sm"
          className="h-6 px-2 text-[0.65rem]"
          onClick={() => setReverse((prev) => !prev)}
        >
          {reverse ? "Tail → Head" : "Head → Tail"}
        </Button>
        <span>
          showing {start + 1}–{start + items.length}
          {elementCount !== undefined
            ? ` of ${elementCount.toLocaleString()}`
            : ""}
        </span>
      </div>
      {error ? (
        <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      ) : null}
      <div className="flex-1 overflow-auto rounded-md border border-border-subtle">
        <table className="min-w-full divide-y divide-border-subtle font-mono text-xs">
          <thead className="bg-surface-panel-elevated text-[0.65rem] uppercase text-text-muted">
            <tr>
              <th className="px-3 py-1.5 text-left w-16">Index</th>
              <th className="px-3 py-1.5 text-left">Value</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {items.map((value, index) => (
              <tr
                key={`${start + index}-${formatValueOneLine(value)}`}
                className="hover:bg-white/5"
              >
                <td className="px-3 py-1 text-text-muted">{start + index}</td>
                <td className="break-all px-3 py-1">
                  {formatValueOneLine(value)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center justify-between gap-2 text-[0.65rem]">
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          disabled={page === 0 || loading}
          onClick={() => setPage((p) => Math.max(0, p - 1))}
        >
          Prev
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          disabled={!hasMore || loading}
          onClick={() => setPage((p) => p + 1)}
        >
          Next
        </Button>
      </div>
    </div>
  );
}
