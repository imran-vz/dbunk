import { IconX } from "@tabler/icons-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ResponsiveEdgePanel } from "@/components/ui/responsive-edge-panel";

import {
  PROTECTED_WORKSPACE_WIDTH,
  ROW_DETAILS_COMPACT_BELOW,
  ROW_DETAILS_WIDTH,
} from "./use-row-details-visibility";

interface RowDetailsPanelProps {
  columns: string[];
  selectedRow: string[] | undefined;
  selectedRowIndex: number | null;
  selectedRowCount: number;
  totalRows: number;
  indexes: number;
  bodyWidth: number;
  wideVisible: boolean;
  overlayOpen: boolean;
  onOverlayOpenChange: (open: boolean) => void;
  onClose: () => void;
}

export function RowDetailsPanel({
  columns,
  selectedRow,
  selectedRowIndex,
  selectedRowCount,
  totalRows,
  indexes,
  bodyWidth,
  wideVisible,
  overlayOpen,
  onOverlayOpenChange,
  onClose,
}: RowDetailsPanelProps) {
  const hasMultiple = selectedRowCount > 1;
  const subtitle = hasMultiple
    ? `${selectedRowCount} rows selected`
    : selectedRowCount === 1
      ? "1 selected"
      : "First visible row";

  return (
    <ResponsiveEdgePanel
      side="right"
      storageKey="dbunk.sidebar.rowDetails"
      title="Row Details"
      width={ROW_DETAILS_WIDTH}
      containerWidth={bodyWidth}
      compactBelow={ROW_DETAILS_COMPACT_BELOW}
      protectedWorkspaceWidth={PROTECTED_WORKSPACE_WIDTH}
      wideVisible={wideVisible}
      open={overlayOpen}
      onOpenChange={onOverlayOpenChange}
    >
      <div
        data-testid="row-details-panel"
        className="flex h-full min-h-0 flex-col gap-3 p-4 text-xs"
      >
        <div className="flex items-start justify-between">
          <div>
            <div className="flex items-center gap-2">
              <span className="text-sm font-semibold text-foreground">
                Row {selectedRowIndex !== null ? selectedRowIndex + 1 : 1}
              </span>
              <Badge variant="success" className="h-5 px-2">
                Selected
              </Badge>
            </div>
            <div className="mt-0.5 text-text-muted">{subtitle}</div>
          </div>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="Close row details"
            onClick={onClose}
          >
            <IconX className="size-3.5" />
          </Button>
        </div>

        <div className="flex flex-1 flex-col gap-2 overflow-auto">
          {hasMultiple ? (
            <MultiSelectMessage />
          ) : selectedRow ? (
            columns.map((column, index) => (
              <RowDetailCell
                key={column}
                column={column}
                value={selectedRow[index]}
              />
            ))
          ) : (
            <div className="rounded-md border border-border-subtle bg-surface-panel-elevated p-3 text-text-muted">
              No row selected
            </div>
          )}
        </div>

        <SummaryCard totalRows={totalRows} indexes={indexes} />
      </div>
    </ResponsiveEdgePanel>
  );
}

function MultiSelectMessage() {
  return (
    <div className="rounded-md border border-accent-green/20 bg-accent-green/10 p-3 text-text-muted">
      <div className="font-semibold text-foreground">
        Multiple rows selected
      </div>
      <div className="mt-1">Select a single row to inspect column values.</div>
    </div>
  );
}

function RowDetailCell({
  column,
  value,
}: {
  column: string;
  value: string | undefined;
}) {
  return (
    <div className="rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2">
      <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
        {column}
      </div>
      <div className="mt-1 truncate font-mono text-[0.75rem] text-foreground">
        {value || "NULL"}
      </div>
    </div>
  );
}

function SummaryCard({
  totalRows,
  indexes,
}: {
  totalRows: number;
  indexes: number;
}) {
  const rows: Array<[string, string]> = [
    ["Total rows", totalRows.toLocaleString()],
    ["Data size", "—"],
    ["Indexes", indexes.toLocaleString()],
    ["Last vacuum", "—"],
    ["Last analyze", "—"],
  ];
  return (
    <div className="rounded-md border border-border-subtle bg-surface-panel-elevated p-3">
      <div className="text-[0.625rem] font-medium uppercase tracking-[0.12em] text-text-muted">
        Summary
      </div>
      <dl className="mt-2 space-y-1.5 text-xs">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-center justify-between">
            <dt className="text-text-muted">{label}</dt>
            <dd className="tabular-nums text-foreground">{value}</dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
