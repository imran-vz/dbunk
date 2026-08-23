// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { GlobalConsoleDock } from "@/components/workbench/dock";
import { useAppStore } from "@/lib/store";

afterEach(cleanup);

beforeEach(() => {
  window.localStorage.clear();
  useAppStore.setState({
    consoleEvents: [],
    consoleUnread: 0,
    dockOpen: false,
  });
});

const seedEvents = () => {
  const { appendConsoleEvent } = useAppStore.getState();
  appendConsoleEvent({
    severity: "info",
    source: "connection",
    message: "Connected to local",
  });
  appendConsoleEvent({
    severity: "warning",
    source: "notice",
    message: "WARNING: sequence exhausted soon",
  });
  appendConsoleEvent({
    severity: "error",
    source: "query",
    message: "Query failed on local",
    detail: "select broken",
  });
};

describe("GlobalConsoleDock", () => {
  it("renders nothing while hidden (never auto-opens)", () => {
    seedEvents();
    render(<GlobalConsoleDock />);
    expect(screen.queryByTestId("global-console-dock")).toBeNull();
    expect(useAppStore.getState().consoleUnread).toBe(3);
  });

  it("shows the event stream when open", () => {
    seedEvents();
    useAppStore.getState().setDockOpen(true);
    render(<GlobalConsoleDock />);

    expect(screen.getByTestId("global-console-dock")).toBeTruthy();
    expect(screen.getByText("Connected to local")).toBeTruthy();
    expect(screen.getByText("Query failed on local")).toBeTruthy();
    expect(screen.getByText("select broken")).toBeTruthy();
  });

  it("filters by severity", () => {
    seedEvents();
    useAppStore.getState().setDockOpen(true);
    render(<GlobalConsoleDock />);

    fireEvent.click(screen.getByRole("button", { name: "Errors" }));
    expect(screen.getByText("Query failed on local")).toBeTruthy();
    expect(screen.queryByText("Connected to local")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Warnings" }));
    expect(screen.getByText(/sequence exhausted/)).toBeTruthy();
    expect(screen.queryByText("Query failed on local")).toBeNull();
  });

  it("toggles with Ctrl+`", () => {
    render(<GlobalConsoleDock />);
    expect(screen.queryByTestId("global-console-dock")).toBeNull();

    fireEvent.keyDown(window, { key: "`", ctrlKey: true });
    expect(screen.getByTestId("global-console-dock")).toBeTruthy();

    fireEvent.keyDown(window, { key: "`", ctrlKey: true });
    expect(screen.queryByTestId("global-console-dock")).toBeNull();
  });

  it("closes from the header button", () => {
    useAppStore.getState().setDockOpen(true);
    render(<GlobalConsoleDock />);
    fireEvent.click(screen.getByRole("button", { name: "Hide console" }));
    expect(useAppStore.getState().dockOpen).toBe(false);
  });
});
