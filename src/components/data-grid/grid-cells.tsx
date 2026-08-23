/**
 * Header and body cell rendering for the virtualized data grid
 * (DESIGN-SYSTEM §5.4). Cells are 12px mono with `tnum`; numbers
 * right-align, NULL renders as a faint italic keyword, multi-line
 * values collapse to one line with a `↵` indicator, and truncation is
 * an in-cell ellipsis (the value inspector shows the full value).
 */

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- virtualized ARIA grid: cells are absolutely positioned divs/buttons; table tags cannot be virtualized. */

import {
  IconArrowRight,
  IconCircleDot,
  IconCornerDownLeft,
  IconExclamationCircle,
  IconKey,
  IconLink,
  IconMath,
  IconStar,
  IconTerminal2,
} from "@tabler/icons-react";
import { memo, useEffect, useState } from "react";

import {
  CELL_EDITORS,
  specializedCellKind,
} from "@/components/data-grid/cell-editors";
import type { CellAlignment } from "@/components/data-grid/grid-model";
import { Input } from "@/components/ui/input";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { GRID_NULL_SENTINEL } from "@/lib/table-browse";
import { cn } from "@/lib/utils";

export type ForeignKeyTarget = {
  schema: string;
  table: string;
  column: string;
};

export type ColumnHeaderMeta = {
  isPrimaryKey?: boolean;
  isForeignKey?: boolean;
  isIndexed?: boolean;
  isUnique?: boolean;
  notNull?: boolean;
  hasDefault?: boolean;
  dataType?: string;
  /** `"MATERIALIZED"` / `"ALIAS"` / `"EPHEMERAL"` for ClickHouse
   *  derived columns; `undefined` otherwise. */
  derivationKind?: string | null;
  /** Free-form description appended to the tooltip — e.g. the
   *  referenced FK target or the default expression. */
  description?: string;
  /** When present, FK cells render a hover arrow that calls
   *  `onFollowForeignKey` with this target + the cell value. */
  foreignKeyTarget?: ForeignKeyTarget;
};

/** Preview cap — full values live in the value inspector. */
const CELL_DISPLAY_CHARACTER_LIMIT = 100;

type MetaRow = {
  key: string;
  Icon: typeof IconKey;
  iconClass: string;
  label: string;
  detail?: string;
};

function buildMetaRows(meta: ColumnHeaderMeta): MetaRow[] {
  const rows: MetaRow[] = [];
  if (meta.isPrimaryKey) {
    rows.push({
      key: "pk",
      Icon: IconKey,
      iconClass: "text-warning",
      label: "Primary key",
    });
  }
  if (meta.isForeignKey) {
    rows.push({
      key: "fk",
      Icon: IconLink,
      iconClass: "text-info",
      label: "Foreign key",
      detail: meta.description,
    });
  }
  if (meta.isUnique) {
    rows.push({
      key: "unique",
      Icon: IconStar,
      iconClass: "text-accent",
      label: "Unique index",
    });
  } else if (meta.isIndexed) {
    rows.push({
      key: "indexed",
      Icon: IconTerminal2,
      iconClass: "text-text-muted",
      label: "Indexed",
    });
  }
  if (meta.notNull) {
    rows.push({
      key: "notnull",
      Icon: IconExclamationCircle,
      iconClass: "text-danger",
      label: "NOT NULL",
    });
  }
  if (meta.hasDefault) {
    rows.push({
      key: "default",
      Icon: IconCircleDot,
      iconClass: "text-warning",
      label: "Has default",
    });
  }
  if (meta.derivationKind) {
    rows.push({
      key: "derived",
      Icon: IconMath,
      iconClass: "text-info",
      label: "Derived",
      detail: meta.derivationKind,
    });
  }
  return rows;
}

/**
 * Column header label: 13px sans name + 11px muted type hint + role
 * icons, with the structural tooltip. In browse mode the label is a
 * sort button (click cycles the server-side sort).
 */
