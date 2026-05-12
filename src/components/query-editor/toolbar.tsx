import {
  IconChevronDown,
  IconDeviceFloppy,
  IconLayoutSidebarRightCollapse,
  IconLayoutSidebarRightExpand,
  IconLoader2,
  IconPlayerPlay,
  IconSparkles,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { Connection } from "@/lib/store";

interface QueryEditorToolbarProps {
  dbSelectorLabel: string;
  connections: Connection[];
  hasEdits: boolean;
  onDiscardEdits: () => void;
  isRunning: boolean;
  isSidebarOpen: boolean;
  onToggleSidebar: () => void;
  onRunCurrent: () => void;
  onRunSelection: () => void;
  onRunAll: () => void;
}

export function QueryEditorToolbar({
  dbSelectorLabel,
  connections,
  hasEdits,
  onDiscardEdits,
  isRunning,
  isSidebarOpen,
  onToggleSidebar,
  onRunCurrent,
  onRunSelection,
  onRunAll,
}: QueryEditorToolbarProps) {
  const SidebarIcon = isSidebarOpen
    ? IconLayoutSidebarRightCollapse
    : IconLayoutSidebarRightExpand;

  return (
    <div className="flex min-h-10 shrink-0 items-center justify-between gap-2 overflow-x-auto border-b border-border-subtle bg-surface-window px-3 py-1.5">
      <div className="flex min-w-0 items-center gap-2">
        <span className="flex size-7 items-center justify-center rounded-sm border border-accent-green/30 bg-accent-green/10 text-accent-green">
          <IconTerminal2 className="size-3.5" />
        </span>
        <h1 className="truncate text-xs font-semibold tracking-tight text-foreground">
          Query Editor
        </h1>
        <DropdownMenu>
          <DropdownMenuTrigger
            aria-label="Connection selector"
            className="ml-1 inline-flex h-7 max-w-56 min-w-0 items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 text-[0.6875rem] font-medium text-foreground transition-colors hover:bg-surface-panel-elevated"
          >
            <span className="size-1.5 rounded-full bg-accent-green" />
            <span className="truncate">{dbSelectorLabel}</span>
            <IconChevronDown className="size-3 text-text-muted" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start">
            {connections.map((connection) => (
              <DropdownMenuItem
                key={connection.id}
                // TODO(FOLLOWUPS): switch the editor's connection
                onClick={() => {}}
              >
                {connection.name} · {connection.engine}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      <div className="flex items-center gap-1.5">
        {hasEdits ? (
          <>
            <Button size="sm" variant="ghost" onClick={onDiscardEdits}>
              <IconX className="size-3.5" /> Discard
            </Button>
            <Button size="sm">
              <IconDeviceFloppy className="size-3.5" /> Save
            </Button>
            <div className="h-5 w-px bg-border-subtle" />
          </>
        ) : null}

        <Button size="sm" variant="outline" aria-label="Format" title="Format">
          <IconSparkles className="size-3.5" />
          <span className="dbunk-optional-label">Format</span>
        </Button>
        <Button
          type="button"
          size="icon-sm"
          variant={isSidebarOpen ? "secondary" : "outline"}
          aria-label={
            isSidebarOpen ? "Hide query sidebar" : "Show query sidebar"
          }
          title={isSidebarOpen ? "Hide query sidebar" : "Show query sidebar"}
          aria-pressed={isSidebarOpen}
          onClick={onToggleSidebar}
        >
          <SidebarIcon className="size-3.5" />
        </Button>

        <div className="flex items-center">
          <Button
            size="sm"
            onClick={onRunCurrent}
            disabled={isRunning}
            aria-busy={isRunning}
            aria-label={isRunning ? "Running" : "Run"}
            className="rounded-r-none"
          >
            {isRunning ? (
              <>
                <IconLoader2 className="size-3.5 animate-spin" />
                Running…
              </>
            ) : (
              <>
                <IconPlayerPlay className="size-3.5" />
                Run
              </>
            )}
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label="Run options"
              className="inline-flex h-6 items-center justify-center rounded-r-sm border-l border-primary-foreground/20 bg-primary px-1.5 text-primary-foreground hover:bg-accent-green-hover disabled:opacity-50"
              disabled={isRunning}
            >
              <IconChevronDown className="size-3.5" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuItem onClick={onRunSelection}>
                Run selection
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onRunCurrent}>
                Run current statement
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onRunAll}>Run all</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
    </div>
  );
}
