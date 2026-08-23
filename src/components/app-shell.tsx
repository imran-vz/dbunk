import { useEffect, useRef, useState } from "react";

import { AppShellHeader } from "@/components/app-shell/app-shell-header";
import { useTauriWindowControls } from "@/components/app-shell/use-tauri-window-controls";
import { windowViewportZoomStyle } from "@/components/app-shell/use-window-viewport-zoom";
import {
  WindowDragFrame,
  WindowDragSurface,
} from "@/components/app-shell/window-drag";
import { CommandPalette } from "@/components/command-palette/command-palette";
import {
  CredentialOnboarding,
  CredentialUnlock,
} from "@/components/credential-onboarding";
import { SafetyConfirmDialog } from "@/components/safety-confirm-dialog";
import { SettingsView } from "@/components/settings-view";
import { Sidebar } from "@/components/sidebar";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";
import { KeyValueWorkbench } from "@/components/workbench/keyvalue-workbench";
import { RelationalWorkbench } from "@/components/workbench/relational-workbench";
import {
  isKeyValueConnection,
  usesWorkbenchShell,
} from "@/components/workbench/workbench-policy";
import { WorkspaceView } from "@/components/workspace-view";
import { useAppStore } from "@/lib/store";
import { applyTheme, subscribeSystem } from "@/lib/theme";
import {
  useContainerWidth,
  useResizableWidth,
} from "@/lib/use-resizable-width";
import { cn } from "@/lib/utils";

const GLOBAL_SIDEBAR_WIDTH = 288;
const GLOBAL_SIDEBAR_MIN_WIDTH = 220;
const GLOBAL_SIDEBAR_MAX_WIDTH = 420;
const GLOBAL_SIDEBAR_COMPACT_BELOW = 980;
const PROTECTED_WORKSPACE_WIDTH = 560;

/**
 * Foreground health-check tick: runs once after the connection list loads,
 * then every 30 s while the tab is visible. Pauses when the user switches
 * tabs to avoid burning credentials checks against a backgrounded app.
 */
