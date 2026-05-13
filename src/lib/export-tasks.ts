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

const STORAGE_KEY = "dbunk.exportTasks.v1";

const storage = () =>
  typeof window === "undefined" ? null : window.localStorage;

export function readExportTasks(): SavedExportTask[] {
  const localStorage = storage();
  if (!localStorage) {
    return [];
  }
  const raw = localStorage.getItem(STORAGE_KEY);
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
  const localStorage = storage();
  if (localStorage) {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  }
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
