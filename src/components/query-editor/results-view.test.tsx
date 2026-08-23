// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  type PinnedResult,
  QueryResultsView,
} from "@/components/query-editor/results-view";
import type { QueryExecution, QuerySessionState } from "@/lib/store";

afterEach(cleanup);

const policyRefusal =
  "This connection is read-only. Edit the connection to unlock writes.";

const baseSession: QuerySessionState = {
  id: "session-1",
  tabId: "tab-1",
  connectionId: "connection-1",
  generation: 1,
  transaction: {
    mode: "autocommit",
    status: "idle",
    manualIsolation: "readCommitted",
  },
  execution: null,
  lastViewedAt: 0,
  budgetOwners: [],
  state: "open",
  policyRefusal: null,
};

const execution: QueryExecution = {
  id: "exec-1",
  status: "completed",
  startedAt: "2026-08-23T00:00:00Z",
  completedAt: "2026-08-23T00:00:01Z",
  runtimeMs: 128,
  resultSets: [
    {
      index: 0,
      columns: ["id", "name"],
      rowChunks: [
        [
          ["1", "ada"],
          ["2", "linus"],
        ],
      ],
      rowCount: 2,
      partial: false,
      completed: true,
    },
    {
      index: 1,
      columns: ["total"],
      rowChunks: [[["2"]]],
      rowCount: 1,
      partial: false,
      completed: true,
    },
  ],
  notices: [],
  error: null,
  omittedRows: 0,
  omittedResultSets: 0,
  omittedNotices: 0,
  omittedMetadataBytes: 0,
  truncationReasons: [],
  retainedBytes: 0,
  tombstone: null,
};

const renderView = (
  overrides: Partial<React.ComponentProps<typeof QueryResultsView>> = {},
) =>
  render(
    <QueryResultsView
      view="results"
      onViewChange={vi.fn()}
      preview={null}
      session={{ ...baseSession, execution }}
      explainPlan={null}
      currentEdits={{}}
      exportFilenameBase="query-results"
      isRunning={false}
      errorMessage={null}
      onCellEdit={vi.fn()}
      resultIndex={0}
      onResultIndexChange={vi.fn()}
      {...overrides}
    />,
  );

describe("QueryResultsView", () => {
  it("renders a policy refusal once when it is also the transient error", () => {
    render(
      <QueryResultsView
        view="results"
        onViewChange={vi.fn()}
        preview={null}
        session={{ ...baseSession, policyRefusal }}
        explainPlan={null}
        currentEdits={{}}
        exportFilenameBase="query-results"
        isRunning={false}
        errorMessage={policyRefusal}
        onCellEdit={vi.fn()}
        resultIndex={0}
        onResultIndexChange={vi.fn()}
      />,
    );

    expect(screen.getAllByText(policyRefusal)).toHaveLength(1);
    expect(screen.getAllByRole("alert")).toHaveLength(1);
  });

  it("shows result-set chips for a multi-result execution and switches on click", () => {
    const onResultIndexChange = vi.fn();
    renderView({ onResultIndexChange });

    const chip = screen.getByRole("button", { name: /2 · 1 row/ });
    fireEvent.click(chip);
    expect(onResultIndexChange).toHaveBeenCalledWith(1);
  });

  it("switches Results/Explain from the pane's own segmented control", () => {
    const onViewChange = vi.fn();
    renderView({ onViewChange });
    fireEvent.click(screen.getByRole("button", { name: "Explain" }));
    expect(onViewChange).toHaveBeenCalledWith("explain");
  });

  it("collapses to the status strip from the pane toolbar", () => {
    const onCollapse = vi.fn();
    renderView({ onCollapse });
    fireEvent.click(
      screen.getByRole("button", { name: "Collapse results pane" }),
    );
    expect(onCollapse).toHaveBeenCalled();
  });

  it("pins the current result as a snapshot and unpins it", () => {
    const onPinResult = vi.fn();
    renderView({ onPinResult, pinnedResults: [] });

    fireEvent.click(screen.getByRole("button", { name: "Pin this result" }));
    expect(onPinResult).toHaveBeenCalledTimes(1);
    // SAFETY: the assertion above proves onPinResult was called; its only
    // call site passes a PinnedResult.
    const pinned = onPinResult.mock.calls[0][0] as PinnedResult;
    expect(pinned.columns).toEqual(["id", "name"]);
    expect(pinned.rows).toEqual([
      ["1", "ada"],
      ["2", "linus"],
    ]);

    const onUnpinResult = vi.fn();
    cleanup();
    renderView({
      onPinResult,
      onUnpinResult,
      pinnedResults: [pinned],
      activePinnedId: pinned.id,
    });
    fireEvent.click(
      screen.getByRole("button", { name: `Unpin ${pinned.label}` }),
    );
    expect(onUnpinResult).toHaveBeenCalledWith(pinned.id);
  });

  it("disables Export/Copy when there is no exportable result", () => {
    renderView({ session: { ...baseSession, execution: null } });
    expect(
      screen
        .getByRole("button", { name: "Export results" })
        .hasAttribute("disabled"),
    ).toBe(true);
    expect(
      screen
        .getByRole("button", { name: "Copy results" })
        .hasAttribute("disabled"),
    ).toBe(true);
  });

  it("enables Export/Copy for a retained result", () => {
    renderView();
    expect(
      screen
        .getByRole("button", { name: "Export results" })
        .hasAttribute("disabled"),
    ).toBe(false);
    expect(
      screen
        .getByRole("button", { name: "Copy results" })
        .hasAttribute("disabled"),
    ).toBe(false);
  });
});
