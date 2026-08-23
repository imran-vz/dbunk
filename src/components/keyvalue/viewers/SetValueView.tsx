/**
 * Set viewer + editor (Tier 2). Same two-mode pattern as Hash, but
 * for unordered `SMEMBERS` / `SSCAN`. Edits queue SADD / SREM and
 * commit together on Save.
 */

import { useMemo, useState } from "react";

import { useRedisFetch } from "@/components/keyvalue/viewers/use-redis-fetch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  applyRedisSetEdits,
  fetchSet,
  formatValueOneLine,
  type SerializedValue,
} from "@/lib/redis/api";

interface SetValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

const FULL_FETCH_THRESHOLD = 500;

export function SetValueView({
  connectionId,
  keyName,
  elementCount,
}: SetValueViewProps) {
  const mode = useMemo<"full" | "scan">(
    () =>
      elementCount !== undefined && elementCount > FULL_FETCH_THRESHOLD
        ? "scan"
        : "full",
    [elementCount],
  );
  const [members, setMembers] = useState<SerializedValue[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [pattern, setPattern] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [pendingRemoves, setPendingRemoves] = useState<Set<string>>(new Set());
  const [pendingAdds, setPendingAdds] = useState<string[]>([]);
  const [newAdd, setNewAdd] = useState("");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  useRedisFetch({
    fetch: () =>
      fetchSet({
        connectionId,
        key: keyName,
        mode,
        count: 200,
        pattern: pattern.trim() ? `*${pattern.trim()}*` : null,
      }),
    onStart: () => {
      setLoading(true);
      setError(null);
      setMembers([]);
      setCursor(null);
    },
    onSuccess: (result) => {
      setMembers(result.members);
      setCursor(result.nextCursor);
    },
    onError: setError,
    onSettled: () => setLoading(false),
    cacheKey: `${connectionId}|${keyName}|${mode}|${pattern}|${reloadTick}`,
  });

  const loadMore = async () => {
    if (!cursor) return;
    setLoading(true);
    try {
      const result = await fetchSet({
        connectionId,
        key: keyName,
        mode: "scan",
        count: 200,
        cursor,
        pattern: pattern.trim() ? `*${pattern.trim()}*` : null,
      });
      setMembers((prev) => [...prev, ...result.members]);
      setCursor(result.nextCursor);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  };

  const resetEdits = () => {
    setPendingRemoves(new Set());
    setPendingAdds([]);
    setNewAdd("");
  };

  const dirty = pendingRemoves.size > 0 || pendingAdds.length > 0;

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await applyRedisSetEdits({
        connectionId,
        key: keyName,
        adds: pendingAdds,
        removes: Array.from(pendingRemoves),
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
      <div className="flex items-center gap-2 text-2xs text-text-muted">
        <Input
          placeholder={
            mode === "scan"
              ? "Filter members (server-side MATCH)…"
              : "Filter loaded members…"
          }
          value={pattern}
          onChange={(event) => setPattern(event.target.value)}
          className="h-7 max-w-sm text-xs"
        />
        <span>
          {members.length.toLocaleString()}{" "}
          {elementCount !== undefined
            ? `of ${elementCount.toLocaleString()}`
            : ""}{" "}
          members · mode: {mode}
        </span>
        <div className="ml-auto flex items-center gap-2">
          {!editing ? (
            <Button
              size="sm"
              variant="outline"
              className="h-7 px-2 text-2xs"
              onClick={() => setEditing(true)}
            >
              Edit
            </Button>
          ) : (
            <>
              <Button
                size="sm"
                variant="ghost"
                className="h-7 px-2 text-2xs"
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
                className="h-7 px-2 text-2xs"
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
        <div className="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
          {error}
        </div>
      ) : null}
      <div className="flex-1 overflow-auto rounded-md border border-border-subtle">
        <ul className="divide-y divide-border-subtle font-mono text-xs">
          {members.map((member) => {
            const name = formatValueOneLine(member);
            const isRemoved = pendingRemoves.has(name);
            return (
              <li
                key={name}
                className={`flex items-center justify-between gap-2 px-3 py-1 hover:bg-white/5 ${
                  isRemoved ? "opacity-40 line-through" : ""
                }`}
              >
                <span className="break-all">{name}</span>
                {editing ? (
                  <button
                    type="button"
                    onClick={() =>
                      setPendingRemoves((prev) => {
                        const next = new Set(prev);
                        if (next.has(name)) next.delete(name);
                        else next.add(name);
                        return next;
                      })
                    }
                    className="text-2xs text-danger hover:underline"
                  >
                    {isRemoved ? "undo" : "remove"}
                  </button>
                ) : null}
              </li>
            );
          })}
          {editing
            ? pendingAdds.map((value) => (
                <li
                  key={`add::${value}`}
                  className="flex items-center justify-between gap-2 bg-accent/5 px-3 py-1"
                >
                  <span className="break-all text-accent">+ {value}</span>
                  <button
                    type="button"
                    onClick={() =>
                      setPendingAdds((prev) =>
                        prev.filter((item) => item !== value),
                      )
                    }
                    className="text-2xs text-danger hover:underline"
                  >
                    remove
                  </button>
                </li>
              ))
            : null}
        </ul>
      </div>
      {editing ? (
        <div className="flex items-center gap-2 text-2xs">
          <Input
            value={newAdd}
            onChange={(event) => setNewAdd(event.target.value)}
            placeholder="new member (SADD)"
            className="h-7 max-w-sm text-xs"
          />
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-2 text-2xs"
            disabled={!newAdd}
            onClick={() => {
              if (!newAdd) return;
              setPendingAdds((prev) =>
                prev.includes(newAdd) ? prev : [...prev, newAdd],
              );
              setNewAdd("");
            }}
          >
            Add
          </Button>
        </div>
      ) : null}
      <div className="flex items-center justify-between text-2xs text-text-muted">
        <span>
          {mode === "scan"
            ? "Filtering all members (server-side SCAN)"
            : "Filtering loaded page only"}
        </span>
        {cursor ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-2xs"
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
