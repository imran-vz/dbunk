import { IconPlus } from "@tabler/icons-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { ColumnChangeKind, NewColumn } from "@/lib/ddl";
import type { ColumnInfo, PgDefaultValue } from "@/lib/store";
import { cn } from "@/lib/utils";

import { ColumnRow } from "./column-row";
import { EmptyRow, MiniSelect, type PgStructureOps, Section } from "./shared";

interface ColumnsSectionProps {
  columns: ColumnInfo[];
  editable: boolean;
  /** Present for editable PostgreSQL tables — forms queue typed ops. */
  pg: PgStructureOps | null;
  onQueueChange: (change: ColumnChangeKind) => void;
}

export function ColumnsSection({
  columns,
  editable,
  pg,
  onQueueChange,
}: ColumnsSectionProps) {
  const [showAddForm, setShowAddForm] = useState(false);
  return (
    <Section
      title="Columns"
      testId="structure-columns"
      action={
        editable ? (
          <Button
            data-testid="structure-add-column"
            variant="outline"
            size="sm"
            className="gap-1"
            onClick={() => setShowAddForm((value) => !value)}
          >
            <IconPlus />
            Add column
          </Button>
        ) : null
      }
    >
      {editable && showAddForm ? (
        <AddColumnForm
          pg={pg}
          onCancel={() => setShowAddForm(false)}
          onSubmit={(column) => {
            onQueueChange({ kind: "add", column });
            setShowAddForm(false);
          }}
        />
      ) : null}
      {columns.length === 0 ? (
        <EmptyRow>No columns reported by the database.</EmptyRow>
      ) : (
        <div className="divide-y divide-border-subtle">
          {columns.map((column) => (
            <ColumnRow
              key={column.name}
              column={column}
              editable={editable}
              pg={pg}
              onQueueChange={onQueueChange}
            />
          ))}
        </div>
      )}
    </Section>
  );
}

export const taggedDefault = (
  kind: "literal" | "expression",
  value: string,
): PgDefaultValue | null =>
  value === ""
    ? null
    : kind === "literal"
      ? { kind: "literal", value }
      : { kind: "expression", sql: value };

interface AddColumnFormProps {
  pg: PgStructureOps | null;
  onSubmit: (column: NewColumn) => void;
  onCancel: () => void;
}

function AddColumnForm({ pg, onSubmit, onCancel }: AddColumnFormProps) {
  const [name, setName] = useState("");
  const [dataType, setDataType] = useState("text");
  const [nullable, setNullable] = useState(true);
  const [defaultValue, setDefaultValue] = useState("");
  const [defaultKind, setDefaultKind] = useState<"literal" | "expression">(
    "literal",
  );

  const submit = () => {
    if (name.trim() === "" || dataType.trim() === "") return;
    if (pg) {
      pg.queueOp({
        op: "addColumn",
        schema: pg.schema,
        table: pg.table,
        column: {
          name: name.trim(),
          dataType: dataType.trim(),
          nullable,
          default: taggedDefault(defaultKind, defaultValue),
        },
      });
      onCancel();
      return;
    }
    onSubmit({
      name: name.trim(),
      dataType: dataType.trim(),
      nullable,
      defaultValue: defaultValue === "" ? null : defaultValue,
    });
  };

  return (
    <div
      data-testid="structure-add-column-form"
      className="flex flex-wrap items-center gap-1.5 border-b border-border-subtle bg-surface-panel px-3 py-2 text-xs"
    >
      <Input
        data-testid="structure-add-column-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        placeholder="column_name"
        className="h-6 max-w-[12rem] font-mono text-xs"
      />
      <Input
        data-testid="structure-add-column-type"
        value={dataType}
        onChange={(event) => setDataType(event.target.value)}
        placeholder="text"
        className="h-6 max-w-[10rem] font-mono text-xs"
      />
      <Button
        data-testid="structure-add-column-nullable"
        variant="ghost"
        size="sm"
        className={cn(
          "px-1.5 text-2xs uppercase tracking-wide",
          nullable ? "text-muted-foreground" : "text-danger",
        )}
        onClick={() => setNullable((value) => !value)}
      >
        {nullable ? "nullable" : "not null"}
      </Button>
      {pg ? (
        <MiniSelect
          testId="structure-add-column-default-kind"
          ariaLabel="Default value kind"
          value={defaultKind}
          options={[
            { value: "literal", label: "Literal" },
            { value: "expression", label: "Expression" },
          ]}
          onChange={(value) =>
            setDefaultKind(value === "expression" ? "expression" : "literal")
          }
        />
      ) : null}
      <Input
        data-testid="structure-add-column-default"
        value={defaultValue}
        onChange={(event) => setDefaultValue(event.target.value)}
        placeholder="default (optional)"
        className="h-6 max-w-[12rem] font-mono text-xs"
      />
      <div className="ml-auto flex items-center gap-1">
        <Button
          data-testid="structure-add-column-cancel"
          variant="ghost"
          size="sm"
          onClick={onCancel}
        >
          Cancel
        </Button>
        <Button
          data-testid="structure-add-column-submit"
          variant="default"
          size="sm"
          onClick={submit}
        >
          Queue add
        </Button>
      </div>
    </div>
  );
}
