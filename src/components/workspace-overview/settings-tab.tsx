import { IconEdit } from "@tabler/icons-react";
import { type ReactNode, useState } from "react";

import { EditConnectionDialog } from "@/components/edit-connection-dialog";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import type { Connection } from "@/lib/store";

import { KeyValue } from "./key-value";

/**
 * Settings sub-tab — read-only mirror of the connection's existing
 * fields plus an Edit button that opens the existing connection
 * dialog. Phase 1 intentionally does not introduce new fields (SSH
 * tunnel, keepalive, statement timeout, etc.) — see PHASES.md.
 */
export function SettingsTab({ connection }: { connection: Connection }) {
  const [isEditing, setIsEditing] = useState(false);

  const rows = buildSettingsRows(connection);

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Connection settings</CardTitle>
          <CardAction>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setIsEditing(true)}
            >
              <IconEdit className="size-3.5" />
              Edit connection
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-x-6 gap-y-3 text-xs sm:grid-cols-3">
          {rows.map(([label, value]) => (
            <KeyValue key={label} label={label} value={value} />
          ))}
        </CardContent>
      </Card>

      <EditConnectionDialog
        connection={isEditing ? connection : null}
        open={isEditing}
        onOpenChange={(open) => {
          if (!open) setIsEditing(false);
        }}
      />
    </>
  );
}

const placeholderValue = <span className="text-text-muted">—</span>;

/**
 * Build the engine-aware row list for the settings panel. Common
 * fields render for every relational connection; engine-specific
 * fields (TLS flavour, ClickHouse URL path) trail the common block.
 * Empty/optional values render as a muted dash so the grid stays
 * symmetric across engines.
 */
function buildSettingsRows(connection: Connection): Array<[string, ReactNode]> {
  const rows: Array<[string, ReactNode]> = [
    ["Name", connection.name],
    ["Engine", connection.engine],
    ["Host", connection.host || placeholderValue],
    ["Port", connection.port ? String(connection.port) : placeholderValue],
    ["Database", connection.database || placeholderValue],
    ["User", connection.user || placeholderValue],
    ["Role", connection.role || placeholderValue],
  ];

  if (connection.engine === "PostgreSQL" || connection.engine === "MySQL") {
    rows.push(["SSL", connection.ssl ? "Enabled" : "Disabled"]);
  } else if (connection.engine === "ClickHouse") {
    rows.push(["HTTPS", connection.useHttps ? "Enabled" : "Disabled"]);
    rows.push(["URL path", connection.urlPath || placeholderValue]);
  }
  // SQLite has no TLS / network-config fields beyond the common block.
  // Redis takes the keyvalue workspace branch and never reaches here.

  return rows;
}
