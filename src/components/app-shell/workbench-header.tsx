import {
  IconChevronDown,
  IconDatabase,
  IconDatabaseOff,
  IconLock,
  IconPlus,
} from "@tabler/icons-react";
import type {
  MouseEvent as ReactMouseEvent,
  PointerEvent as ReactPointerEvent,
} from "react";
import { useState } from "react";

import {
  MACOS_TRAFFIC_LIGHT_HEADER_INSET_PX,
  needsMacTitlebarGutter,
} from "@/components/app-shell/macos-titlebar";
import { connectionStatusTone } from "@/components/connection-status";
import { EnvironmentBadge } from "@/components/environment-badge";
import { NewConnectionDialog } from "@/components/new-connection-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { StatusDot } from "@/components/ui/status-dot";
import { type Connection, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface WorkbenchHeaderProps {
  activeConnection: Connection | undefined;
  isWindowFullscreen: boolean;
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  onDoubleClick: (event: ReactMouseEvent<HTMLElement>) => void;
}

function isConnectedStatus(status: Connection["status"]): boolean {
  return status === "Connected" || status === "Read only";
}

/**
 * Window header (§3.1): connection switcher on the DropdownMenu
 * primitive, environment tag, read-only badge. Height `--h-header`;
 * the macOS traffic-light inset comes from the single constant in
 * `macos-titlebar.ts`.
 */
export function WorkbenchHeader({
  activeConnection,
  isWindowFullscreen,
  onPointerDown,
  onDoubleClick,
}: WorkbenchHeaderProps) {
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
  const readOnly = activeConnection?.status === "Read only";

  return (
    <header
      data-slot="top-bar"
      data-testid="workbench-header"
      data-window-fullscreen={isWindowFullscreen}
      data-window-drag-region
      onDoubleClick={onDoubleClick}
      onPointerDown={onPointerDown}
      className="relative flex h-(--h-header) shrink-0 items-center gap-3 border-b border-border-subtle bg-surface-window pr-3 select-none"
      style={{
        paddingLeft: needsMacTitlebarGutter(isWindowFullscreen)
          ? MACOS_TRAFFIC_LIGHT_HEADER_INSET_PX
          : 12,
      }}
    >
      <DropdownMenu>
        <DropdownMenuTrigger
          render={<Button type="button" variant="outline" size="sm" />}
          data-testid="connection-switcher"
        >
          <IconDatabase className="text-accent" />
          <span className="font-semibold text-foreground">
            {activeConnection?.name ?? "Select connection"}
          </span>
          <EnvironmentBadge environment={activeConnection?.environment} short />
          {readOnly ? (
            <Badge variant="warning" data-testid="read-only-badge">
              <IconLock /> Read-only
            </Badge>
          ) : null}
          {activeConnection ? (
            <span className="font-normal text-text-muted">
              {activeConnection.engine}
            </span>
          ) : null}
          <IconChevronDown className="text-text-muted" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-72">
          {connections.map((connection) => {
            const isActive = connection.id === activeConnection?.id;
            return (
              <DropdownMenuItem
                key={connection.id}
                data-active={isActive || undefined}
                className={cn(isActive && "bg-accent-subdued")}
                onClick={() => {
                  setActiveConnectionId(connection.id);
                  if (!isConnectedStatus(connection.status)) {
                    void connectConnection(connection.id);
                  }
                }}
              >
                <StatusDot tone={connectionStatusTone(connection.status)} />
                <span className="min-w-0 flex-1">
                  <span className="flex items-center gap-1.5 truncate font-medium text-foreground">
                    <span className="truncate">{connection.name}</span>
                    <EnvironmentBadge
                      environment={connection.environment}
                      short
                    />
                  </span>
                  <span className="block truncate text-2xs text-text-muted">
                    {connection.engine} · {connection.host}
                  </span>
                </span>
              </DropdownMenuItem>
            );
          })}
          {connections.length > 0 ? <DropdownMenuSeparator /> : null}
          <DropdownMenuItem onClick={() => setNewConnectionOpen(true)}>
            <IconPlus />
            New connection
          </DropdownMenuItem>
          {activeConnection && isConnectedStatus(activeConnection.status) ? (
            <DropdownMenuItem
              onClick={() => disconnectConnection(activeConnection.id)}
            >
              <IconDatabaseOff />
              Disconnect
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      <div
        data-window-drag-region
        aria-hidden="true"
        className="h-full flex-1"
      />

      <div className="flex items-center gap-2 text-2xs text-text-muted">
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
