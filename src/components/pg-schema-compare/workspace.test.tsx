/* oxlint-disable anti-slop/no-module-mocking -- The desktop boundary is unavailable in jsdom. */
// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/tauri")>()),
  isTauri: () => true,
}));

import {
  createSchemaCompareClient,
  type SchemaCompareInvoke,
} from "@/lib/pg-schema-compare/client";
import { createSchemaCompareObserver } from "@/lib/pg-schema-compare/observer";
import {
  SCHEMA_COMPARE_NORMALIZATION_VERSION,
  SCHEMA_COMPARE_SCOPE,
  type SchemaCompareObjectSummary,
  type SchemaCompareStatus,
  schemaCompareReadRequest,
  schemaCompareResultRequest,
  schemaCompareStartRequest,
} from "@/lib/pg-schema-compare/protocol";
import { type Connection, useAppStore } from "@/lib/store";

import { schemaCompareForm } from "./use-compare-form";
import { SchemaCompareWorkspace } from "./workspace";

const postgres = (id: string, name: string): Connection => ({
  id,
  name,
  database: `${id}-db`,
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "4 ms",
  ssl: false,
  environment: id === "staging" ? "staging" : "development",
});
const staging = postgres("staging", "Acme staging");
const dev = postgres("dev", "Acme development");
const transport = "4f2b9150-58d7-4a77-8819-90bccf0329b9";
const orders: SchemaCompareObjectSummary = {
  kind: "changed",
  fieldCount: 1,
  changedFields: 1,
  incomparableFields: 0,
  source: { kind: "table", name: "orders" },
  target: { kind: "table", name: "orders" },
};

/** A native double: admission, status, reads and acknowledgement in memory. */
function fakeNative() {
  const jobs: SchemaCompareStatus[] = [];
  let loseStartResponse = false;
  let releaseFailure: { kind: "busy" } | null = null;
  const invoke = vi.fn<SchemaCompareInvoke>(async (command, payload) => {
    switch (command) {
      case "start_pg_schema_compare": {
        const request = schemaCompareStartRequest.parse(payload?.payload);
        const existing = jobs.find((j) => j.requestId === request.requestId);
        if (existing) return existing;
        const job: SchemaCompareStatus = {
          jobId: `job-${jobs.length + 1}`,
          requestId: request.requestId,
          source: request.source,
          target: request.target,
          sourceObjects: 12,
          targetObjects: 9,
          phase: "comparing",
        };
        jobs.push(job);
        if (loseStartResponse) throw "lost response";
        return job;
      }
      case "list_pg_schema_compares":
        return [...jobs];
      case "cancel_pg_schema_compare":
        return jobs.find((j) => j.jobId === payload?.jobId);
      case "release_pg_schema_compare": {
        if (releaseFailure) throw releaseFailure;
        const index = jobs.findIndex((j) => j.jobId === payload?.jobId);
        if (index >= 0) jobs.splice(index, 1);
        return null;
      }
      case "get_pg_schema_compare_transport":
        return transport;
      case "acknowledge_pg_schema_compare":
        return null;
      case "read_pg_schema_compare": {
        const request = schemaCompareResultRequest.parse(payload?.request);
        const read = schemaCompareReadRequest.parse(payload?.read);
        const base = {
          responseId: payload?.responseId,
          identity: request.identity,
        };
        switch (read.kind) {
          case "metadata":
            return {
              ...base,
              detail: {
                metadata: {
                  identity: request.identity,
                  source: {
                    endpoint: request.source,
                    serverVersion: "16.15",
                    serverVersionNum: 160015,
                    capturedAt: "2026-09-14T10:42:03Z",
                  },
                  target: {
                    endpoint: request.target,
                    serverVersion: "16.15",
                    serverVersionNum: 160015,
                    capturedAt: "2026-09-14T10:42:04Z",
                  },
                  consistency: "independentTransactions",
                  coverage: {
                    scope: SCHEMA_COMPARE_SCOPE,
                    normalizationVersion: SCHEMA_COMPARE_NORMALIZATION_VERSION,
                    excludedRelations: 0,
                    incomparableFields: 0,
                    excludedCategories: [],
                  },
                },
                kind: "changed",
                objectCount: 1,
                sourceExcludedCounts: [],
                targetExcludedCounts: [],
              },
            };
          case "objects":
            return { ...base, offset: 0, nextOffset: null, items: [orders] };
          case "fields":
            return {
              ...base,
              offset: 0,
              nextOffset: null,
              items: [
                {
                  kind: "changed",
                  path: { kind: "column", name: "total", field: "type" },
                  source: {
                    side: "source",
                    valueId: 0,
                    rawBytes: 4,
                    valueKind: "text",
                  },
                  target: {
                    side: "target",
                    valueId: 1,
                    rawBytes: 0,
                    valueKind: "text",
                  },
                },
              ],
            };
          case "value":
            return {
              ...base,
              value: read.value,
              offset: 0,
              text: "🦀",
              nextOffset: 4,
              complete: true,
            };
          case "eligibility":
            return {
              ...base,
              detail: {
                object: read.object,
                side: read.side,
                eligibility: { kind: "eligible" },
              },
            };
        }
      }
    }
    throw new Error(`unexpected command ${command}`);
  });
  return {
    invoke,
    jobs,
    complete(jobId: string) {
      const index = jobs.findIndex((j) => j.jobId === jobId);
      const job = jobs[index];
      if (!job) throw new Error("missing job");
      jobs[index] = { ...job, phase: "completed", resultId: `result-${jobId}` };
    },
    fail(jobId: string) {
      const index = jobs.findIndex((j) => j.jobId === jobId);
      const job = jobs[index];
      if (!job) throw new Error("missing job");
      jobs[index] = {
        ...job,
        phase: "failed",
        failure: { kind: "captureChanged" },
      };
    },
    loseStartResponse(next: boolean) {
      loseStartResponse = next;
    },
    failRelease(next: { kind: "busy" } | null) {
      releaseFailure = next;
    },
  };
}

