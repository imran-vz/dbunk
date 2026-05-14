import { IconActivityHeartbeat } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import type { Connection } from "@/lib/store";

import { formatConnectionLatency, formatLastChecked } from "./format";

export function HealthBanner({ connection }: { connection: Connection }) {
  const status = connection.status;
  const isHealthy = status === "Connected" || status === "Read only";

  if (isHealthy) {
    return (
      <HealthyBanner
        latency={formatConnectionLatency(connection.latency)}
        lastChecked={formatLastChecked(connection.lastActivityAt)}
      />
    );
  }

  return <UnreachableBanner errorMessage={connection.errorMessage} />;
}

function HealthyBanner({
  latency,
  lastChecked,
}: {
  latency: string | null;
  lastChecked: string;
}) {
  return (
    <section className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-accent-green/25 bg-accent-green-subdued/40 px-4 py-3">
      <div className="flex items-center gap-3">
        <span className="flex size-9 items-center justify-center rounded-md bg-accent-green/15 text-accent-green">
          <IconActivityHeartbeat className="size-5" />
        </span>
        <div>
          <div className="text-sm font-medium text-accent-green-hover">
            Your connection is healthy
          </div>
          <div className="text-xs text-accent-green-hover/70">
            {latency
              ? `Round-trip ${latency}. Last checked ${lastChecked}.`
              : `Last checked ${lastChecked}.`}
          </div>
        </div>
      </div>
      <Button size="sm" variant="outline">
        View health checks
      </Button>
    </section>
  );
}

function UnreachableBanner({
  errorMessage,
}: {
  errorMessage: string | undefined;
}) {
  return (
    <section className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3">
      <div className="flex items-center gap-3">
        <span className="flex size-9 items-center justify-center rounded-md bg-danger/15 text-danger">
          <IconActivityHeartbeat className="size-5" />
        </span>
        <div>
          <div className="text-sm font-medium text-danger">
            Connection unreachable
          </div>
          <div className="font-mono text-[0.6875rem] text-danger/80">
            {errorMessage ?? "Last health check failed."}
          </div>
        </div>
      </div>
      <Button size="sm" variant="outline">
        View health checks
      </Button>
    </section>
  );
}
