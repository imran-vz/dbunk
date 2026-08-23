// @vitest-environment jsdom
/**
 * Panel primitive invariants (DESIGN-SYSTEM §3.2/§3.6): collapsed
 * costs 0px with the sash kept as the edge-drag restore path; size
 * and the user-collapsed flag persist; pressure-collapse and
 * user-collapse stay distinct so pressure never clobbers the
 * persisted user intent.
 */
import { act, cleanup, render, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { Panel, usePanelState } from "./panel";

const OPTIONS = {
  storageKey: "test.panel",
  defaultSize: 260,
  min: 180,
  max: 500,
  snapThreshold: 90,
};

beforeEach(() => {
  window.localStorage.clear();
});

afterEach(() => {
  cleanup();
});

describe("usePanelState", () => {
  it("persists size and user-collapsed across instances", () => {
    const first = renderHook(() => usePanelState(OPTIONS));
    act(() => {
      first.result.current.setSize(320);
      first.result.current.collapse();
    });
    first.unmount();

    const second = renderHook(() => usePanelState(OPTIONS));
    expect(second.result.current.size).toBe(320);
    expect(second.result.current.userCollapsed).toBe(true);
  });

  it("keeps pressure-collapse distinct from the persisted user flag", () => {
    const { result } = renderHook(() => usePanelState(OPTIONS));
    act(() => {
      result.current.setPressureCollapsed(true);
    });
    expect(result.current.collapsed).toBe(true);
    expect(result.current.userCollapsed).toBe(false);
    expect(window.localStorage.getItem("test.panel.collapsed")).toBe("0");

    // Space returns → pressure lifts → panel restores itself.
    act(() => {
      result.current.setPressureCollapsed(false);
    });
    expect(result.current.collapsed).toBe(false);
  });

  it("expand clears both user and pressure collapse", () => {
    const { result } = renderHook(() => usePanelState(OPTIONS));
    act(() => {
      result.current.collapse();
      result.current.setPressureCollapsed(true);
    });
    act(() => {
      result.current.expand(300);
    });
    expect(result.current.collapsed).toBe(false);
    expect(result.current.size).toBe(300);
  });

  it("clamps sizes to min/max", () => {
    const { result } = renderHook(() => usePanelState(OPTIONS));
    act(() => result.current.setSize(50));
    expect(result.current.size).toBe(180);
    act(() => result.current.setSize(9000));
    expect(result.current.size).toBe(500);
  });
});

describe("Panel", () => {
  function Harness() {
    const state = usePanelState(OPTIONS);
    return (
      <Panel side="left" state={state} ariaLabel="Resize test panel">
        <div data-testid="panel-content">content</div>
      </Panel>
    );
  }

  it("renders content at its size when expanded", () => {
    const { getByTestId, getByRole } = render(<Harness />);
    expect(getByTestId("panel-content")).toBeTruthy();
    expect(getByRole("separator")).toBeTruthy();
  });

  it("collapsed costs 0px — only the sash restore strip remains", () => {
    window.localStorage.setItem("test.panel.collapsed", "1");
    const { queryByTestId, getByRole } = render(<Harness />);
    expect(queryByTestId("panel-content")).toBeNull();
    // The sash stays as the edge-drag restore affordance.
    expect(getByRole("separator")).toBeTruthy();
  });

  function RestoreHarness() {
    const state = usePanelState(OPTIONS);
    return (
      <Panel
        side="left"
        state={state}
        ariaLabel="Resize test panel"
        restoreLabel="Show test panel"
      >
        <div data-testid="panel-content">content</div>
      </Panel>
    );
  }

  it("renders a visible restore control when collapsed with restoreLabel", () => {
    window.localStorage.setItem("test.panel.collapsed", "1");
    const { getByRole, getByTestId, queryByTestId } = render(
      <RestoreHarness />,
    );
    expect(queryByTestId("panel-content")).toBeNull();

    // The persisted collapse would otherwise leave only the invisible
    // 1px sash strip as the way back — across app restarts.
    const restore = getByRole("button", { name: "Show test panel" });
    act(() => restore.click());
    expect(getByTestId("panel-content")).toBeTruthy();
    expect(window.localStorage.getItem("test.panel.collapsed")).toBe("0");
  });
});
