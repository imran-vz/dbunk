/**
 * Redis server tab. Six v1 cards from Q12 of the grilling session
 * (Identity, Keyspace, Memory, Clients, Replication, Modules) plus
 * Slow log + persistence when present. Tier 2 adds four admin cards
 * (CLIENT LIST, ACL LIST, CONFIG GET, LATENCY LATEST) and a global
 * auto-refresh interval picker; each card owns its own fetch and
 * reloads on the configured tick.
 */

import { IconRefresh } from "@tabler/icons-react";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  type AclListEntry,
  type ClientListEntry,
  fetchKeyValueOverview,
  fetchRedisAclList,
  fetchRedisClientList,
  fetchRedisConfig,
  fetchRedisLatency,
  type KeyValueOverviewStats,
  type LatencyEntry,
  type RedisConfigEntry,
  setRedisConfig,
} from "@/lib/redis/api";

interface ServerTabProps {
  connectionId: string;
  dbNumber: number;
}

const REFRESH_INTERVALS: ReadonlyArray<{ label: string; ms: number | null }> = [
  { label: "Manual", ms: null },
  { label: "5s", ms: 5_000 },
  { label: "15s", ms: 15_000 },
  { label: "30s", ms: 30_000 },
  { label: "1m", ms: 60_000 },
];

