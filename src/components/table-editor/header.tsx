import {
  IconDotsVertical,
  IconLayoutSidebarRight,
  IconTable,
} from "@tabler/icons-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export type SubTab = "data" | "schema" | "indexes" | "relations";

export const SUB_TABS: ReadonlyArray<{ id: SubTab; label: string }> = [
  { id: "data", label: "Data" },
  { id: "schema", label: "Schema" },
  { id: "indexes", label: "Indexes" },
  { id: "relations", label: "Relations" },
];

interface TableEditorHeaderProps {
  title: string;
  schemaBadge: string;
  rowCountLabel: string;
  activeSubTab: SubTab;
  onSubTabChange: (next: SubTab) => void;
  showRowDetailsToggle: boolean;
  rowDetailsVisible: boolean;
  onToggleRowDetails: () => void;
  onOpenSql: () => void;
  onRefresh: () => void;
  onExportTableDdl: () => void;
  onOpenCopyTable: () => void;
  onRunMaintenance: (action: "vacuum" | "analyze" | "reindex") => void;
}

export function TableEditorHeader({
  title,
  schemaBadge,
  rowCountLabel,
  activeSubTab,
  onSubTabChange,
  showRowDetailsToggle,
  rowDetailsVisible,
  onToggleRowDetails,
  onOpenSql,
  onRefresh,
  onExportTableDdl,
  onOpenCopyTable,
  onRunMaintenance,
}: TableEditorHeaderProps) {
  return (
    <div className="shrink-0 border-b border-border-subtle bg-surface-window px-3 pt-2">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <div className="flex size-7 items-center justify-center rounded-sm border border-accent-green/30 bg-accent-green/10 text-accent-green">
            <IconTable className="size-3.5" />
          </div>
          <h1 className="truncate text-sm font-semibold tracking-tight text-foreground">
            {title}
          </h1>
          <Badge variant="outline">{rowCountLabel}</Badge>
          <Badge variant="outline">{schemaBadge}</Badge>
        </div>
        <div className="flex items-center gap-1.5">
          {showRowDetailsToggle ? (
            <Button
              size="sm"
              variant={rowDetailsVisible ? "secondary" : "outline"}
              aria-pressed={rowDetailsVisible}
              aria-label={
                rowDetailsVisible ? "Hide row details" : "Show row details"
              }
              title={
                rowDetailsVisible ? "Hide row details" : "Show row details"
              }
              onClick={onToggleRowDetails}
            >
              <IconLayoutSidebarRight className="size-3.5" />
              <span className="dbunk-optional-label">Details</span>
            </Button>
          ) : null}
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label="Table actions"
              title="Table actions"
              className={cn(
                "inline-flex h-7 items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 text-[0.6875rem] font-medium text-foreground transition-colors hover:bg-surface-panel-elevated",
              )}
            >
              <IconDotsVertical className="size-3.5 text-text-muted" />
              <span className="dbunk-optional-label">Table actions</span>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuItem onClick={onOpenSql}>
                Open in SQL
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onExportTableDdl}>
                Export table DDL
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onOpenCopyTable}>
                Copy to table…
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => onRunMaintenance("vacuum")}>
                VACUUM table
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => onRunMaintenance("analyze")}>
                ANALYZE table
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => onRunMaintenance("reindex")}>
                REINDEX table
              </DropdownMenuItem>
              <DropdownMenuItem onClick={onRefresh}>
                Refresh data
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>
      <div className="mt-1.5 flex items-end gap-1">
        {SUB_TABS.map(({ id, label }) => {
          const isActive = activeSubTab === id;
          return (
            <button
              key={id}
              type="button"
              aria-current={isActive ? "page" : undefined}
              onClick={() => onSubTabChange(id)}
              className={cn(
                "relative h-7 px-2.5 text-xs font-medium transition-colors",
                isActive
                  ? "text-foreground"
                  : "text-text-muted hover:text-foreground",
              )}
            >
              {label}
              {isActive ? (
                <span className="absolute inset-x-3 bottom-0 h-0.5 rounded-full bg-accent-green" />
              ) : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}
