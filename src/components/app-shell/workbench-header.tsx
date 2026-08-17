import {
  IconChevronDown,
  IconDatabase,
  IconDatabaseOff,
  IconPlus,
} from "@tabler/icons-react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { useState } from "react";

import { needsMacTitlebarGutter } from "@/components/app-shell/macos-titlebar";
import { connectionStatusTone } from "@/components/connection-status";
import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { StatusDot } from "@/components/ui/status-dot";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface WorkbenchHeaderProps {
  activeConnection: Connection | undefined;
  isWindowFullscreen: boolean;
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  onDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
}

export function WorkbenchHeader({
  activeConnection,
  isWindowFullscreen,
  onPointerDown,
  onDoubleClick,
}: WorkbenchHeaderProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [newConnectionOpen, setNewConnectionOpen] = useState(false);
  const {
    connections,
    setActiveConnectionId,
    connectConnection,
    disconnectConnection,
  } = useAppStore();

  const hostLabel = activeConnection
    ? activeConnection.host || activeConnection.database
    : "No connection";

  return (
    <header
      data-slot="top-bar"
      data-testid="workbench-header"
      data-window-fullscreen={isWindowFullscreen}
      data-window-drag-region
      role="toolbar"
      aria-label="Connection toolbar"
      onDoubleClick={onDoubleClick}
      onPointerDown={onPointerDown}
      className={cn(
        "relative flex shrink-0 items-center gap-3 border-b border-border-subtle bg-surface-window pr-3 select-none",
        needsMacTitlebarGutter(isWindowFullscreen) ? "h-12 pl-22" : "h-10 pl-3",
      )}
    >
      <div className="relative">
        <button
          type="button"
          aria-haspopup="listbox"
          aria-expanded={menuOpen}
          onClick={() => setMenuOpen((open) => !open)}
          className="flex items-center gap-2 rounded-md border border-border-subtle bg-surface-panel-elevated px-2.5 py-1.5 text-[12px] transition-colors hover:bg-surface-panel"
        >
          <IconDatabase className="size-3.5 text-accent" />
          <span className="font-semibold text-foreground">
            {activeConnection?.name ?? "Select connection"}
          </span>
          {activeConnection ? (
            <span className="text-text-muted">· {activeConnection.engine}</span>
          ) : null}
          <IconChevronDown className="size-3.5 text-text-muted" />
        </button>
        {menuOpen ? (
          <>
            <button
              type="button"
              aria-label="Close connection menu"
              className="fixed inset-0 z-40"
              onClick={() => setMenuOpen(false)}
            />
            <div
              role="listbox"
              aria-label="Connections"
              className="absolute top-full left-0 z-50 mt-1 w-72 overflow-hidden rounded-md border border-border-subtle bg-surface-panel-elevated shadow-lg"
            >
              <div className="max-h-64 overflow-auto py-1">
                {connections.map((connection) => {
                  const isActive = connection.id === activeConnection?.id;
                  const tone = connectionStatusTone(connection.status);
                  return (
                    <button
                      key={connection.id}
                      type="button"
                      role="option"
                      aria-selected={isActive}
                      onClick={() => {
                        setActiveConnectionId(connection.id);
                        setMenuOpen(false);
                      }}
                      onDoubleClick={() => {
                        void connectConnection(connection.id);
                      }}
                      className={cn(
                        "flex w-full items-center gap-2.5 px-3 py-2 text-left text-xs transition-colors hover:bg-surface-panel",
                        isActive && "bg-accent-subdued",
                      )}
                    >
                      <StatusDot tone={tone} />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate font-medium text-foreground">
                          {connection.name}
                        </span>
                        <span className="block truncate text-[10px] text-text-muted">
                          {connection.engine} · {connection.host}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
              <div className="border-t border-border-subtle p-1">
                <button
                  type="button"
                  className="flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-xs text-text-muted hover:bg-surface-panel hover:text-foreground"
                  onClick={() => {
                    setMenuOpen(false);
                    setNewConnectionOpen(true);
                  }}
                >
                  <IconPlus className="size-3.5" />
                  New connection
                </button>
                {activeConnection &&
                (activeConnection.status === "Connected" ||
                  activeConnection.status === "Read only") ? (
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 rounded px-2.5 py-1.5 text-xs text-text-muted hover:bg-surface-panel hover:text-foreground"
                    onClick={() => {
                      disconnectConnection(activeConnection.id);
                      setMenuOpen(false);
                    }}
                  >
                    <IconDatabaseOff className="size-3.5" />
                    Disconnect
                  </button>
                ) : null}
              </div>
            </div>
          </>
        ) : null}
      </div>

      <div
        data-window-drag-region
        aria-hidden="true"
        className="h-full flex-1"
      />

      <div className="flex items-center gap-2 text-[11px] text-text-muted">
        {activeConnection ? (
          <>
            <StatusDot
              tone={connectionStatusTone(activeConnection.status)}
              className="size-1.5"
            />
            <span>{hostLabel}</span>
          </>
        ) : null}
      </div>

      <NewConnectionDialog
        open={newConnectionOpen}
        onOpenChange={setNewConnectionOpen}
      />
    </header>
  );
}
