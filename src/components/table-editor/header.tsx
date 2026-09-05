import { IconDotsVertical, IconLayoutSidebarRight } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

export type SubTab =
  | "data"
  | "schema"
  | "indexes"
  | "relations"
  | "schema-map"
  | "transfer"
  | "specialized";

export const SUB_TABS: ReadonlyArray<{ id: SubTab; label: string }> = [
  { id: "data", label: "Data" },
  { id: "schema", label: "Schema" },
  { id: "indexes", label: "Indexes" },
  { id: "relations", label: "Relations" },
  { id: "schema-map", label: "Schema Map" },
  { id: "transfer", label: "Transfer" },
  { id: "specialized", label: "Specialized" },
];

interface TableEditorHeaderProps {
  title: string;
  schemaBadge: string;
  rowCountLabel: string;
  showRowDetailsToggle: boolean;
  rowDetailsVisible: boolean;
  onToggleRowDetails: () => void;
  onOpenSql: () => void;
  onRefresh: () => void;
  onExportTableDdl: () => void;
  onOpenCopyTable: () => void;
  onRunMaintenance: (action: "vacuum" | "analyze" | "reindex") => void;
  /** Table Seeding is engine-gated (PostgreSQL-first, ADR-0020). */
  showSeedAction: boolean;
  onOpenSeedTable: () => void;
  onOpenXlsxImport?: () => void;
  onOpenBackupRestore?: (operation: "backup" | "restore") => void;
}

export function TableEditorHeader({
  title,
  schemaBadge,
  rowCountLabel,
  showRowDetailsToggle,
  rowDetailsVisible,
  onToggleRowDetails,
  onOpenSql,
  onRefresh,
  onExportTableDdl,
  onOpenCopyTable,
  onRunMaintenance,
  showSeedAction,
  onOpenSeedTable,
  onOpenXlsxImport,
  onOpenBackupRestore,
}: TableEditorHeaderProps) {
  const actions = (
    <div className="ml-auto flex items-center gap-1.5">
      {showRowDetailsToggle ? (
        <Button
          size="sm"
          variant={rowDetailsVisible ? "secondary" : "outline"}
          aria-pressed={rowDetailsVisible}
          aria-label={
            rowDetailsVisible ? "Hide row details" : "Show row details"
          }
          title={rowDetailsVisible ? "Hide row details" : "Show row details"}
          onClick={onToggleRowDetails}
        >
          <IconLayoutSidebarRight />
          <span className="dbunk-optional-label">Details</span>
        </Button>
      ) : null}
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label="Table actions"
          title="Table actions"
          className={cn(
            "inline-flex h-7 items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 text-2xs font-medium text-foreground transition-colors hover:bg-surface-panel-elevated",
          )}
        >
          <IconDotsVertical className="size-3.5 text-text-muted" />
          <span className="dbunk-optional-label">Table actions</span>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          {onOpenBackupRestore ? (
            <>
              <DropdownMenuItem onClick={() => onOpenBackupRestore("backup")}>
                Back up table…
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => onOpenBackupRestore("restore")}>
                Restore database…
              </DropdownMenuItem>
            </>
          ) : null}
          <DropdownMenuItem onClick={onOpenSql}>Open in SQL</DropdownMenuItem>
          <DropdownMenuItem onClick={onExportTableDdl}>
            Export table DDL
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onOpenCopyTable}>
            Copy to table…
          </DropdownMenuItem>
          {onOpenXlsxImport ? (
            <DropdownMenuItem onClick={onOpenXlsxImport}>
              Import XLSX…
            </DropdownMenuItem>
          ) : null}
          {showSeedAction ? (
            <DropdownMenuItem onClick={onOpenSeedTable}>
              Seed table…
            </DropdownMenuItem>
          ) : null}
          <DropdownMenuItem onClick={() => onRunMaintenance("vacuum")}>
            VACUUM table
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => onRunMaintenance("analyze")}>
            ANALYZE table
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => onRunMaintenance("reindex")}>
            REINDEX table
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onRefresh}>Refresh data</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );

  return (
    <div className="flex shrink-0 items-center gap-3 border-b border-border-subtle px-3 py-1.5 text-2xs text-text-muted">
      <span className="font-medium text-foreground">
        {schemaBadge}.{title}
      </span>
      <span>{rowCountLabel}</span>
      {actions}
    </div>
  );
}
