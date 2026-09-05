/* oxlint-disable anti-slop/no-unknown-parameters -- Tauri rejections are validated by the job error schema at this boundary. */
/** The only IPC boundary for PostgreSQL backup/restore and native path selection. */
import { z } from "zod";

import { decodeStatementSummaries } from "@/lib/safety-policy";
import { tauriInvoke } from "@/lib/tauri";

const summaries = z.unknown().transform((value, context) => {
  const decoded = decodeStatementSummaries(value);
  if (decoded) return decoded;
  context.addIssue({ code: "custom", message: "Invalid statement summaries" });
  return z.NEVER;
});
const jobError = z.discriminatedUnion("kind", [
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
  z.object({ kind: z.literal("destinationExists") }),
  z.object({ kind: z.literal("toolUnavailable"), tool: z.string() }),
  z.object({
    kind: z.literal("toolFailed"),
    tool: z.string(),
    exitCode: z.number().nullable(),
    message: z.string(),
  }),
  z.object({
    kind: z.literal("io"),
    operation: z.string(),
    message: z.string(),
  }),
  z.object({ kind: z.literal("timeout"), operation: z.string() }),
  z.object({ kind: z.literal("policyBlocked"), reason: z.string() }),
  z.object({
    kind: z.literal("policyNeedsConfirmation"),
    statements: summaries,
  }),
  z.object({ kind: z.literal("cancelled") }),
]);
export type PgToolError = z.infer<typeof jobError>;
export type PgToolFailure = PgToolError | { kind: "transport" };
export const pgToolSnapshotSchema = z.object({
  jobId: z.string(),
  connectionId: z.string(),
  kind: z.enum(["backup", "restore"]),
  format: z.enum(["plain", "custom"]),
  fileName: z.string(),
  phase: z.enum([
    "queued",
    "preflight",
    "running",
    "finalizing",
    "completed",
    "cancelling",
    "cancelled",
    "failed",
  ]),
  startedAt: z.string(),
  finishedAt: z.string().nullable(),
  bytesProcessed: z.number().nonnegative().nullable(),
  totalBytes: z.number().nonnegative().nullable(),
  toolVersion: z.string().nullable(),
  failure: jobError.nullable(),
});
export type PgToolJob = z.infer<typeof pgToolSnapshotSchema>;
export type PgArchiveFormat = PgToolJob["format"];
export type PgBackupScope =
  | { kind: "database" }
  | { kind: "schema"; schema: string }
  | { kind: "table"; schema: string; table: string };
export type PgBackupRequest = {
  connectionId: string;
  destinationPath: string;
  format: PgArchiveFormat;
  scope: PgBackupScope;
  clean: boolean;
};
export type PgRestoreRequest = {
  connectionId: string;
  sourcePath: string;
  format: PgArchiveFormat;
  clean: boolean;
  confirmed: boolean;
};

// Unknown transport failures must never echo paths, SQL, or credentials.
export function decodePgToolError(value: unknown): PgToolFailure {
  return jobError.safeParse(value).data ?? { kind: "transport" };
}
export function formatPgToolError(error: PgToolFailure): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return "Backup and restore are available for PostgreSQL only.";
    case "invalidRequest":
      return `${error.field}: ${error.reason}`;
    case "connectionClosing":
      return "Connection settings changed or the connection is closing. Review the target again.";
    case "jobLimitReached":
      return "A job is already active on this connection, or all four job slots are busy.";
    case "jobNotFound":
      return "This job expired or is no longer available.";
    case "jobActive":
      return "An active job cannot be dismissed.";
    case "destinationExists":
      return "This file already exists. Choose a new destination; backups never overwrite files.";
    case "toolUnavailable":
      return `${error.tool} is unavailable. Install a supported PostgreSQL client and check PATH.`;
    case "toolFailed":
      return `${error.tool}: ${error.message}`;
    case "io":
      return `${error.operation}: ${error.message}`;
    case "timeout":
      return `${error.operation} timed out. Cleanup may still be pending.`;
    case "policyBlocked":
      return error.reason;
    case "policyNeedsConfirmation":
      return "Restore needs safety confirmation.";
    case "cancelled":
      return "Operation cancelled.";
    case "transport":
      return "Unable to observe the job service. Refresh jobs before trying another start.";
  }
}
export const isPgToolJobActive = (job: PgToolJob) =>
  !["completed", "failed", "cancelled"].includes(job.phase);

export const pgToolClient = {
  startBackup: async (payload: PgBackupRequest) =>
    pgToolSnapshotSchema.parse(
      await tauriInvoke("start_pg_backup", { payload }),
    ),
  startRestore: async (payload: PgRestoreRequest) =>
    pgToolSnapshotSchema.parse(
      await tauriInvoke("start_pg_restore", { payload }),
    ),
  get: async (jobId: string) =>
    pgToolSnapshotSchema.parse(await tauriInvoke("get_pg_tool_job", { jobId })),
  list: async (connectionId: string | null = null) =>
    z
      .array(pgToolSnapshotSchema)
      .parse(await tauriInvoke("list_pg_tool_jobs", { connectionId })),
  cancel: async (jobId: string) =>
    pgToolSnapshotSchema.parse(
      await tauriInvoke("cancel_pg_tool_job", { jobId }),
    ),
  release: async (jobId: string) => {
    await tauriInvoke("release_pg_tool_job", { jobId });
  },
  pick: async (kind: PgToolJob["kind"], format: PgArchiveFormat) =>
    z
      .string()
      .nullable()
      .parse(await tauriInvoke("pick_pg_tool_file", { kind, format })),
};
