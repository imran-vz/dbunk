// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { QueryResultsView } from "@/components/query-editor/results-view";
import type { QuerySessionState } from "@/lib/store";

const policyRefusal =
  "This connection is read-only. Edit the connection to unlock writes.";

const blockedSession: QuerySessionState = {
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
  policyRefusal,
};

describe("QueryResultsView", () => {
  it("renders a policy refusal once when it is also the transient error", () => {
    render(
      <QueryResultsView
        view="results"
        onViewChange={vi.fn()}
        preview={null}
        session={blockedSession}
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
});
