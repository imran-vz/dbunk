import { IconRefresh } from "@tabler/icons-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { ManagedServerWithStatus } from "@/lib/store";
import { useAppStore } from "@/lib/store";

/**
 * Settings → Local Databases: Managed Servers (ADR-0019) with status
 * derived live from Docker. Start/Stop are routine; Recreate restores
 * an orphaned server from its surviving volume; Destroy is the one
 * deliberately destructive action (container + data volume).
 */
export function ManagedServersTab() {
  const servers = useAppStore((state) => state.managedServers);
  const status = useAppStore((state) => state.managedServersStatus);
  const loadManagedServers = useAppStore((state) => state.loadManagedServers);

  useEffect(() => {
    if (status.state === "idle") {
      void loadManagedServers();
    }
  }, [loadManagedServers, status.state]);

  useEffect(() => {
    const interval = window.setInterval(() => {
      if (useAppStore.getState().managedServersStatus.state !== "loading") {
        void loadManagedServers();
      }
    }, 5_000);
    return () => window.clearInterval(interval);
  }, [loadManagedServers]);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-5 py-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="text-sm font-semibold">Local Databases</h2>
          <p className="text-xs text-text-muted">
            Databases dbunk provisioned with Docker. They keep running when
            dbunk quits and never auto-start at boot.
          </p>
        </div>
        <Button
          size="sm"
          variant="outline"
          aria-label="Refresh status"
          onClick={() => {
            void loadManagedServers();
          }}
        >
          <IconRefresh className="size-3.5" />
          Refresh
        </Button>
      </div>

      {status.state === "error" ? (
        <p className="rounded-sm border border-destructive/40 bg-destructive/10 px-2 py-1.5 text-xs text-destructive">
          {status.error}
        </p>
      ) : null}

      {servers.length === 0 && status.state === "ready" ? (
        <p className="text-xs text-text-muted">
          No local databases yet. Create one from the "New Local DB" button in
          the top bar.
        </p>
      ) : null}

      <div className="flex flex-col gap-2">
        {servers.map((server) => (
          <ManagedServerRow key={server.id} server={server} />
        ))}
      </div>
    </div>
  );
}

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const STATUS_TONE: Record<string, string> = {
  running: "text-accent",
  starting: "text-warning",
  stopped: "text-text-muted",
  orphaned: "text-destructive",
};

function ManagedServerRow({ server }: { server: ManagedServerWithStatus }) {
  const startManagedServer = useAppStore((state) => state.startManagedServer);
  const stopManagedServer = useAppStore((state) => state.stopManagedServer);
  const destroyManagedServer = useAppStore(
    (state) => state.destroyManagedServer,
  );
  const recreateManagedServer = useAppStore(
    (state) => state.recreateManagedServer,
  );

  const [busy, setBusy] = useState(false);
  const [confirmingDestroy, setConfirmingDestroy] = useState(false);

  const run = async (
    action: (id: string) => Promise<string | null>,
    successMessage: string,
  ) => {
    setBusy(true);
    const error = await action(server.id);
    setBusy(false);
    if (error) {
      toast.error(error);
    } else {
      toast.success(successMessage);
    }
  };

  return (
    <div
      data-testid={`managed-server-${server.name}`}
      className="flex flex-col gap-2 rounded-md border border-border-subtle bg-surface-panel px-3 py-2"
    >
      <div className="flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-xs font-semibold">{server.name}</span>
          <Badge variant="outline">
            {server.engine} {server.version}
          </Badge>
          <Badge variant="outline">port {server.port}</Badge>
          <span
            className={`text-[0.625rem] font-medium uppercase tracking-wide ${
              STATUS_TONE[server.status] ?? "text-text-muted"
            }`}
          >
            {server.status}
          </span>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {server.status === "running" ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => {
                void run(stopManagedServer, `${server.name} stopped`);
              }}
            >
              Stop
            </Button>
          ) : null}
          {server.status === "stopped" ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() => {
                void run(startManagedServer, `${server.name} started`);
              }}
            >
              Start
            </Button>
          ) : null}
          {server.status === "orphaned" && server.volumeExists ? (
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              title="The container was removed outside dbunk, but its data volume survived. Recreate the container and reattach the data."
              onClick={() => {
                void run(
                  recreateManagedServer,
                  `${server.name} recreated with its data`,
                );
              }}
            >
              Recreate
            </Button>
          ) : null}
          {confirmingDestroy ? (
            <>
              <Button
                size="sm"
                variant="destructive"
                disabled={busy}
                data-testid={`confirm-destroy-${server.name}`}
                onClick={() => {
                  setConfirmingDestroy(false);
                  void run(destroyManagedServer, `${server.name} destroyed`);
                }}
              >
                Confirm destroy
              </Button>
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => setConfirmingDestroy(false)}
              >
                Keep
              </Button>
            </>
          ) : (
            <Button
              size="sm"
              variant="ghost"
              disabled={busy}
              className="text-destructive"
              onClick={() => setConfirmingDestroy(true)}
            >
              Destroy
            </Button>
          )}
        </div>
      </div>
      {confirmingDestroy ? (
        <p className="text-[0.625rem] text-destructive">
          Destroy deletes the container and its data volume permanently. The
          saved connection stays in your sidebar until you delete it.
        </p>
      ) : null}
      {server.status === "orphaned" && !server.volumeExists ? (
        <p className="text-[0.625rem] text-text-muted">
          Both the container and its data volume were removed outside dbunk.
          Destroy removes this record.
        </p>
      ) : null}
      <p className="text-[0.625rem] text-text-muted">
        {server.database}@127.0.0.1:{server.port} · container{" "}
        {server.containerName}
      </p>
    </div>
  );
}
