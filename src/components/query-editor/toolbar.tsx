import {
  IconCheck,
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
import { storageClassFor } from "@/lib/engine-policy";
import type { Connection } from "@/lib/store";

interface QueryEditorToolbarProps {
  dbSelectorLabel: string;
  connections: Connection[];
  currentConnectionId: string;
  onRetargetConnection: (connectionId: string) => void;
  hasEdits: boolean;
  onDiscardEdits: () => void;
  isRunning: boolean;
  isSidebarOpen: boolean;
  onToggleSidebar: () => void;
  onRunCurrent: () => void;
  onRunSelection: () => void;
  onRunAll: () => void;
  onExplain: () => void;
  onFormat: () => void;
  onInsertSnippet: (sql: string) => void;
}

export function QueryEditorToolbar({
  dbSelectorLabel,
  connections,
  currentConnectionId,
  onRetargetConnection,
  hasEdits,
  onDiscardEdits,
  isRunning,
  isSidebarOpen,
  onToggleSidebar,
  onRunCurrent,
  onRunSelection,
  onRunAll,
  onExplain,
  onFormat,
  onInsertSnippet,
}: QueryEditorToolbarProps) {
  const relationalConnections = connections.filter(
    (connection) => storageClassFor(connection.engine) === "relational",
  );
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
            {relationalConnections.length === 0 ? (
              <div className="px-2 py-1.5 text-[0.6875rem] text-text-muted">
                No relational connections available.
              </div>
            ) : null}
            {relationalConnections.map((connection) => {
              const isCurrent = connection.id === currentConnectionId;
              return (
                <DropdownMenuItem
                  key={connection.id}
                  disabled={isRunning || isCurrent}
                  onClick={() => onRetargetConnection(connection.id)}
                >
                  {isCurrent ? (
                    <IconCheck className="size-3 text-accent-green" />
                  ) : (
                    <span className="size-3" />
                  )}
                  {connection.name} · {connection.engine}
                </DropdownMenuItem>
              );
            })}
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

        <Button
          size="sm"
          variant="outline"
          aria-label="Format SQL"
          title="Format SQL (Cmd/Ctrl+Shift+F)"
          onClick={onFormat}
          disabled={isRunning}
        >
          <IconSparkles className="size-3.5" />
          <span className="dbunk-optional-label">Format</span>
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger className="inline-flex h-7 items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 text-[0.6875rem] font-medium text-foreground hover:bg-surface-panel-elevated">
            Snippets
            <IconChevronDown className="size-3 text-text-muted" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {SQL_SNIPPETS.map((snippet) => (
              <DropdownMenuItem
                key={snippet.label}
                onClick={() => onInsertSnippet(snippet.sql)}
              >
                {snippet.label}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          size="sm"
          variant="outline"
          onClick={onExplain}
          disabled={isRunning}
          aria-label="Run EXPLAIN"
        >
          EXPLAIN
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

const SQL_SNIPPETS = [
  {
    label: "Top rows",
    sql: "select *\nfrom public.table_name\nlimit 100;",
  },
  {
    label: "Grouped count",
    sql: "select column_name, count(*)\nfrom public.table_name\ngroup by column_name\norder by count(*) desc;",
  },
  {
    label: "Recent rows",
    sql: "select *\nfrom public.table_name\nwhere created_at >= now() - interval '7 days'\norder by created_at desc;",
  },
] as const;
