import {
  IconAlertTriangle,
  IconCheck,
  IconKey,
  IconLink,
  IconPlus,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  type ColumnChangeKind,
  classifyDestructive,
  generatePostgresDdl,
  type NewColumn,
  type PendingChange,
} from "@/lib/ddl/postgres";
import {
  type ColumnInfo,
  type ConstraintInfo,
  type DatabaseEngine,
  type ForeignKeyInfo,
  type IndexInfo,
  type StructureCapabilities,
  type StructureCommitStatus,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface TableStructureViewProps {
  connectionId: string;
  schema: string;
  tableName: string;
  className?: string;
}

const fallbackCapabilities: StructureCapabilities = {
  columns: false,
  primaryKey: false,
  foreignKeys: false,
  indexes: false,
  constraints: false,
};

export function TableStructureView({
  connectionId,
  schema,
  tableName,
  className,
}: TableStructureViewProps) {
  const key = tableStructureKey(connectionId, schema, tableName);
  const loadTableStructure = useAppStore((state) => state.loadTableStructure);
  const addPendingStructureChange = useAppStore(
    (state) => state.addPendingStructureChange,
  );
  const removePendingStructureChange = useAppStore(
    (state) => state.removePendingStructureChange,
  );
  const commitStructureChanges = useAppStore(
    (state) => state.commitStructureChanges,
  );
  const structure = useAppStore((state) => state.tableStructure[key]);
  const status = useAppStore((state) => state.tableStructureStatus[key]);
  const pendingChanges = useAppStore(
    (state) => state.pendingStructureChanges[key],
  );
  const commitStatus = useAppStore((state) => state.structureCommitStatus[key]);
  const engine = useAppStore(
    (state) =>
      state.connections.find((connection) => connection.id === connectionId)
        ?.engine,
  );

  const [showPreview, setShowPreview] = useState(false);

  useEffect(() => {
    if (connectionId && schema && tableName) {
      void loadTableStructure(connectionId, schema, tableName);
    }
  }, [connectionId, schema, tableName, loadTableStructure]);

  const isLoading = status?.state === "loading";
  const errorMessage = status?.state === "error" ? status.error : null;
  const capabilities = structure?.capabilities ?? fallbackCapabilities;
  const editable = engine === "PostgreSQL";
  const pending = pendingChanges ?? [];

  const handleRetry = () => {
    if (connectionId && schema && tableName) {
      void loadTableStructure(connectionId, schema, tableName);
    }
  };

  const queueChange = (change: ColumnChangeKind) => {
    addPendingStructureChange(key, { schema, table: tableName, change });
  };

  const previewSql = useMemo(
    () =>
      generatePostgresDdl(
        schema,
        tableName,
        pending.map((entry) => entry.change),
      ),
    [schema, tableName, pending],
  );

  const handleCommit = async () => {
    if (pending.length === 0) {
      return;
    }
    const { destructive } = classifyDestructive(
      pending.map((entry) => entry.change),
    );
    if (destructive.length > 0) {
      const summary = destructive
        .map((change) => describeChange(change))
        .join("\n");
      const ok = window.confirm(
        `These changes are destructive and may lose data:\n\n${summary}\n\nProceed?`,
      );
      if (!ok) {
        return;
      }
    }
    await commitStructureChanges(key);
  };

  return (
    <div
      className={cn(
        "flex h-full min-h-0 flex-col overflow-hidden bg-background",
        className,
      )}
    >
      {isLoading ? (
        <div
          data-testid="structure-loading"
          className="h-0.5 w-full animate-pulse bg-primary"
        />
      ) : null}
      {errorMessage ? (
        <div
          data-testid="structure-error"
          role="alert"
          className="flex items-center gap-2 border-b border-destructive/40 bg-destructive/10 px-4 py-2 text-xs text-destructive"
        >
          <IconAlertTriangle className="size-4" />
          <span>Failed to load structure: {errorMessage}</span>
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 px-2 text-xs"
            onClick={handleRetry}
          >
            Retry
          </Button>
        </div>
      ) : null}
      <div className="flex-1 overflow-auto p-6">
        <div className="mx-auto max-w-4xl space-y-8">
          <header className="space-y-1">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span>{schema}</span>
              <span>·</span>
              <span>{tableName}</span>
            </div>
            <h2 className="font-mono text-lg font-semibold text-foreground">
              {tableName}
            </h2>
            {structure?.primaryKey && structure.primaryKey.length > 0 ? (
              <p className="text-xs text-muted-foreground">
                Primary key: {structure.primaryKey.join(", ")}
              </p>
            ) : null}
          </header>

          <ColumnsSection
            columns={structure?.columns ?? []}
            editable={editable}
            onQueueChange={queueChange}
          />

          {editable ? (
            <PendingChangesSection
              pending={pending}
              previewSql={previewSql}
              showPreview={showPreview}
              commitStatus={commitStatus}
              onTogglePreview={() => setShowPreview((value) => !value)}
              onRemove={(id) => removePendingStructureChange(key, id)}
              onCommit={() => {
                void handleCommit();
              }}
            />
          ) : null}

          <PrimaryKeySection
            primaryKey={structure?.primaryKey ?? null}
            supported={capabilities.primaryKey}
            engine={engine}
          />

          <ForeignKeysSection
            foreignKeys={structure?.foreignKeys ?? []}
            supported={capabilities.foreignKeys}
            engine={engine}
          />

          <IndexesSection
            indexes={structure?.indexes ?? []}
            supported={capabilities.indexes}
            engine={engine}
          />

          <ConstraintsSection
            constraints={structure?.constraints ?? []}
            supported={capabilities.constraints}
            engine={engine}
          />
        </div>
      </div>
    </div>
  );
}

function ColumnsSection({
  columns,
  editable,
  onQueueChange,
}: {
  columns: ColumnInfo[];
  editable: boolean;
  onQueueChange: (change: ColumnChangeKind) => void;
}) {
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
            className="h-7 gap-1 px-2 text-xs"
            onClick={() => setShowAddForm((value) => !value)}
          >
            <IconPlus className="size-3.5" />
            Add column
          </Button>
        ) : null
      }
    >
      {editable && showAddForm ? (
        <AddColumnForm
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
        <div className="divide-y divide-border/50">
          {columns.map((column) => (
            <ColumnRow
              key={column.name}
              column={column}
              editable={editable}
              onQueueChange={onQueueChange}
            />
          ))}
        </div>
      )}
    </Section>
  );
}

