import { IconDatabase, IconTerminal2 } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { StatusDot } from "@/components/ui/status-dot";
import type { Connection } from "@/lib/store";

import { KeyValue } from "./key-value";

export function DisconnectedConnectionCard({
  connection,
  onNewQuery,
  onConnect,
}: {
  connection: Connection;
  onNewQuery: () => void;
  onConnect: () => void;
}) {
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <Card className="w-full max-w-xl">
        <CardHeader>
          <CardTitle>{connection.name}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-4 text-xs">
          <div className="flex items-center gap-2 text-text-muted">
            <StatusDot tone="neutral" />
            <span>{connection.status}</span>
            <span>·</span>
            <span>{connection.engine}</span>
            <span>·</span>
            <span>{connection.database}</span>
          </div>
          <div className="grid grid-cols-2 gap-3 rounded-md border border-border-subtle bg-surface-panel p-3">
            <KeyValue
              label="Host"
              value={`${connection.host || "--"}:${connection.port || "--"}`}
            />
            <KeyValue label="User" value={connection.user || "--"} />
          </div>
          {connection.errorMessage ? (
            <div className="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-danger">
              {connection.errorMessage}
            </div>
          ) : null}
          <div className="flex items-center justify-end gap-2">
            <Button size="sm" variant="outline" onClick={onNewQuery}>
              <IconTerminal2 />
              New Query
            </Button>
            <Button size="sm" onClick={onConnect}>
              <IconDatabase />
              Connect
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

export function NoConnectionCard() {
  return (
    <div className="flex flex-1 items-center justify-center p-6">
      <Card className="max-w-md">
        <CardHeader>
          <CardTitle>No database connected</CardTitle>
        </CardHeader>
        <CardContent className="text-xs text-text-muted">
          Connect a database or create a new query to begin.
        </CardContent>
      </Card>
    </div>
  );
}
