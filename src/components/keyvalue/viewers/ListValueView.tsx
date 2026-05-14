/**
 * List viewer + editor (Tier 2). Offset-paginated `LRANGE` with a
 * direction toggle and page-size selector; per-page edits queue up
 * (`LSET` for in-place updates, append for new tail entries, and
 * tag-and-`LREM` for deletes — see `key_ops.rs::apply_list_edits`).
 *
 * Edits only touch the visible page. Index-based deletes use the
 * absolute position (head-relative when `reverse=false`, tail-relative
 * when `reverse=true`, which Redis handles natively via negative
 * indices). The page is reloaded after Save.
 */

import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  applyRedisListEdits,
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
  const [editing, setEditing] = useState(false);
  const [pendingSets, setPendingSets] = useState<Record<number, string>>({});
  const [pendingDeletes, setPendingDeletes] = useState<Set<number>>(new Set());
  const [pendingAppends, setPendingAppends] = useState<string[]>([]);
  const [newAppend, setNewAppend] = useState("");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  const start = page * pageSize;
  const stop = start + pageSize - 1;

  // biome-ignore lint/correctness/useExhaustiveDependencies: reloadTick is the explicit refetch trigger after Save
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
  }, [connectionId, keyName, start, stop, reverse, reloadTick]);

  const hasMore = elementCount === undefined || start + pageSize < elementCount;

  const resetEdits = () => {
    setPendingSets({});
    setPendingDeletes(new Set());
    setPendingAppends([]);
    setNewAppend("");
  };

  const cellValue = (absIndex: number, value: SerializedValue): string =>
    pendingSets[absIndex] !== undefined
      ? pendingSets[absIndex]
      : formatValueOneLine(value);

  const dirty =
    Object.keys(pendingSets).length > 0 ||
    pendingDeletes.size > 0 ||
    pendingAppends.length > 0;

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      // Tail-first paging: absIndex 0 is the tail of the list, which
      // Redis addresses as -1. The backend's tail-window LRANGE
      // (key_inspector::fetch_list) ensures the displayed row matches.
      const toRedisIndex = (absIndex: number): number =>
        reverse ? -(absIndex + 1) : absIndex;
      await applyRedisListEdits({
        connectionId,
        key: keyName,
        sets: Object.entries(pendingSets).map(([index, value]) => ({
          index: toRedisIndex(Number(index)),
          value,
        })),
        deletes: Array.from(pendingDeletes).map(toRedisIndex),
        appends: pendingAppends,
      });
      resetEdits();
      setEditing(false);
      setReloadTick((t) => t + 1);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

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
        <div className="ml-auto flex items-center gap-2">
          {!editing ? (
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2 text-[0.65rem]"
              onClick={() => setEditing(true)}
            >
              Edit
            </Button>
          ) : (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-[0.65rem]"
                disabled={saving}
                onClick={() => {
                  setEditing(false);
                  resetEdits();
                }}
              >
                Cancel
              </Button>
              <Button
                size="sm"
                className="h-7 px-2 text-[0.65rem]"
                disabled={saving || !dirty}
                onClick={() => {
                  void handleSave();
                }}
              >
                {saving ? "Saving…" : "Save"}
              </Button>
            </>
          )}
        </div>
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
              {editing ? <th className="w-16 px-3 py-1.5"></th> : null}
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {items.map((value, offset) => {
              const absIndex = start + offset;
              const isDeleted = pendingDeletes.has(absIndex);
              return (
                <tr
                  key={`${absIndex}-${formatValueOneLine(value)}`}
                  className={`hover:bg-white/5 ${isDeleted ? "opacity-40 line-through" : ""}`}
                >
                  <td className="px-3 py-1 text-text-muted">{absIndex}</td>
                  <td className="break-all px-3 py-1">
                    {editing && !isDeleted ? (
                      <input
                        value={cellValue(absIndex, value)}
                        onChange={(event) =>
                          setPendingSets((prev) => ({
                            ...prev,
                            [absIndex]: event.target.value,
                          }))
                        }
                        className="w-full bg-transparent font-mono text-xs outline-none"
                      />
                    ) : (
                      cellValue(absIndex, value)
                    )}
                  </td>
                  {editing ? (
                    <td className="px-3 py-1 text-right">
                      <button
                        type="button"
                        onClick={() =>
                          setPendingDeletes((prev) => {
                            const next = new Set(prev);
                            if (next.has(absIndex)) next.delete(absIndex);
                            else next.add(absIndex);
                            return next;
                          })
                        }
                        className="text-[0.65rem] text-destructive hover:underline"
                      >
                        {isDeleted ? "undo" : "delete"}
                      </button>
                    </td>
                  ) : null}
                </tr>
              );
            })}
            {editing
              ? pendingAppends.map((value, index) => (
                  <tr
                    // biome-ignore lint/suspicious/noArrayIndexKey: append-order identity
                    key={`append-${index}`}
                    className="bg-accent-green/5"
                  >
                    <td className="px-3 py-1 text-[0.65rem] text-accent-green">
                      new
                    </td>
                    <td className="break-all px-3 py-1">
                      <input
                        value={value}
                        onChange={(event) =>
                          setPendingAppends((prev) => {
                            const next = prev.slice();
                            next[index] = event.target.value;
                            return next;
                          })
                        }
                        className="w-full bg-transparent font-mono text-xs outline-none"
                      />
                    </td>
                    <td className="px-3 py-1 text-right">
                      <button
                        type="button"
                        onClick={() =>
                          setPendingAppends((prev) =>
                            prev.filter((_, i) => i !== index),
                          )
                        }
                        className="text-[0.65rem] text-destructive hover:underline"
                      >
                        remove
                      </button>
                    </td>
                  </tr>
                ))
              : null}
            {editing ? (
              <tr>
                <td className="px-3 py-1 text-[0.65rem] text-text-muted">
                  append
                </td>
                <td className="px-3 py-1">
                  <input
                    value={newAppend}
                    onChange={(event) => setNewAppend(event.target.value)}
                    placeholder="new value (RPUSH)"
                    className="w-full bg-transparent font-mono text-xs outline-none"
                  />
                </td>
                <td className="px-3 py-1 text-right">
                  <button
                    type="button"
                    disabled={!newAppend}
                    onClick={() => {
                      if (!newAppend) return;
                      setPendingAppends((prev) => [...prev, newAppend]);
                      setNewAppend("");
                    }}
                    className="text-[0.65rem] text-primary hover:underline disabled:opacity-30"
                  >
                    add
                  </button>
                </td>
              </tr>
            ) : null}
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
