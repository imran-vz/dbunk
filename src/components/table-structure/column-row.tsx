import { IconCheck, IconKey, IconTrash } from "@tabler/icons-react";
import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { ColumnChangeKind } from "@/lib/ddl";
import type { ColumnInfo } from "@/lib/store";
import { cn } from "@/lib/utils";

import { taggedDefault } from "./columns-section";
import { MiniSelect, type PgStructureOps } from "./shared";

interface ColumnRowProps {
  column: ColumnInfo;
  editable: boolean;
  /** Present for editable PostgreSQL tables — actions queue typed ops. */
  pg: PgStructureOps | null;
  onQueueChange: (change: ColumnChangeKind) => void;
}

export function ColumnRow({
  column,
  editable,
  pg,
  onQueueChange,
}: ColumnRowProps) {
  const [editing, setEditing] = useState(false);

  return (
    <div
      data-testid={`structure-column-${column.name}`}
      className="px-3 py-2 text-xs transition hover:bg-surface-panel"
    >
      <div className="grid grid-cols-[minmax(0,1.15fr)_minmax(0,0.95fr)_minmax(0,1fr)_auto] items-center gap-3">
        <NameCell column={column} />
        <TypeCell
          column={column}
          editable={editable}
          onToggleNullable={() => {
            if (pg) {
              pg.queueOp({
                op: "setColumnNullable",
                schema: pg.schema,
                table: pg.table,
                name: column.name,
                nullable: !column.nullable,
              });
              return;
            }
            onQueueChange({
              kind: "set_nullable",
              columnName: column.name,
              nullable: !column.nullable,
            });
          }}
        />
        <DefaultCell column={column} />
        <RowActions
          column={column}
          editable={editable}
          editing={editing}
          onToggleEdit={() => setEditing((value) => !value)}
          onDrop={() => {
            if (pg) {
              pg.queueOp({
                op: "dropColumn",
                schema: pg.schema,
                table: pg.table,
                name: column.name,
                cascade: false,
              });
              return;
            }
            onQueueChange({ kind: "drop", columnName: column.name });
          }}
        />
      </div>
      {editable && editing ? (
        <ColumnEditPanel
          column={column}
          pg={pg}
          onQueueChange={onQueueChange}
          onDone={() => setEditing(false)}
        />
      ) : null}
    </div>
  );
}

function NameCell({ column }: { column: ColumnInfo }) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="text-xs tabular-nums text-muted-foreground">
        {column.ordinalPosition}
      </span>
      <span className="truncate font-mono text-foreground">{column.name}</span>
      {column.isPrimaryKey ? (
        <Badge variant="secondary" className="gap-1 px-1.5 text-2xs uppercase">
          <IconKey className="size-3" /> PK
        </Badge>
      ) : null}
    </div>
  );
}

function TypeCell({
  column,
  editable,
  onToggleNullable,
}: {
  column: ColumnInfo;
  editable: boolean;
  onToggleNullable: () => void;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="truncate font-mono text-info">{column.dataType}</span>
      {editable ? (
        <Button
          data-testid={`structure-toggle-nullable-${column.name}`}
          variant="ghost"
          size="sm"
          className={cn(
            "px-1.5 text-2xs uppercase tracking-wide",
            column.nullable ? "text-muted-foreground" : "text-danger",
          )}
          onClick={onToggleNullable}
        >
          {column.nullable ? "nullable" : "not null"}
        </Button>
      ) : (
        <span
          className={cn(
            "text-2xs uppercase tracking-wide",
            column.nullable ? "text-muted-foreground" : "text-danger",
          )}
        >
          {column.nullable ? "nullable" : "not null"}
        </span>
      )}
    </div>
  );
}

function DefaultCell({ column }: { column: ColumnInfo }) {
  return (
    <div className="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
      {column.defaultValue ? (
        <span className="font-mono" title={`Default: ${column.defaultValue}`}>
          {column.defaultValue}
        </span>
      ) : (
        <span className="text-muted-foreground/60">—</span>
      )}
    </div>
  );
}

function RowActions({
  column,
  editable,
  editing,
  onToggleEdit,
  onDrop,
}: {
  column: ColumnInfo;
  editable: boolean;
  editing: boolean;
  onToggleEdit: () => void;
  onDrop: () => void;
}) {
  if (!editable) return <div className="flex items-center justify-end gap-1" />;
  return (
    <div className="flex items-center justify-end gap-1">
      <Button
        data-testid={`structure-edit-column-${column.name}`}
        variant="ghost"
        size="sm"
        className="px-1.5 text-xs"
        aria-label={`Edit column ${column.name}`}
        onClick={onToggleEdit}
      >
        {editing ? "Close" : "Edit"}
      </Button>
      <Button
        data-testid={`structure-drop-column-${column.name}`}
        variant="ghost"
        size="sm"
        className="px-1.5 text-danger hover:text-danger/80"
        aria-label={`Drop column ${column.name}`}
        onClick={onDrop}
      >
        <IconTrash />
      </Button>
    </div>
  );
}

