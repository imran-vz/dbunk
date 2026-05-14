/**
 * Sorted-set viewer + editor (Tier 2). Mode picker (rank / byscore),
 * direction toggle. Offset pagination for rank, score-range inputs
 * for byscore. Editor lets the user change a member's score
 * (`ZADD`), remove a member (`ZREM`), and add new members; all
 * commit together on Save.
 */

import { useState } from "react";

import { useRedisFetch } from "@/components/keyvalue/viewers/use-redis-fetch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  applyRedisSortedSetEdits,
  fetchSortedSet,
  formatValueOneLine,
  type SerializedValue,
  type SortedSetUpsert,
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
  const [editing, setEditing] = useState(false);
  const [pendingScores, setPendingScores] = useState<Record<string, number>>(
    {},
  );
  const [pendingRemoves, setPendingRemoves] = useState<Set<string>>(new Set());
  const [pendingAdds, setPendingAdds] = useState<SortedSetUpsert[]>([]);
  const [newMember, setNewMember] = useState("");
  const [newScore, setNewScore] = useState("0");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);

  useRedisFetch({
    fetch: () =>
      fetchSortedSet({
        connectionId,
        key: keyName,
        mode,
        start: page * pageSize,
        stop: (page + 1) * pageSize - 1,
        reverse,
        scoreMin: mode === "byscore" ? scoreMin : undefined,
        scoreMax: mode === "byscore" ? scoreMax : undefined,
      }),
    onStart: () => {
      setLoading(true);
      setError(null);
    },
    onSuccess: (result) => setEntries(result.entries),
    onError: setError,
    onSettled: () => setLoading(false),
    cacheKey: `${connectionId}|${keyName}|${mode}|${page}|${pageSize}|${reverse}|${scoreMin}|${scoreMax}|${reloadTick}`,
  });

  const resetEdits = () => {
    setPendingScores({});
    setPendingRemoves(new Set());
    setPendingAdds([]);
    setNewMember("");
    setNewScore("0");
  };

  const dirty =
    Object.keys(pendingScores).length > 0 ||
    pendingRemoves.size > 0 ||
    pendingAdds.length > 0;

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      const upserts: SortedSetUpsert[] = [
        ...Object.entries(pendingScores).map(([member, score]) => ({
          member,
          score,
        })),
        ...pendingAdds,
      ];
      await applyRedisSortedSetEdits({
        connectionId,
        key: keyName,
        upserts,
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
              <th className="w-32 px-3 py-1.5 text-right">Score</th>
              <th className="px-3 py-1.5 text-left">Member</th>
              {editing ? <th className="w-16 px-3 py-1.5"></th> : null}
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {entries.map(([member, score]) => {
              const name = formatValueOneLine(member);
              const isRemoved = pendingRemoves.has(name);
              const displayScore =
                pendingScores[name] !== undefined ? pendingScores[name] : score;
              return (
                <tr
                  key={name}
                  className={`hover:bg-white/5 ${isRemoved ? "opacity-40 line-through" : ""}`}
                >
                  <td className="px-3 py-1 text-right text-text-secondary">
                    {editing && !isRemoved ? (
                      <input
                        type="number"
                        step="any"
                        value={displayScore}
                        onChange={(event) =>
                          setPendingScores((prev) => ({
                            ...prev,
                            [name]: Number(event.target.value),
                          }))
                        }
                        className="w-full bg-transparent text-right font-mono text-xs outline-none"
                      />
                    ) : (
                      displayScore
                    )}
                  </td>
                  <td className="break-all px-3 py-1">{name}</td>
                  {editing ? (
                    <td className="px-3 py-1 text-right">
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
                        className="text-[0.65rem] text-destructive hover:underline"
                      >
                        {isRemoved ? "undo" : "remove"}
                      </button>
                    </td>
                  ) : null}
                </tr>
              );
            })}
            {editing
              ? pendingAdds.map((upsert, index) => (
                  <tr
                    // biome-ignore lint/suspicious/noArrayIndexKey: order is the identity
                    key={`add-${index}`}
                    className="bg-accent-green/5"
                  >
                    <td className="px-3 py-1 text-right text-accent-green">
                      {upsert.score}
                    </td>
                    <td className="break-all px-3 py-1 text-accent-green">
                      + {upsert.member}
                    </td>
                    <td className="px-3 py-1 text-right">
                      <button
                        type="button"
                        onClick={() =>
                          setPendingAdds((prev) =>
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
          </tbody>
        </table>
      </div>
      {editing ? (
        <div className="flex flex-wrap items-center gap-2 text-[0.65rem]">
          <Input
            value={newMember}
            onChange={(event) => setNewMember(event.target.value)}
            placeholder="new member"
            className="h-7 max-w-sm text-xs"
          />
          <Input
            value={newScore}
            type="number"
            step="any"
            onChange={(event) => setNewScore(event.target.value)}
            placeholder="score"
            className="h-7 w-24 text-xs"
          />
          <Button
            size="sm"
            variant="outline"
            className="h-7 px-2 text-[0.65rem]"
            disabled={!newMember}
            onClick={() => {
              if (!newMember) return;
              setPendingAdds((prev) => [
                ...prev,
                { member: newMember, score: Number(newScore) || 0 },
              ]);
              setNewMember("");
              setNewScore("0");
            }}
          >
            Add
          </Button>
        </div>
      ) : null}
      {mode === "rank" ? (
        <div className="flex items-center justify-between gap-2 text-[0.65rem]">
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem]"
            disabled={page === 0 || loading || editing}
            onClick={() => setPage((p) => Math.max(0, p - 1))}
          >
            Prev
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem]"
            disabled={entries.length < pageSize || loading || editing}
            onClick={() => setPage((p) => p + 1)}
          >
            Next
          </Button>
        </div>
      ) : null}
    </div>
  );
}
