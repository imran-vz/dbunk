import {
  IconCommand,
  IconLayoutSidebarLeftCollapse,
  IconPlus,
  IconSearch,
  IconTerminal2,
} from "@tabler/icons-react";
import {
  type CSSProperties,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { flushSync } from "react-dom";
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
import {
  tauriOnWindowFullscreenChange,
  tauriPrepareWindowZoomTransition,
  tauriRestoreWindowTrafficLightPosition,
  tauriStartDragging,
  tauriToggleWindowZoom,
  type WindowViewportZoomTransition,
} from "@/lib/tauri";
import {
  useContainerWidth,
  useResizableWidth,
} from "@/lib/use-resizable-width";
import { cn } from "@/lib/utils";
import logo from "../assets/logo.png";

const GLOBAL_SIDEBAR_WIDTH = 288;
const GLOBAL_SIDEBAR_MIN_WIDTH = 220;
const GLOBAL_SIDEBAR_MAX_WIDTH = 420;
const GLOBAL_SIDEBAR_COMPACT_BELOW = 980;
const PROTECTED_WORKSPACE_WIDTH = 560;
const WINDOW_VIEWPORT_ZOOM_MS = 280;
const WINDOW_TRAFFIC_LIGHT_RESTORE_DELAYS_MS = [120, 340] as const;
const TOP_BAR_INTERACTIVE_SELECTOR = [
  "a[href]",
  "button",
  "input",
  "select",
  "textarea",
  "[contenteditable='true']",
  "[role='button']",
  "[role='checkbox']",
  "[role='combobox']",
  "[role='menuitem']",
  "[role='textbox']",
].join(",");

type WindowViewportZoomState = WindowViewportZoomTransition & {
  id: number;
  active: boolean;
  scaleX: number;
  scaleY: number;
};

export function AppShell() {
  const [isClient, setIsClient] = useState(false);
  const [newConnectionOpen, setNewConnectionOpen] = useState(false);
  const [leftSidebarOverlayOpen, setLeftSidebarOverlayOpen] = useState(false);
  const [isWindowFullscreen, setIsWindowFullscreen] = useState(false);
  const [windowViewportZoom, setWindowViewportZoom] =
    useState<WindowViewportZoomState | null>(null);
  const windowViewportZoomId = useRef(0);
  const windowViewportZoomTimeout = useRef<number | null>(null);
  const wasWindowFullscreen = useRef(false);
  const trafficLightRestoreTimeouts = useRef<number[]>([]);
  const [shellBodyRef, shellBodyWidth] = useContainerWidth<HTMLDivElement>();
  const { width: globalSidebarWidth, setWidth: setGlobalSidebarWidth } =
    useResizableWidth({
      storageKey: "dbunk.sidebar.globalWidth",
      defaultWidth: GLOBAL_SIDEBAR_WIDTH,
      min: GLOBAL_SIDEBAR_MIN_WIDTH,
      max: GLOBAL_SIDEBAR_MAX_WIDTH,
    });

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
  const handleTopBarPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.detail > 1) {
      return;
    }
    if (!shouldStartTopBarDrag(event)) {
      return;
    }
    event.preventDefault();
    void tauriStartDragging().catch(() => undefined);
  };
  const handleTopBarDoubleClick = (event: ReactMouseEvent<HTMLElement>) => {
    if (!shouldStartTopBarDrag(event)) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    void handleNativeWindowZoom().catch(() => undefined);
  };

  const handleNativeWindowZoom = async () => {
    const transition = await tauriPrepareWindowZoomTransition();
    const willAnimateViewport = startWindowViewportZoom(transition);
    if (willAnimateViewport) {
      await nextAnimationFrame();
    }
    await tauriToggleWindowZoom();
  };

  const clearWindowTrafficLightRestoreTimers = useCallback(() => {
    for (const timeout of trafficLightRestoreTimeouts.current) {
      window.clearTimeout(timeout);
    }
    trafficLightRestoreTimeouts.current = [];
  }, []);

  const restoreWindowTrafficLightPosition = useCallback(() => {
    clearWindowTrafficLightRestoreTimers();
    void tauriRestoreWindowTrafficLightPosition().catch(() => undefined);
    for (const delay of WINDOW_TRAFFIC_LIGHT_RESTORE_DELAYS_MS) {
      const timeout = window.setTimeout(() => {
        trafficLightRestoreTimeouts.current =
          trafficLightRestoreTimeouts.current.filter((id) => id !== timeout);
        void tauriRestoreWindowTrafficLightPosition().catch(() => undefined);
      }, delay);
      trafficLightRestoreTimeouts.current.push(timeout);
    }
  }, [clearWindowTrafficLightRestoreTimers]);

  const startWindowViewportZoom = (
    transition: WindowViewportZoomTransition | null,
  ) => {
    if (!transition || prefersReducedMotion()) {
      return false;
    }
    const scaleX = transition.fromWidth / transition.toWidth;
    const scaleY = transition.fromHeight / transition.toHeight;
    if (
      transition.toWidth < transition.fromWidth ||
      transition.toHeight < transition.fromHeight ||
      !Number.isFinite(scaleX) ||
      !Number.isFinite(scaleY) ||
      scaleX <= 0 ||
      scaleY <= 0 ||
      (Math.abs(scaleX - 1) < 0.015 && Math.abs(scaleY - 1) < 0.015)
    ) {
      return false;
    }

    const id = windowViewportZoomId.current + 1;
    windowViewportZoomId.current = id;
    if (windowViewportZoomTimeout.current) {
      window.clearTimeout(windowViewportZoomTimeout.current);
    }

    flushSync(() => {
      setWindowViewportZoom({
        ...transition,
        id,
        active: false,
        scaleX,
        scaleY,
      });
    });

    window.requestAnimationFrame(() => {
      setWindowViewportZoom((current) =>
        current?.id === id ? { ...current, active: true } : current,
      );
    });
    windowViewportZoomTimeout.current = window.setTimeout(() => {
      setWindowViewportZoom((current) => (current?.id === id ? null : current));
    }, WINDOW_VIEWPORT_ZOOM_MS + 140);
    return true;
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
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void tauriOnWindowFullscreenChange((fullscreen) => {
      if (!disposed) {
        const didExitFullscreen = wasWindowFullscreen.current && !fullscreen;
        wasWindowFullscreen.current = fullscreen;
        setIsWindowFullscreen(fullscreen);
        if (didExitFullscreen) {
          restoreWindowTrafficLightPosition();
        } else if (fullscreen) {
          clearWindowTrafficLightRestoreTimers();
        }
      }
    })
      .then((unsubscribe) => {
        if (disposed) {
          unsubscribe();
          return;
        }
        unlisten = unsubscribe;
      })
      .catch(() => undefined);

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [clearWindowTrafficLightRestoreTimers, restoreWindowTrafficLightPosition]);

  useEffect(() => {
    return () => {
      if (windowViewportZoomTimeout.current) {
        window.clearTimeout(windowViewportZoomTimeout.current);
      }
      clearWindowTrafficLightRestoreTimers();
    };
  }, [clearWindowTrafficLightRestoreTimers]);

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
      <div className="fixed inset-0 flex items-center justify-center bg-surface-app text-xs text-text-muted">
        <WindowDragSurface
          onDoubleClick={handleTopBarDoubleClick}
          onPointerDown={handleTopBarPointerDown}
        />
        Loading settings…
      </div>
    );
  }

  if (appSettingsStatus.state === "error") {
    return (
      <div className="fixed inset-0 flex items-center justify-center bg-surface-app p-6 text-foreground">
        <WindowDragSurface
          onDoubleClick={handleTopBarDoubleClick}
          onPointerDown={handleTopBarPointerDown}
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
        onDoubleClick={handleTopBarDoubleClick}
        onPointerDown={handleTopBarPointerDown}
      >
        <CredentialOnboarding />
      </WindowDragFrame>
    );
  }

  if (appSettings?.credentialState === "needs-unlock") {
    return (
      <WindowDragFrame
        onDoubleClick={handleTopBarDoubleClick}
        onPointerDown={handleTopBarPointerDown}
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
      <header
        data-slot="top-bar"
        data-testid="app-top-bar"
        data-window-fullscreen={isWindowFullscreen}
        data-window-drag-region
        role="toolbar"
        aria-label="Application toolbar"
        // The macOS window uses `titleBarStyle: Overlay` (see
        // src-tauri/tauri.conf.json), so the OS draws the red/yellow/green
        // controls on top of our content. The traffic lights are positioned
        // at x=18, y=26 there. Reserve 78 px
        // on the left so the sidebar toggle and `dbunk` mark don't sit under
        // them. Other
        // platforms render the OS chrome above our header so the spacer is
        // dead weight there — addressed in designs/FOLLOWUPS.md (cross-
        // platform window chrome) when we ship Windows/Linux builds.
        //
        // Drag behaviour stays on a private marker instead of
        // `data-tauri-drag-region`: Tauri's built-in titlebar double-click
        // handling would race our explicit `toggleMaximize()` call.
        className={cn(
          "flex h-12 shrink-0 items-center gap-2.5 border-b border-border-subtle bg-surface-window pr-2.5 select-none transition-[padding] duration-150",
          isWindowFullscreen ? "pl-2.5" : "pl-22",
        )}
        onDoubleClick={handleTopBarDoubleClick}
        onPointerDown={handleTopBarPointerDown}
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
          data-window-drag-region
          className="flex items-center gap-1.5 text-sm font-semibold tracking-tight"
        >
          <img src={logo} alt="dbunk" className="size-[1.375rem]" />
        </div>

        <div
          data-window-drag-region
          className="relative ml-1 hidden min-w-40 max-w-md flex-1 md:block"
        >
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
          <Input
            placeholder="Search tables"
            aria-label="Search tables"
            className="h-8 pl-8 pr-13 text-xs"
          />
          <span className="pointer-events-none absolute right-2 top-1/2 flex h-[1.125rem] -translate-y-1/2 items-center gap-0.5 rounded-sm border border-border-subtle bg-surface-app px-1.5 text-[0.58rem] text-text-muted">
            <IconCommand className="size-2.5" />K
          </span>
        </div>

        {/* Explicit flexible drag spacer between the search and the right
            actions. Always rendered so the right group stays anchored to the
            window edge even on screens too small to show the search. */}
        <div
          data-window-drag-region
          aria-hidden="true"
          data-testid="window-drag-spacer"
          className="h-full flex-1"
        />

        <div data-window-drag-region className="flex items-center gap-1.5">
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
            className="size-7 rounded-full text-[0.625rem] font-semibold"
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
    </div>
  );
}

