/**
 * MySQL engine-specific fields. See `pg-fields.tsx` — both engines
 * share `host-auth` policy and render the same SSL toggle.
 */

import { HostAuthSslField } from "./host-auth-fields";
import type { ConnectionFormApi } from "./use-connection-form";

export function MySqlFields({ form }: { form: ConnectionFormApi }) {
  return <HostAuthSslField form={form} />;
}
