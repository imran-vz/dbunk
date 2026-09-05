// @vitest-environment jsdom
import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  pgToolClient,
  type PgBackupScope,
  type PgToolJob,
} from "@/lib/pg-tool-jobs/client";
import { pgToolReview } from "@/lib/pg-tool-jobs/lifecycle";
import { pgToolObserver } from "@/lib/pg-tool-jobs/observer";
import { type Connection, useAppStore } from "@/lib/store";

import { useToolForm } from "./use-tool-form";

const connection: Connection = {
  id: "conn-1",
  name: "Local",
  database: "postgres",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "4 ms",
  ssl: false,
};
const queuedJob: PgToolJob = {
  jobId: "job-1",
  connectionId: connection.id,
  kind: "backup",
  format: "custom",
  fileName: "archive.dump",
  phase: "queued",
  startedAt: "2026-09-05T00:00:00Z",
  finishedAt: null,
  bytesProcessed: null,
  totalBytes: null,
  toolVersion: null,
  failure: null,
};
const initialStore = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialStore, true);
  useAppStore.setState({ connections: [connection] });
  pgToolReview.setState({ revision: 0, closing: 0 });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

async function choosePath(
  result: ReturnType<typeof renderToolForm>["result"],
  path: string | null,
) {
  vi.spyOn(pgToolClient, "pick").mockResolvedValueOnce(path);
  await act(async () => {
    await result.current.pick();
  });
}

function renderToolForm(
  scope: PgBackupScope = { kind: "database" },
  operation: PgToolJob["kind"] = "backup",
  target: Connection = connection,
) {
  return renderHook(() => useToolForm(target, scope, operation));
}

describe("PostgreSQL tool form payloads", () => {
  it.each([
    { kind: "database" } as const,
    { kind: "schema", schema: "Sales Ops" } as const,
    {
      kind: "table",
      schema: "Sales Ops",
      table: "Order.Items",
    } as const,
  ])("preserves exact $kind backup scope names", async (scope) => {
    const start = vi
      .spyOn(pgToolObserver, "startBackup")
      .mockResolvedValue(queuedJob);
    const { result } = renderToolForm(scope);
    await choosePath(result, "/tmp/archive.dump");

    await act(async () => {
      await result.current.submit();
    });

    expect(start).toHaveBeenCalledWith({
      connectionId: connection.id,
      destinationPath: "/tmp/archive.dump",
      format: "custom",
      scope,
      clean: false,
    });
  });

  it("normalizes clean for plain and custom backup and restore payloads", async () => {
    const backup = vi
      .spyOn(pgToolObserver, "startBackup")
      .mockResolvedValue(queuedJob);
    const backupForm = renderToolForm();
    act(() => backupForm.result.current.setClean(true));
    await choosePath(backupForm.result, "/tmp/custom.dump");
    await act(async () => backupForm.result.current.submit());
    expect(backup).toHaveBeenLastCalledWith(
      expect.objectContaining({ format: "custom", clean: false }),
    );

    act(() => {
      backupForm.result.current.changeFormat("plain");
      backupForm.result.current.setClean(true);
    });
    await choosePath(backupForm.result, "/tmp/plain.sql");
    await act(async () => backupForm.result.current.submit());
    expect(backup).toHaveBeenLastCalledWith(
      expect.objectContaining({ format: "plain", clean: true }),
    );

    const restore = vi
      .spyOn(pgToolObserver, "startRestore")
      .mockResolvedValue({ ...queuedJob, kind: "restore" });
    const restoreForm = renderToolForm({ kind: "database" }, "restore");
    act(() => restoreForm.result.current.setClean(true));
    await choosePath(restoreForm.result, "/tmp/custom.dump");
    await act(async () => restoreForm.result.current.submit());
    expect(restore).toHaveBeenLastCalledWith(
      expect.objectContaining({ format: "custom", clean: true }),
    );

    act(() => {
      restoreForm.result.current.changeFormat("plain");
      restoreForm.result.current.setClean(true);
    });
    await choosePath(restoreForm.result, "/tmp/plain.sql");
    await act(async () => restoreForm.result.current.submit());
    expect(restore).toHaveBeenLastCalledWith(
      expect.objectContaining({ format: "plain", clean: false }),
    );
  });

  it("surfaces a read-only refusal without retrying", async () => {
    const start = vi
      .spyOn(pgToolObserver, "startRestore")
      .mockRejectedValue({ kind: "policyBlocked", reason: "Read only" });
    const { result } = renderToolForm({ kind: "database" }, "restore", {
      ...connection,
      readOnly: true,
    });
    await choosePath(result, "/tmp/archive.dump");

    await act(async () => {
      await result.current.submit();
    });

    expect(start).toHaveBeenCalledTimes(1);
    expect(result.current.error).toBe("Read only");
    expect(result.current.path).toBe("/tmp/archive.dump");
  });

  it("keeps a colliding destination selected so another file can be chosen", async () => {
    const start = vi
      .spyOn(pgToolObserver, "startBackup")
      .mockRejectedValueOnce({ kind: "destinationExists" })
      .mockResolvedValueOnce(queuedJob);
    const { result } = renderToolForm();
    await choosePath(result, "/tmp/existing.dump");

    await act(async () => {
      await result.current.submit();
    });
    expect(result.current.path).toBe("/tmp/existing.dump");
    expect(result.current.error).toMatch(/already exists/i);

    await choosePath(result, "/tmp/new.dump");
    await act(async () => {
      await result.current.submit();
    });
    expect(start).toHaveBeenLastCalledWith(
      expect.objectContaining({ destinationPath: "/tmp/new.dump" }),
    );
    expect(result.current.path).toBeNull();
  });

  it("does nothing on picker cancellation and restores a table context into its database", async () => {
    const start = vi
      .spyOn(pgToolObserver, "startRestore")
      .mockResolvedValue({ ...queuedJob, kind: "restore" });
    const { result } = renderToolForm(
      { kind: "table", schema: "public", table: "users" },
      "restore",
    );

    await choosePath(result, null);
    await act(async () => {
      await result.current.submit();
    });
    expect(start).not.toHaveBeenCalled();

    await choosePath(result, "/tmp/archive.dump");
    await act(async () => {
      await result.current.submit();
    });
    expect(start).toHaveBeenCalledWith({
      connectionId: connection.id,
      sourcePath: "/tmp/archive.dump",
      format: "custom",
      clean: false,
      confirmed: false,
    });
  });
});