function WindowDragFrame({
  children,
  onDoubleClick,
  onPointerDown,
}: {
  children: ReactNode;
  onDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
}) {
  return (
    <div className="fixed inset-0 overflow-hidden bg-surface-app">
      <WindowDragSurface
        onDoubleClick={onDoubleClick}
        onPointerDown={onPointerDown}
      />
      {children}
    </div>
  );
}

function WindowDragSurface({
  onDoubleClick,
  onPointerDown,
}: {
  onDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
}) {
  return (
    <div
      data-window-drag-region
      data-testid="window-drag-surface"
      aria-hidden="true"
      className="absolute inset-x-0 top-0 z-50 h-10 select-none"
      onDoubleClick={onDoubleClick}
      onPointerDown={onPointerDown}
    />
  );
}

function shouldStartTopBarDrag(
  event: ReactMouseEvent<HTMLElement> | ReactPointerEvent<HTMLElement>,
) {
  if (event.button !== 0 || event.defaultPrevented) {
    return false;
  }
  const target = event.target;
  if (!(target instanceof Element)) {
    return false;
  }
  if (target.closest(TOP_BAR_INTERACTIVE_SELECTOR)) {
    return false;
  }
  return Boolean(target.closest("[data-window-drag-region]"));
}

function windowViewportZoomStyle(
  state: WindowViewportZoomState | null,
): CSSProperties | undefined {
  if (!state) {
    return undefined;
  }
  return {
    width: `${state.toWidth}px`,
    height: `${state.toHeight}px`,
    transform: state.active
      ? "scale(1)"
      : `scale(${state.scaleX}, ${state.scaleY})`,
    transitionDuration: `${WINDOW_VIEWPORT_ZOOM_MS}ms`,
  };
}

function prefersReducedMotion() {
  if (typeof window === "undefined") {
    return false;
  }
  return Boolean(
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches,
  );
}

function nextAnimationFrame() {
  return new Promise<void>((resolve) => {
    if (typeof window === "undefined") {
      resolve();
      return;
    }
    window.requestAnimationFrame(() => resolve());
  });
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
