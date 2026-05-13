/**
 * Stream viewer — `XRANGE`/`XREVRANGE` with cursor pagination via
 * stream IDs. Renders entries as ID + flat key/value table.
 */

import { useState } from "react";

import { useRedisFetch } from "@/components/keyvalue/viewers/use-redis-fetch";
import { Button } from "@/components/ui/button";
import {
  fetchStream,
  formatValueOneLine,
  type StreamEntry,
} from "@/lib/redis/api";

interface StreamValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

export function StreamValueView({
  connectionId,
  keyName,
  elementCount,
}: StreamValueViewProps) {
  const [reverse, setReverse] = useState(true); // newest-first default per Q15
  const [entries, setEntries] = useState<StreamEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useRedisFetch({
    fetch: () =>
      fetchStream({
        connectionId,
        key: keyName,
        count: 200,
        reverse,
      }),
    onStart: () => {
      setLoading(true);
      setError(null);
    },
    onSuccess: (result) => setEntries(result.entries),
    onError: setError,
    onSettled: () => setLoading(false),
    cacheKey: `${connectionId}|${keyName}|${reverse}`,
  });

  const loadMore = async () => {
    if (entries.length === 0) return;
    const lastId = entries[entries.length - 1].id;
    setLoading(true);
    try {
      const result = await fetchStream({
        connectionId,
        key: keyName,
        start: reverse ? "-" : lastId,
        end: reverse ? lastId : "+",
        count: 200,
        reverse,
      });
      // Drop the first entry if it matches the lastId boundary.
      const next = result.entries.filter((e) => e.id !== lastId);
      setEntries((prev) => [...prev, ...next]);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <div className="flex items-center gap-2 text-[0.65rem] text-text-muted">
        <Button
          variant={reverse ? "default" : "outline"}
          size="sm"
          className="h-6 px-2 text-[0.65rem]"
          onClick={() => setReverse((prev) => !prev)}
        >
          {reverse ? "Newest first" : "Oldest first"}
        </Button>
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
              <th className="px-3 py-1.5 text-left">ID</th>
              <th className="px-3 py-1.5 text-left">Fields</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {entries.map((entry) => (
              <tr key={entry.id} className="hover:bg-white/5">
                <td className="whitespace-nowrap px-3 py-1 text-text-secondary">
                  {entry.id}
                </td>
                <td className="px-3 py-1">
                  <div className="flex flex-col gap-0.5">
                    {entry.fields.map(([k, v]) => (
                      <div
                        key={formatValueOneLine(k)}
                        className="flex gap-2 break-all"
                      >
                        <span className="text-text-muted">
                          {formatValueOneLine(k)}
                        </span>
                        <span>{formatValueOneLine(v)}</span>
                      </div>
                    ))}
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center justify-between text-[0.65rem]">
        <span className="text-text-muted">
          Stream consumer groups (XINFO GROUPS) — deferred to Tier 2.
        </span>
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          disabled={loading || entries.length === 0}
          onClick={() => {
            void loadMore();
          }}
        >
          {loading ? "Loading…" : "Load more"}
        </Button>
      </div>
    </div>
  );
}