const initialStore = useAppStore.getState();
const initialForm = schemaCompareForm.getState();

beforeEach(() => {
  useAppStore.setState(initialStore, true);
  useAppStore.setState({
    connections: [staging, dev],
    schemaExplorer: { dev: [{ name: "public", tables: [] }] },
  });
  schemaCompareForm.setState(initialForm, true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

function setup() {
  const native = fakeNative();
  const client = createSchemaCompareClient(native.invoke);
  const observer = createSchemaCompareObserver(client);
  const unmount = observer.mount();
  const view = render(
    <SchemaCompareWorkspace
      connection={staging}
      client={client}
      observer={observer}
    />,
  );
  return { native, observer, view, unmount };
}
function fillTarget(schema = "public") {
  fireEvent.change(
    screen.getByRole("combobox", { name: "Target connection" }),
    {
      target: { value: dev.id },
    },
  );
  fireEvent.change(screen.getByLabelText("Source schema"), {
    target: { value: "public" },
  });
  fireEvent.change(screen.getByLabelText("Target schema"), {
    target: { value: schema },
  });
}

describe("schema comparison workspace", () => {
  it("seeds Source from the active connection and validates the draft without rewriting it", async () => {
    const { native, unmount } = setup();
    try {
      const source = screen.getByRole("combobox", {
        name: "Source connection",
      });
      expect(
        within(source).getByRole("option", { selected: true }).textContent,
      ).toContain("Acme staging");
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      expect(screen.getByRole("alert").textContent).toBe(
        "Enter the source schema name.",
      );
      fillTarget(" public ");
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      await waitFor(() => expect(native.jobs).toHaveLength(1));
      // Typed identifiers are compared exactly; nothing is trimmed.
      expect(native.jobs[0]?.target.schema).toBe(" public ");
    } finally {
      unmount();
    }
  });

  it("admits one request, follows it to completion and inspects a field lazily", async () => {
    const { native, observer, unmount } = setup();
    try {
      fillTarget();
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      await screen.findByText("Comparing");
      expect(
        screen.getByText("Source: 12 objects read · Target: 9 objects read"),
      ).toBeTruthy();
      // Both endpoint identities stay visible on the accepted job.
      expect(screen.getAllByText("Acme staging").length).toBeGreaterThan(0);
      expect(screen.getAllByText("Acme development").length).toBeGreaterThan(0);

      native.complete("job-1");
      await observer.refresh();
      await screen.findByText("orders");
      // Overall kind above the result and the object's own kind in the list.
      expect(screen.getAllByText("Changed")).toHaveLength(2);
      expect(screen.getByText("1–1 of 1")).toBeTruthy();
      expect(screen.getByText("Coverage & capture")).toBeTruthy();

      fireEvent.click(screen.getByRole("button", { name: /orders/ }));
      const field = await screen.findByRole("button", { name: /total/ });
      expect(screen.getAllByText("Select to inspect")).toHaveLength(2);
      fireEvent.click(field);
      const source = await screen.findByRole("region", {
        name: "Source value",
      });
      await within(source).findByText("🦀");
      const target = screen.getByRole("region", { name: "Target value" });
      expect(within(target).getByText("Empty string")).toBeTruthy();
      const valueReads = native.invoke.mock.calls.filter(
        ([command, payload]) =>
          command === "read_pg_schema_compare" &&
          schemaCompareReadRequest.parse(payload?.read).kind === "value",
      );
      expect(valueReads).toHaveLength(1);
    } finally {
      unmount();
    }
  });

  it("holds a lost start response and resolves it by request id without a second start", async () => {
    const { native, unmount } = setup();
    try {
      native.loseStartResponse(true);
      fillTarget();
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      await screen.findByText("Comparing");
      const starts = native.invoke.mock.calls.filter(
        ([command]) => command === "start_pg_schema_compare",
      );
      expect(starts).toHaveLength(1);
      expect(native.jobs).toHaveLength(1);
      expect(screen.queryByText("Admission not confirmed")).toBeNull();
    } finally {
      unmount();
    }
  });

  it("keeps a job whose dismissal failed and offers the action again", async () => {
    const { native, observer, unmount } = setup();
    try {
      fillTarget();
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      await screen.findByText("Comparing");
      native.complete("job-1");
      await observer.refresh();
      await screen.findByText("orders");
      native.failRelease({ kind: "busy" });
      fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
      await screen.findByText(/Another comparison is using/);
      expect(screen.getByRole("button", { name: "Dismiss" })).toBeTruthy();
      native.failRelease(null);
      fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
      await waitFor(() =>
        expect(screen.queryByRole("button", { name: "Dismiss" })).toBeNull(),
      );
      expect(screen.getByText(/Compare two schemas/)).toBeTruthy();
      expect(screen.queryByText("orders")).toBeNull();
    } finally {
      unmount();
    }
  });

  it("reruns a capture-changed job with its own endpoints, not the edited draft", async () => {
    const { native, observer, unmount } = setup();
    try {
      fillTarget();
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      await screen.findByText("Comparing");
      native.fail("job-1");
      await observer.refresh();
      const rerun = await screen.findByRole("button", { name: "Run again" });
      fireEvent.change(screen.getByLabelText("Target schema"), {
        target: { value: "edited" },
      });
      expect(
        screen.getByText("Draft endpoints differ from this comparison."),
      ).toBeTruthy();
      fireEvent.click(rerun);
      await waitFor(() => expect(native.jobs).toHaveLength(2));
      expect(native.jobs[1]?.target.schema).toBe("public");
      expect(native.jobs[1]?.requestId).not.toBe(native.jobs[0]?.requestId);
      // The draft now shows the endpoints that were actually submitted.
      expect(screen.getByLabelText("Target schema")).toHaveProperty(
        "value",
        "public",
      );
    } finally {
      unmount();
    }
  });

  it("drops a result as soon as one of its connections is removed", async () => {
    const { native, observer, unmount } = setup();
    try {
      fillTarget();
      fireEvent.click(screen.getByRole("button", { name: "Compare" }));
      await screen.findByText("Comparing");
      native.complete("job-1");
      await observer.refresh();
      await screen.findByText("orders");
      useAppStore.setState({ connections: [staging] });
      await screen.findByText(/This comparison is unavailable/);
      expect(screen.queryByText("orders")).toBeNull();
      expect(screen.queryByText("Coverage & capture")).toBeNull();
    } finally {
      unmount();
    }
  });
});
