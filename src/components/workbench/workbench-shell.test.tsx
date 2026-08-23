/* oxlint-disable anti-slop/no-unknown-parameters -- Test fixtures use controlled values. */
// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { WorkbenchShell } from "@/components/workbench/workbench-shell";

afterEach(cleanup);

describe("WorkbenchShell", () => {
  it("keeps min-w-0 on its root so the grid can scroll horizontally", () => {
    // Regression: the shell root is a flex item of AppShell's row
    // container. Without min-w-0 its automatic minimum size is the
    // content's min-content width — a wide data grid then stretches the
    // whole shell past the viewport, AppShell's overflow-hidden clips
    // it, and the grid's own scroll container never overflows (no
    // horizontal scrollbar, columns cut off at the window edge).
    const { container } = render(
      <WorkbenchShell
        activeConnection={undefined}
        isWindowFullscreen={false}
        onPointerDown={() => {}}
        onDoubleClick={() => {}}
        railItems={[]}
        activeRail="tables"
        onRailChange={() => {}}
        onOpenSettings={() => {}}
        statusItems={[]}
      >
        <div>content</div>
      </WorkbenchShell>,
    );

    const root = container.firstElementChild;
    expect(root?.className).toContain("min-w-0");
    expect(root?.className).toContain("flex-1");
  });
});
