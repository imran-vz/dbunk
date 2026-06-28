import type { ReactNode } from "react";
import { useMemo } from "react";
import {
  ActivityRail,
  type RailItem,
} from "@/components/app-shell/activity-rail";
import { WorkbenchHeader } from "@/components/app-shell/workbench-header";
import { connectionStatusItem } from "@/components/connection-status";
import { StatusBar, type StatusBarItem } from "@/components/status-bar";
import type { Connection } from "@/lib/store";

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

  return (
    <div className="flex min-h-0 flex-1 flex-col">
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
      <StatusBar
        items={statusItems.length > 0 ? statusItems : defaultStatusItems}
        className="w-full"
      />
    </div>
  );
}
