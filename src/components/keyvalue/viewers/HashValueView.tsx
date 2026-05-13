/**
 * Hash viewer + editor (Phase 1.4). Two-mode read (full vs SCAN) +
 * pending-edits map keyed by field name. Edits and deletes commit
 * together on Save.
 */

import { useMemo, useState } from "react";

import { useRedisFetch } from "@/components/keyvalue/viewers/use-redis-fetch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  deleteRedisHashFields,
  fetchHash,
  formatValueOneLine,
  type SerializedValue,
  setRedisHashFields,
} from "@/lib/redis/api";

interface HashValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

const FULL_FETCH_THRESHOLD = 500;

export function HashValueView({
  connectionId,
  keyName,
  elementCount,
}: HashValueViewProps) {
  const mode = useMemo<"full" | "scan">(
    () =>
      elementCount !== undefined && elementCount > FULL_FETCH_THRESHOLD
        ? "scan"
        : "full",
    [elementCount],
  );
  const [entries, setEntries] = useState<
    Array<[SerializedValue, SerializedValue]>
  >([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [pattern, setPattern] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [pendingSets, setPendingSets] = useState<Record<string, string>>({});
  const [pendingDeletes, setPendingDeletes] = useState<Set<string>>(new Set());
  const [newField, setNewField] = useState("");
  const [newValue, setNewValue] = useState("");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  useRedisFetch({
    fetch: () =>
      fetchHash({
        connectionId,
        key: keyName,
        mode,
        count: 200,
        pattern: pattern.trim() ? `*${pattern.trim()}*` : null,
      }),
    onStart: () => {
      setLoading(true);
      setError(null);
      setEntries([]);
      setCursor(null);
    },
    onSuccess: (result) => {
      setEntries(result.entries);
      setCursor(result.nextCursor);
    },
    onError: setError,
    onSettled: () => setLoading(false),
    cacheKey: `${connectionId}|${keyName}|${mode}|${pattern}|${reloadTick}`,
  });

  const fieldName = (entry: [SerializedValue, SerializedValue]): string =>
    entry[0].kind === "string" ? entry[0].value : formatValueOneLine(entry[0]);

  const cellValue = (entry: [SerializedValue, SerializedValue]): string => {
    const name = fieldName(entry);
    return pendingSets[name] !== undefined
      ? pendingSets[name]
      : formatValueOneLine(entry[1]);
  };

  const loadMore = async () => {
    if (!cursor) return;
    setLoading(true);
    try {
      const result = await fetchHash({
        connectionId,
        key: keyName,
        mode: "scan",
        count: 200,
        cursor,
        pattern: pattern.trim() ? `*${pattern.trim()}*` : null,
      });
      setEntries((prev) => [...prev, ...result.entries]);
      setCursor(result.nextCursor);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      const sets = Object.entries(pendingSets);
      const deletes = Array.from(pendingDeletes);
      await Promise.all([
        sets.length > 0
          ? setRedisHashFields({
              connectionId,
              key: keyName,
              entries: sets,
            })
          : Promise.resolve(),
        deletes.length > 0
          ? deleteRedisHashFields({
              connectionId,
              key: keyName,
              fields: deletes,
            })
          : Promise.resolve(),
      ]);
      setPendingSets({});
      setPendingDeletes(new Set());
      setEditing(false);
      setReloadTick((t) => t + 1);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  const dirty = Object.keys(pendingSets).length > 0 || pendingDeletes.size > 0;

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-4">
      <div className="flex items-center gap-2 text-[0.65rem] text-text-muted">
        <Input
          placeholder={
            mode === "scan"
              ? "Filter fields (server-side MATCH)…"
              : "Filter loaded fields…"
          }
          value={pattern}
          onChange={(event) => setPattern(event.target.value)}
          className="h-7 max-w-sm text-xs"
        />
        <span>
          {entries.length.toLocaleString()}{" "}
          {elementCount !== undefined
            ? `of ${elementCount.toLocaleString()}`
            : ""}{" "}
          fields · mode: {mode}
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
                onClick={() => {
                  setEditing(false);
                  setPendingSets({});
                  setPendingDeletes(new Set());
                  setNewField("");
                  setNewValue("");
                }}
                disabled={saving}
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
              <th className="px-3 py-1.5 text-left">Field</th>
              <th className="px-3 py-1.5 text-left">Value</th>
              {editing ? <th className="w-16 px-3 py-1.5"></th> : null}
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {entries.map((entry) => {
              const name = fieldName(entry);
              const isDeleted = pendingDeletes.has(name);
              return (
                <tr
                  key={name}
                  className={`hover:bg-white/5 ${isDeleted ? "opacity-40 line-through" : ""}`}
                >
                  <td className="px-3 py-1 text-text-secondary">{name}</td>
                  <td className="break-all px-3 py-1">
                    {editing && !isDeleted ? (
                      <input
                        value={cellValue(entry)}
                        onChange={(event) => {
                          setPendingSets((prev) => ({
                            ...prev,
                            [name]: event.target.value,
                          }));
                        }}
                        className="w-full bg-transparent font-mono text-xs outline-none"
                      />
                    ) : (
                      cellValue(entry)
                    )}
                  </td>
                  {editing ? (
                    <td className="px-3 py-1 text-right">
                      <button
                        type="button"
                        onClick={() => {
                          setPendingDeletes((prev) => {
                            const next = new Set(prev);
                            if (next.has(name)) next.delete(name);
                            else next.add(name);
                            return next;
                          });
                        }}
                        className="text-[0.65rem] text-destructive hover:underline"
                      >
                        {isDeleted ? "undo" : "delete"}
                      </button>
                    </td>
                  ) : null}
                </tr>
              );
            })}
            {editing ? (
              <tr>
                <td className="px-3 py-1">
                  <input
                    value={newField}
                    onChange={(event) => setNewField(event.target.value)}
                    placeholder="new field…"
                    className="w-full bg-transparent font-mono text-xs outline-none"
                  />
                </td>
                <td className="px-3 py-1">
                  <input
                    value={newValue}
                    onChange={(event) => setNewValue(event.target.value)}
                    placeholder="value"
                    className="w-full bg-transparent font-mono text-xs outline-none"
                  />
                </td>
                <td className="px-3 py-1 text-right">
                  <button
                    type="button"
                    disabled={!newField.trim()}
                    onClick={() => {
                      const name = newField.trim();
                      if (!name) return;
                      setPendingSets((prev) => ({ ...prev, [name]: newValue }));
                      setNewField("");
                      setNewValue("");
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
      <div className="flex items-center justify-between text-[0.65rem] text-text-muted">
        <span>
          {mode === "scan"
            ? "Filtering all fields (server-side SCAN)"
            : "Filtering loaded page only"}
        </span>
        {cursor ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem]"
            onClick={() => {
              void loadMore();
            }}
            disabled={loading || editing}
          >
            {loading ? "Loading…" : "Load more"}
          </Button>
        ) : null}
      </div>
    </div>
  );
}
