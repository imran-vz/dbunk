import {
  IconCommand,
  IconLayoutSidebarLeftCollapse,
  IconPlus,
  IconSearch,
  IconTerminal2,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";
import { ConnectionsView } from "@/components/connections-view";
import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { Sidebar } from "@/components/sidebar";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { WorkspaceView } from "@/components/workspace-view";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

export function AppShell() {
  const [isClient, setIsClient] = useState(false);
  const [newConnectionOpen, setNewConnectionOpen] = useState(false);

  const {
    activeView,
    activeConnectionId,
    connections,
    isLeftSidebarOpen,
    setEditorTheme,
    toggleLeftSidebar,
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
    void loadConnections();
    void loadQueryHistory();
    void loadSavedQueries();
  }, [loadConnections, loadQueryHistory, loadSavedQueries]);

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
    if (document.visibilityState === "visible") {
      start();
    }
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [runHealthChecks]);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-surface-app text-foreground">
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
        className="flex h-14 shrink-0 items-center gap-3 border-b border-border-subtle bg-surface-window pl-[78px] pr-3 select-none"
      >
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label="Toggle sidebar"
          onClick={toggleLeftSidebar}
          className="text-text-muted"
        >
          <IconLayoutSidebarLeftCollapse className="size-4" />
        </Button>
        <div
          data-tauri-drag-region
          className="flex items-center gap-2 text-sm font-semibold tracking-tight"
        >
          <span className="flex size-6 items-center justify-center rounded-md border border-accent-green/25 bg-accent-green/10 text-[0.68rem] font-semibold text-accent-green">
            db
          </span>
          <span className="text-foreground">dbunk</span>
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
              <Button type="button" size="sm" variant="outline">
                <IconPlus className="size-3.5" />
                New Connection
              </Button>
            }
          />
          <Button
            type="button"
            size="sm"
            onClick={createNewQueryTab}
            className="gap-2"
          >
            <IconTerminal2 className="size-3.5" />
            Run Query
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

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <div
          className={cn(
            "shrink-0 overflow-hidden transition-all duration-300 ease-in-out",
            isLeftSidebarOpen ? "w-72" : "w-0",
          )}
        >
          <Sidebar />
        </div>
        <div className="flex min-w-0 flex-1 flex-col bg-surface-app">
          {activeView === "workspace" ? (
            <WorkspaceView isClient={isClient} />
          ) : (
            <ConnectionsView />
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
