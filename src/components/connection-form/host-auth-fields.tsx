/**
 * The single-bool SSL toggle for `host-auth` engines whose policy says
 * `tlsControls: "toggle"` (MySQL today). PostgreSQL moved to
 * `<TlsFields>` in ADR-0025; this stays separate so the wording and the
 * `ssl` wiring live in one place for whichever engines keep the toggle.
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
