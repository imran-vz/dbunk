import { storageClassFor } from "@/lib/engine-policy";
import type { Connection, WorkspaceTab } from "@/lib/store";

/** Relational object/table tabs belong to Tables; SQL editors to Queries. */
export const relationalRailForTab = (
  kind: WorkspaceTab["kind"],
): "tables" | "queries" => (kind === "query" ? "queries" : "tables");

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
