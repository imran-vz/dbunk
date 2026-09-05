import { refreshAfterPgExternalWrite } from "@/lib/pg-tool-jobs/restore-refresh";

import type { PgTransferJob } from "./client";

/** A committed or uncertain import invalidates connection data without dropping drafts. */
export async function refreshAfterPgCsvImport(job: PgTransferJob) {
  await refreshAfterPgExternalWrite({
    connectionId: job.connectionId,
    operation: "CSV import",
    warning:
      job.phase === "outcomeUnknown"
        ? "CSV import outcome is unknown. Rows may be committed. Inspect the target before retrying."
        : undefined,
  });
}
