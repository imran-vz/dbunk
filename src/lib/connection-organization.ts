/**
 * Connection list organization (Plan 010, mock A): group by folder
 * with ungrouped last; inside each group favorites pin first, then
 * most-recent activity, then name. Pure so the ordering contract is
 * unit-tested without rendering.
 */

import type { Connection } from "@/lib/store";

export type ConnectionFolderGroup = {
  /** Folder name; empty string = the ungrouped tail. */
  folder: string;
  connections: Connection[];
};

const activityMs = (connection: Connection): number => {
  if (!connection.lastActivityAt) return 0;
  const parsed = Date.parse(connection.lastActivityAt);
  return Number.isNaN(parsed) ? 0 : parsed;
};

const compareWithinGroup = (a: Connection, b: Connection): number => {
  const favorite = Number(b.isFavorite ?? false) - Number(a.isFavorite ?? false);
  if (favorite !== 0) return favorite;
  const recency = activityMs(b) - activityMs(a);
  if (recency !== 0) return recency;
  return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
};

export function organizeConnections(
  connections: Connection[],
): ConnectionFolderGroup[] {
  const groups = new Map<string, Connection[]>();
  for (const connection of connections) {
    const folder = connection.folder?.trim() ?? "";
    const bucket = groups.get(folder);
    if (bucket) bucket.push(connection);
    else groups.set(folder, [connection]);
  }
  return [...groups.entries()]
    .sort(([a], [b]) => {
      // Ungrouped ("") always last; folders alphabetically before it.
      if (a === "") return 1;
      if (b === "") return -1;
      return a.localeCompare(b, undefined, { sensitivity: "base" });
    })
    .map(([folder, members]) => ({
      folder,
      connections: [...members].sort(compareWithinGroup),
    }));
}
