/**
 * PostgreSQL engine-specific fields. PG and MySQL share the same
 * connection-form policy (`host-auth` + SSL toggle), so the actual
 * field rendering is centralised in `<HostAuthSslField>` and the
 * engine-named exports are thin aliases — keeps each engine
 * file present for symmetry with the other engine modules.
 */

import { HostAuthSslField } from "./host-auth-fields";
import type { ConnectionFormApi } from "./use-connection-form";

export function PgFields({ form }: { form: ConnectionFormApi }) {
  return <HostAuthSslField form={form} />;
}
