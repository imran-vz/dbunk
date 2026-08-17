/**
 * Stream viewer + editor (Tier 2). `XRANGE`/`XREVRANGE` with cursor
 * pagination via stream IDs. Entries render as ID + flat key/value
 * table. The editor queues `XADD` appends, `XDEL` removals, and an
 * optional `XTRIM MAXLEN ~` cap; consumer groups (`XINFO GROUPS` /
 * `XINFO CONSUMERS`) render in a collapsible read-only panel below
 * the table.
 */

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { useRedisFetch } from "@/components/keyvalue/viewers/use-redis-fetch";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  applyRedisStreamEdits,
  createStreamGroup,
  destroyStreamGroup,
  fetchStream,
  fetchStreamGroups,
  formatValueOneLine,
  type StreamConsumerInfo,
  type StreamEntry,
  type StreamGroupInfo,
} from "@/lib/redis/api";

interface StreamValueViewProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

type DraftAppend = {
  id: string;
  fields: Array<[string, string]>;
};

const EMPTY_DRAFT: DraftAppend = {
  id: "",
  fields: [["", ""]],
};

export function StreamValueView({
  connectionId,
  keyName,
  elementCount,
}: StreamValueViewProps) {
  const [reverse, setReverse] = useState(true); // newest-first default per Q15
  const [entries, setEntries] = useState<StreamEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [pendingDeletes, setPendingDeletes] = useState<Set<string>>(new Set());
  const [pendingAppends, setPendingAppends] = useState<DraftAppend[]>([]);
  const [draft, setDraft] = useState<DraftAppend>(EMPTY_DRAFT);
  const [trimMaxlen, setTrimMaxlen] = useState<string>("");
  const [saving, setSaving] = useState(false);
  const [reloadTick, setReloadTick] = useState(0);
  const [groups, setGroups] = useState<StreamGroupInfo[]>([]);
  const [consumers, setConsumers] = useState<StreamConsumerInfo[]>([]);
  const [groupsLoading, setGroupsLoading] = useState(false);
  const [groupsError, setGroupsError] = useState<string | null>(null);
  const [showGroups, setShowGroups] = useState(false);

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
    cacheKey: `${connectionId}|${keyName}|${reverse}|${reloadTick}`,
  });

  useEffect(() => {
    if (!showGroups) return;
    let cancelled = false;
    setGroupsLoading(true);
    setGroupsError(null);
    fetchStreamGroups({ connectionId, key: keyName })
      .then((result) => {
        if (cancelled) return;
        setGroups(result.groups);
        setConsumers(result.consumers);
      })
      .catch((err) => {
        if (cancelled) return;
        setGroupsError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setGroupsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId, keyName, showGroups, reloadTick]);

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

  const resetEdits = () => {
    setPendingDeletes(new Set());
    setPendingAppends([]);
    setDraft(EMPTY_DRAFT);
    setTrimMaxlen("");
  };

  const dirty =
    pendingDeletes.size > 0 ||
    pendingAppends.length > 0 ||
    trimMaxlen.trim() !== "";

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      const maxlen = trimMaxlen.trim() === "" ? null : Number(trimMaxlen);
      if (maxlen !== null && (Number.isNaN(maxlen) || maxlen < 0)) {
        throw new Error("MAXLEN must be a non-negative number");
      }
      await applyRedisStreamEdits({
        connectionId,
        key: keyName,
        appends: pendingAppends.map((entry) => ({
          id: entry.id.trim() === "" ? null : entry.id.trim(),
          fields: entry.fields.filter(([k]) => k.length > 0),
        })),
        deletes: Array.from(pendingDeletes),
        trimMaxlen: maxlen,
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
              <th className="px-3 py-1.5 text-left">ID</th>
              <th className="px-3 py-1.5 text-left">Fields</th>
              {editing ? <th className="w-16 px-3 py-1.5"></th> : null}
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {entries.map((entry) => {
              const isDeleted = pendingDeletes.has(entry.id);
              return (
                <tr
                  key={entry.id}
                  className={`hover:bg-white/5 ${isDeleted ? "opacity-40 line-through" : ""}`}
                >
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
                  {editing ? (
                    <td className="px-3 py-1 text-right">
                      <button
                        type="button"
                        onClick={() =>
                          setPendingDeletes((prev) => {
                            const next = new Set(prev);
                            if (next.has(entry.id)) next.delete(entry.id);
                            else next.add(entry.id);
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
          </tbody>
        </table>
      </div>
      {editing ? (
        <div className="flex flex-col gap-2 rounded-md border border-accent/30 bg-accent/5 p-2 text-[0.65rem]">
          <div className="font-semibold text-foreground">Append entries</div>
          {pendingAppends.map((entry, index) => (
            <div
              // oxlint-disable-next-line react/no-array-index-key -- order is the identity
              key={`pending-${index}`}
              className="rounded border border-border-subtle bg-surface-panel-elevated p-1.5"
            >
              <div className="flex items-center gap-2">
                <span className="font-mono text-text-muted">id</span>
                <span className="font-mono text-foreground">
                  {entry.id || "(auto)"}
                </span>
                {entry.fields.map(([k, v], i) => (
                  // oxlint-disable-next-line react/no-array-index-key -- order is the identity
                  <span key={i} className="font-mono">
                    <span className="text-text-muted">{k}</span>=
                    <span>{v}</span>
                  </span>
                ))}
                <button
                  type="button"
                  className="ml-auto text-destructive hover:underline"
                  onClick={() =>
                    setPendingAppends((prev) =>
                      prev.filter((_, i) => i !== index),
                    )
                  }
                >
                  remove
                </button>
              </div>
            </div>
          ))}
          <DraftEditor
            draft={draft}
            onChange={setDraft}
            onAdd={() => {
              const fields = draft.fields.filter(([k]) => k.length > 0);
              if (fields.length === 0) return;
              setPendingAppends((prev) => [...prev, { id: draft.id, fields }]);
              setDraft(EMPTY_DRAFT);
            }}
          />
          <div className="flex items-center gap-2">
            <span className="text-text-muted">Trim MAXLEN ~</span>
            <Input
              value={trimMaxlen}
              onChange={(event) => setTrimMaxlen(event.target.value)}
              placeholder="leave blank for no trim"
              className="h-6 w-32 text-xs"
              aria-label="Stream MAXLEN trim count"
            />
          </div>
        </div>
      ) : null}
      <div className="flex items-center justify-between text-[0.65rem]">
        <button
          type="button"
          className="text-text-muted hover:underline"
          onClick={() => setShowGroups((prev) => !prev)}
        >
          {showGroups ? "Hide" : "Show"} consumer groups
        </button>
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          disabled={loading || entries.length === 0 || editing}
          onClick={() => {
            void loadMore();
          }}
        >
          {loading ? "Loading…" : "Load more"}
        </Button>
      </div>
      {showGroups ? (
        <ConsumerGroupsPanel
          connectionId={connectionId}
          keyName={keyName}
          groups={groups}
          consumers={consumers}
          loading={groupsLoading}
          error={groupsError}
          onChanged={() => setReloadTick((t) => t + 1)}
        />
      ) : null}
    </div>
  );
}

function DraftEditor({
  draft,
  onChange,
  onAdd,
}: {
  draft: DraftAppend;
  onChange: (next: DraftAppend) => void;
  onAdd: () => void;
}) {
  return (
    <div className="rounded border border-border-subtle bg-surface-panel-elevated p-1.5">
      <div className="flex items-center gap-2">
        <span className="text-text-muted">id</span>
        <Input
          value={draft.id}
          onChange={(event) => onChange({ ...draft, id: event.target.value })}
          placeholder="* (auto)"
          className="h-6 w-32 font-mono text-xs"
        />
        <Button
          size="sm"
          variant="outline"
          className="ml-auto h-6 px-2 text-[0.65rem]"
          onClick={onAdd}
        >
          Stage append
        </Button>
      </div>
      <div className="mt-1.5 flex flex-col gap-1">
        {draft.fields.map(([key, value], index) => (
          // oxlint-disable-next-line react/no-array-index-key -- field-row identity is index
          <div key={`field-${index}`} className="flex items-center gap-1.5">
            <Input
              value={key}
              onChange={(event) =>
                onChange({
                  ...draft,
                  fields: draft.fields.map((row, i) =>
                    i === index ? [event.target.value, row[1]] : row,
                  ),
                })
              }
              placeholder="field"
              className="h-6 max-w-xs font-mono text-xs"
            />
            <Input
              value={value}
              onChange={(event) =>
                onChange({
                  ...draft,
                  fields: draft.fields.map((row, i) =>
                    i === index ? [row[0], event.target.value] : row,
                  ),
                })
              }
              placeholder="value"
              className="h-6 flex-1 font-mono text-xs"
            />
            {draft.fields.length > 1 ? (
              <button
                type="button"
                className="text-[0.65rem] text-destructive hover:underline"
                onClick={() =>
                  onChange({
                    ...draft,
                    fields: draft.fields.filter((_, i) => i !== index),
                  })
                }
              >
                ×
              </button>
            ) : null}
          </div>
        ))}
        <button
          type="button"
          className="self-start text-[0.65rem] text-primary hover:underline"
          onClick={() =>
            onChange({ ...draft, fields: [...draft.fields, ["", ""]] })
          }
        >
          + field
        </button>
      </div>
    </div>
  );
}

function ConsumerGroupsPanel({
  connectionId,
  keyName,
  groups,
  consumers,
  loading,
  error,
  onChanged,
}: {
  connectionId: string;
  keyName: string;
  groups: StreamGroupInfo[];
  consumers: StreamConsumerInfo[];
  loading: boolean;
  error: string | null;
  onChanged: () => void;
}) {
  const [newGroupName, setNewGroupName] = useState("");
  const [newGroupStartId, setNewGroupStartId] = useState("$");
  const [mkstream, setMkstream] = useState(false);
  const [creating, setCreating] = useState(false);

  const handleCreate = async () => {
    const group = newGroupName.trim();
    const startId = newGroupStartId.trim();
    if (!group || !startId) return;
    setCreating(true);
    try {
      await createStreamGroup({
        connectionId,
        key: keyName,
        group,
        startId,
        mkstream,
      });
      toast.success(`Created group ${group}`);
      setNewGroupName("");
      onChanged();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    } finally {
      setCreating(false);
    }
  };

  const handleDestroy = async (group: string) => {
    if (
      !window.confirm(`Destroy consumer group ${group}? This is permanent.`)
    ) {
      return;
    }
    try {
      await destroyStreamGroup({ connectionId, key: keyName, group });
      toast.success(`Destroyed group ${group}`);
      onChanged();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="rounded-md border border-border-subtle bg-surface-panel-elevated/40 p-2 text-[0.65rem]">
      <div className="flex items-center gap-2">
        <span className="font-semibold text-foreground">Consumer groups</span>
        {loading ? <span className="text-text-muted">loading…</span> : null}
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        <Input
          value={newGroupName}
          onChange={(event) => setNewGroupName(event.target.value)}
          placeholder="group name"
          className="h-6 max-w-[10rem] font-mono text-xs"
          aria-label="New group name"
        />
        <Input
          value={newGroupStartId}
          onChange={(event) => setNewGroupStartId(event.target.value)}
          placeholder="$"
          className="h-6 max-w-[6rem] font-mono text-xs"
          aria-label="New group start ID"
          title="$ = new entries only · 0 = replay all · or a specific ms-seq ID"
        />
        <label className="flex items-center gap-1 text-text-muted">
          <input
            type="checkbox"
            checked={mkstream}
            onChange={(event) => setMkstream(event.target.checked)}
          />
          MKSTREAM
        </label>
        <Button
          size="sm"
          variant="outline"
          className="h-6 px-2 text-[0.65rem]"
          disabled={creating || !newGroupName.trim() || !newGroupStartId.trim()}
          onClick={() => {
            void handleCreate();
          }}
        >
          {creating ? "Creating…" : "Create group"}
        </Button>
      </div>
      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-destructive">
          {error}
        </div>
      ) : groups.length === 0 ? (
        <div className="mt-2 text-text-muted">
          No consumer groups defined on this stream.
        </div>
      ) : (
        <>
          <table className="mt-2 min-w-full divide-y divide-border-subtle font-mono">
            <thead className="text-[0.6rem] uppercase text-text-muted">
              <tr>
                <th className="px-2 py-1 text-left">Group</th>
                <th className="px-2 py-1 text-right">Consumers</th>
                <th className="px-2 py-1 text-right">Pending</th>
                <th className="px-2 py-1 text-left">Last delivered</th>
                <th className="w-16 px-2 py-1"></th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border-subtle">
              {groups.map((group) => (
                <tr key={group.name}>
                  <td className="px-2 py-1">{group.name}</td>
                  <td className="px-2 py-1 text-right">{group.consumers}</td>
                  <td className="px-2 py-1 text-right">{group.pending}</td>
                  <td className="px-2 py-1">{group.lastDeliveredId}</td>
                  <td className="px-2 py-1 text-right">
                    <button
                      type="button"
                      onClick={() => {
                        void handleDestroy(group.name);
                      }}
                      className="text-destructive hover:underline"
                    >
                      destroy
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {consumers.length > 0 ? (
            <table className="mt-2 min-w-full divide-y divide-border-subtle font-mono">
              <thead className="text-[0.6rem] uppercase text-text-muted">
                <tr>
                  <th className="px-2 py-1 text-left">Group</th>
                  <th className="px-2 py-1 text-left">Consumer</th>
                  <th className="px-2 py-1 text-right">Pending</th>
                  <th className="px-2 py-1 text-right">Idle (ms)</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border-subtle">
                {consumers.map((consumer) => (
                  <tr key={`${consumer.group}/${consumer.name}`}>
                    <td className="px-2 py-1 text-text-muted">
                      {consumer.group}
                    </td>
                    <td className="px-2 py-1">{consumer.name}</td>
                    <td className="px-2 py-1 text-right">{consumer.pending}</td>
                    <td className="px-2 py-1 text-right">{consumer.idleMs}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : null}
        </>
      )}
    </div>
  );
}
