/**
 * Redis server tab — six v1 cards from Q12 of the grilling session:
 * Identity, Keyspace, Memory, Clients, Replication, Modules. Slow
 * log + persistence are stretch (rendered when present).
 */

import { IconRefresh } from "@tabler/icons-react";
import { useCallback, useEffect, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  fetchKeyValueOverview,
  type KeyValueOverviewStats,
} from "@/lib/redis/api";

interface ServerTabProps {
  connectionId: string;
  dbNumber: number;
}

export function ServerTab({ connectionId, dbNumber }: ServerTabProps) {
  const [stats, setStats] = useState<KeyValueOverviewStats | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between border-b border-border-subtle bg-surface-panel/60 px-4 py-2 text-xs">
        <span className="text-text-muted">
          Server overview{" "}
          {stats?.identity.version ? `· v${stats.identity.version}` : ""}
        </span>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-[0.65rem]"
          onClick={() => {
            void load();
          }}
          disabled={loading}
        >
          <IconRefresh className="size-3" />
          {loading ? "Refreshing…" : "Refresh"}
        </Button>
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
        </div>
      ) : null}
    </div>
  );
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
