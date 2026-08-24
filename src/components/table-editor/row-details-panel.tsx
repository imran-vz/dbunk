import { IconX } from "@tabler/icons-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Panel, usePanelState } from "@/components/ui/panel";

interface RowDetailsPanelProps {
  columns: string[];
  selectedRow: string[] | undefined;
  selectedRowIndex: number | null;
  selectedRowCount: number;
  totalRows: number;
  indexes: number;
  /** Parent-controlled presence: false unmounts the panel entirely. */
  visible: boolean;
  onClose: () => void;
}

export function RowDetailsPanel({
  columns,
  selectedRow,
  selectedRowIndex,
  selectedRowCount,
  totalRows,
  indexes,
  visible,
  onClose,
}: RowDetailsPanelProps) {
  const panel = usePanelState({
    storageKey: "dbunk.panel.row-details",
    defaultSize: 340,
    min: 280,
    max: () => Math.round(window.innerWidth * 0.5),
    snapThreshold: 140,
  });

  if (!visible) return null;

  const hasMultiple = selectedRowCount > 1;
  const subtitle = hasMultiple
    ? `${selectedRowCount} rows selected`
    : selectedRowCount === 1
      ? "1 selected"
      : "First visible row";

  return (
    <Panel side="right" state={panel} ariaLabel="Resize row details panel">
      <div
        data-testid="row-details-panel"
        className="flex h-full min-h-0 flex-col bg-surface-window text-xs"
      >
        <div className="flex h-8 shrink-0 items-center justify-between gap-2 border-b border-border-subtle px-2">
          <span className="truncate text-2xs font-semibold uppercase tracking-[0.12em] text-text-muted">
            Row Details
          </span>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="Close row details"
            onClick={onClose}
          >
            <IconX />
          </Button>
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-3 p-4">
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
      </div>
    </Panel>
  );
}

function MultiSelectMessage() {
  return (
    <div className="rounded-md border border-accent/20 bg-accent/10 p-3 text-text-muted">
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
      <div className="text-2xs font-medium uppercase tracking-[0.12em] text-text-muted">
        {column}
      </div>
      <div className="mt-1 truncate font-mono text-xs text-foreground">
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
      <div className="text-2xs font-medium uppercase tracking-[0.12em] text-text-muted">
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