function ColumnRow({
  column,
  editable,
  onQueueChange,
}: {
  column: ColumnInfo;
  editable: boolean;
  onQueueChange: (change: ColumnChangeKind) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [renameValue, setRenameValue] = useState(column.name);
  const [typeValue, setTypeValue] = useState(column.dataType);
  const [defaultValue, setDefaultValue] = useState(column.defaultValue ?? "");

  const cancelEditing = () => {
    setEditing(false);
    setRenameValue(column.name);
    setTypeValue(column.dataType);
    setDefaultValue(column.defaultValue ?? "");
  };

  return (
    <div
      data-testid={`structure-column-${column.name}`}
      className="px-4 py-3 text-sm"
    >
      <div className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <span className="text-xs tabular-nums text-muted-foreground">
            {column.ordinalPosition}
          </span>
          <span className="truncate font-mono text-foreground">
            {column.name}
          </span>
          {column.isPrimaryKey ? (
            <Badge
              variant="secondary"
              className="gap-1 px-1.5 text-[0.625rem] uppercase"
            >
              <IconKey className="size-3" /> PK
            </Badge>
          ) : null}
        </div>
        <div className="flex items-center gap-2 min-w-0">
          <span className="truncate font-mono text-blue-400">
            {column.dataType}
          </span>
          {editable ? (
            <Button
              data-testid={`structure-toggle-nullable-${column.name}`}
              variant="ghost"
              size="sm"
              className={cn(
                "h-7 px-1.5 text-[0.625rem] uppercase tracking-wide",
                column.nullable ? "text-muted-foreground" : "text-rose-400",
              )}
              onClick={() =>
                onQueueChange({
                  kind: "set_nullable",
                  columnName: column.name,
                  nullable: !column.nullable,
                })
              }
            >
              {column.nullable ? "nullable" : "not null"}
            </Button>
          ) : column.nullable ? (
            <span className="text-[0.625rem] uppercase tracking-wide text-muted-foreground">
              nullable
            </span>
          ) : (
            <span className="text-[0.625rem] uppercase tracking-wide text-rose-400">
              not null
            </span>
          )}
        </div>
        <div className="flex items-center gap-2 min-w-0 text-xs text-muted-foreground">
          {column.defaultValue ? (
            <span
              className="font-mono"
              title={`Default: ${column.defaultValue}`}
            >
              {column.defaultValue}
            </span>
          ) : (
            <span className="text-muted-foreground/60">—</span>
          )}
        </div>
        <div className="flex items-center justify-end gap-1">
          {editable ? (
            <>
              <Button
                data-testid={`structure-edit-column-${column.name}`}
                variant="ghost"
                size="sm"
                className="h-7 px-1.5 text-xs"
                aria-label={`Edit column ${column.name}`}
                onClick={() => (editing ? cancelEditing() : setEditing(true))}
              >
                {editing ? "Close" : "Edit"}
              </Button>
              <Button
                data-testid={`structure-drop-column-${column.name}`}
                variant="ghost"
                size="sm"
                className="h-7 px-1.5 text-rose-400 hover:text-rose-300"
                aria-label={`Drop column ${column.name}`}
                onClick={() =>
                  onQueueChange({ kind: "drop", columnName: column.name })
                }
              >
                <IconTrash className="size-3.5" />
              </Button>
            </>
          ) : null}
        </div>
      </div>
      {editable && editing ? (
        <div
          data-testid={`structure-edit-row-${column.name}`}
          className="mt-3 grid gap-2 rounded-md border border-border/50 bg-muted/20 p-3 sm:grid-cols-3"
        >
          <div className="flex flex-col gap-1">
            <span className="text-[0.625rem] uppercase tracking-wide text-muted-foreground">
              Rename
            </span>
            <div className="flex items-center gap-1">
              <Input
                data-testid={`structure-rename-input-${column.name}`}
                aria-label={`Rename column ${column.name}`}
                value={renameValue}
                onChange={(event) => setRenameValue(event.target.value)}
                className="h-7 font-mono text-xs"
              />
              <Button
                data-testid={`structure-rename-confirm-${column.name}`}
                variant="ghost"
                size="sm"
                disabled={renameValue === column.name || renameValue === ""}
                className="h-7 px-1.5"
                aria-label="Queue rename"
                onClick={() => {
                  onQueueChange({
                    kind: "rename",
                    columnName: column.name,
                    newName: renameValue,
                  });
                  setRenameValue(column.name);
                  setEditing(false);
                }}
              >
                <IconCheck className="size-3.5" />
              </Button>
            </div>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-[0.625rem] uppercase tracking-wide text-muted-foreground">
              Type
            </span>
            <div className="flex items-center gap-1">
              <Input
                data-testid={`structure-type-input-${column.name}`}
                aria-label={`Change type of ${column.name}`}
                value={typeValue}
                onChange={(event) => setTypeValue(event.target.value)}
                className="h-7 font-mono text-xs text-blue-400"
              />
              <Button
                data-testid={`structure-type-confirm-${column.name}`}
                variant="ghost"
                size="sm"
                disabled={typeValue === column.dataType || typeValue === ""}
                className="h-7 px-1.5"
                aria-label="Queue type change"
                onClick={() => {
                  onQueueChange({
                    kind: "set_type",
                    columnName: column.name,
                    newType: typeValue,
                  });
                  setTypeValue(column.dataType);
                  setEditing(false);
                }}
              >
                <IconCheck className="size-3.5" />
              </Button>
            </div>
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-[0.625rem] uppercase tracking-wide text-muted-foreground">
              Default
            </span>
            <div className="flex items-center gap-1">
              <Input
                data-testid={`structure-default-input-${column.name}`}
                aria-label={`Set default for ${column.name}`}
                value={defaultValue}
                onChange={(event) => setDefaultValue(event.target.value)}
                placeholder="(none)"
                className="h-7 font-mono text-xs"
              />
              <Button
                data-testid={`structure-default-confirm-${column.name}`}
                variant="ghost"
                size="sm"
                disabled={defaultValue === (column.defaultValue ?? "")}
                className="h-7 px-1.5"
                aria-label="Queue default change"
                onClick={() => {
                  onQueueChange({
                    kind: "set_default",
                    columnName: column.name,
                    default: defaultValue === "" ? null : defaultValue,
                  });
                  setDefaultValue(column.defaultValue ?? "");
                  setEditing(false);
                }}
              >
                <IconCheck className="size-3.5" />
              </Button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function AddColumnForm({
  onSubmit,
  onCancel,
}: {
  onSubmit: (column: NewColumn) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState("");
  const [dataType, setDataType] = useState("text");
  const [nullable, setNullable] = useState(true);
  const [defaultValue, setDefaultValue] = useState("");

  const submit = () => {
    if (name.trim() === "" || dataType.trim() === "") {
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
      className="flex flex-wrap items-center gap-2 border-b border-border/50 bg-muted/20 px-4 py-3 text-sm"
    >
      <Input
        data-testid="structure-add-column-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        placeholder="column_name"
        className="h-7 max-w-[12rem] font-mono text-xs"
      />
      <Input
        data-testid="structure-add-column-type"
        value={dataType}
        onChange={(event) => setDataType(event.target.value)}
        placeholder="text"
        className="h-7 max-w-[10rem] font-mono text-xs"
      />
      <Button
        data-testid="structure-add-column-nullable"
        variant="ghost"
        size="sm"
        className={cn(
          "h-7 px-2 text-[0.625rem] uppercase tracking-wide",
          nullable ? "text-muted-foreground" : "text-rose-400",
        )}
        onClick={() => setNullable((value) => !value)}
      >
        {nullable ? "nullable" : "not null"}
      </Button>
      <Input
        data-testid="structure-add-column-default"
        value={defaultValue}
        onChange={(event) => setDefaultValue(event.target.value)}
        placeholder="default (optional)"
        className="h-7 max-w-[12rem] font-mono text-xs"
      />
      <div className="ml-auto flex items-center gap-1">
        <Button
          data-testid="structure-add-column-cancel"
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={onCancel}
        >
          Cancel
        </Button>
        <Button
          data-testid="structure-add-column-submit"
          variant="default"
          size="sm"
          className="h-7 px-2 text-xs"
          onClick={submit}
        >
          Queue add
        </Button>
      </div>
    </div>
  );
}

function describeChange(change: ColumnChangeKind): string {
  switch (change.kind) {
    case "add":
      return `Add column ${change.column.name} ${change.column.dataType}`;
    case "drop":
      return `Drop column ${change.columnName}`;
    case "rename":
      return `Rename ${change.columnName} -> ${change.newName}`;
    case "set_type":
      return `Change type of ${change.columnName} to ${change.newType}`;
    case "set_nullable":
      return change.nullable
        ? `Make ${change.columnName} nullable`
        : `Make ${change.columnName} NOT NULL`;
    case "set_default": {
      if (change.default === null) {
        return `Drop default for ${change.columnName}`;
      }
      return `Set default of ${change.columnName} to ${change.default}`;
    }
  }
}

function PendingChangesSection({
  pending,
  previewSql,
  showPreview,
  commitStatus,
  onTogglePreview,
  onRemove,
  onCommit,
}: {
  pending: PendingChange[];
  previewSql: string;
  showPreview: boolean;
  commitStatus: StructureCommitStatus | undefined;
  onTogglePreview: () => void;
  onRemove: (id: string) => void;
  onCommit: () => void;
}) {
  const isRunning = commitStatus?.state === "running";
  const errorMessage =
    commitStatus?.state === "error" ? commitStatus.error : null;
  const successRuntime =
    commitStatus?.state === "success" ? commitStatus.runtimeMs : null;

  return (
    <Section
      title="Pending changes"
      testId="structure-pending-section"
      action={
        <div className="flex items-center gap-2">
          <Button
            data-testid="structure-preview-sql"
            variant="outline"
            size="sm"
            className="h-7 px-2 text-xs"
            onClick={onTogglePreview}
            disabled={pending.length === 0}
          >
            {showPreview ? "Hide SQL" : "Preview SQL"}
          </Button>
          <Button
            data-testid="structure-commit"
            variant="default"
            size="sm"
            className="h-7 px-2 text-xs"
            onClick={onCommit}
            disabled={pending.length === 0 || isRunning}
          >
            {isRunning ? "Committing..." : "Commit"}
          </Button>
        </div>
      }
    >
      {pending.length === 0 ? (
        <EmptyRow>No pending changes.</EmptyRow>
      ) : (
        <ul className="divide-y divide-border/50">
          {pending.map((entry) => (
            <li
              key={entry.id}
              data-testid={`structure-pending-${entry.id}`}
              className="flex items-center gap-2 px-4 py-2 text-sm"
            >
              <span className="truncate text-foreground">
                {describeChange(entry.change)}
              </span>
              <Button
                data-testid={`structure-remove-pending-${entry.id}`}
                variant="ghost"
                size="sm"
                className="ml-auto h-7 px-1.5 text-muted-foreground"
                aria-label="Remove pending change"
                onClick={() => onRemove(entry.id)}
              >
                <IconX className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      )}
      {showPreview && previewSql ? (
        <pre
          data-testid="structure-sql-preview"
          className="overflow-x-auto whitespace-pre-wrap border-t border-border/50 bg-muted/30 px-4 py-3 font-mono text-xs text-foreground"
        >
          {previewSql}
        </pre>
      ) : null}
      {errorMessage ? (
        <div
          data-testid="structure-commit-error"
          role="alert"
          className="flex items-center gap-2 border-t border-destructive/40 bg-destructive/10 px-4 py-2 text-xs text-destructive"
        >
          <IconAlertTriangle className="size-3.5" />
          <span>Commit failed: {errorMessage}</span>
        </div>
      ) : null}
      {successRuntime !== null && successRuntime !== undefined ? (
        <div
          data-testid="structure-commit-success"
          className="border-t border-border/50 bg-emerald-500/10 px-4 py-2 text-xs text-emerald-400"
        >
          Committed in {successRuntime} ms.
        </div>
      ) : null}
    </Section>
  );
}

function PrimaryKeySection({
  primaryKey,
  supported,
  engine,
}: {
  primaryKey: string[] | null;
  supported: boolean;
  engine: DatabaseEngine | undefined;
}) {
  return (
    <Section title="Primary key" testId="structure-primary-key">
      {!supported ? (
        <UnsupportedNotice engine={engine} feature="Primary key" />
      ) : !primaryKey || primaryKey.length === 0 ? (
        <EmptyRow>This table has no primary key.</EmptyRow>
      ) : (
        <div className="flex items-center gap-2 px-4 py-3 text-sm">
          <IconKey className="size-4 text-muted-foreground" />
          <span className="font-mono text-foreground">
            ({primaryKey.join(", ")})
          </span>
        </div>
      )}
    </Section>
  );
}

function ForeignKeysSection({
  foreignKeys,
  supported,
  engine,
}: {
  foreignKeys: ForeignKeyInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
}) {
  return (
    <Section title="Foreign keys" testId="structure-foreign-keys">
      {!supported ? (
        <UnsupportedNotice engine={engine} feature="Foreign keys" />
      ) : foreignKeys.length === 0 ? (
        <EmptyRow>No foreign keys defined.</EmptyRow>
      ) : (
        <div className="divide-y divide-border/50">
          {foreignKeys.map((fk) => (
            <div
              key={fk.name}
              className="flex flex-wrap items-center gap-2 px-4 py-3 text-sm"
            >
              <IconLink className="size-3.5 text-muted-foreground" />
              <span className="font-mono text-foreground">{fk.name}</span>
              <span className="text-muted-foreground">
                ({fk.columns.join(", ")})
              </span>
              <span className="text-muted-foreground">→</span>
              <span className="font-mono text-foreground">
                {fk.referencedSchema}.{fk.referencedTable}
              </span>
              <span className="text-muted-foreground">
                ({fk.referencedColumns.join(", ")})
              </span>
              {fk.onUpdate || fk.onDelete ? (
                <span className="ml-auto text-[0.625rem] uppercase tracking-wide text-muted-foreground">
                  {fk.onUpdate ? `ON UPDATE ${fk.onUpdate}` : null}
                  {fk.onUpdate && fk.onDelete ? " · " : ""}
                  {fk.onDelete ? `ON DELETE ${fk.onDelete}` : null}
                </span>
              ) : null}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function IndexesSection({
  indexes,
  supported,
  engine,
}: {
  indexes: IndexInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
}) {
  return (
    <Section title="Indexes" testId="structure-indexes">
      {!supported ? (
        <UnsupportedNotice engine={engine} feature="Indexes" />
      ) : indexes.length === 0 ? (
        <EmptyRow>No indexes defined.</EmptyRow>
      ) : (
        <div className="divide-y divide-border/50">
          {indexes.map((index) => (
            <div
              key={index.name}
              className="flex flex-wrap items-center gap-2 px-4 py-3 text-sm"
            >
              {index.isUnique ? (
                <Badge variant="outline" className="text-[0.625rem] uppercase">
                  unique
                </Badge>
              ) : null}
              {index.isPrimary ? (
                <Badge
                  variant="secondary"
                  className="text-[0.625rem] uppercase"
                >
                  primary
                </Badge>
              ) : null}
              <span className="font-mono text-foreground">{index.name}</span>
              <span className="text-muted-foreground">
                ({index.columns.join(", ")})
              </span>
              {index.method ? (
                <span className="ml-auto text-[0.625rem] uppercase tracking-wide text-muted-foreground">
                  using {index.method}
                </span>
              ) : null}
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function ConstraintsSection({
  constraints,
  supported,
  engine,
}: {
  constraints: ConstraintInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
}) {
  return (
    <Section title="Constraints" testId="structure-constraints">
      {!supported ? (
        <UnsupportedNotice engine={engine} feature="Constraints" />
      ) : constraints.length === 0 ? (
        <EmptyRow>No additional constraints defined.</EmptyRow>
      ) : (
        <div className="divide-y divide-border/50">
          {constraints.map((constraint) => (
            <div
              key={constraint.name}
              className="flex flex-col gap-1 px-4 py-3 text-sm"
            >
              <div className="flex items-center gap-2">
                <Badge variant="outline" className="text-[0.625rem] uppercase">
                  {constraint.kind}
                </Badge>
                <span className="font-mono text-foreground">
                  {constraint.name}
                </span>
              </div>
              <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs text-muted-foreground">
                {constraint.definition}
              </pre>
            </div>
          ))}
        </div>
      )}
    </Section>
  );
}

function Section({
  title,
  testId,
  action,
  children,
}: {
  title: string;
  testId: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section data-testid={testId} className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
          {title}
        </h3>
        {action ? <div>{action}</div> : null}
      </div>
      <div className="overflow-hidden rounded-lg border border-border/50 bg-card/50">
        {children}
      </div>
    </section>
  );
}

function EmptyRow({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-4 py-3 text-sm text-muted-foreground">{children}</div>
  );
}

function UnsupportedNotice({
  engine,
  feature,
}: {
  engine: DatabaseEngine | undefined;
  feature: string;
}) {
  const engineLabel = engine ?? "this engine";
  return (
    <div className="px-4 py-3 text-sm text-muted-foreground">
      {feature} are not supported on {engineLabel}.
    </div>
  );
}