function useForegroundHealthCheck(
  credentialState: string | undefined,
  // oxlint-disable-next-line anti-slop/no-unknown-returns -- The value is handled at a typed library or domain boundary here.
  runHealthChecks: () => Promise<unknown>,
) {
  useEffect(() => {
    if (credentialState !== "ready") {
      return;
    }
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = () => {
      if (timer) return;
      // Run once immediately so the UI doesn't sit on stale cached statuses
      // for 30 s after launch.
      void runHealthChecks();
      timer = setInterval(() => {
        void runHealthChecks();
      }, 30_000);
    };
    const stop = () => {
      if (timer) {
        clearInterval(timer);
        timer = null;
      }
    };
    const onVisibility = () => {
      if (document.visibilityState === "visible") start();
      else stop();
    };
    if (document.visibilityState === "visible") {
      start();
    }
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [credentialState, runHealthChecks]);
}

export function AppShell() {
  const [isClient, setIsClient] = useState(false);
  const [newConnectionOpen, setNewConnectionOpen] = useState(false);
  const [leftSidebarOverlayOpen, setLeftSidebarOverlayOpen] = useState(false);
  const [shellBodyRef, shellBodyWidth] = useContainerWidth<HTMLDivElement>();
  const { width: globalSidebarWidth, setWidth: setGlobalSidebarWidth } =
    useResizableWidth({
      storageKey: "dbunk.sidebar.globalWidth",
      defaultWidth: GLOBAL_SIDEBAR_WIDTH,
      min: GLOBAL_SIDEBAR_MIN_WIDTH,
      max: GLOBAL_SIDEBAR_MAX_WIDTH,
    });

  const {
    isWindowFullscreen,
    windowViewportZoom,
    onTopBarPointerDown,
    onTopBarDoubleClick,
  } = useTauriWindowControls();

  const {
    activeView,
    activeConnectionId,
    connections,
    appSettings,
    appSettingsStatus,
    isLeftSidebarOpen,
    setEditorTheme,
    toggleLeftSidebar,
    loadAppSettings,
    loadBastionServers,
    loadConnections,
    loadManagedServers,
    loadQueryHistory,
    loadSavedQueries,
    runHealthChecks,
    createNewQueryTab,
  } = useAppStore();
  const themeMode = appSettings?.theme ?? "system";
  const themePreset = appSettings?.themePreset ?? "default";

  const isShellCompact =
    shellBodyWidth > 0 && shellBodyWidth < GLOBAL_SIDEBAR_COMPACT_BELOW;
  const density =
    shellBodyWidth > 0 && shellBodyWidth < 900 ? "compact" : "cozy";

  const activeConnection = connections.find(
    (connection) => connection.id === activeConnectionId,
  );
  const usesWorkbench = usesWorkbenchShell(appSettings?.credentialState);

  const handleLeftSidebarToggle = () => {
    if (isShellCompact) {
      setLeftSidebarOverlayOpen((open) => !open);
      return;
    }
    toggleLeftSidebar();
  };

  useEffect(() => {
    setIsClient(true);
  }, []);

  // Mirror `<html>.dark` onto Monaco's theme. Bail when the resolved
  // value hasn't changed so unrelated class mutations don't trigger a
  // store write + re-render storm.
  useEffect(() => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof document === "undefined") return;
    let last: "vs" | "vs-dark" | null = null;
    const sync = () => {
      const next = document.documentElement.classList.contains("dark")
        ? "vs-dark"
        : "vs";
      if (next === last) return;
      last = next;
      setEditorTheme(next);
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => observer.disconnect();
  }, [setEditorTheme]);

  // Follow OS theme when mode is "system". Preset is read through a
  // ref so changing presets doesn't tear down and re-add the matchMedia
  // listener — only mode actually matters for the subscription.
  const themePresetRef = useRef(themePreset);
  themePresetRef.current = themePreset;
  useEffect(() => {
    if (themeMode !== "system") return;
    return subscribeSystem(() => applyTheme("system", themePresetRef.current));
  }, [themeMode]);

  useEffect(() => {
    void loadAppSettings();
  }, [loadAppSettings]);

  useEffect(() => {
    if (appSettings?.credentialState !== "ready") {
      return;
    }
    void loadConnections();
    void loadBastionServers();
    void loadManagedServers();
    void loadQueryHistory();
    void loadSavedQueries();
  }, [
    appSettings?.credentialState,
    loadBastionServers,
    loadConnections,
    loadManagedServers,
    loadQueryHistory,
    loadSavedQueries,
  ]);

  useForegroundHealthCheck(appSettings?.credentialState, runHealthChecks);

  if (
    appSettingsStatus.state === "loading" ||
    appSettingsStatus.state === "idle"
  ) {
    return (
      <div className="fixed inset-0 flex items-center justify-center bg-surface-app text-xs text-text-muted">
        <WindowDragSurface
          onDoubleClick={onTopBarDoubleClick}
          onPointerDown={onTopBarPointerDown}
        />
        Loading settings…
      </div>
    );
  }

  if (appSettingsStatus.state === "error") {
    return (
      <div className="fixed inset-0 flex items-center justify-center bg-surface-app p-6 text-foreground">
        <WindowDragSurface
          onDoubleClick={onTopBarDoubleClick}
          onPointerDown={onTopBarPointerDown}
        />
        <div className="max-w-md rounded-lg border border-danger/30 bg-danger/10 p-5 text-sm text-danger">
          {appSettingsStatus.error}
        </div>
      </div>
    );
  }

  if (appSettings?.credentialState === "needs-onboarding") {
    return (
      <WindowDragFrame
        onDoubleClick={onTopBarDoubleClick}
        onPointerDown={onTopBarPointerDown}
      >
        <CredentialOnboarding />
      </WindowDragFrame>
    );
  }

  if (appSettings?.credentialState === "needs-unlock") {
    return (
      <WindowDragFrame
        onDoubleClick={onTopBarDoubleClick}
        onPointerDown={onTopBarPointerDown}
      >
        <CredentialUnlock />
      </WindowDragFrame>
    );
  }

  return (
    <div
      data-density={density}
      data-testid="app-shell"
      data-window-viewport-zoom={
        windowViewportZoom
          ? windowViewportZoom.active
            ? "active"
            : "idle"
          : undefined
      }
      style={windowViewportZoomStyle(windowViewportZoom)}
      className={cn(
        "fixed flex min-h-0 min-w-0 flex-col overflow-hidden bg-surface-app text-foreground",
        windowViewportZoom ? "top-0 left-0" : "inset-0",
      )}
    >
      {!usesWorkbench ? (
        <AppShellHeader
          isWindowFullscreen={isWindowFullscreen}
          isShellCompact={isShellCompact}
          leftSidebarOverlayOpen={leftSidebarOverlayOpen}
          isLeftSidebarOpen={isLeftSidebarOpen}
          newConnectionOpen={newConnectionOpen}
          setNewConnectionOpen={setNewConnectionOpen}
          onLeftSidebarToggle={handleLeftSidebarToggle}
          onCreateNewQueryTab={createNewQueryTab}
          onPointerDown={onTopBarPointerDown}
          onDoubleClick={onTopBarDoubleClick}
        />
      ) : null}

      <div
        ref={shellBodyRef}
        className="relative flex min-h-0 flex-1 overflow-hidden"
      >
        {usesWorkbench ? (
          activeView === "settings" ||
          !isKeyValueConnection(activeConnection) ? (
            <RelationalWorkbench
              isClient={isClient}
              isWindowFullscreen={isWindowFullscreen}
              onPointerDown={onTopBarPointerDown}
              onDoubleClick={onTopBarDoubleClick}
              settingsView={activeView === "settings" ? <SettingsView /> : null}
            />
          ) : (
            <KeyValueWorkbench
              isWindowFullscreen={isWindowFullscreen}
              onPointerDown={onTopBarPointerDown}
              onDoubleClick={onTopBarDoubleClick}
            />
          )
        ) : (
          <>
            <ResponsiveEdgePanel
              side="left"
              storageKey="dbunk.sidebar.global"
              title="Sidebar"
              width={globalSidebarWidth}
              containerWidth={shellBodyWidth}
              compactBelow={GLOBAL_SIDEBAR_COMPACT_BELOW}
              protectedWorkspaceWidth={PROTECTED_WORKSPACE_WIDTH}
              wideVisible={isLeftSidebarOpen}
              open={leftSidebarOverlayOpen}
              onOpenChange={setLeftSidebarOverlayOpen}
              resizer={{
                onResize: setGlobalSidebarWidth,
                min: GLOBAL_SIDEBAR_MIN_WIDTH,
                max: GLOBAL_SIDEBAR_MAX_WIDTH,
                ariaLabel: "Resize connections and tables sidebar",
              }}
              className="bg-surface-sidebar"
            >
              <Sidebar className="border-r-0" />
            </ResponsiveEdgePanel>
            <div className="flex min-w-0 flex-1 flex-col bg-surface-app">
              {activeView === "workspace" ? (
                <WorkspaceView isClient={isClient} />
              ) : (
                <SettingsView />
              )}
            </div>
          </>
        )}
      </div>
      <CommandPalette />
      <SafetyConfirmDialog />
    </div>
  );
}
