import {
  IconCommand,
  IconLayoutSidebarLeftCollapse,
  IconPlus,
  IconSearch,
  IconTerminal2,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";
import { ConnectionsView } from "@/components/connections-view";
import {
  CredentialOnboarding,
  CredentialUnlock,
} from "@/components/credential-onboarding";
import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { SettingsView } from "@/components/settings-view";
import { Sidebar } from "@/components/sidebar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";
import { WorkspaceView } from "@/components/workspace-view";
import { useAppStore } from "@/lib/store";
import { useContainerWidth } from "@/lib/use-resizable-width";
import logo from "../assets/logo.png";

const GLOBAL_SIDEBAR_WIDTH = 288;
const GLOBAL_SIDEBAR_COMPACT_BELOW = 980;
const PROTECTED_WORKSPACE_WIDTH = 560;

export function AppShell() {
  const [isClient, setIsClient] = useState(false);
  const [newConnectionOpen, setNewConnectionOpen] = useState(false);
  const [leftSidebarOverlayOpen, setLeftSidebarOverlayOpen] = useState(false);
  const [shellBodyRef, shellBodyWidth] = useContainerWidth<HTMLDivElement>();

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
    if (typeof document === "undefined") {
      return;
    }
    document.documentElement.classList.add("dark");
    setEditorTheme(
      document.documentElement.classList.contains("dark") ? "vs-dark" : "vs",
    );
  }, [setEditorTheme]);

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

  // Foreground health-check tick: runs once after the connection list loads,
  // then every 30 s while the tab is visible. Pauses when the user switches
  // tabs to avoid burning credentials checks against a backgrounded app.
  useEffect(() => {
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
      if (document.visibilityState === "visible") {
        start();
      } else {
        stop();
      }
    };
    if (appSettings?.credentialState !== "ready") {
      return;
    }
    if (document.visibilityState === "visible") {
      start();
    }
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [appSettings?.credentialState, runHealthChecks]);

  if (
    appSettingsStatus.state === "loading" ||
    appSettingsStatus.state === "idle"
  ) {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-surface-app text-xs text-text-muted">
        Loading settings…
      </div>
    );
  }

  if (appSettingsStatus.state === "error") {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-surface-app p-6 text-foreground">
        <div className="max-w-md rounded-lg border border-danger/30 bg-danger/10 p-5 text-sm text-danger">
          {appSettingsStatus.error}
        </div>
      </div>
    );
  }

  if (appSettings?.credentialState === "needs-onboarding") {
    return <CredentialOnboarding />;
  }

  if (appSettings?.credentialState === "needs-unlock") {
    return <CredentialUnlock />;
  }

  return (
    <div
      data-density={density}
      className="flex h-screen w-screen flex-col overflow-hidden bg-surface-app text-foreground"
    >
      <header
        data-slot="top-bar"
        data-tauri-drag-region
        // The macOS window uses `titleBarStyle: Overlay` (see
        // src-tauri/tauri.conf.json), so the OS draws the red/yellow/green
        // controls on top of our content. The traffic lights are positioned
        // at x=18, y=21 there, centered in this 56 px header. Reserve 78 px
        // on the left so the sidebar toggle and `dbunk` mark don't sit under
        // them. Other
        // platforms render the OS chrome above our header so the spacer is
        // dead weight there — addressed in designs/FOLLOWUPS.md (cross-
        // platform window chrome) when we ship Windows/Linux builds.
        //
        // Drag behaviour: each non-interactive wrapper carries its own
        // `data-tauri-drag-region` so empty space around buttons/inputs is
        // unambiguously a drag handle on every platform. Buttons + inputs
        // get `no-drag` automatically as native interactive elements.
        className="flex h-14 shrink-0 items-center gap-3 border-b border-border-subtle bg-surface-window pl-22 pr-3 select-none"
      >
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label={
            isShellCompact
              ? leftSidebarOverlayOpen
                ? "Hide sidebar"
                : "Show sidebar"
              : isLeftSidebarOpen
                ? "Hide sidebar"
                : "Show sidebar"
          }
          title={
            isShellCompact
              ? leftSidebarOverlayOpen
                ? "Hide sidebar"
                : "Show sidebar"
              : isLeftSidebarOpen
                ? "Hide sidebar"
                : "Show sidebar"
          }
          onClick={handleLeftSidebarToggle}
          className="text-text-muted"
        >
          <IconLayoutSidebarLeftCollapse className="size-4" />
        </Button>
        <div
          data-tauri-drag-region
          className="flex items-center gap-2 text-sm font-semibold tracking-tight"
        >
          <img src={logo} alt="dbunk" className="size-6" />
        </div>

        <div
          data-tauri-drag-region
          className="relative ml-2 hidden min-w-44 max-w-md flex-1 md:block"
        >
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
          <Input
            placeholder="Search tables"
            aria-label="Search tables"
            className="h-9 rounded-md pl-8 pr-13 text-xs"
          />
          <span className="pointer-events-none absolute right-2 top-1/2 flex h-5 -translate-y-1/2 items-center gap-0.5 rounded border border-border-subtle bg-surface-app px-1.5 text-[0.62rem] text-text-muted">
            <IconCommand className="size-2.5" />K
          </span>
        </div>

        {/* Explicit flexible drag spacer between the search and the right
            actions. Always rendered so the right group stays anchored to the
            window edge even on screens too small to show the search. */}
        <div
          data-tauri-drag-region
          aria-hidden="true"
          className="h-full flex-1"
        />

        <div data-tauri-drag-region className="flex items-center gap-2">
          <NewConnectionDialog
            open={newConnectionOpen}
            onOpenChange={setNewConnectionOpen}
            trigger={
              <Button
                type="button"
                size="sm"
                variant="outline"
                aria-label="New Connection"
                title="New Connection"
              >
                <IconPlus className="size-3.5" />
                <span className="dbunk-optional-label">New Connection</span>
              </Button>
            }
          />
          <Button
            type="button"
            size="sm"
            onClick={createNewQueryTab}
            aria-label="Run Query"
            title="Run Query"
            className="gap-2"
          >
            <IconTerminal2 className="size-3.5" />
            <span className="dbunk-optional-label">Run Query</span>
          </Button>
          <Button
            type="button"
            size="icon-sm"
            variant="secondary"
            aria-label="Account menu"
            className="size-8 rounded-full text-[0.65rem] font-semibold"
          >
            {initialsFor(activeConnection?.user) ?? "AD"}
          </Button>
        </div>
      </header>

      <div
        ref={shellBodyRef}
        className="relative flex min-h-0 flex-1 overflow-hidden"
      >
        <ResponsiveEdgePanel
          side="left"
          storageKey="dbunk.sidebar.global"
          title="Sidebar"
          width={GLOBAL_SIDEBAR_WIDTH}
          containerWidth={shellBodyWidth}
          compactBelow={GLOBAL_SIDEBAR_COMPACT_BELOW}
          protectedWorkspaceWidth={PROTECTED_WORKSPACE_WIDTH}
          wideVisible={isLeftSidebarOpen}
          open={leftSidebarOverlayOpen}
          onOpenChange={setLeftSidebarOverlayOpen}
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
    </div>
  );
}

function initialsFor(user: string | undefined): string | null {
  if (!user) return null;
  const stem = user.split("@")[0] ?? user;
  const parts = stem.split(/[._-]/).filter(Boolean);
  if (parts.length === 0) return null;
  if (parts.length === 1) {
    return parts[0].slice(0, 2).toUpperCase();
  }
  return (parts[0][0] + parts[1][0]).toUpperCase();
}