export function ColumnHeaderLabel({
  name,
  meta,
  onSortClick,
}: {
  name: string;
  meta: ColumnHeaderMeta | undefined;
  onSortClick?: (event: React.MouseEvent<HTMLElement>) => void;
}) {
  const rows = meta ? buildMetaRows(meta) : [];

  const label = (
    <span className="flex min-w-0 items-center gap-1">
      {meta?.isPrimaryKey ? (
        <IconKey
          className="size-3 shrink-0 text-warning"
          aria-label="primary key"
        />
      ) : null}
      {meta?.isForeignKey ? (
        <IconLink
          className="size-3 shrink-0 text-info"
          aria-label="foreign key"
        />
      ) : null}
      <span className="truncate text-xs font-medium text-text-secondary">
        {name}
      </span>
      {meta?.dataType ? (
        <span className="truncate font-mono text-2xs text-text-muted">
          {meta.dataType}
        </span>
      ) : null}
    </span>
  );

  if (!meta) {
    if (!onSortClick) return label;
    return (
      <button
        type="button"
        className="flex h-full w-full min-w-0 items-center px-2 text-left"
        onClick={onSortClick}
      >
        {label}
      </button>
    );
  }

  if (rows.length === 0 && !meta.dataType && !onSortClick) return label;

  return (
    <Tooltip>
      <TooltipTrigger
        render={(props) => (
          <button
            type="button"
            {...props}
            className={cn(
              "flex h-full w-full min-w-0 items-center bg-transparent p-0 px-2 text-left outline-none",
              onSortClick ? "cursor-pointer" : "cursor-help",
            )}
            onClick={(event) => {
              props.onClick?.(event);
              onSortClick?.(event);
            }}
          >
            {label}
          </button>
        )}
      />
      <TooltipContent className="w-64 p-0">
        <div className="flex items-baseline gap-2 border-b border-border-subtle/60 px-3 py-2">
          <span className="truncate font-mono text-xs font-semibold text-foreground">
            {name}
          </span>
          {meta.dataType ? (
            <span className="ml-auto shrink-0 rounded-sm bg-info/15 px-1.5 py-0.5 font-mono text-2xs text-info">
              {meta.dataType}
            </span>
          ) : null}
        </div>
        {rows.length > 0 ? (
          <ul className="space-y-1 px-3 py-2">
            {rows.map(({ key, Icon, iconClass, label: rowLabel, detail }) => (
              <li key={key} className="flex items-baseline gap-2 text-2xs">
                <Icon
                  className={cn("size-3 shrink-0 self-center", iconClass)}
                  aria-hidden
                />
                <span className="text-foreground">{rowLabel}</span>
                {detail ? (
                  <span className="truncate font-mono text-text-muted">
                    {detail}
                  </span>
                ) : null}
              </li>
            ))}
          </ul>
        ) : null}
      </TooltipContent>
    </Tooltip>
  );
}

export interface GridBodyCellProps {
  value: string;
  originalValue: string;
  isDirty: boolean;
  alignment: CellAlignment;
  isFocused: boolean;
  isSelected: boolean;
  isEditing: boolean;
  editSeed?: string;
  editable: boolean;
  /** Staged-deletion strikethrough (§5.4) — applied to the cell text. */
  lineThrough?: boolean;
  readOnlyReason?: string;
  columnName: string;
  columnType?: string;
  style: React.CSSProperties;
  className?: string;
  cellKey: string;
  ariaColIndex: number;
  onActivate: (extend: boolean) => void;
  onStartEdit: () => void;
  onCommit: (value: string, moveRight: boolean) => void;
  onCancelEdit: () => void;
  foreignKeyTarget?: ForeignKeyTarget;
  onFollowForeignKey?: () => void;
}

