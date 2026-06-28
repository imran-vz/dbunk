import { storageClassFor } from "@/lib/engine-policy";
import type { Connection } from "@/lib/store";

/** True when the app should render the Workbench Rail shell instead of legacy chrome. */
export function usesWorkbenchShell(
  credentialState: string | undefined,
): boolean {
  return credentialState === "ready";
}

export function isKeyValueConnection(
  connection: Connection | undefined,
): connection is Extract<Connection, { engine: "Redis" }> {
  return (
    connection !== undefined &&
    storageClassFor(connection.engine) === "keyvalue"
  );
}

export function firstRelationalConnection(
  connections: Connection[],
): Connection | undefined {
  return connections.find(
    (connection) => storageClassFor(connection.engine) === "relational",
  );
}
