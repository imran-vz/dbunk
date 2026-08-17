import {
  IconDatabasePlus,
  IconLayoutSidebarLeftCollapse,
  IconPlus,
  IconSearch,
  IconSettings,
  IconTerminal2,
} from "@tabler/icons-react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { useState } from "react";

import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { NewLocalDatabaseDialog } from "@/components/new-local-database-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { isMacPlatform, Kbd } from "@/components/ui/kbd";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

import logo from "../../assets/logo.png";

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
  onLeftSidebarToggle,
  onCreateNewQueryTab,
  onPointerDown,
  onDoubleClick,
}: AppShellHeaderProps) {
  const openSettings = useAppStore((state) => state.openSettings);
  const [newLocalDatabaseOpen, setNewLocalDatabaseOpen] = useState(false);
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
      // at x=18, y=26 there; reserve 78 px on the left so the sidebar
      // toggle and `dbunk` mark don't sit under them. On Windows / Linux
      // the OS chrome renders above our header, so the spacer is gated
      // on `isMacPlatform()`.
      //
      // Drag behaviour stays on a private marker instead of
      // `data-tauri-drag-region`: Tauri's built-in titlebar double-click
      // handling would race our explicit `toggleMaximize()` call.
      className={cn(
        "flex h-12 shrink-0 items-center gap-2.5 border-b border-border-subtle bg-surface-window pr-2.5 select-none transition-[padding] duration-150",
        isWindowFullscreen || !isMacPlatform() ? "pl-2.5" : "pl-22",
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
          className="h-8 cursor-pointer pl-8 pr-13 text-xs"
          readOnly
          onFocus={(event) => {
            event.currentTarget.blur();
            window.dispatchEvent(new CustomEvent("dbunk:open-command-palette"));
          }}
          onClick={() => {
            window.dispatchEvent(new CustomEvent("dbunk:open-command-palette"));
          }}
        />
        <Kbd
          keys={["mod", "k"]}
          className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-[0.58rem]"
        />
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
        <NewLocalDatabaseDialog
          open={newLocalDatabaseOpen}
          onOpenChange={setNewLocalDatabaseOpen}
          trigger={
            <Button
              type="button"
              size="sm"
              variant="outline"
              aria-label="New Local Database"
              title="Create a local Postgres or MySQL with Docker"
            >
              <IconDatabasePlus className="size-3.5" />
              <span className="dbunk-optional-label">New Local DB</span>
            </Button>
          }
        />
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
          variant="ghost"
          aria-label="Open Settings"
          title="Settings"
          onClick={() => openSettings()}
          className="text-text-muted"
        >
          <IconSettings className="size-4" />
        </Button>
      </div>
    </header>
  );
}
