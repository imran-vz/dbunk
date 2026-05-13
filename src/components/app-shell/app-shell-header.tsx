import {
  IconCommand,
  IconLayoutSidebarLeftCollapse,
  IconPlus,
  IconSearch,
  IconTerminal2,
} from "@tabler/icons-react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { Connection } from "@/lib/store";
import { cn } from "@/lib/utils";
import logo from "../../assets/logo.png";

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

function sidebarToggleLabel(
  isShellCompact: boolean,
  leftSidebarOverlayOpen: boolean,
  isLeftSidebarOpen: boolean,
): "Hide sidebar" | "Show sidebar" {
  if (isShellCompact) {
    return leftSidebarOverlayOpen ? "Hide sidebar" : "Show sidebar";
  }
  return isLeftSidebarOpen ? "Hide sidebar" : "Show sidebar";
}

export interface AppShellHeaderProps {
  isWindowFullscreen: boolean;
  isShellCompact: boolean;
  leftSidebarOverlayOpen: boolean;
  isLeftSidebarOpen: boolean;
  newConnectionOpen: boolean;
  setNewConnectionOpen: (open: boolean) => void;
  activeConnection: Connection | undefined;
  onLeftSidebarToggle: () => void;
  onCreateNewQueryTab: () => void;
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  onDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
}

export function AppShellHeader({
  isWindowFullscreen,
  isShellCompact,
  leftSidebarOverlayOpen,
  isLeftSidebarOpen,
  newConnectionOpen,
  setNewConnectionOpen,
  activeConnection,
  onLeftSidebarToggle,
  onCreateNewQueryTab,
  onPointerDown,
  onDoubleClick,
}: AppShellHeaderProps) {
  const toggleLabel = sidebarToggleLabel(
    isShellCompact,
    leftSidebarOverlayOpen,
    isLeftSidebarOpen,
  );
  return (
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
      onDoubleClick={onDoubleClick}
      onPointerDown={onPointerDown}
    >
      <Button
        type="button"
        size="icon-sm"
        variant="ghost"
        aria-label={toggleLabel}
        title={toggleLabel}
        onClick={onLeftSidebarToggle}
        className="text-text-muted"
      >
        <IconLayoutSidebarLeftCollapse className="size-4" />
      </Button>
      <div
        data-window-drag-region
        className="flex items-center gap-1.5 text-sm font-semibold tracking-tight"
      >
        <img src={logo} alt="dbunk" className="size-5.5" />
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
        <span className="pointer-events-none absolute right-2 top-1/2 flex h-4.5 -translate-y-1/2 items-center gap-0.5 rounded-sm border border-border-subtle bg-surface-app px-1.5 text-[0.58rem] text-text-muted">
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
          onClick={onCreateNewQueryTab}
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
  );
}
