/**
 * Keyspace browser — sidebar component for Redis connections.
 *
 * Hybrid mode (Q2 of the grilling session):
 *  - default mode: a flat SCAN-paged list of keys, optionally
 *    grouped by the connection's separator into a prefix tree.
 *  - search-active mode: typing >2 chars switches the panel into
 *    flat result mode with `SCAN MATCH *foo*` (debounced).
 *  - type-filter chips: clicking adds `TYPE <name>` to the SCAN.
 *
 * The actual prefix-tree expansion (lazy `SCAN MATCH prefix:*` per
 * branch) is deferred to Phase 2 — v1.2 ships the flat list with
 * client-side tree grouping over what's already loaded.
 */

import {
  IconAlertTriangle,
  IconBraces,
  IconDatabaseStar,
  IconHash,
  IconLayoutList,
  IconList,
  IconSearch,
  IconWaveSine,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { type ScannedKey, scanKeys } from "@/lib/redis/api";
import type { Connection } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface KeyspaceBrowserProps {
  connection: Connection;
  onOpenKey: (key: string, type: string) => void;
  activeKey?: string;
}

const KEY_TYPES = [
  { id: "string", label: "Strings" },
  { id: "hash", label: "Hashes" },
  { id: "list", label: "Lists" },
  { id: "set", label: "Sets" },
  { id: "zset", label: "Sorted sets" },
  { id: "stream", label: "Streams" },
  { id: "ReJSON-RL", label: "JSON" },
] as const;

export function KeyspaceBrowser({
  connection,
  onOpenKey,
  activeKey,
}: KeyspaceBrowserProps) {
  const [search, setSearch] = useState("");
  const [typeFilter, setTypeFilter] = useState<string | null>(null);
  const [keys, setKeys] = useState<ScannedKey[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeq = useRef(0);
  // Replica-warning dismiss is session-local — a future migration can
  // persist `dismissed_replica_warning_at` on the connection record.
  const [replicaWarningDismissed, setReplicaWarningDismissed] = useState(false);
  const capabilities = useAppStore(
    (state) => state.redisCapabilitiesByConnection[connection.id],
  );
  const showReplicaWarning =
    !replicaWarningDismissed &&
    capabilities?.role === "master" &&
    (capabilities.connectedSlaves ?? 0) > 0;

  const pattern = useMemo(() => {
    const trimmed = search.trim();
    if (trimmed.length < 2) return "*";
    return `*${trimmed}*`;
  }, [search]);

  const runScan = useCallback(
    async (fromCursor: string | null) => {
      const seq = ++requestSeq.current;
      setLoading(true);
      setError(null);
      try {
        const result = await scanKeys({
          connectionId: connection.id,
          pattern,
          count: 200,
          typeFilter: typeFilter ?? undefined,
          cursor: fromCursor,
        });
        if (requestSeq.current !== seq) return;
        setKeys((prev) =>
          fromCursor === null ? result.keys : [...prev, ...result.keys],
        );
        setCursor(result.nextCursor);
      } catch (err) {
        if (requestSeq.current !== seq) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq.current === seq) {
          setLoading(false);
        }
      }
    },
    [connection.id, pattern, typeFilter],
  );

  // Restart scan when pattern or filter changes.
  useEffect(() => {
    setKeys([]);
    setCursor(null);
    void runScan(null);
  }, [runScan]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-2 p-2 text-xs">
      {showReplicaWarning ? (
        <output className="flex items-start gap-2 rounded-md border border-warning/30 bg-warning/10 px-2 py-1.5 text-[0.65rem] text-warning">
          <IconAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
          <div className="flex-1">
            This is a primary with{" "}
            <span className="font-semibold">
              {capabilities?.connectedSlaves}
            </span>{" "}
            replica{capabilities?.connectedSlaves === 1 ? "" : "s"} attached —
            writes propagate. Be especially careful with destructive commands
            like <code className="font-mono">FLUSHDB</code> /{" "}
            <code className="font-mono">FLUSHALL</code>.
          </div>
          <button
            type="button"
            onClick={() => setReplicaWarningDismissed(true)}
            aria-label="Dismiss replica warning"
            className="text-warning/70 hover:text-warning"
          >
            <IconX className="size-3" />
          </button>
        </output>
      ) : null}
      <div className="relative">
        <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
        <Input
          placeholder="Filter keys… (≥2 chars)"
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          className="h-8 pl-7 text-xs"
        />
      </div>
      <div className="flex flex-wrap gap-1">
        {KEY_TYPES.map((t) => (
          <button
            key={t.id}
            type="button"
            onClick={() =>
              setTypeFilter((prev) => (prev === t.id ? null : t.id))
            }
            className={cn(
              "rounded-md border px-2 py-0.5 text-[0.625rem]",
              typeFilter === t.id
                ? "border-primary bg-primary/15 text-primary"
                : "border-border-subtle text-text-muted hover:border-border hover:text-foreground",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>
      {error ? (
        <div
          role="alert"
          className="rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-[0.65rem] text-destructive"
        >
          {error}
        </div>
      ) : null}
      <div className="flex-1 overflow-auto rounded-md border border-border-subtle bg-surface-panel">
        {keys.length === 0 && !loading ? (
          <div className="px-2 py-3 text-[0.65rem] text-text-muted">
            No keys match. Adjust the filter or clear the search.
          </div>
        ) : (
          <ul className="divide-y divide-border-subtle">
            {keys.map((key) => (
              <li key={`${key.name}::${key.type}`}>
                <button
                  type="button"
                  onClick={() => onOpenKey(key.name, key.type)}
                  className={cn(
                    "flex w-full items-center gap-2 px-2 py-1 text-left hover:bg-white/5",
                    activeKey === key.name && "bg-primary/10 text-primary",
                  )}
                >
                  <KeyTypeIcon type={key.type} />
                  <span className="truncate font-mono text-[0.65rem]">
                    {key.name}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="flex items-center justify-between text-[0.65rem] text-text-muted">
        <span>{keys.length.toLocaleString()} loaded</span>
        {cursor ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem]"
            disabled={loading}
            onClick={() => {
              void runScan(cursor);
            }}
          >
            {loading ? "Loading…" : "Load more"}
          </Button>
        ) : keys.length > 0 ? (
          <span>scan complete</span>
        ) : null}
      </div>
    </div>
  );
}

function KeyTypeIcon({ type }: { type: string }) {
  const className = "size-3 shrink-0";
  switch (type) {
    case "hash":
      return <IconHash className={cn(className, "text-blue-400")} />;
    case "list":
      return <IconList className={cn(className, "text-amber-400")} />;
    case "set":
      return <IconLayoutList className={cn(className, "text-emerald-400")} />;
    case "zset":
      return <IconDatabaseStar className={cn(className, "text-pink-400")} />;
    case "stream":
      return <IconWaveSine className={cn(className, "text-purple-400")} />;
    case "ReJSON-RL":
      return <IconBraces className={cn(className, "text-indigo-400")} />;
    default:
      return <span className="size-2 shrink-0 rounded-full bg-text-muted/50" />;
  }
}
