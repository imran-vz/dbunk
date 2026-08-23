import type { ReactNode } from "react";
import { useMemo } from "react";

import {
  ActivityRail,
  type RailItem,
} from "@/components/app-shell/activity-rail";
import { WorkbenchHeader } from "@/components/app-shell/workbench-header";
import { connectionStatusItem } from "@/components/connection-status";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import { GlobalConsoleDock } from "@/components/workbench/dock";
import { type Connection, useAppStore } from "@/lib/store";

export interface WorkbenchShellProps<T extends string> {
  activeConnection: Connection | undefined;
  isWindowFullscreen: boolean;
  onPointerDown: React.PointerEventHandler<HTMLElement>;
  onDoubleClick: React.MouseEventHandler<HTMLElement>;
  railItems: ReadonlyArray<RailItem<T>>;
  activeRail: T;
  onRailChange: (id: T) => void;
  onOpenSettings: () => void;
  statusItems: StatusBarItem[];
  leftPanel?: ReactNode;
  toolbar?: ReactNode;
  children: ReactNode;
}

export function WorkbenchShell<T extends string>({
  activeConnection,
  isWindowFullscreen,
  onPointerDown,
  onDoubleClick,
  railItems,
  activeRail,
  onRailChange,
  onOpenSettings,
  statusItems,
  leftPanel,
  toolbar,
  children,
}: WorkbenchShellProps<T>) {
  const consoleUnread = useAppStore((state) => state.consoleUnread);
  const dockOpen = useAppStore((state) => state.dockOpen);
  const toggleDock = useAppStore((state) => state.toggleDock);

  const defaultStatusItems = useMemo((): StatusBarItem[] => {
    if (!activeConnection) {
      return [{ id: "idle", value: "No connection" }];
    }
    return [
      connectionStatusItem(activeConnection),
      {
        id: "engine",
        value: `${activeConnection.engine}${activeConnection.latency ? ` · ${activeConnection.latency}` : ""}`,
        align: "right",
      },
    ];
  }, [activeConnection]);

  // Dock badge (§5.6): the status bar is the console's only ambient
  // affordance — new events while hidden increment the count here.
  const consoleBadge = useMemo(
    (): StatusBarItem => ({
      id: "console-badge",
      value:
        consoleUnread > 0 && !dockOpen
          ? `Console · ${consoleUnread}`
          : "Console",
      tone: consoleUnread > 0 && !dockOpen ? "info" : undefined,
      align: "right",
      onClick: toggleDock,
    }),
    [consoleUnread, dockOpen, toggleDock],
  );

  return (
    // min-w-0 is load-bearing: as a flex item of AppShell's row, this
    // root's automatic minimum size would otherwise be its content's
    // min-content width (a wide data grid), blowing out the layout and
    // silently disabling the grid's own horizontal scroll.
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex min-h-0 flex-1">
        <ActivityRail
          items={railItems}
          active={activeRail}
          onChange={onRailChange}
          onOpenSettings={onOpenSettings}
          isWindowFullscreen={isWindowFullscreen}
          onPointerDown={onPointerDown}
          onDoubleClick={onDoubleClick}
        />
        <div className="flex min-w-0 flex-1 flex-col">
          <WorkbenchHeader
            activeConnection={activeConnection}
            isWindowFullscreen={isWindowFullscreen}
            onPointerDown={onPointerDown}
            onDoubleClick={onDoubleClick}
          />
          <div className="flex min-h-0 flex-1">
            {leftPanel}
            <div className="flex min-w-0 flex-1 flex-col bg-surface-app">
              {toolbar}
              <div className="flex min-h-0 flex-1 flex-col">{children}</div>
            </div>
          </div>
        </div>
      </div>
      <GlobalConsoleDock />
      <StatusBar
        items={[
          ...(statusItems.length > 0 ? statusItems : defaultStatusItems),
          consoleBadge,
        ]}
        className="w-full"
      />
    </div>
  );
}
