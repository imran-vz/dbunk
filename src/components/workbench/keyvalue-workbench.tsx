import { useCallback, useMemo, useState } from "react";

import {
  KEYVALUE_RAIL_ITEMS,
  type KeyValueRailId,
} from "@/components/app-shell/activity-rail";
import { KeyValueWorkspace } from "@/components/keyvalue/KeyValueWorkspace";
import { isKeyValueConnection } from "@/components/workbench/workbench-policy";
import { WorkbenchShell } from "@/components/workbench/workbench-shell";
import { useAppStore } from "@/lib/store";

interface KeyValueWorkbenchProps {
  isWindowFullscreen: boolean;
  onPointerDown: React.PointerEventHandler<HTMLElement>;
  onDoubleClick: React.MouseEventHandler<HTMLElement>;
  settingsView?: React.ReactNode;
}

export function KeyValueWorkbench({
  isWindowFullscreen,
  onPointerDown,
  onDoubleClick,
  settingsView,
}: KeyValueWorkbenchProps) {
  const [rail, setRail] = useState<KeyValueRailId>("keys");
  const { activeConnectionId, connections, openSettings, setActiveView } =
    useAppStore();

  const activeConnection = useMemo(() => {
    const selected = connections.find(
      (connection) => connection.id === activeConnectionId,
    );
    if (isKeyValueConnection(selected)) {
      return selected;
    }
    return connections.find(isKeyValueConnection);
  }, [activeConnectionId, connections]);

  const handleOpenSettings = useCallback(() => {
    openSettings();
    setActiveView("settings");
  }, [openSettings, setActiveView]);

  const handleRailChange = useCallback(
    (next: KeyValueRailId) => {
      setRail(next);
      setActiveView("workspace");
    },
    [setActiveView],
  );

  const content = settingsView ? (
    <div className="min-h-0 flex-1 overflow-hidden">{settingsView}</div>
  ) : activeConnection ? (
    <KeyValueWorkspace
      activeConnection={activeConnection}
      variant="workbench"
      activeSection={rail}
    />
  ) : (
    <div className="flex min-h-0 flex-1 items-center justify-center p-6 text-sm text-text-muted">
      No Redis connection selected.
    </div>
  );

  return (
    <WorkbenchShell
      activeConnection={activeConnection}
      isWindowFullscreen={isWindowFullscreen}
      onPointerDown={onPointerDown}
      onDoubleClick={onDoubleClick}
      railItems={KEYVALUE_RAIL_ITEMS}
      activeRail={settingsView ? "keys" : rail}
      onRailChange={handleRailChange}
      onOpenSettings={handleOpenSettings}
      statusItems={[]}
    >
      {content}
    </WorkbenchShell>
  );
}