export function ServerTab({ connectionId, dbNumber }: ServerTabProps) {
  const [stats, setStats] = useState<KeyValueOverviewStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [intervalMs, setIntervalMs] = useState<number | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await fetchKeyValueOverview({ connectionId });
      setStats(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, [connectionId]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (intervalMs === null) return;
    const timer = window.setInterval(() => {
      setRefreshTick((tick) => tick + 1);
      void load();
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs, load]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between border-b border-border-subtle bg-surface-panel/60 px-4 py-2 text-xs">
        <span className="text-text-muted">
          Server overview{" "}
          {stats?.identity.version ? `· v${stats.identity.version}` : ""}
        </span>
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1 text-[0.65rem] text-text-muted">
            Auto-refresh
            <select
              value={intervalMs ?? ""}
              onChange={(event) =>
                setIntervalMs(
                  event.target.value === "" ? null : Number(event.target.value),
                )
              }
              className="rounded border border-border-subtle bg-surface-panel px-1 py-0.5 text-xs"
            >
              {REFRESH_INTERVALS.map((option) => (
                <option key={option.label} value={option.ms ?? ""}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
          <Button
            size="sm"
            variant="ghost"
            className="h-7 px-2 text-[0.65rem]"
            onClick={() => {
              setRefreshTick((tick) => tick + 1);
              void load();
            }}
            disabled={loading}
          >
            <IconRefresh className="size-3" />
            {loading ? "Refreshing…" : "Refresh"}
          </Button>
        </div>
      </header>
      {error ? (
        <div className="m-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
        </div>
      ) : null}
      {stats ? (
        <div className="grid flex-1 gap-3 overflow-auto p-4 lg:grid-cols-2 xl:grid-cols-3">
          <IdentityCard stats={stats} />
          <KeyspaceCard stats={stats} dbNumber={dbNumber} />
          <MemoryCard stats={stats} />
          <ClientsCard stats={stats} />
          <ReplicationCard stats={stats} />
          <ModulesCard stats={stats} />
          {stats.slowLog ? <SlowLogCard stats={stats} /> : null}
          {stats.persistence ? <PersistenceCard stats={stats} /> : null}
          <ClientListCard
            connectionId={connectionId}
            refreshTick={refreshTick}
          />
          <AclListCard connectionId={connectionId} refreshTick={refreshTick} />
          <ConfigCard connectionId={connectionId} refreshTick={refreshTick} />
          <LatencyCard connectionId={connectionId} refreshTick={refreshTick} />
        </div>
      ) : null}
    </div>
  );
}

function useFetched<T>(
  fetcher: () => Promise<T>,
  cacheKey: string,
): { data: T | null; error: string | null; loading: boolean } {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /* oxlint-disable react-hooks/exhaustive-deps -- `fetcher` is recreated every render; cacheKey is the real input */
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetcher()
      .then((result) => {
        if (!cancelled) setData(result);
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
  }, [cacheKey]);
  /* oxlint-enable react-hooks/exhaustive-deps */
  return { data, error, loading };
}

function ClientListCard({
  connectionId,
  refreshTick,
}: {
  connectionId: string;
  refreshTick: number;
}) {
  const { data, error, loading } = useFetched(
    () => fetchRedisClientList({ connectionId }),
    `clients|${connectionId}|${refreshTick}`,
  );
  return (
    <Card className="xl:col-span-2">
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">
          Clients{" "}
          {data ? (
            <span className="text-text-muted">({data.entries.length})</span>
          ) : null}
        </CardTitle>
      </CardHeader>
      <CardContent className="text-xs">
        {error ? (
          <CardError message={error} />
        ) : loading && !data ? (
          <CardSkeleton />
        ) : data && data.entries.length === 0 ? (
          <CardEmpty>No connected clients.</CardEmpty>
        ) : (
          <div className="max-h-48 overflow-auto">
            <table className="min-w-full divide-y divide-border-subtle font-mono text-[0.65rem]">
              <thead className="text-[0.6rem] uppercase text-text-muted">
                <tr>
                  <th className="px-2 py-1 text-left">id</th>
                  <th className="px-2 py-1 text-left">addr</th>
                  <th className="px-2 py-1 text-left">name</th>
                  <th className="px-2 py-1 text-right">idle</th>
                  <th className="px-2 py-1 text-left">cmd</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border-subtle">
                {data?.entries.map((entry: ClientListEntry) => (
                  <tr key={entry.id}>
                    <td className="px-2 py-1 text-text-muted">{entry.id}</td>
                    <td className="px-2 py-1">{entry.addr}</td>
                    <td className="px-2 py-1 text-text-secondary">
                      {entry.name || "—"}
                    </td>
                    <td className="px-2 py-1 text-right">
                      {entry.idleSeconds}s
                    </td>
                    <td className="px-2 py-1">{entry.command || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function AclListCard({
  connectionId,
  refreshTick,
}: {
  connectionId: string;
  refreshTick: number;
}) {
  const { data, error, loading } = useFetched(
    () => fetchRedisAclList({ connectionId }),
    `acl|${connectionId}|${refreshTick}`,
  );
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">
          ACL users{" "}
          {data ? (
            <span className="text-text-muted">({data.entries.length})</span>
          ) : null}
        </CardTitle>
      </CardHeader>
      <CardContent className="text-xs">
        {error ? (
          <CardError message={error} />
        ) : loading && !data ? (
          <CardSkeleton />
        ) : (
          <ul className="max-h-48 space-y-1 overflow-auto">
            {data?.entries.map((entry: AclListEntry) => (
              <li
                key={entry.username || entry.rules}
                className="rounded bg-surface-panel/40 px-2 py-1 font-mono text-[0.65rem]"
              >
                <div className="text-foreground">{entry.username || "—"}</div>
                <div className="break-all text-text-muted">{entry.rules}</div>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}

function ConfigCard({
  connectionId,
  refreshTick,
}: {
  connectionId: string;
  refreshTick: number;
}) {
  const [pattern, setPattern] = useState("*max*");
  const [submitted, setSubmitted] = useState(pattern);
  const [localTick, setLocalTick] = useState(0);
  const { data, error, loading } = useFetched(
    () => fetchRedisConfig({ connectionId, pattern: submitted }),
    `config|${connectionId}|${submitted}|${refreshTick}|${localTick}`,
  );
  const [editKey, setEditKey] = useState<string | null>(null);
  const [editValue, setEditValue] = useState("");
  const [saving, setSaving] = useState(false);
  const startEdit = (entry: RedisConfigEntry) => {
    setEditKey(entry.key);
    setEditValue(entry.value);
  };
  const cancelEdit = () => {
    setEditKey(null);
    setEditValue("");
  };
  const saveEdit = async () => {
    if (!editKey) return;
    const confirmed = window.confirm(
      `Run CONFIG SET ${editKey} ${editValue}? This changes server-wide configuration.`,
    );
    if (!confirmed) return;
    setSaving(true);
    try {
      await setRedisConfig({ connectionId, key: editKey, value: editValue });
      toast.success(`CONFIG SET ${editKey}`);
      cancelEdit();
      setLocalTick((tick) => tick + 1);
    } catch (err) {
      toast.error(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">
          Config{" "}
          {data ? (
            <span className="text-text-muted">({data.entries.length})</span>
          ) : null}
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-2 text-xs">
        <form
          className="flex items-center gap-2"
          onSubmit={(event) => {
            event.preventDefault();
            setSubmitted(pattern || "*");
          }}
        >
          <input
            value={pattern}
            onChange={(event) => setPattern(event.target.value)}
            placeholder="pattern (e.g., *max*)"
            className="h-6 flex-1 rounded border border-border-subtle bg-surface-panel px-2 font-mono text-[0.65rem]"
          />
          <Button
            size="sm"
            variant="outline"
            type="submit"
            className="h-6 px-2 text-[0.65rem]"
          >
            Query
          </Button>
        </form>
        {error ? (
          <CardError message={error} />
        ) : loading && !data ? (
          <CardSkeleton />
        ) : data && data.entries.length === 0 ? (
          <CardEmpty>No CONFIG keys match.</CardEmpty>
        ) : (
          <table className="max-h-44 min-w-full divide-y divide-border-subtle overflow-auto font-mono text-[0.65rem]">
            <tbody className="divide-y divide-border-subtle">
              {data?.entries.map((entry: RedisConfigEntry) => (
                <tr key={entry.key}>
                  <td className="px-2 py-1 text-text-muted">{entry.key}</td>
                  <td className="px-2 py-1 break-all">
                    {editKey === entry.key ? (
                      <input
                        value={editValue}
                        onChange={(event) => setEditValue(event.target.value)}
                        className="h-6 w-full rounded border border-border-subtle bg-surface-panel px-1 font-mono text-[0.65rem]"
                        aria-label={`Value for ${entry.key}`}
                      />
                    ) : (
                      entry.value || "—"
                    )}
                  </td>
                  <td className="px-2 py-1 text-right">
                    {editKey === entry.key ? (
                      <span className="flex items-center justify-end gap-1">
                        <button
                          type="button"
                          disabled={saving}
                          onClick={() => {
                            void saveEdit();
                          }}
                          className="text-accent hover:underline disabled:opacity-50"
                        >
                          save
                        </button>
                        <button
                          type="button"
                          onClick={cancelEdit}
                          className="text-text-muted hover:underline"
                        >
                          cancel
                        </button>
                      </span>
                    ) : (
                      <button
                        type="button"
                        onClick={() => startEdit(entry)}
                        className="text-text-muted hover:text-foreground hover:underline"
                      >
                        edit
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  );
}

function LatencyCard({
  connectionId,
  refreshTick,
}: {
  connectionId: string;
  refreshTick: number;
}) {
  const { data, error, loading } = useFetched(
    () => fetchRedisLatency({ connectionId }),
    `latency|${connectionId}|${refreshTick}`,
  );
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Latency events</CardTitle>
      </CardHeader>
      <CardContent className="text-xs">
        {error ? (
          <CardError message={error} />
        ) : loading && !data ? (
          <CardSkeleton />
        ) : data && data.entries.length === 0 ? (
          <CardEmpty>No latency events recorded.</CardEmpty>
        ) : (
          <table className="min-w-full divide-y divide-border-subtle font-mono text-[0.65rem]">
            <thead className="text-[0.6rem] uppercase text-text-muted">
              <tr>
                <th className="px-2 py-1 text-left">event</th>
                <th className="px-2 py-1 text-right">latest</th>
                <th className="px-2 py-1 text-right">max</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border-subtle">
              {data?.entries.map((entry: LatencyEntry) => (
                <tr key={entry.event}>
                  <td className="px-2 py-1 text-text-secondary">
                    {entry.event}
                  </td>
                  <td className="px-2 py-1 text-right">
                    {entry.latestLatencyMs} ms
                  </td>
                  <td className="px-2 py-1 text-right text-text-muted">
                    {entry.maxLatencyMs} ms
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  );
}

function CardError({ message }: { message: string }) {
  return (
    <div className="rounded-md border border-destructive/30 bg-destructive/10 px-2 py-1 text-[0.65rem] text-destructive">
      {message}
    </div>
  );
}

function CardSkeleton() {
  return <div className="h-12 animate-pulse rounded-md bg-surface-panel/40" />;
}

function CardEmpty({ children }: { children: React.ReactNode }) {
  return <div className="text-[0.65rem] text-text-muted">{children}</div>;
}

function IdentityCard({ stats }: { stats: KeyValueOverviewStats }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Identity</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        <Row label="Version" value={stats.identity.version ?? "—"} />
        <Row label="Mode" value={stats.identity.mode ?? "standalone"} />
        <Row
          label="Uptime"
          value={
            stats.identity.uptimeSeconds !== undefined
              ? formatDuration(stats.identity.uptimeSeconds)
              : "—"
          }
        />
        <Row label="OS" value={stats.identity.os ?? "—"} />
      </CardContent>
    </Card>
  );
}

function KeyspaceCard({
  stats,
  dbNumber,
}: {
  stats: KeyValueOverviewStats;
  dbNumber: number;
}) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Keyspace</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        {stats.keyspace.length === 0 ? (
          <p className="text-text-muted">Empty across all DBs.</p>
        ) : (
          stats.keyspace.map((db) => (
            <Row
              key={db.dbNumber}
              label={`DB ${db.dbNumber}${db.dbNumber === dbNumber ? " (active)" : ""}`}
              value={`${db.keys.toLocaleString()} keys · ${db.expires.toLocaleString()} with TTL`}
            />
          ))
        )}
      </CardContent>
    </Card>
  );
}

function MemoryCard({ stats }: { stats: KeyValueOverviewStats }) {
  if (!stats.memory) return <UnavailableCard title="Memory" />;
  const m = stats.memory;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Memory</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        <Row label="Used" value={formatBytes(m.usedMemory)} />
        <Row label="Peak" value={formatBytes(m.usedMemoryPeak)} />
        <Row label="RSS" value={formatBytes(m.usedMemoryRss)} />
        <Row
          label="Fragmentation"
          value={
            m.fragmentationRatio !== undefined
              ? m.fragmentationRatio.toFixed(2)
              : "—"
          }
        />
        <Row label="Max" value={formatBytes(m.maxmemory)} />
        <Row label="Policy" value={m.maxmemoryPolicy ?? "—"} />
      </CardContent>
    </Card>
  );
}

function ClientsCard({ stats }: { stats: KeyValueOverviewStats }) {
  if (!stats.clients) return <UnavailableCard title="Clients" />;
  const c = stats.clients;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Clients</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        <Row label="Connected" value={c.connectedClients.toLocaleString()} />
        <Row label="Max" value={c.maxclients?.toLocaleString() ?? "—"} />
        <Row label="Blocked" value={c.blockedClients.toLocaleString()} />
      </CardContent>
    </Card>
  );
}

function ReplicationCard({ stats }: { stats: KeyValueOverviewStats }) {
  if (!stats.replication) return <UnavailableCard title="Replication" />;
  const r = stats.replication;
  const isReplica = r.role === "replica" || r.role === "slave";
  return (
    <Card>
      <CardHeader className="pb-2 flex items-center justify-between">
        <CardTitle className="text-xs">Replication</CardTitle>
        <Badge variant={isReplica ? "destructive" : "secondary"}>
          {r.role}
        </Badge>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        {isReplica ? (
          <>
            <Row
              label="Master"
              value={`${r.masterHost ?? "?"}:${r.masterPort ?? "?"}`}
            />
            <Row label="Link status" value={r.masterLinkStatus ?? "—"} />
          </>
        ) : (
          <Row
            label="Replicas attached"
            value={(r.connectedSlaves ?? 0).toLocaleString()}
          />
        )}
      </CardContent>
    </Card>
  );
}

function ModulesCard({ stats }: { stats: KeyValueOverviewStats }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Modules</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        {!stats.modules || stats.modules.length === 0 ? (
          <p className="text-text-muted">No modules loaded.</p>
        ) : (
          stats.modules.map((m) => (
            <Row key={m.name} label={m.name} value={`v${m.version}`} />
          ))
        )}
      </CardContent>
    </Card>
  );
}

function SlowLogCard({ stats }: { stats: KeyValueOverviewStats }) {
  const entries = stats.slowLog ?? [];
  return (
    <Card className="lg:col-span-2 xl:col-span-3">
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Slow log (last 25)</CardTitle>
      </CardHeader>
      <CardContent>
        {entries.length === 0 ? (
          <p className="text-xs text-text-muted">No entries.</p>
        ) : (
          <table className="w-full font-mono text-xs">
            <thead className="text-[0.65rem] uppercase text-text-muted">
              <tr>
                <th className="py-1 text-left">ID</th>
                <th className="py-1 text-right">Duration (μs)</th>
                <th className="py-1 text-left">Command</th>
              </tr>
            </thead>
            <tbody>
              {entries.slice(0, 25).map((entry) => (
                <tr
                  key={entry.id}
                  className="border-t border-border-subtle hover:bg-white/5"
                >
                  <td className="py-1 text-text-muted">{entry.id}</td>
                  <td className="py-1 text-right">
                    {entry.durationUs.toLocaleString()}
                  </td>
                  <td className="break-all py-1">{entry.command}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </CardContent>
    </Card>
  );
}

function PersistenceCard({ stats }: { stats: KeyValueOverviewStats }) {
  const p = stats.persistence;
  if (!p) return null;
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">Persistence</CardTitle>
      </CardHeader>
      <CardContent className="space-y-1 text-xs">
        <Row
          label="RDB last save"
          value={
            p.rdbLastSaveTime
              ? new Date(p.rdbLastSaveTime * 1000).toLocaleString()
              : "—"
          }
        />
        <Row
          label="Changes since save"
          value={p.rdbChangesSinceLastSave?.toLocaleString() ?? "—"}
        />
        <Row label="AOF enabled" value={p.aofEnabled ? "yes" : "no"} />
      </CardContent>
    </Card>
  );
}

function UnavailableCard({ title }: { title: string }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-xs">{title}</CardTitle>
      </CardHeader>
      <CardContent className="text-xs text-text-muted">
        Unavailable — your Redis user may lack permission for this section.
      </CardContent>
    </Card>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-2">
      <span className="text-text-muted">{label}</span>
      <span className="font-mono">{value}</span>
    </div>
  );
}

function formatBytes(value?: number): string {
  if (value === undefined || value === null) return "—";
  if (value === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log10(value) / 3));
  return `${(value / 10 ** (i * 3)).toFixed(2)} ${units[i]}`;
}

function formatDuration(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h`;
  return `${Math.floor(seconds / 86400)}d`;
}
