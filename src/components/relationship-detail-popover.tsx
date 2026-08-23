import { IconX } from "@tabler/icons-react";

import type { SchemaForeignKey } from "@/lib/schema-graph";

interface RelationshipDetailPopoverProps {
  foreignKey: SchemaForeignKey;
  onClose: () => void;
}

const columnList = (columns: string[]): string =>
  columns.length > 0 ? columns.join(", ") : "—";

const yesNoUnknown = (value: boolean | undefined): string =>
  value === undefined ? "Unknown" : value ? "Yes" : "No";

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <dt className="shrink-0 text-muted-foreground">{label}</dt>
      <dd className="min-w-0 break-words text-right font-medium">{value}</dd>
    </div>
  );
}

/**
 * Relationship Detail Popover — authoritative backend metadata for the
 * Focused Relationship Edge. Deliberately excludes trigger metadata:
 * Trigger Indicators live on Table Cards and Column Rows.
 */
/* oxlint-disable jsx-a11y/prefer-tag-over-role -- This anchored nonmodal popover does not use native dialog lifecycle behavior. */
export function RelationshipDetailPopover({
  foreignKey,
  onClose,
}: RelationshipDetailPopoverProps) {
  return (
    /* oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- This is a nonmodal anchored popover, so native dialog lifecycle semantics do not apply. */
    <div
      role="dialog"
      data-testid="relationship-detail-popover"
      aria-label={`Relationship details for ${foreignKey.constraintName}`}
      className="absolute right-2 top-2 z-20 w-72 rounded-md border border-border bg-card p-0 text-2xs text-card-foreground shadow-lg"
    >
      <div className="flex items-center gap-2 border-b border-border/70 px-2.5 py-1.5">
        <span
          className="min-w-0 flex-1 truncate font-semibold"
          title={foreignKey.constraintName}
        >
          {foreignKey.constraintName}
        </span>
        <button
          type="button"
          aria-label="Close relationship details"
          onClick={onClose}
          className="shrink-0 rounded-sm p-0.5 text-muted-foreground hover:bg-muted hover:text-foreground"
        >
          <IconX className="size-3" aria-hidden />
        </button>
      </div>
      <dl className="flex flex-col gap-1 px-2.5 py-2">
        <DetailRow
          label="Type"
          value={foreignKey.relationshipType ?? "foreign key"}
        />
        <DetailRow
          label="Cardinality"
          value={foreignKey.cardinality ?? "unknown"}
        />
        {foreignKey.cardinalityReason ? (
          <div className="text-muted-foreground">
            {foreignKey.cardinalityReason}
          </div>
        ) : null}
        <DetailRow
          label="Referencing"
          value={`${foreignKey.fromSchema}.${foreignKey.fromTable} (${columnList(
            foreignKey.fromColumns,
          )})`}
        />
        <DetailRow
          label="Referenced"
          value={`${foreignKey.toSchema}.${foreignKey.toTable} (${columnList(
            foreignKey.toColumns,
          )})`}
        />
        <DetailRow label="On update" value={foreignKey.onUpdate ?? "—"} />
        <DetailRow label="On delete" value={foreignKey.onDelete ?? "—"} />
        <DetailRow
          label="FK columns nullable"
          value={yesNoUnknown(foreignKey.fkColumnsNullable)}
        />
        <DetailRow
          label="FK columns unique"
          value={yesNoUnknown(foreignKey.fkColumnsUnique)}
        />
        {foreignKey.isJunctionParticipant ? (
          <div
            data-testid="relationship-junction-participation"
            className="mt-1 rounded-sm border border-primary/40 bg-primary/10 px-1.5 py-1 text-primary"
          >
            Participates in a junction-table (many-to-many) path.
          </div>
        ) : null}
      </dl>
    </div>
  );
}
/* oxlint-enable jsx-a11y/prefer-tag-over-role */