function GridBodyCellImpl({
  value,
  originalValue,
  isDirty,
  alignment,
  isFocused,
  isSelected,
  isEditing,
  editSeed,
  editable,
  lineThrough,
  readOnlyReason,
  columnName,
  columnType,
  style,
  className,
  cellKey,
  ariaColIndex,
  onActivate,
  onStartEdit,
  onCommit,
  onCancelEdit,
  foreignKeyTarget,
  onFollowForeignKey,
}: GridBodyCellProps) {
  // Track the specialized-editor overlay locally; plain inline editing
  // state is owned by the grid so keyboard flows can drive it.
  const [overlayOpen, setOverlayOpen] = useState(false);
  const isNull = value === GRID_NULL_SENTINEL;
  const newlineIndex = value.indexOf("\n");
  const isMultiline = newlineIndex !== -1;
  const preview = (isMultiline ? value.slice(0, newlineIndex) : value).slice(
    0,
    CELL_DISPLAY_CHARACTER_LIMIT,
  );
  const specializedKind = specializedCellKind(columnType);
  const SpecializedEditor = specializedKind
    ? CELL_EDITORS[specializedKind]
    : null;

  // Keyboard/dblclick asked for an edit on a specialized column —
  // route into the overlay editor instead of the inline input.
  const wantsOverlay = isEditing && editable && SpecializedEditor !== null;
  useEffect(() => {
    if (wantsOverlay) setOverlayOpen(true);
  }, [wantsOverlay]);

  const canFollowFk =
    foreignKeyTarget !== undefined &&
    onFollowForeignKey !== undefined &&
    value !== "" &&
    !isNull;

  if (isEditing && editable && !SpecializedEditor) {
    return (
      <div
        style={style}
        role="gridcell"
        aria-colindex={ariaColIndex}
        className={cn("absolute", className)}
        data-grid-editing
      >
        <Input
          // oxlint-disable-next-line jsx-a11y/no-autofocus -- keyboard-initiated cell editor; focus must land in the input it opened.
          autoFocus
          className="h-full w-full rounded-none border-0 bg-surface-input px-2 py-0 font-mono text-xs shadow-none focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-accent"
          defaultValue={editSeed ?? value}
          onFocus={(event) => {
            // Seeded edits (typing replaces) keep the caret at the end;
            // plain edits select-all like DataGrip.
            if (editSeed === undefined) event.currentTarget.select();
          }}
          onBlur={(event) => onCommit(event.target.value, false)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onCommit(event.currentTarget.value, false);
            } else if (event.key === "Tab") {
              event.preventDefault();
              onCommit(event.currentTarget.value, true);
            } else if (event.key === "Escape") {
              event.stopPropagation();
              onCancelEdit();
            }
          }}
        />
      </div>
    );
  }

  return (
    <div
      style={style}
      role="gridcell"
      aria-colindex={ariaColIndex}
      aria-selected={isSelected || isFocused || undefined}
      className={cn("group/cell absolute flex items-center", className)}
    >
      <button
        type="button"
        data-grid-cell={cellKey}
        tabIndex={isFocused ? 0 : -1}
        title={
          readOnlyReason ?? (isDirty ? `Original: ${originalValue}` : undefined)
        }
        className={cn(
          "h-full w-full truncate border-r border-b border-border-subtle px-2 text-left font-mono text-xs text-text-secondary tabular-nums outline-none",
          alignment === "right" && "text-right",
          alignment === "center" && "text-center",
          isDirty && "bg-warning/10 text-warning",
          isSelected && !isDirty && "bg-accent-overlay/60",
          isFocused && "shadow-[inset_0_0_0_1px_var(--accent)]",
          lineThrough && "line-through",
          readOnlyReason && "text-text-muted/70",
          canFollowFk && "pr-6",
        )}
        onClick={(event) => onActivate(event.shiftKey)}
        onDoubleClick={() => {
          if (editable) onStartEdit();
        }}
        onContextMenu={() => onActivate(false)}
      >
        {isNull && !isDirty ? (
          <span className="italic text-text-disabled">NULL</span>
        ) : (
          <>
            {preview}
            {isMultiline ? (
              <IconCornerDownLeft
                aria-label="multi-line value"
                className="ml-1 inline size-3 text-text-muted"
              />
            ) : null}
          </>
        )}
      </button>
      {canFollowFk ? (
        <button
          type="button"
          tabIndex={-1}
          onClick={(event) => {
            event.stopPropagation();
            onFollowForeignKey?.();
          }}
          className="absolute top-1/2 right-1 -translate-y-1/2 rounded-sm border border-border-subtle bg-surface-panel-elevated px-1 py-0.5 text-text-muted opacity-0 transition-opacity group-hover/cell:opacity-100 hover:text-foreground focus:opacity-100 focus:outline-none"
          aria-label={`Follow foreign key to ${foreignKeyTarget.schema}.${foreignKeyTarget.table}`}
        >
          <IconArrowRight className="size-3" />
        </button>
      ) : null}
      {overlayOpen && SpecializedEditor ? (
        <SpecializedEditor
          initialValue={value}
          columnName={columnName}
          onSave={(literal) => {
            setOverlayOpen(false);
            onCommit(literal, false);
          }}
          onCancel={() => {
            setOverlayOpen(false);
            onCancelEdit();
          }}
        />
      ) : null}
    </div>
  );
}

export const GridBodyCell = memo(GridBodyCellImpl);
