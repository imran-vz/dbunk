/* oxlint-disable anti-slop/no-unknown-parameters -- Tauri rejections are validated at this IPC boundary. */
import { z } from "zod";

import { decodeStatementSummaries } from "@/lib/safety-policy";
import { tauriInvoke } from "@/lib/tauri";

const safeCounter = z.number().int().nonnegative().max(Number.MAX_SAFE_INTEGER);
const statementSummaries = z.unknown().transform((value, context) => {
  const decoded = decodeStatementSummaries(value);
  if (decoded) return decoded;
  context.addIssue({ code: "custom", message: "Invalid statement summaries" });
  return z.NEVER;
});

const transferErrorSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("unsupportedEngine") }),
  z.object({
    kind: z.literal("invalidRequest"),
    field: z.string(),
    reason: z.string(),
  }),
  z.object({ kind: z.literal("connectionClosing") }),
  z.object({ kind: z.literal("jobLimitReached") }),
  z.object({ kind: z.literal("jobNotFound") }),
  z.object({ kind: z.literal("jobActive") }),
  z.object({ kind: z.literal("inspectionExpired") }),
  z.object({ kind: z.literal("sourceChanged") }),
  z.object({ kind: z.literal("targetChanged") }),
  z.object({ kind: z.literal("destinationExists") }),
  z.object({ kind: z.literal("unsupportedTarget"), reason: z.string() }),
  z.object({
    kind: z.literal("exportLimitExceeded"),
    limit: z.enum(["field", "record"]),
  }),
  z.object({
    kind: z.literal("csv"),
    record: safeCounter,
    column: z.number().int().nonnegative().nullable(),
    reason: z.string(),
  }),
  z.object({
    kind: z.literal("database"),
    code: z.string().nullable(),
    reason: z.string(),
  }),
  z.object({
    kind: z.literal("io"),
    operation: z.string(),
    reason: z.string(),
  }),
  z.object({ kind: z.literal("timeout"), operation: z.string() }),
  z.object({ kind: z.literal("policyBlocked"), reason: z.string() }),
  z.object({
    kind: z.literal("policyNeedsConfirmation"),
    statements: statementSummaries,
  }),
  z.object({ kind: z.literal("cancelled") }),
  z.object({ kind: z.literal("outcomeUnknown") }),
]);

export type PgTransferError = z.infer<typeof transferErrorSchema>;
export type PgTransferFailure = PgTransferError | { kind: "transport" };
export type PgTransferDirection = "import" | "export";
export type PgCsvOptions = {
  header: boolean;
  delimiter: string;
  quote: string;
  escape: string;
  nullToken: string;
};

const sourceColumnSchema = z.object({
  index: z.number().int().nonnegative(),
  name: z.string(),
});
const targetColumnSchema = z.object({
  name: z.string(),
  dataType: z.string(),
  nullable: z.boolean(),
  hasDefault: z.boolean(),
  generated: z.boolean(),
  identity: z.boolean(),
});
const csvOptionsSchema = z.object({
  header: z.boolean(),
  delimiter: z.string(),
  quote: z.string(),
  escape: z.string(),
  nullToken: z.string(),
});
export const pgTransferInspectionSchema = z.object({
  inspectionToken: z.string(),
  connectionId: z.string(),
  schema: z.string(),
  table: z.string(),
  direction: z.enum(["import", "export"]),
  fileName: z.string().nullable(),
  totalBytes: safeCounter.nullable(),
  sourceColumns: z.array(sourceColumnSchema),
  targetColumns: z.array(targetColumnSchema),
  sampleRows: z.array(z.array(z.string().nullable())),
  sampleTruncated: z.boolean(),
  options: csvOptionsSchema,
});
export type PgTransferInspection = z.infer<typeof pgTransferInspectionSchema>;
export type PgTransferColumnMapping = {
  sourceIndex: number;
  targetColumn: string;
};
export type PgTransferInspectRequest = {
  connectionId: string;
  schema: string;
  table: string;
  direction: PgTransferDirection;
  sourcePath: string | null;
  options: PgCsvOptions;
};
export type PgTransferStartImportRequest = {
  inspectionToken: string;
  mapping: PgTransferColumnMapping[];
  confirmed: boolean;
};
export type PgTransferStartExportRequest = {
  inspectionToken: string;
  destinationPath: string;
};

