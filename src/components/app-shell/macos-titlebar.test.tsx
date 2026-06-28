// @vitest-environment jsdom
import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import {
  ActivityRail,
  RELATIONAL_RAIL_ITEMS,
} from "@/components/app-shell/activity-rail";
import { MACOS_TRAFFIC_LIGHT_GUTTER_PX } from "@/components/app-shell/macos-titlebar";
import { WorkbenchHeader } from "@/components/app-shell/workbench-header";

vi.mock("@/lib/store", () => ({
  useAppStore: () => ({
    connections: [],
    setActiveConnectionId: vi.fn(),
    connectConnection: vi.fn(),
    disconnectConnection: vi.fn(),
  }),
}));

describe("macOS titlebar gutter", () => {
  it("reserves vertical space above the activity rail brand mark", () => {
    Object.defineProperty(window.navigator, "platform", {
      value: "MacIntel",
      configurable: true,
    });

    render(
      <ActivityRail
        items={RELATIONAL_RAIL_ITEMS}
        active="tables"
        onChange={() => {}}
        onOpenSettings={() => {}}
        isWindowFullscreen={false}
      />,
    );

    const gutter =
      screen.getByLabelText("Workbench sections").firstElementChild;
    expect(gutter).toBeTruthy();
    expect((gutter as HTMLElement).style.height).toBe(
      `${MACOS_TRAFFIC_LIGHT_GUTTER_PX}px`,
    );
  });

  it("uses pl-22 and h-12 on the workbench header", () => {
    Object.defineProperty(window.navigator, "platform", {
      value: "MacIntel",
      configurable: true,
    });

    render(
      <WorkbenchHeader
        activeConnection={undefined}
        isWindowFullscreen={false}
        onPointerDown={() => {}}
        onDoubleClick={() => {}}
      />,
    );

    const header = screen.getByTestId("workbench-header");
    expect(header.className).toContain("pl-22");
    expect(header.className).toContain("h-12");
  });
});
