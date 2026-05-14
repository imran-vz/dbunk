// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EmptyState, ErrorState, LoadingState } from "./state-panel";

afterEach(() => {
  cleanup();
});

describe("LoadingState", () => {
  it("renders the supplied label", () => {
    render(<LoadingState label="Loading sessions…" />);
    expect(screen.getByText("Loading sessions…")).toBeTruthy();
  });

  it("exposes a polite live region for screen readers", () => {
    render(<LoadingState />);
    expect(screen.getByRole("status")).toBeTruthy();
  });
});

describe("ErrorState", () => {
  it("renders the message and the retry control when provided", () => {
    const onRetry = vi.fn();
    render(<ErrorState message="boom" onRetry={onRetry} />);
    expect(screen.getByText("boom")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(onRetry).toHaveBeenCalled();
  });

  it("omits retry when no callback is supplied", () => {
    render(<ErrorState message="boom" />);
    expect(screen.queryByRole("button", { name: /retry/i })).toBeNull();
  });
});

describe("EmptyState", () => {
  it("renders title and description", () => {
    render(
      <EmptyState
        title="No sessions"
        description="Run a query to see entries here."
      />,
    );
    expect(screen.getByText("No sessions")).toBeTruthy();
    expect(screen.getByText(/Run a query/)).toBeTruthy();
  });
});
