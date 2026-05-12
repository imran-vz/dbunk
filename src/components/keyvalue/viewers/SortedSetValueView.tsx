/**
 * Sorted-set viewer — mode picker (rank / byscore), direction
 * toggle. Offset pagination for rank, score-range inputs for byscore.
 */

import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  fetchSortedSet,
  formatValueOneLine,
  type SerializedValue,
} from "@/lib/redis/api";

interface SortedSetValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

const PAGE_SIZES = [50, 100, 200, 500, 1000] as const;

export function SortedSetValueView({
  connectionId,
  keyName,
  elementCount,
}: SortedSetValueViewProps) {
  const [mode, setMode] = useState<"rank" | "byscore">("rank");
  const [page, setPage] = useState(0);
  const [pageSize, setPageSize] = useState<number>(200);
  const [reverse, setReverse] = useState(false);
  const [scoreMin, setScoreMin] = useState("-inf");
  const [scoreMax, setScoreMax] = useState("+inf");
  const [entries, setEntries] = useState<Array<[SerializedValue, number]>>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetchSortedSet({
      connectionId,
      key: keyName,
      mode,
      start: page * pageSize,
      stop: (page + 1) * pageSize - 1,
      reverse,
      scoreMin: mode === "byscore" ? scoreMin : undefined,
      scoreMax: mode === "byscore" ? scoreMax : undefined,
    })
      .then((result) => {
        if (!cancelled) setEntries(result.entries);
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
  }, [
    connectionId,
    keyName,
    mode,
    page,
    pageSize,
    reverse,
    scoreMin,
    scoreMax,
  ]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <div className="flex flex-wrap items-center gap-2 text-[0.65rem] text-text-muted">
        <div className="inline-flex rounded-md border border-border-subtle">
          <button
            type="button"
            onClick={() => {
              setMode("rank");
              setPage(0);
            }}
            className={`px-2 py-0.5 text-[0.65rem] ${
              mode === "rank"
                ? "bg-primary/15 text-primary"
                : "text-text-muted hover:text-foreground"
            }`}
          >
            By rank
          </button>
          <button
            type="button"
            onClick={() => {
              setMode("byscore");
              setPage(0);
            }}
            className={`border-l border-border-subtle px-2 py-0.5 text-[0.65rem] ${
              mode === "byscore"
                ? "bg-primary/15 text-primary"
                : "text-text-muted hover:text-foreground"
            }`}
          >
            By score
          </button>
        </div>
        <Button
          variant={reverse ? "default" : "outline"}
          size="sm"
          className="h-6 px-2 text-[0.65rem]"
          onClick={() => setReverse((prev) => !prev)}
        >
          {reverse ? "Desc" : "Asc"}
        </Button>
        {mode === "byscore" ? (
          <>
            <Input
              value={scoreMin}
              onChange={(e) => setScoreMin(e.target.value)}
              className="h-6 w-24 text-xs"
            />
            <span>…</span>
            <Input
              value={scoreMax}
              onChange={(e) => setScoreMax(e.target.value)}
              className="h-6 w-24 text-xs"
            />
          </>
        ) : (
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
        )}
        <span>
          {entries.length.toLocaleString()}
          {elementCount !== undefined
            ? ` of ${elementCount.toLocaleString()}`
            : ""}{" "}
          entries
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
              <th className="w-24 px-3 py-1.5 text-right">Score</th>
              <th className="px-3 py-1.5 text-left">Member</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {entries.map(([member, score]) => (
              <tr
                key={`${score}-${formatValueOneLine(member)}`}
                className="hover:bg-white/5"
              >
                <td className="px-3 py-1 text-right text-text-secondary">
                  {score}
                </td>
                <td className="break-all px-3 py-1">
                  {formatValueOneLine(member)}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {mode === "rank" ? (
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
            disabled={entries.length < pageSize || loading}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
          </Button>
        </div>
      ) : null}
    </div>
  );
}
