/**
 * SSL toggle shared by PostgreSQL + MySQL. Both engines map to the
 * `host-auth` policy and surface a single bool — extracting once keeps
 * the dupe-detector happy and locks the wording in one place.
 */

import { ToggleSwitchRow } from "./field-helpers";
import type { ConnectionFormApi } from "./use-connection-form";

export function HostAuthSslField({ form }: { form: ConnectionFormApi }) {
  return (
    <form.Field name="ssl">
      {(field) => (
        <ToggleSwitchRow
          id="connection-ssl"
          title="SSL"
          description="Negotiate TLS on the wire protocol."
          checked={field.state.value ?? true}
          onCheckedChange={(value) => field.handleChange(value)}
        />
      )}
    </form.Field>
  );
}
