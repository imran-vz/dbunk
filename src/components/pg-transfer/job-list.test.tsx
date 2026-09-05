// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import type { PgTransferJob } from "@/lib/pg-transfer/client";

import { PgTransferJobDetails } from "./job-list";

const job: PgTransferJob = {
  jobId: "job-1",
  connectionId: "conn-1",
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  phase: "running",
  startedAt: new Date().toISOString(),
  finishedAt: null,
  totalBytes: 100,
  bytesProcessed: 50,
  rowsProcessed: 12,
  rowsCommitted: null,
  failure: null,
};

afterEach(cleanup);

describe("PostgreSQL transfer result states", () => {
  it.each([
    ["preparing", "Preparing", "Preparing the append-only transaction."],
    ["running", "Running", "12 rows sent. Commit pending."],
    [
      "cancelling",
      "Cancellation requested",
      "Waiting for rollback or commit confirmation. Do not retry yet.",
    ],
    [
      "finalizing",
      "Finalizing",
      "Committing. Waiting for database acknowledgement.",
    ],
    ["completed", "Completed", "12 rows committed."],
    ["cancelled", "Cancelled", "Import cancelled. No rows committed."],
    ["failed", "Failed", "Import failed. No rows committed."],
    [
      "outcomeUnknown",
      "Outcome unknown",
      "Rows may be committed. Inspect the target before importing again.",
    ],
  ] as const)("renders %s truthfully", (phase, label, copy) => {
    render(
      <PgTransferJobDetails
        job={{
          ...job,
          phase,
          rowsCommitted: phase === "completed" ? 12 : null,
        }}
      />,
    );
    expect(screen.getByText(label)).toBeTruthy();
    expect(screen.getByText(copy)).toBeTruthy();
    if (phase !== "completed") {
      expect(screen.queryByText(/^\d[\d,]* rows committed/i)).toBeNull();
    }
  });
});
