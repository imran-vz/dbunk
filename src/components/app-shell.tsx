import { useEffect, useMemo, useState } from "react";
import { AppShellHeader } from "@/components/app-shell/app-shell-header";
import { useTauriWindowControls } from "@/components/app-shell/use-tauri-window-controls";
import { windowViewportZoomStyle } from "@/components/app-shell/use-window-viewport-zoom";
import {
  WindowDragFrame,
  WindowDragSurface,
} from "@/components/app-shell/window-drag";
import { CommandPalette } from "@/components/command-palette/command-palette";
import { ConnectionsView } from "@/components/connections-view";
import {
  CredentialOnboarding,
  CredentialUnlock,
} from "@/components/credential-onboarding";
import { SettingsView } from "@/components/settings-view";
import { Sidebar } from "@/components/sidebar";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";
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
    appSettings,
    appSettingsStatus,
    connections,
    isLeftSidebarOpen,
    setEditorTheme,
    toggleLeftSidebar,
    loadAppSettings,
    loadConnections,
    loadQueryHistory,
    loadSavedQueries,
    runHealthChecks,
    createNewQueryTab,
  } = useAppStore();
  const themeMode = appSettings?.theme ?? "system";

  const activeConnection = useMemo(
    () =>
      connections.find((connection) => connection.id === activeConnectionId) ??
      connections[0],
    [activeConnectionId, connections],
  );
  const isShellCompact =
    shellBodyWidth > 0 && shellBodyWidth < GLOBAL_SIDEBAR_COMPACT_BELOW;
  const density =
    shellBodyWidth > 0 && shellBodyWidth < 900 ? "compact" : "cozy";

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

  // Keep Monaco's theme in sync with whatever owns the `.dark` class on
  // <html>. The pre-paint script sets it on first load; the preferences
  // menu (via `setTheme`) toggles it at runtime. Observing the class
  // attribute makes the editor follow without each writer having to
  // know Monaco exists.
  useEffect(() => {
    if (typeof document === "undefined") return;
    const sync = () => {
      setEditorTheme(
        document.documentElement.classList.contains("dark") ? "vs-dark" : "vs",
      );
    };
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });
    return () => observer.disconnect();
  }, [setEditorTheme]);

  // When the user picks "System", follow OS changes live.
  useEffect(() => {
    if (themeMode !== "system") return;
    return subscribeSystem(() => applyTheme("system"));
  }, [themeMode]);

  useEffect(() => {
    void loadAppSettings();
  }, [loadAppSettings]);

  useEffect(() => {
    if (appSettings?.credentialState !== "ready") {
      return;
    }
    void loadConnections();
    void loadQueryHistory();
    void loadSavedQueries();
  }, [
    appSettings?.credentialState,
    loadConnections,
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
      <AppShellHeader
        isWindowFullscreen={isWindowFullscreen}
        isShellCompact={isShellCompact}
        leftSidebarOverlayOpen={leftSidebarOverlayOpen}
        isLeftSidebarOpen={isLeftSidebarOpen}
        newConnectionOpen={newConnectionOpen}
        setNewConnectionOpen={setNewConnectionOpen}
        activeConnection={activeConnection}
        onLeftSidebarToggle={handleLeftSidebarToggle}
        onCreateNewQueryTab={createNewQueryTab}
        onPointerDown={onTopBarPointerDown}
        onDoubleClick={onTopBarDoubleClick}
      />

      <div
        ref={shellBodyRef}
        className="relative flex min-h-0 flex-1 overflow-hidden"
      >
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
          ) : activeView === "connections" ? (
            <ConnectionsView />
          ) : (
            <SettingsView />
          )}
        </div>
      </div>
      <CommandPalette />
    </div>
  );
}
