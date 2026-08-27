/**
 * MySQL engine-specific fields: the single SSL toggle. PostgreSQL shares
 * the `host-auth` policy but renders `<TlsFields>` instead
 * (`tlsControls: "postgres-modes"`, ADR-0025).
 */

import { HostAuthSslField } from "./host-auth-fields";
import type { ConnectionFormApi } from "./use-connection-form";

export function MySqlFields({ form }: { form: ConnectionFormApi }) {
  return <HostAuthSslField form={form} />;
}
