import type {
  ExportCompression,
  ExportEncoding,
  ExportFormat,
} from "@/lib/export";

export interface WholeTableExportSettings {
  format: ExportFormat;
  encoding: ExportEncoding;
  compression: ExportCompression;
  nullAs: string;
}

export type PgWholeTableExportRoute =
  | {
      kind: "nativeCsv";
      options: { header: true; nullToken: string };
    }
  | { kind: "refused"; message: string }
  | { kind: "buffered"; warning?: string };

/** Keeps PostgreSQL's unbounded CSV cases away from the renderer export path. */
export function routeWholeTableExport(
  engine: string | undefined,
  settings: WholeTableExportSettings,
): PgWholeTableExportRoute {
  if (engine !== "PostgreSQL") return { kind: "buffered" };
  if (settings.format !== "csv") {
    return {
      kind: "buffered",
      warning: `Whole-table ${settings.format.toUpperCase()} export loads all rows into app memory.`,
    };
  }
  if (settings.encoding !== "utf-8" || settings.compression !== "none") {
    return {
      kind: "refused",
      message:
        "Native PostgreSQL whole-table CSV export supports UTF-8 without compression. Change the export options and try again.",
    };
  }
  return {
    kind: "nativeCsv",
    options: { nullToken: settings.nullAs, header: true },
  };
}
