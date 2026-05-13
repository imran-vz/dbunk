/**
 * ClickHouse engine-specific advanced fields: HTTPS toggle + optional
 * URL path. Rendered inside the parent's `Advanced Options` collapsible.
 */

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import { ToggleSwitchRow } from "./field-helpers";
import type { ConnectionFormApi } from "./use-connection-form";

export function ClickHouseFields({ form }: { form: ConnectionFormApi }) {
  return (
    <>
      <form.Field name="useHttps">
        {(field) => (
          <ToggleSwitchRow
            id="connection-use-https"
            title="Use HTTPS"
            description="Connect to ClickHouse over TLS (port 8443)."
            checked={field.state.value ?? false}
            onCheckedChange={(value) => field.handleChange(value)}
          />
        )}
      </form.Field>
      <form.Field name="urlPath">
        {(field) => (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-url-path">URL path</Label>
            <Input
              id="connection-url-path"
              placeholder="/clickhouse (optional)"
              value={field.state.value ?? ""}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={field.handleBlur}
            />
          </div>
        )}
      </form.Field>
    </>
  );
}
