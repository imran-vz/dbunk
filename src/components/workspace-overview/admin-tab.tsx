import { IconRefresh, IconShieldX, IconX } from "@tabler/icons-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import type { Connection } from "@/lib/store";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

type PgSessionInfo = {
  pid: number;
  user: string;
  database: string | null;
  applicationName: string;
  clientAddr: string | null;
  state: string | null;
  waitEventType: string | null;
  waitEvent: string | null;
  queryAgeSeconds: number | null;
  transactionAgeSeconds: number | null;
  query: string;
};

type PgLockInfo = {
  pid: number;
  lockType: string;
  relation: string | null;
  mode: string;
  granted: boolean;
  blockedBy: number[];
  query: string;
};

type PgPendingTransactionInfo = {
  pid: number;
  user: string;
  state: string | null;
  transactionAgeSeconds: number | null;
  query: string;
};

type PgAdminSnapshot = {
  sessions: PgSessionInfo[];
  locks: PgLockInfo[];
  pendingTransactions: PgPendingTransactionInfo[];
  stats: {
    databaseSizeBytes: number;
    cacheHitRatio: number | null;
    activeSessions: number;
    idleInTransaction: number;
    blockedLocks: number;
  };
};

export function AdminTab({ connection }: { connection: Connection }) {
  const [snapshot, setSnapshot] = useState<PgAdminSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const load = useCallback(async () => {
    if (!isTauri()) {
      setError("Admin tools require the desktop runtime.");
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setSnapshot(
        await tauriInvoke<PgAdminSnapshot>("load_pg_admin_snapshot", {
          payload: { connectionId: connection.id },
        }),
      );
    } catch (error) {
      setError(errorToMessage(error));
    } finally {
      setLoading(false);
    }
  }, [connection.id]);

  useEffect(() => {
    void load();
  }, [load]);

  const backendAction = async (command: string, pid: number) => {
    try {
      await tauriInvoke(command, {
        payload: { connectionId: connection.id, pid },
      });
      await load();
    } catch (error) {
      setError(errorToMessage(error));
    }
  };

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Admin dashboard</CardTitle>
          <Button size="sm" variant="ghost" onClick={() => void load()}>
            <IconRefresh
              className={loading ? "size-3.5 animate-spin" : "size-3.5"}
            />
            Refresh
          </Button>
        </CardHeader>
        <CardContent className="grid gap-3 text-xs md:grid-cols-5">
          <Metric
            label="Active sessions"
            value={snapshot?.stats.activeSessions ?? 0}
          />
          <Metric
            label="Idle in transaction"
            value={snapshot?.stats.idleInTransaction ?? 0}
          />
          <Metric
            label="Blocked locks"
            value={snapshot?.stats.blockedLocks ?? 0}
          />
          <Metric
            label="Cache hit"
            value={
              snapshot?.stats.cacheHitRatio == null
                ? "—"
                : `${Math.round(snapshot.stats.cacheHitRatio * 1000) / 10}%`
            }
          />
          <Metric
            label="Database size"
            value={formatBytes(snapshot?.stats.databaseSizeBytes ?? 0)}
          />
        </CardContent>
      </Card>

      {error ? (
        <div className="rounded-md border border-danger/30 bg-danger/5 px-3 py-2 text-xs text-danger">
          {error}
        </div>
      ) : null}

      <SessionCard
        sessions={snapshot?.sessions ?? []}
        onCancel={(pid) => backendAction("cancel_pg_backend", pid)}
        onTerminate={(pid) => backendAction("terminate_pg_backend", pid)}
      />
      <LocksCard locks={snapshot?.locks ?? []} />
      <PendingTransactionsCard rows={snapshot?.pendingTransactions ?? []} />
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="rounded-md border border-border-subtle bg-surface-panel px-3 py-2">
      <div className="text-[0.625rem] uppercase text-text-muted">{label}</div>
      <div className="mt-1 text-sm font-semibold">{value}</div>
    </div>
  );
}

function SessionCard({
  sessions,
  onCancel,
  onTerminate,
}: {
  sessions: PgSessionInfo[];
  onCancel: (pid: number) => void;
  onTerminate: (pid: number) => void;
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Sessions</CardTitle>
      </CardHeader>
      <CardContent>
        <AdminTable
          columns={["PID", "User", "State", "Wait", "Query", "Actions"]}
          rows={sessions.map((session) => ({
            key: String(session.pid),
            cells: [
              session.pid,
              session.user,
              session.state ?? "—",
              [session.waitEventType, session.waitEvent]
                .filter(Boolean)
                .join(" / ") || "—",
              session.query,
              <div key={session.pid} className="flex gap-1">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => onCancel(session.pid)}
                >
                  <IconX className="size-3.5" />
                  Cancel
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  className="border-danger/40 text-danger"
                  onClick={() => onTerminate(session.pid)}
                >
                  <IconShieldX className="size-3.5" />
                  Terminate
                </Button>
              </div>,
            ],
          }))}
        />
      </CardContent>
    </Card>
  );
}

function LocksCard({ locks }: { locks: PgLockInfo[] }) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Locks</CardTitle>
      </CardHeader>
      <CardContent>
        <AdminTable
          columns={["PID", "Type", "Relation", "Mode", "Granted", "Blocked by"]}
          rows={locks.map((lock, index) => ({
            key: `${lock.pid}-${lock.lockType}-${lock.mode}-${index}`,
            cells: [
              lock.pid,
              lock.lockType,
              lock.relation ?? "—",
              lock.mode,
              lock.granted ? "yes" : "no",
              lock.blockedBy.length > 0 ? lock.blockedBy.join(", ") : "—",
            ],
          }))}
        />
      </CardContent>
    </Card>
  );
}

function PendingTransactionsCard({
  rows,
}: {
  rows: PgPendingTransactionInfo[];
}) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Pending transactions</CardTitle>
      </CardHeader>
      <CardContent>
        <AdminTable
          columns={["PID", "User", "State", "Age", "Query"]}
          rows={rows.map((row) => ({
            key: String(row.pid),
            cells: [
              row.pid,
              row.user,
              row.state ?? "—",
              `${row.transactionAgeSeconds ?? 0}s`,
              row.query,
            ],
          }))}
        />
      </CardContent>
    </Card>
  );
}

function AdminTable({
  columns,
  rows,
}: {
  columns: string[];
  rows: Array<{ key: string; cells: ReactNode[] }>;
}) {
  if (rows.length === 0) {
    return (
      <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-xs text-text-muted">
        No rows.
      </div>
    );
  }
  return (
    <div className="overflow-auto rounded-md border border-border-subtle">
      <table className="w-full text-left text-xs">
        <thead className="bg-surface-panel text-[0.625rem] uppercase text-text-muted">
          <tr>
            {columns.map((column) => (
              <th key={column} className="px-2 py-1">
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.key} className="border-t border-border-subtle">
              {row.cells.map((cell, cellIndex) => (
                <td
                  key={`${row.key}-${columns[cellIndex]}`}
                  className="max-w-96 px-2 py-1 align-top"
                >
                  <span className="line-clamp-2 break-words">{cell}</span>
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${Math.round(bytes / 102.4) / 10} KB`;
  if (bytes < 1024 ** 3) return `${Math.round(bytes / 1024 / 102.4) / 10} MB`;
  return `${Math.round(bytes / 1024 / 1024 / 102.4) / 10} GB`;
}
