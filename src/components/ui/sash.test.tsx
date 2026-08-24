// @vitest-environment jsdom
/**
 * Pointer + keyboard verification of the sash spec (DESIGN-SYSTEM
 * §3.4) — P3 acceptance: drag resize with clamping, snap-close below
 * threshold (and reopen on the same drag), double-click auto-fit,
 * Alt+double-click collapse, keyboard resize / Enter-collapse /
 * Home/End.
 */
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Sash } from "./resizer-handle";

afterEach(() => {
  cleanup();
});

function renderSash(
  overrides: Partial<React.ComponentProps<typeof Sash>> = {},
) {
  const handlers = {
    onResize: vi.fn(),
    onCollapse: vi.fn(),
    onExpand: vi.fn(),
    onAutoFit: vi.fn(),
  };
  const utils = render(
    <Sash
      value={260}
      min={180}
      max={500}
      snapThreshold={90}
      ariaLabel="Resize test pane"
      {...handlers}
      {...overrides}
    />,
  );
  const sash = utils.getByRole("separator");
  return { sash, ...handlers };
}

const drag = (
  sash: HTMLElement,
  from: number,
  to: number,
  axis: "clientX" | "clientY" = "clientX",
) => {
  fireEvent.pointerDown(sash, { [axis]: from, pointerId: 1 });
  fireEvent.pointerMove(sash, { [axis]: to, pointerId: 1 });
  fireEvent.pointerUp(sash, { [axis]: to, pointerId: 1 });
};

describe("Sash", () => {
  it("resizes live during drag, clamped to min/max", () => {
    const { sash, onResize } = renderSash();
    drag(sash, 300, 340);
    expect(onResize).toHaveBeenCalledWith(300);

    onResize.mockClear();
    drag(sash, 300, 900);
    expect(onResize).toHaveBeenCalledWith(500);

    onResize.mockClear();
    drag(sash, 300, 250);
    expect(onResize).toHaveBeenCalledWith(210);
  });

  it("snaps closed when dragged below the threshold, and reopens past it", () => {
    const { sash, onCollapse, onExpand } = renderSash();
    fireEvent.pointerDown(sash, { clientX: 300, pointerId: 1 });
    // 260 - 200 = 60 < 90 → snap closed.
    fireEvent.pointerMove(sash, { clientX: 100, pointerId: 1 });
    expect(onCollapse).toHaveBeenCalledTimes(1);
    // Same drag back out past the threshold → reopen.
    fireEvent.pointerMove(sash, { clientX: 250, pointerId: 1 });
    expect(onExpand).toHaveBeenCalledWith(210);
    fireEvent.pointerUp(sash, { clientX: 250, pointerId: 1 });
  });

  it("double-click auto-fits; Alt+double-click collapses", () => {
    const { sash, onAutoFit, onCollapse } = renderSash();
    fireEvent.doubleClick(sash);
    expect(onAutoFit).toHaveBeenCalledTimes(1);
    expect(onCollapse).not.toHaveBeenCalled();

    fireEvent.doubleClick(sash, { altKey: true });
    expect(onCollapse).toHaveBeenCalledTimes(1);
  });

  it("arrow keys resize in 8px steps (32px with Shift)", () => {
    const { sash, onResize } = renderSash();
    fireEvent.keyDown(sash, { key: "ArrowRight" });
    expect(onResize).toHaveBeenCalledWith(268);
    fireEvent.keyDown(sash, { key: "ArrowLeft", shiftKey: true });
    expect(onResize).toHaveBeenCalledWith(228);
  });

  it("Enter toggles collapse; Home/End jump to min/max", () => {
    const { sash, onCollapse, onResize } = renderSash();
    fireEvent.keyDown(sash, { key: "Enter" });
    expect(onCollapse).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(sash, { key: "Home" });
    expect(onResize).toHaveBeenCalledWith(180);
    fireEvent.keyDown(sash, { key: "End" });
    expect(onResize).toHaveBeenCalledWith(500);
  });

  it("Enter on a collapsed sash expands", () => {
    const { sash, onExpand } = renderSash({ collapsed: true });
    fireEvent.keyDown(sash, { key: "Enter" });
    expect(onExpand).toHaveBeenCalledWith(260);
  });

  it("edge-drag on a collapsed sash reopens once past the threshold", () => {
    const { sash, onExpand } = renderSash({ collapsed: true });
    fireEvent.pointerDown(sash, { clientX: 0, pointerId: 1 });
    fireEvent.pointerMove(sash, { clientX: 40, pointerId: 1 });
    expect(onExpand).not.toHaveBeenCalled();
    fireEvent.pointerMove(sash, { clientX: 200, pointerId: 1 });
    expect(onExpand).toHaveBeenCalledWith(200);
    fireEvent.pointerUp(sash, { clientX: 200, pointerId: 1 });
  });

  it("exposes ARIA separator semantics with range values", () => {
    const { sash } = renderSash();
    expect(sash.getAttribute("aria-orientation")).toBe("vertical");
    expect(sash.getAttribute("aria-valuenow")).toBe("260");
    expect(sash.getAttribute("aria-valuemin")).toBe("180");
    expect(sash.getAttribute("aria-valuemax")).toBe("500");
  });
});