export const pgTransferSnapshotSchema = z.object({
  jobId: z.string(),
  connectionId: z.string(),
  schema: z.string(),
  table: z.string(),
  direction: z.enum(["import", "export"]),
  fileName: z.string(),
  phase: z.enum([
    "preparing",
    "running",
    "cancelling",
    "finalizing",
    "completed",
    "cancelled",
    "failed",
    "outcomeUnknown",
  ]),
  startedAt: z.string(),
  finishedAt: z.string().nullable(),
  totalBytes: safeCounter.nullable(),
  bytesProcessed: safeCounter,
  rowsProcessed: safeCounter.nullable(),
  rowsCommitted: safeCounter.nullable(),
  failure: transferErrorSchema.nullable(),
});
export type PgTransferJob = z.infer<typeof pgTransferSnapshotSchema>;

export const DEFAULT_PG_CSV_OPTIONS: PgCsvOptions = {
  header: true,
  delimiter: ",",
  quote: '"',
  escape: '"',
  nullToken: "\\N",
};

// Unknown transport failures must never echo paths, row values, or credentials.
export function decodePgTransferError(value: unknown): PgTransferFailure {
  return transferErrorSchema.safeParse(value).data ?? { kind: "transport" };
}

export function formatPgTransferError(error: PgTransferFailure): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return "Native CSV transfer is available for PostgreSQL only.";
    case "invalidRequest":
      return `${error.field}: ${error.reason}`;
    case "connectionClosing":
      return "Connection settings changed or the connection is closing. Review the transfer again.";
    case "jobLimitReached":
      return "A transfer is already active on this connection, or all four transfer slots are busy.";
    case "jobNotFound":
      return "This transfer expired or is no longer available.";
    case "jobActive":
      return "An active transfer cannot be dismissed.";
    case "inspectionExpired":
      return "This review expired. Inspect the file and table again.";
    case "sourceChanged":
      return "The source file changed after review. Inspect it again before importing.";
    case "targetChanged":
      return "The table changed after review. Inspect it again before starting.";
    case "destinationExists":
      return "This destination already exists. Choose a new file; exports never overwrite files.";
    case "unsupportedTarget":
      return error.reason;
    case "exportLimitExceeded":
      return error.limit === "field"
        ? "A database field exceeds the 1 MiB CSV export limit."
        : "A database row exceeds the 8 MiB CSV export limit.";
    case "csv":
      return `CSV record ${error.record.toLocaleString()}${error.column === null ? "" : `, column ${error.column}`}: ${error.reason}`;
    case "database":
      return error.code ? `${error.reason} (${error.code})` : error.reason;
    case "io":
      return `${error.operation}: ${error.reason}`;
    case "timeout":
      return `${error.operation} timed out. Cleanup may still be pending.`;
    case "policyBlocked":
      return error.reason;
    case "policyNeedsConfirmation":
      return "Import needs safety confirmation.";
    case "cancelled":
      return "Transfer cancelled.";
    case "outcomeUnknown":
      return "The connection was lost while committing. Check the target before trying this import again.";
    case "transport":
      return "Unable to observe the transfer service. Refresh transfers before trying another start.";
  }
}

export const isPgTransferJobActive = (job: PgTransferJob) =>
  !["completed", "cancelled", "failed", "outcomeUnknown"].includes(job.phase);

export const pgTransferClient = {
  inspect: async (payload: PgTransferInspectRequest) =>
    pgTransferInspectionSchema.parse(
      await tauriInvoke("inspect_pg_transfer", { payload }),
    ),
  releaseInspection: async (inspectionToken: string) => {
    await tauriInvoke("release_pg_transfer_inspection", { inspectionToken });
  },
  startImport: async (payload: PgTransferStartImportRequest) =>
    pgTransferSnapshotSchema.parse(
      await tauriInvoke("start_pg_csv_import", { payload }),
    ),
  startExport: async (payload: PgTransferStartExportRequest) =>
    pgTransferSnapshotSchema.parse(
      await tauriInvoke("start_pg_csv_export", { payload }),
    ),
  get: async (jobId: string) =>
    pgTransferSnapshotSchema.parse(
      await tauriInvoke("get_pg_transfer_job", { jobId }),
    ),
  list: async (connectionId: string | null = null) =>
    z
      .array(pgTransferSnapshotSchema)
      .parse(await tauriInvoke("list_pg_transfer_jobs", { connectionId })),
  cancel: async (jobId: string) =>
    pgTransferSnapshotSchema.parse(
      await tauriInvoke("cancel_pg_transfer_job", { jobId }),
    ),
  release: async (jobId: string) => {
    await tauriInvoke("release_pg_transfer_job", { jobId });
  },
  pick: async (direction: PgTransferDirection) =>
    z
      .string()
      .nullable()
      .parse(await tauriInvoke("pick_pg_transfer_file", { direction })),
};
