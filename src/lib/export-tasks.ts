/* oxlint-disable anti-slop/no-runtime-typeof -- Export task errors cross the untyped promise-rejection boundary and are normalized here. */
import type {
  ExportCompression,
  ExportEncoding,
  ExportFormat,
} from "@/lib/export";

export type TableExportScope = {
  connectionId: string;
  schema: string;
  table: string;
};

export type SavedExportTask = {
  id: string;
  name: string;
  scope: TableExportScope;
  format: ExportFormat;
  encoding: ExportEncoding;
  compression: ExportCompression;
  nullAs: string;
  createdAt: string;
};

import { uiGet, uiSet } from "@/lib/ui-state";

const STORAGE_KEY = "dbunk.exportTasks.v1";

export function readExportTasks(): SavedExportTask[] {
  const raw = uiGet(STORAGE_KEY);
  if (!raw) {
    return [];
  }
  try {
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
}

export function saveExportTask(task: SavedExportTask): SavedExportTask[] {
  const next = [
    task,
    ...readExportTasks().filter((entry) => entry.id !== task.id),
  ];
  uiSet(STORAGE_KEY, JSON.stringify(next));
  return next;
}

export function findExportTask(
  scope: TableExportScope,
): SavedExportTask | null {
  return (
    readExportTasks().find(
      (task) =>
        task.scope.connectionId === scope.connectionId &&
        task.scope.schema === scope.schema &&
        task.scope.table === scope.table,
    ) ?? null
  );
}

export function createExportTask(
  scope: TableExportScope,
  format: ExportFormat,
  encoding: ExportEncoding,
  compression: ExportCompression,
  nullAs: string,
): SavedExportTask {
  return {
    id: `${scope.connectionId}:${scope.schema}:${scope.table}:${Date.now()}`,
    name: `${scope.schema}.${scope.table} ${format.toUpperCase()} export`,
    scope,
    format,
    encoding,
    compression,
    nullAs,
    createdAt: new Date().toISOString(),
  };
}
