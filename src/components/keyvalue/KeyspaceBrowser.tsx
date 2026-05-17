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
  IconKey,
  IconLayoutList,
  IconList,
  IconSearch,
  IconStar,
  IconStarFilled,
  IconWaveSine,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  type BulkDeleteByPatternResult,
  type BulkExpireByPatternResult,
  type BulkRenameByPrefixResult,
  bulkDeleteByPattern,
  bulkExpireByPattern,
  bulkRenameByPrefix,
  cancelScanSession,
  closeScanSession,
  fetchString,
  formatValueOneLine,
  openScanSession,
  type ScannedKey,
  scanKeys,
} from "@/lib/redis/api";
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

const EMPTY_KEYS: string[] = [];

/**
 * Side-by-side string-value comparison. v1: only works for string-
 * typed keys (the most common ask). Hash / list / set / zset / stream
 * comparison is a follow-up that needs per-type renderers.
 */
function CompareKeysButton({ connectionId }: { connectionId: string }) {
  const [open, setOpen] = useState(false);
  const [leftKey, setLeftKey] = useState("");
  const [rightKey, setRightKey] = useState("");
  const [leftValue, setLeftValue] = useState<string | null>(null);
  const [rightValue, setRightValue] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setLeftKey("");
    setRightKey("");
    setLeftValue(null);
    setRightValue(null);
    setError(null);
  };

  const handleCompare = async () => {
    if (!leftKey.trim() || !rightKey.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const [left, right] = await Promise.all([
        fetchString({ connectionId, key: leftKey.trim() }),
        fetchString({ connectionId, key: rightKey.trim() }),
      ]);
      setLeftValue(formatValueOneLine(left.value));
      setRightValue(formatValueOneLine(right.value));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <Button
        size="sm"
        variant="ghost"
        className="h-6 px-2 text-[0.65rem]"
        onClick={() => setOpen(true)}
      >
        Compare…
      </Button>
    );
  }

  const equal =
    leftValue !== null && rightValue !== null && leftValue === rightValue;

  return (
    <div className="w-full rounded-md border border-border-subtle bg-surface-panel-elevated/40 p-2">
      <div className="flex flex-wrap items-center gap-1.5">
        <Input
          value={leftKey}
          onChange={(event) => setLeftKey(event.target.value)}
          placeholder="key A"
          className="h-6 flex-1 font-mono text-[0.65rem]"
          aria-label="Left key"
        />
        <Input
          value={rightKey}
          onChange={(event) => setRightKey(event.target.value)}
          placeholder="key B"
          className="h-6 flex-1 font-mono text-[0.65rem]"
          aria-label="Right key"
        />
        <Button
          size="sm"
          variant="outline"
          className="h-6 px-2 text-[0.65rem]"
          disabled={busy || !leftKey.trim() || !rightKey.trim()}
          onClick={() => {
            void handleCompare();
          }}
        >
          {busy ? "Loading…" : "Compare"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          onClick={() => {
            setOpen(false);
            reset();
          }}
        >
          Close
        </Button>
      </div>
      {error ? (
        <div className="mt-2 rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-[0.65rem] text-destructive">
          {error}
        </div>
      ) : null}
      {leftValue !== null && rightValue !== null ? (
        <div className="mt-2 space-y-1">
          <div
            className={cn(
              "text-[0.65rem] font-semibold",
              equal ? "text-accent-green" : "text-warning",
            )}
          >
            {equal ? "Values are identical." : "Values differ."}
          </div>
          <div className="grid grid-cols-2 gap-2">
            <pre className="max-h-32 overflow-auto rounded-md border border-border-subtle bg-surface-panel p-1 font-mono text-[0.6rem]">
              {leftValue}
            </pre>
            <pre className="max-h-32 overflow-auto rounded-md border border-border-subtle bg-surface-panel p-1 font-mono text-[0.6rem]">
              {rightValue}
            </pre>
          </div>
        </div>
      ) : null}
    </div>
  );
}

type BulkOp = "delete" | "expire" | "rename";
type BulkPreview =
  | { kind: "delete"; result: BulkDeleteByPatternResult }
  | { kind: "expire"; result: BulkExpireByPatternResult }
  | { kind: "rename"; result: BulkRenameByPrefixResult };