interface ColumnEditPanelProps {
  column: ColumnInfo;
  pg: PgStructureOps | null;
  onQueueChange: (change: ColumnChangeKind) => void;
  onDone: () => void;
}

function ColumnEditPanel({
  column,
  pg,
  onQueueChange,
  onDone,
}: ColumnEditPanelProps) {
  const [renameValue, setRenameValue] = useState(column.name);
  const [typeValue, setTypeValue] = useState(column.dataType);
  const [usingValue, setUsingValue] = useState("");
  const [defaultValue, setDefaultValue] = useState(column.defaultValue ?? "");
  const [defaultKind, setDefaultKind] = useState<"literal" | "expression">(
    "expression",
  );

  return (
    <div
      data-testid={`structure-edit-row-${column.name}`}
      className="mt-2 grid gap-2 rounded-sm border border-border-subtle bg-surface-panel p-2 sm:grid-cols-3"
    >
      <EditField label="Rename">
        <Input
          data-testid={`structure-rename-input-${column.name}`}
          aria-label={`Rename column ${column.name}`}
          value={renameValue}
          onChange={(event) => setRenameValue(event.target.value)}
          className="h-6 font-mono text-xs"
        />
        <Button
          data-testid={`structure-rename-confirm-${column.name}`}
          variant="ghost"
          size="sm"
          disabled={renameValue === column.name || renameValue === ""}
          className="px-1.5"
          aria-label="Queue rename"
          onClick={() => {
            if (pg) {
              pg.queueOp({
                op: "renameColumn",
                schema: pg.schema,
                table: pg.table,
                name: column.name,
                newName: renameValue,
              });
            } else {
              onQueueChange({
                kind: "rename",
                columnName: column.name,
                newName: renameValue,
              });
            }
            setRenameValue(column.name);
            onDone();
          }}
        >
          <IconCheck />
        </Button>
      </EditField>
      <EditField label="Type">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <Input
            data-testid={`structure-type-input-${column.name}`}
            aria-label={`Change type of ${column.name}`}
            value={typeValue}
            onChange={(event) => setTypeValue(event.target.value)}
            className="h-6 font-mono text-xs text-info"
          />
          {pg ? (
            <Input
              data-testid={`structure-using-input-${column.name}`}
              aria-label={`USING expression for ${column.name}`}
              value={usingValue}
              onChange={(event) => setUsingValue(event.target.value)}
              placeholder="USING expression (optional)"
              className="h-6 font-mono text-xs"
            />
          ) : null}
        </div>
        <Button
          data-testid={`structure-type-confirm-${column.name}`}
          variant="ghost"
          size="sm"
          disabled={typeValue === column.dataType || typeValue === ""}
          className="px-1.5"
          aria-label="Queue type change"
          onClick={() => {
            if (pg) {
              pg.queueOp({
                op: "alterColumnType",
                schema: pg.schema,
                table: pg.table,
                name: column.name,
                newType: typeValue,
                using: usingValue.trim() === "" ? null : usingValue.trim(),
              });
            } else {
              onQueueChange({
                kind: "set_type",
                columnName: column.name,
                newType: typeValue,
              });
            }
            setTypeValue(column.dataType);
            setUsingValue("");
            onDone();
          }}
        >
          <IconCheck />
        </Button>
      </EditField>
      <EditField label="Default">
        <div className="flex min-w-0 flex-1 flex-col gap-1">
          <Input
            data-testid={`structure-default-input-${column.name}`}
            aria-label={`Set default for ${column.name}`}
            value={defaultValue}
            onChange={(event) => setDefaultValue(event.target.value)}
            placeholder="(none)"
            className="h-6 font-mono text-xs"
          />
          {pg ? (
            <MiniSelect
              testId={`structure-default-kind-${column.name}`}
              ariaLabel={`Default kind for ${column.name}`}
              value={defaultKind}
              options={[
                { value: "literal", label: "Literal" },
                { value: "expression", label: "Expression" },
              ]}
              onChange={(value) =>
                setDefaultKind(value === "literal" ? "literal" : "expression")
              }
            />
          ) : null}
        </div>
        <Button
          data-testid={`structure-default-confirm-${column.name}`}
          variant="ghost"
          size="sm"
          disabled={defaultValue === (column.defaultValue ?? "")}
          className="px-1.5"
          aria-label="Queue default change"
          onClick={() => {
            if (pg) {
              pg.queueOp({
                op: "setColumnDefault",
                schema: pg.schema,
                table: pg.table,
                name: column.name,
                default: taggedDefault(defaultKind, defaultValue),
              });
            } else {
              onQueueChange({
                kind: "set_default",
                columnName: column.name,
                default: defaultValue === "" ? null : defaultValue,
              });
            }
            setDefaultValue(column.defaultValue ?? "");
            onDone();
          }}
        >
          <IconCheck />
        </Button>
      </EditField>
    </div>
  );
}

function EditField({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-2xs uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <div className="flex items-center gap-1">{children}</div>
    </div>
  );
}
