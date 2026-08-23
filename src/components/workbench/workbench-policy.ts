import { storageClassFor } from "@/lib/engine-policy";
import type { Connection } from "@/lib/store";

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
