import type { Connection } from "./types";

export function bastionIdsReferencedByConnection(
  connection: Connection,
): string[] {
  if (connection.engine === "SQLite") {
    return [];
  }
  const tunnel = connection.sshTunnel;
  if (!tunnel?.enabled) {
    return [];
  }
  const ids = [...(tunnel.jumpChain ?? []), tunnel.bastionServerId]
    .map((id) => id?.trim())
    .filter((id): id is string => Boolean(id));
  return [...new Set(ids)];
}

export function connectionReferencesBastion(
  connection: Connection,
  bastionId: string,
): boolean {
  return bastionIdsReferencedByConnection(connection).includes(bastionId);
}