/**
 * Pattern-based bulk DELETE / EXPIRE with a forced dry-run preview
 * step. Lives at the bottom of the keyspace browser so the scan +
 * mutation operate on the same connection without leaking dialog
 * state through the rest of the tree.
 */
function BulkDeleteButton({ connectionId }: { connectionId: string }) {
  const [open, setOpen] = useState(false);
  const [op, setOp] = useState<BulkOp>("delete");
  const [pattern, setPattern] = useState("");
  const [newPrefix, setNewPrefix] = useState("");
  const [ttlSeconds, setTtlSeconds] = useState("3600");
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [confirmText, setConfirmText] = useState("");
  const [busy, setBusy] = useState(false);

  const reset = () => {
    setPattern("");
    setNewPrefix("");
    setPreview(null);
    setConfirmText("");
    setBusy(false);
  };

  const ttlNumber = Number(ttlSeconds);
  const ttlValid =
    op !== "expire" ||
    (Number.isFinite(ttlNumber) &&
      ttlNumber > 0 &&
      Number.isInteger(ttlNumber));
  const renameValid =
    op !== "rename" || (newPrefix.length > 0 && newPrefix !== pattern);

  const handlePreview = async () => {
    if (!pattern.trim() || !ttlValid || !renameValid) return;
    setBusy(true);
    try {
      if (op === "delete") {
        const result = await bulkDeleteByPattern({
          connectionId,
          pattern: pattern.trim(),
          dryRun: true,
        });
        setPreview({ kind: "delete", result });
      } else if (op === "expire") {
        const result = await bulkExpireByPattern({
          connectionId,
          pattern: pattern.trim(),
          ttlSeconds: ttlNumber,
          dryRun: true,
        });
        setPreview({ kind: "expire", result });
      } else {
        const result = await bulkRenameByPrefix({
          connectionId,
          oldPrefix: pattern,
          newPrefix,
          dryRun: true,
        });
        setPreview({ kind: "rename", result });
      }
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const handleApply = async () => {
    if (!preview) return;
    setBusy(true);
    try {
      if (preview.kind === "delete") {
        const result = await bulkDeleteByPattern({
          connectionId,
          pattern: pattern.trim(),
          dryRun: false,
        });
        toast.success(
          `Deleted ${result.deleted.toLocaleString()} key${result.deleted === 1 ? "" : "s"}`,
        );
      } else if (preview.kind === "expire") {
        const result = await bulkExpireByPattern({
          connectionId,
          pattern: pattern.trim(),
          ttlSeconds: ttlNumber,
          dryRun: false,
        });
        toast.success(
          `EXPIRE applied to ${result.expired.toLocaleString()} key${result.expired === 1 ? "" : "s"}`,
        );
      } else {
        const result = await bulkRenameByPrefix({
          connectionId,
          oldPrefix: pattern,
          newPrefix,
          dryRun: false,
        });
        toast.success(
          `Renamed ${result.renamed.toLocaleString()} key${result.renamed === 1 ? "" : "s"} (${result.skipped.toLocaleString()} skipped — destination existed)`,
        );
      }
      setOpen(false);
      reset();
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  if (!open) {
    return (
      <Button
        size="sm"
        variant="ghost"
        className="h-6 px-2 text-[0.65rem]"
        onClick={() => setOpen(true)}
      >
        Bulk ops…
      </Button>
    );
  }

  const matched = preview?.result.matched ?? 0;
  const scanned = preview?.result.scanned ?? 0;
  const truncated = preview?.result.truncated ?? false;
  const confirmWord =
    op === "delete" ? "DELETE" : op === "expire" ? "EXPIRE" : "RENAME";
  const applyLabel =
    op === "delete"
      ? `Delete ${matched}`
      : op === "expire"
        ? `EXPIRE ${matched}`
        : `Rename ${matched}`;

  return (
    <div className="w-full rounded-md border border-border-subtle bg-surface-panel-elevated/40 p-2">
      <div className="flex flex-wrap items-center gap-1.5">
        <select
          value={op}
          onChange={(event) => {
            setOp(event.target.value as BulkOp);
            setPreview(null);
            setConfirmText("");
          }}
          className="h-6 rounded border border-border-subtle bg-surface-panel px-1 text-[0.65rem]"
          aria-label="Bulk operation"
        >
          <option value="delete">DEL</option>
          <option value="expire">EXPIRE</option>
          <option value="rename">RENAME prefix</option>
        </select>
        <Input
          value={pattern}
          onChange={(event) => setPattern(event.target.value)}
          placeholder={
            op === "rename" ? "old prefix" : "pattern (e.g. session:*)"
          }
          className="h-6 flex-1 font-mono text-[0.65rem]"
          aria-label="Bulk pattern"
        />
        {op === "expire" ? (
          <Input
            type="number"
            value={ttlSeconds}
            onChange={(event) => setTtlSeconds(event.target.value)}
            placeholder="ttl seconds"
            className="h-6 w-24 font-mono text-[0.65rem]"
            aria-label="TTL seconds"
          />
        ) : null}
        {op === "rename" ? (
          <Input
            value={newPrefix}
            onChange={(event) => setNewPrefix(event.target.value)}
            placeholder="new prefix"
            className="h-6 w-32 font-mono text-[0.65rem]"
            aria-label="New prefix"
          />
        ) : null}
        <Button
          size="sm"
          variant="outline"
          className="h-6 px-2 text-[0.65rem]"
          disabled={busy || !pattern.trim() || !ttlValid || !renameValid}
          onClick={() => {
            void handlePreview();
          }}
        >
          {busy && !preview ? "Scanning…" : "Preview"}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          onClick={() => {
            setOpen(false);
            reset();
          }}
        >
          Close
        </Button>
      </div>
      {preview ? (
        <div className="mt-2 text-[0.6rem]">
          <div className="text-text-secondary">
            Scanned {scanned.toLocaleString()} · matched{" "}
            <span className="font-semibold text-warning">
              {matched.toLocaleString()}
            </span>
            {truncated ? " (truncated at 10k)" : ""}
          </div>
          {preview?.kind === "rename" ? (
            preview.result.sample.length > 0 ? (
              <ul className="mt-1 max-h-32 overflow-auto font-mono text-text-muted">
                {preview.result.sample.map(([oldName, newName]) => (
                  <li key={oldName} className="truncate">
                    {oldName} → {newName}
                  </li>
                ))}
              </ul>
            ) : null
          ) : (preview?.result.sample.length ?? 0) > 0 ? (
            <ul className="mt-1 max-h-32 overflow-auto font-mono text-text-muted">
              {(preview?.result.sample as string[] | undefined)?.map((name) => (
                <li key={name} className="truncate">
                  {name}
                </li>
              ))}
            </ul>
          ) : null}
          {matched > 0 ? (
            <div className="mt-2 flex items-center gap-1.5">
              <Input
                value={confirmText}
                onChange={(event) => setConfirmText(event.target.value)}
                placeholder={`Type "${confirmWord}" to confirm`}
                className="h-6 flex-1 font-mono text-[0.65rem]"
                aria-label="Confirm bulk operation"
              />
              <Button
                size="sm"
                variant="destructive"
                className="h-6 px-2 text-[0.65rem]"
                disabled={busy || confirmText !== confirmWord}
                onClick={() => {
                  void handleApply();
                }}
              >
                {busy ? "Applying…" : applyLabel}
              </Button>
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

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
  // Per-session DB picker. Defaults to the connection record's
  // `dbNumber`; switching reopens the scan session against the new
  // DB so the keyspace browser can peek at other DBs without
  // mutating the shared manager that the rest of the workspace
  // (key inspector, CLI) routes through. Opening a key on a
  // non-default DB will read from the manager's DB instead — that
  // limitation is called out in the picker's title.
  const browseDb = connection.engine === "Redis" ? connection.dbNumber : 0;
  const [activeDb, setActiveDb] = useState<number>(browseDb);
  // Scan session: opened once per mount + connection. SCANs route
  // through its dedicated connection so a Cancel click can issue
  // CLIENT KILL ID against the in-flight server-side work.
  const scanSessionIdRef = useRef(
    `scan-${connection.id}-${globalThis.crypto?.randomUUID?.() ?? Date.now()}`,
  );
  const scanSessionReadyRef = useRef(false);
  // Replica-warning dismiss is session-local — a future migration can
  // persist `dismissed_replica_warning_at` on the connection record.
  const [replicaWarningDismissed, setReplicaWarningDismissed] = useState(false);
  const capabilities = useAppStore(
    (state) => state.redisCapabilitiesByConnection[connection.id],
  );
  const aclSelf = useAppStore(
    (state) => state.redisAclSelfByConnection[connection.id],
  );
  const watchedKeys = useAppStore(
    (state) => state.watchedKeysByConnection[connection.id] ?? EMPTY_KEYS,
  );
  const pinKey = useAppStore((state) => state.pinKey);
  const unpinKey = useAppStore((state) => state.unpinKey);
  const watchedSet = useMemo(() => new Set(watchedKeys), [watchedKeys]);
  const showReplicaWarning =
    !replicaWarningDismissed &&
    capabilities?.role === "master" &&
    (capabilities.connectedSlaves ?? 0) > 0;
  const showAclHint =
    aclSelf !== undefined && !aclSelf.allKeys && aclSelf.keyPatterns.length > 0;

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
          sessionId: scanSessionReadyRef.current
            ? scanSessionIdRef.current
            : undefined,
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

  // Open a dedicated scan session on mount (and whenever the user
  // picks a different DB) so SCAN calls run on a killable
  // connection scoped to the chosen DB. Falls back to the shared
  // manager on open failure (e.g., the server rejects the second
  // connection) — canceling won't kill and DB selection won't
  // apply in that case but the rest of the flow keeps working.
  useEffect(() => {
    const sessionId = scanSessionIdRef.current;
    let cancelled = false;
    scanSessionReadyRef.current = false;
    openScanSession({
      connectionId: connection.id,
      sessionId,
      dbNumber: activeDb,
    })
      .then(() => {
        if (!cancelled) scanSessionReadyRef.current = true;
      })
      .catch((err) => {
        console.warn("openScanSession failed", err);
      });
    return () => {
      cancelled = true;
      scanSessionReadyRef.current = false;
      void closeScanSession({ sessionId }).catch(() => {});
    };
  }, [connection.id, activeDb]);

  // Restart scan when pattern, filter, or DB changes. `activeDb`
  // doesn't change `runScan`'s identity (it's read off the scan
  // session, not the closure) but we still want to re-fire the
  // SCAN when the user picks a new DB.
  // biome-ignore lint/correctness/useExhaustiveDependencies: activeDb intentionally triggers re-scan
  useEffect(() => {
    setKeys([]);
    setCursor(null);
    void runScan(null);
  }, [runScan, activeDb]);

  const handleCancelScan = async () => {
    requestSeq.current++; // Invalidate the in-flight response.
    setLoading(false);
    if (!scanSessionReadyRef.current) return;
    try {
      await cancelScanSession({
        connectionId: connection.id,
        sessionId: scanSessionIdRef.current,
      });
    } catch (err) {
      console.warn("cancelScanSession failed", err);
    } finally {
      // The cancelled session is dropped server-side. Open a fresh
      // one so subsequent scans are also cancellable.
      scanSessionReadyRef.current = false;
      openScanSession({
        connectionId: connection.id,
        sessionId: scanSessionIdRef.current,
      })
        .then(() => {
          scanSessionReadyRef.current = true;
        })
        .catch(() => {
          // Non-fatal; fall back to shared manager.
        });
    }
  };

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
      {showAclHint ? (
        <output className="flex items-start gap-2 rounded-md border border-border-subtle bg-surface-panel-elevated/60 px-2 py-1.5 text-[0.65rem] text-text-secondary">
          <IconKey className="mt-0.5 size-3.5 shrink-0 text-text-muted" />
          <div className="flex-1">
            Connected as{" "}
            <span className="font-mono text-foreground">
              {aclSelf.username}
            </span>
            . Visible keys are restricted to:
            <div className="mt-1 flex flex-wrap gap-1 font-mono">
              {aclSelf.keyPatterns.map((pattern) => (
                <span
                  key={pattern}
                  className="rounded bg-surface-panel px-1.5 py-0.5 text-[0.6rem] text-foreground"
                >
                  {pattern}
                </span>
              ))}
            </div>
          </div>
        </output>
      ) : null}
      <div className="flex items-center gap-1.5">
        <select
          value={activeDb}
          onChange={(event) => setActiveDb(Number(event.target.value))}
          className="h-8 rounded border border-border-subtle bg-surface-panel px-1 text-[0.65rem]"
          aria-label="Browse DB"
          title="Switch the keyspace browser to a different DB. Key tabs continue to use the connection's default DB."
        >
          {Array.from({ length: 16 }, (_, n) => n).map((n) => (
            <option key={`db-${n}`} value={n}>
              db {n}
              {n === browseDb ? " (default)" : ""}
            </option>
          ))}
        </select>
        <div className="relative flex-1">
          <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
          <Input
            placeholder="Filter keys… (≥2 chars)"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            className="h-8 pl-7 text-xs"
          />
        </div>
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
        {watchedKeys.length > 0 ? (
          <div className="border-b border-border-subtle bg-surface-panel-elevated/40">
            <div className="px-2 py-1 text-[0.6rem] uppercase tracking-wide text-text-muted">
              Watched ({watchedKeys.length})
            </div>
            <ul className="divide-y divide-border-subtle">
              {watchedKeys.map((name) => {
                const matched = keys.find((k) => k.name === name);
                const type = matched?.type ?? "string";
                return (
                  <li key={`watched::${name}`} className="group flex">
                    <button
                      type="button"
                      onClick={() => onOpenKey(name, type)}
                      className={cn(
                        "flex flex-1 items-center gap-2 px-2 py-1 text-left hover:bg-white/5",
                        activeKey === name && "bg-primary/10 text-primary",
                      )}
                    >
                      <KeyTypeIcon type={type} />
                      <span className="truncate font-mono text-[0.65rem]">
                        {name}
                      </span>
                    </button>
                    <button
                      type="button"
                      onClick={() => unpinKey(connection.id, name)}
                      aria-label={`Unwatch ${name}`}
                      className="px-2 text-warning hover:text-warning/80"
                    >
                      <IconStarFilled className="size-3" />
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ) : null}
        {keys.length === 0 && !loading ? (
          <div className="px-2 py-3 text-[0.65rem] text-text-muted">
            No keys match. Adjust the filter or clear the search.
          </div>
        ) : (
          <ul className="divide-y divide-border-subtle">
            {keys.map((key) => {
              const isWatched = watchedSet.has(key.name);
              return (
                <li key={`${key.name}::${key.type}`} className="group flex">
                  <button
                    type="button"
                    onClick={() => onOpenKey(key.name, key.type)}
                    className={cn(
                      "flex flex-1 items-center gap-2 px-2 py-1 text-left hover:bg-white/5",
                      activeKey === key.name && "bg-primary/10 text-primary",
                    )}
                  >
                    <KeyTypeIcon type={key.type} />
                    <span className="truncate font-mono text-[0.65rem]">
                      {key.name}
                    </span>
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      isWatched
                        ? unpinKey(connection.id, key.name)
                        : pinKey(connection.id, key.name)
                    }
                    aria-label={
                      isWatched ? `Unwatch ${key.name}` : `Watch ${key.name}`
                    }
                    className={cn(
                      "px-2 opacity-0 transition-opacity group-hover:opacity-100",
                      isWatched
                        ? "text-warning opacity-100 hover:text-warning/80"
                        : "text-text-muted hover:text-foreground",
                    )}
                  >
                    {isWatched ? (
                      <IconStarFilled className="size-3" />
                    ) : (
                      <IconStar className="size-3" />
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2 text-[0.65rem] text-text-muted">
        <span>{keys.length.toLocaleString()} loaded</span>
        <CompareKeysButton connectionId={connection.id} />
        <BulkDeleteButton connectionId={connection.id} />
        {loading ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem] text-destructive hover:text-destructive"
            onClick={() => {
              void handleCancelScan();
            }}
          >
            Cancel scan
          </Button>
        ) : cursor ? (
          <Button
            size="sm"
            variant="ghost"
            className="h-6 px-2 text-[0.65rem]"
            onClick={() => {
              void runScan(cursor);
            }}
          >
            Load more
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
