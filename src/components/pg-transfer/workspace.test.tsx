/* oxlint-disable anti-slop/no-module-mocking, anti-slop/require-safety-comment-for-type-assertion -- Native file and transfer commands are unavailable in jsdom; role queries establish input element types. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/tauri")>()),
  isTauri: () => true,
}));

import {
  DEFAULT_PG_CSV_OPTIONS,
  pgTransferClient,
  type PgTransferInspection,
  type PgTransferJob,
} from "@/lib/pg-transfer/client";
import { pgTransferObserver } from "@/lib/pg-transfer/observer";
import { type Connection, useAppStore } from "@/lib/store";

import { PgTransferWorkspace } from "./workspace";

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
const inspection: PgTransferInspection = {
  inspectionToken: "review-1",
  connectionId: "conn-1",
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  totalBytes: 32,
  sourceColumns: [
    { index: 0, name: "email" },
    { index: 1, name: "email" },
  ],
  targetColumns: [
    {
      name: "email",
      dataType: "text",
      nullable: false,
      hasDefault: false,
      generated: false,
      identity: false,
    },
    {
      name: "name",
      dataType: "text",
      nullable: false,
      hasDefault: false,
      generated: false,
      identity: false,
    },
  ],
  sampleRows: [["ada@example.test", "Ada"]],
  sampleTruncated: true,
  options: DEFAULT_PG_CSV_OPTIONS,
};
const job: PgTransferJob = {
  jobId: "job-1",
  connectionId: "conn-1",
  schema: "public",
  table: "users",
  direction: "import",
  fileName: "users.csv",
  phase: "preparing",
  startedAt: "2026-09-05T00:00:00Z",
  finishedAt: null,
  totalBytes: 32,
  bytesProcessed: 0,
  rowsProcessed: null,
  rowsCommitted: null,
  failure: null,
};
const initialStore = useAppStore.getState();

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

beforeEach(() => {
  useAppStore.setState(initialStore, true);
  useAppStore.setState({ connections: [connection] });
  pgTransferObserver.store.setState({
    jobs: [],
    error: null,
    refreshing: false,
    observedAt: null,
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  useAppStore.setState(initialStore, true);
  pgTransferObserver.store.setState({
    jobs: [],
    error: null,
    refreshing: false,
    observedAt: null,
  });
});

describe("PostgreSQL CSV Transfer workspace", () => {
  it("maps duplicate headers by source index and starts the reviewed mapping", async () => {
    vi.spyOn(pgTransferClient, "pick").mockResolvedValue("/tmp/users.csv");
    vi.spyOn(pgTransferClient, "inspect").mockResolvedValue(inspection);
    const start = vi
      .spyOn(pgTransferObserver, "startImport")
      .mockResolvedValue(job);
    render(
      <PgTransferWorkspace
        connection={connection}
        schema="public"
        table="users"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    const first = await screen.findByRole("combobox", {
      name: "Map source column 1",
    });
    const second = screen.getByRole("combobox", {
      name: "Map source column 2",
    });
    expect((first as HTMLSelectElement).value).toBe("email");
    expect((second as HTMLSelectElement).value).toBe("");
    fireEvent.change(second, { target: { value: "name" } });

    fireEvent.click(screen.getByRole("button", { name: "Import CSV…" }));
    await waitFor(() =>
      expect(start).toHaveBeenCalledWith({
        inspectionToken: "review-1",
        mapping: [
          { sourceIndex: 0, targetColumn: "email" },
          { sourceIndex: 1, targetColumn: "name" },
        ],
        confirmed: false,
      }),
    );
  });

  it("does nothing when the native picker is cancelled", async () => {
    vi.spyOn(pgTransferClient, "pick").mockResolvedValue(null);
    const inspect = vi.spyOn(pgTransferClient, "inspect");
    render(
      <PgTransferWorkspace
        connection={connection}
        schema="public"
        table="users"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    await waitFor(() => expect(pgTransferClient.pick).toHaveBeenCalledOnce());
    expect(inspect).not.toHaveBeenCalled();
    expect(
      (screen.getByLabelText("CSV source file") as HTMLInputElement).value,
    ).toBe("No file selected");
  });

  it("does not inspect when the native picker resolves after unmount", async () => {
    const picked = deferred<string | null>();
    vi.spyOn(pgTransferClient, "pick").mockReturnValue(picked.promise);
    const inspect = vi.spyOn(pgTransferClient, "inspect");
    const rendered = render(
      <PgTransferWorkspace
        connection={connection}
        schema="public"
        table="users"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    await waitFor(() => expect(pgTransferClient.pick).toHaveBeenCalledOnce());
    rendered.unmount();
    await act(async () => picked.resolve("/tmp/users.csv"));

    expect(inspect).not.toHaveBeenCalled();
  });

  it("releases an inspection that resolves after unmount", async () => {
    vi.spyOn(pgTransferClient, "pick").mockResolvedValue("/tmp/users.csv");
    const reviewed = deferred<PgTransferInspection>();
    vi.spyOn(pgTransferClient, "inspect").mockReturnValue(reviewed.promise);
    const release = vi
      .spyOn(pgTransferClient, "releaseInspection")
      .mockResolvedValue();
    const rendered = render(
      <PgTransferWorkspace
        connection={connection}
        schema="public"
        table="users"
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    await waitFor(() =>
      expect(pgTransferClient.inspect).toHaveBeenCalledOnce(),
    );
    rendered.unmount();
    await act(async () => reviewed.resolve(inspection));

    expect(release).toHaveBeenCalledWith(inspection.inspectionToken);
  });

  it("releases a review when CSV settings change", async () => {
    vi.spyOn(pgTransferClient, "pick").mockResolvedValue("/tmp/users.csv");
    vi.spyOn(pgTransferClient, "inspect").mockResolvedValue(inspection);
    const release = vi
      .spyOn(pgTransferClient, "releaseInspection")
      .mockResolvedValue();
    render(
      <PgTransferWorkspace
        connection={connection}
        schema="public"
        table="users"
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
    await screen.findByText("Preview");

    fireEvent.change(screen.getByRole("combobox", { name: "CSV delimiter" }), {
      target: { value: ";" },
    });
    expect(release).toHaveBeenCalledWith("review-1");
    expect(screen.queryByText("Preview")).toBeNull();
    expect(screen.getByRole("button", { name: "Inspect again" })).toBeTruthy();
  });

  it.each(["target", "intent"] as const)(
    "resets pending inspection on a new %s without disturbing the next inspection",
    async (reset) => {
      vi.spyOn(pgTransferClient, "pick").mockResolvedValue("/tmp/users.csv");
      const first = deferred<PgTransferInspection>();
      const second = deferred<PgTransferInspection>();
      const inspect = vi
        .spyOn(pgTransferClient, "inspect")
        .mockReturnValueOnce(first.promise)
        .mockReturnValueOnce(second.promise);
      const release = vi
        .spyOn(pgTransferClient, "releaseInspection")
        .mockResolvedValue();
      const rendered = render(
        <PgTransferWorkspace
          connection={connection}
          schema="public"
          table="users"
        />,
      );
      fireEvent.click(screen.getByRole("button", { name: "Browse…" }));
      await waitFor(() => expect(inspect).toHaveBeenCalledTimes(1));

      const table = reset === "target" ? "other_users" : "users";
      rendered.rerender(
        <PgTransferWorkspace
          connection={connection}
          schema="public"
          table={table}
          intent={
            reset === "intent" ? { id: 1, direction: "import" } : undefined
          }
        />,
      );
      const browse = screen.getByRole("button", {
        name: "Browse…",
      }) as HTMLButtonElement;
      expect(browse.disabled).toBe(false);
      fireEvent.click(browse);
      await waitFor(() => expect(inspect).toHaveBeenCalledTimes(2));

      await act(async () => first.resolve(inspection));
      expect(release).toHaveBeenCalledWith(inspection.inspectionToken);
      expect(browse.disabled).toBe(true);
      expect(screen.queryByText("Preview")).toBeNull();

      await act(async () =>
        second.resolve({ ...inspection, table, inspectionToken: "review-2" }),
      );
      expect(browse.disabled).toBe(false);
      expect(screen.getByText("Preview")).toBeTruthy();
    },
  );
});
