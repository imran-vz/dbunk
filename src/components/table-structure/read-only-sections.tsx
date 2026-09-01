import { IconKey, IconLink, IconPlus, IconTrash } from "@tabler/icons-react";
import { useState } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { RelationalPolicy } from "@/lib/engine-policy";
import type {
  ConstraintInfo,
  DatabaseEngine,
  ForeignKeyInfo,
  IndexInfo,
  PgReferentialAction,
} from "@/lib/store";
import {
  asPgReferentialAction,
  buildAddForeignKeyOp,
  buildCreateIndexOp,
  PG_REFERENTIAL_ACTIONS,
} from "@/lib/structure-changes";

import {
  EmptyRow,
  MiniSelect,
  type PgStructureOps,
  Section,
  UnsupportedNotice,
} from "./shared";

const splitColumnList = (value: string): string[] =>
  value
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);

function InlineForm({
  testId,
  children,
}: {
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-wrap items-center gap-1.5 border-b border-border-subtle bg-surface-panel px-3 py-2 text-xs"
    >
      {children}
    </div>
  );
}

function InlineToggle({
  testId,
  label,
  active,
  onToggle,
}: {
  testId: string;
  label: string;
  active: boolean;
  onToggle: () => void;
}) {
  return (
    <Button
      data-testid={testId}
      variant="ghost"
      size="sm"
      className={
        active
          ? "px-1.5 text-2xs uppercase tracking-wide text-foreground"
          : "px-1.5 text-2xs uppercase tracking-wide text-muted-foreground/60"
      }
      onClick={onToggle}
    >
      {label}
    </Button>
  );
}

export function ClickHousePhysicalLayout({
  partitionBy,
  sampleBy,
}: {
  partitionBy: string | null;
  sampleBy: string | null;
}) {
  return (
    <Section title="Physical layout" testId="structure-physical-layout">
      <div className="divide-y divide-border-subtle">
        {partitionBy ? (
          <LayoutRow label="PARTITION BY" value={partitionBy} />
        ) : null}
        {sampleBy ? <LayoutRow label="SAMPLE BY" value={sampleBy} /> : null}
      </div>
    </Section>
  );
}

function LayoutRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-1 px-3 py-2 text-xs">
      <span className="text-2xs uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <span className="font-mono text-foreground">{value}</span>
    </div>
  );
}

export function PrimaryKeySection({
  primaryKey,
  supported,
  policy,
}: {
  primaryKey: string[] | null;
  supported: boolean;
  policy: RelationalPolicy;
}) {
  const isClickHouse = policy.engine === "ClickHouse";
  return (
    <Section title={policy.labels.primaryKey} testId="structure-primary-key">
      {!supported ? (
        <UnsupportedNotice
          engine={policy.engine}
          feature={policy.labels.primaryKey}
        />
      ) : !primaryKey || primaryKey.length === 0 ? (
        <EmptyRow>{policy.labels.noPrimaryKey}</EmptyRow>
      ) : (
        <div className="space-y-1 px-3 py-2">
          <div className="flex items-center gap-2 text-xs">
            <IconKey className="size-4 text-muted-foreground" />
            <span className="font-mono text-foreground">
              ({primaryKey.join(", ")})
            </span>
          </div>
          {isClickHouse ? (
            <p className="text-2xs text-muted-foreground">
              ClickHouse uses the sorting key as a sparse primary index. It does
              not enforce uniqueness.
            </p>
          ) : null}
        </div>
      )}
    </Section>
  );
}

export function ForeignKeysSection({
  foreignKeys,
  supported,
  engine,
  policy,
  pg = null,
}: {
  foreignKeys: ForeignKeyInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
  pg?: PgStructureOps | null;
}) {
  const [showAddForm, setShowAddForm] = useState(false);
  const canEdit = supported && pg !== null;
  return (
    <Section
      title="Foreign keys"
      testId="structure-foreign-keys"
      action={
        canEdit ? (
          <Button
            data-testid="structure-add-fk"
            variant="outline"
            size="sm"
            className="gap-1"
            onClick={() => setShowAddForm((value) => !value)}
          >
            <IconPlus />
            Add foreign key
          </Button>
        ) : null
      }
    >
      {canEdit && showAddForm && pg ? (
        <AddForeignKeyForm pg={pg} onDone={() => setShowAddForm(false)} />
      ) : null}
      <ForeignKeysBody
        foreignKeys={foreignKeys}
        supported={supported}
        engine={engine}
        policy={policy}
        pg={pg}
      />
    </Section>
  );
}

function AddForeignKeyForm({
  pg,
  onDone,
}: {
  pg: PgStructureOps;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [columns, setColumns] = useState("");
  const [referencedSchema, setReferencedSchema] = useState(pg.schema);
  const [referencedTable, setReferencedTable] = useState("");
  const [referencedColumns, setReferencedColumns] = useState("");
  const [onUpdate, setOnUpdate] = useState<PgReferentialAction>("no-action");
  const [onDelete, setOnDelete] = useState<PgReferentialAction>("no-action");

  const localColumns = splitColumnList(columns);
  const remoteColumns = splitColumnList(referencedColumns);
  const valid =
    localColumns.length > 0 &&
    remoteColumns.length > 0 &&
    referencedTable.trim() !== "";

  return (
    <InlineForm testId="structure-add-fk-form">
      <Input
        data-testid="structure-fk-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        placeholder="constraint name (optional)"
        className="h-6 max-w-48 font-mono text-xs"
      />
      <Input
        data-testid="structure-fk-columns"
        value={columns}
        onChange={(event) => setColumns(event.target.value)}
        placeholder="columns, comma separated"
        className="h-6 max-w-48 font-mono text-xs"
      />
      <span className="text-muted-foreground">→</span>
      <Input
        data-testid="structure-fk-ref-schema"
        value={referencedSchema}
        onChange={(event) => setReferencedSchema(event.target.value)}
        placeholder="schema"
        className="h-6 max-w-32 font-mono text-xs"
      />
      <Input
        data-testid="structure-fk-ref-table"
        value={referencedTable}
        onChange={(event) => setReferencedTable(event.target.value)}
        placeholder="referenced table"
        className="h-6 max-w-40 font-mono text-xs"
      />
      <Input
        data-testid="structure-fk-ref-columns"
        value={referencedColumns}
        onChange={(event) => setReferencedColumns(event.target.value)}
        placeholder="referenced columns"
        className="h-6 max-w-40 font-mono text-xs"
      />
      <MiniSelect
        testId="structure-fk-on-update"
        ariaLabel="ON UPDATE action"
        value={onUpdate}
        options={PG_REFERENTIAL_ACTIONS}
        onChange={(value) => setOnUpdate(asPgReferentialAction(value))}
      />
      <MiniSelect
        testId="structure-fk-on-delete"
        ariaLabel="ON DELETE action"
        value={onDelete}
        options={PG_REFERENTIAL_ACTIONS}
        onChange={(value) => setOnDelete(asPgReferentialAction(value))}
      />
      <div className="ml-auto flex items-center gap-1">
        <Button variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button
          data-testid="structure-fk-submit"
          variant="default"
          size="sm"
          disabled={!valid}
          onClick={() => {
            pg.queueOp(
              buildAddForeignKeyOp({
                schema: pg.schema,
                table: pg.table,
                name,
                columns: localColumns,
                referencedSchema,
                referencedTable,
                referencedColumns: remoteColumns,
                onUpdate,
                onDelete,
                deferrable: false,
              }),
            );
            onDone();
          }}
        >
          Queue add
        </Button>
      </div>
    </InlineForm>
  );
}

function ForeignKeysBody({
  foreignKeys,
  supported,
  engine,
  policy,
  pg,
}: {
  foreignKeys: ForeignKeyInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
  pg: PgStructureOps | null;
}) {
  if (!supported) {
    if (!policy.hasForeignKeys) {
      return (
        <div className="px-3 py-2 text-xs text-muted-foreground">
          {policy.foreignKeysUnsupportedCopy}
        </div>
      );
    }
    return <UnsupportedNotice engine={engine} feature="Foreign keys" />;
  }
  if (foreignKeys.length === 0) {
    return <EmptyRow>No foreign keys defined.</EmptyRow>;
  }
  return (
    <div className="divide-y divide-border-subtle">
      {foreignKeys.map((fk) => (
        <ForeignKeyRow key={fk.name} fk={fk} pg={pg} />
      ))}
    </div>
  );
}

function ForeignKeyRow({
  fk,
  pg,
}: {
  fk: ForeignKeyInfo;
  pg: PgStructureOps | null;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-2 text-xs">
      <IconLink className="size-3.5 text-muted-foreground" />
      <span className="font-mono text-foreground">{fk.name}</span>
      <span className="text-muted-foreground">({fk.columns.join(", ")})</span>
      <span className="text-muted-foreground">→</span>
      <span className="font-mono text-foreground">
        {fk.referencedSchema}.{fk.referencedTable}
      </span>
      <span className="text-muted-foreground">
        ({fk.referencedColumns.join(", ")})
      </span>
      {fk.onUpdate || fk.onDelete ? (
        <span className="ml-auto text-2xs uppercase tracking-wide text-muted-foreground">
          {fk.onUpdate ? `ON UPDATE ${fk.onUpdate}` : null}
          {fk.onUpdate && fk.onDelete ? " · " : ""}
          {fk.onDelete ? `ON DELETE ${fk.onDelete}` : null}
        </span>
      ) : null}
      {pg ? (
        <Button
          data-testid={`structure-drop-fk-${fk.name}`}
          variant="ghost"
          size="sm"
          className={
            fk.onUpdate || fk.onDelete
              ? "px-1.5 text-danger hover:text-danger/80"
              : "ml-auto px-1.5 text-danger hover:text-danger/80"
          }
          aria-label={`Drop foreign key ${fk.name}`}
          onClick={() =>
            pg.queueOp({
              op: "dropConstraint",
              schema: pg.schema,
              table: pg.table,
              name: fk.name,
              cascade: false,
            })
          }
        >
          <IconTrash />
        </Button>
      ) : null}
    </div>
  );
}

export function IndexesSection({
  indexes,
  supported,
  engine,
  policy,
  pg = null,
}: {
  indexes: IndexInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
  pg?: PgStructureOps | null;
}) {
  const [showAddForm, setShowAddForm] = useState(false);
  const title = policy.labels.indexes;
  const canEdit = supported && pg !== null;
  return (
    <Section
      title={title}
      testId="structure-indexes"
      action={
        canEdit ? (
          <Button
            data-testid="structure-add-index"
            variant="outline"
            size="sm"
            className="gap-1"
            onClick={() => setShowAddForm((value) => !value)}
          >
            <IconPlus />
            New index
          </Button>
        ) : null
      }
    >
      {canEdit && showAddForm && pg ? (
        <AddIndexForm pg={pg} onDone={() => setShowAddForm(false)} />
      ) : null}
      {!supported ? (
        <UnsupportedNotice engine={engine} feature={title} />
      ) : indexes.length === 0 ? (
        <EmptyRow>{policy.labels.noIndexes}</EmptyRow>
      ) : (
        <div className="divide-y divide-border-subtle">
          {indexes.map((index) => (
            <IndexRow key={index.name} index={index} pg={pg} />
          ))}
        </div>
      )}
    </Section>
  );
}

function AddIndexForm({
  pg,
  onDone,
}: {
  pg: PgStructureOps;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [columns, setColumns] = useState("");
  const [method, setMethod] = useState("btree");
  const [wherePredicate, setWherePredicate] = useState("");
  const [unique, setUnique] = useState(false);
  const [concurrently, setConcurrently] = useState(true);

  const expressions = splitColumnList(columns);

  return (
    <InlineForm testId="structure-add-index-form">
      <Input
        data-testid="structure-index-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        placeholder="index name (optional)"
        className="h-6 max-w-48 font-mono text-xs"
      />
      <Input
        data-testid="structure-index-columns"
        value={columns}
        onChange={(event) => setColumns(event.target.value)}
        placeholder="columns / expressions, comma separated"
        className="h-6 max-w-[16rem] font-mono text-xs"
      />
      <Input
        data-testid="structure-index-method"
        value={method}
        onChange={(event) => setMethod(event.target.value)}
        placeholder="btree"
        className="h-6 max-w-24 font-mono text-xs"
      />
      <Input
        data-testid="structure-index-where"
        value={wherePredicate}
        onChange={(event) => setWherePredicate(event.target.value)}
        placeholder="WHERE predicate (optional)"
        className="h-6 max-w-48 font-mono text-xs"
      />
      <InlineToggle
        testId="structure-index-unique"
        label="unique"
        active={unique}
        onToggle={() => setUnique((value) => !value)}
      />
      <InlineToggle
        testId="structure-index-concurrently"
        label="concurrently"
        active={concurrently}
        onToggle={() => setConcurrently((value) => !value)}
      />
      <div className="ml-auto flex items-center gap-1">
        <Button variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button
          data-testid="structure-index-submit"
          variant="default"
          size="sm"
          disabled={expressions.length === 0 || method.trim() === ""}
          onClick={() => {
            pg.queueOp(
              buildCreateIndexOp({
                schema: pg.schema,
                table: pg.table,
                name,
                unique,
                method,
                columnExpressions: expressions,
                include: [],
                wherePredicate,
                concurrently,
              }),
            );
            onDone();
          }}
        >
          Queue add
        </Button>
      </div>
    </InlineForm>
  );
}

function IndexRow({
  index,
  pg,
}: {
  index: IndexInfo;
  pg: PgStructureOps | null;
}) {
  const [confirmingDrop, setConfirmingDrop] = useState(false);
  const [concurrently, setConcurrently] = useState(false);
  // The primary-key index is dropped through its constraint, not DROP INDEX.
  const droppable = pg !== null && !index.isPrimary;
  return (
    <div className="px-3 py-2 text-xs">
      <div className="flex flex-wrap items-center gap-2">
        {index.isUnique ? (
          <Badge variant="outline" className="text-2xs uppercase">
            unique
          </Badge>
        ) : null}
        {index.isPrimary ? (
          <Badge variant="secondary" className="text-2xs uppercase">
            primary
          </Badge>
        ) : null}
        <span className="font-mono text-foreground">{index.name}</span>
        <span className="text-muted-foreground">
          ({index.columns.join(", ")})
        </span>
        {index.method ? (
          <span className="ml-auto text-2xs uppercase tracking-wide text-muted-foreground">
            using {index.method}
          </span>
        ) : null}
        {droppable && pg ? (
          <Button
            data-testid={`structure-drop-index-${index.name}`}
            variant="ghost"
            size="sm"
            className={
              index.method
                ? "px-1.5 text-danger hover:text-danger/80"
                : "ml-auto px-1.5 text-danger hover:text-danger/80"
            }
            aria-label={`Drop index ${index.name}`}
            onClick={() => setConfirmingDrop((value) => !value)}
          >
            <IconTrash />
          </Button>
        ) : null}
      </div>
      {confirmingDrop && pg ? (
        <div
          data-testid={`structure-drop-index-confirm-${index.name}`}
          className="mt-2 flex flex-wrap items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 py-1.5"
        >
          <span className="text-muted-foreground">Drop {index.name}?</span>
          <InlineToggle
            testId={`structure-drop-index-concurrently-${index.name}`}
            label="concurrently"
            active={concurrently}
            onToggle={() => setConcurrently((value) => !value)}
          />
          <div className="ml-auto flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setConfirmingDrop(false)}
            >
              Cancel
            </Button>
            <Button
              data-testid={`structure-drop-index-submit-${index.name}`}
              variant="default"
              size="sm"
              onClick={() => {
                pg.queueOp({
                  op: "dropIndex",
                  schema: pg.schema,
                  name: index.name,
                  concurrently,
                  cascade: false,
                });
                setConfirmingDrop(false);
              }}
            >
              Queue drop
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

export function ConstraintsSection({
  constraints,
  supported,
  engine,
  pg = null,
}: {
  constraints: ConstraintInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  pg?: PgStructureOps | null;
}) {
  const [openForm, setOpenForm] = useState<"check" | "unique" | "pk" | null>(
    null,
  );
  const canEdit = supported && pg !== null;
  const toggleForm = (form: "check" | "unique" | "pk") =>
    setOpenForm((current) => (current === form ? null : form));
  return (
    <Section
      title="Constraints"
      testId="structure-constraints"
      action={
        canEdit ? (
          <div className="flex items-center gap-1">
            <Button
              data-testid="structure-add-check"
              variant="outline"
              size="sm"
              onClick={() => toggleForm("check")}
            >
              Add check
            </Button>
            <Button
              data-testid="structure-add-unique"
              variant="outline"
              size="sm"
              onClick={() => toggleForm("unique")}
            >
              Add unique
            </Button>
            {pg && !pg.hasPrimaryKey ? (
              <Button
                data-testid="structure-add-pk"
                variant="outline"
                size="sm"
                onClick={() => toggleForm("pk")}
              >
                Add primary key
              </Button>
            ) : null}
          </div>
        ) : null
      }
    >
      {canEdit && openForm === "check" && pg ? (
        <AddCheckForm pg={pg} onDone={() => setOpenForm(null)} />
      ) : null}
      {canEdit && openForm === "unique" && pg ? (
        <AddColumnsConstraintForm
          kind="unique"
          pg={pg}
          onDone={() => setOpenForm(null)}
        />
      ) : null}
      {canEdit && openForm === "pk" && pg ? (
        <AddColumnsConstraintForm
          kind="pk"
          pg={pg}
          onDone={() => setOpenForm(null)}
        />
      ) : null}
      {!supported ? (
        <UnsupportedNotice engine={engine} feature="Constraints" />
      ) : constraints.length === 0 ? (
        <EmptyRow>No additional constraints defined.</EmptyRow>
      ) : (
        <div className="divide-y divide-border-subtle">
          {constraints.map((constraint) => (
            <ConstraintRow
              key={constraint.name}
              constraint={constraint}
              pg={pg}
            />
          ))}
        </div>
      )}
    </Section>
  );
}

function AddCheckForm({
  pg,
  onDone,
}: {
  pg: PgStructureOps;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [expression, setExpression] = useState("");
  return (
    <InlineForm testId="structure-add-check-form">
      <Input
        data-testid="structure-check-name"
        value={name}
        onChange={(event) => setName(event.target.value)}
        placeholder="constraint name (optional)"
        className="h-6 max-w-48 font-mono text-xs"
      />
      <Input
        data-testid="structure-check-expression"
        value={expression}
        onChange={(event) => setExpression(event.target.value)}
        placeholder="check expression, e.g. price > 0"
        className="h-6 max-w-[18rem] flex-1 font-mono text-xs"
      />
      <div className="ml-auto flex items-center gap-1">
        <Button variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button
          data-testid="structure-check-submit"
          variant="default"
          size="sm"
          disabled={expression.trim() === ""}
          onClick={() => {
            pg.queueOp({
              op: "addCheck",
              schema: pg.schema,
              table: pg.table,
              name: name.trim() === "" ? null : name.trim(),
              expression: expression.trim(),
              notValid: false,
            });
            onDone();
          }}
        >
          Queue add
        </Button>
      </div>
    </InlineForm>
  );
}

function AddColumnsConstraintForm({
  kind,
  pg,
  onDone,
}: {
  kind: "unique" | "pk";
  pg: PgStructureOps;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [columns, setColumns] = useState("");
  const columnList = splitColumnList(columns);
  const idPrefix = kind === "unique" ? "structure-unique" : "structure-pk";
  return (
    <InlineForm testId={`${idPrefix}-form`}>
      <Input
        data-testid={`${idPrefix}-name`}
        value={name}
        onChange={(event) => setName(event.target.value)}
        placeholder="constraint name (optional)"
        className="h-6 max-w-48 font-mono text-xs"
      />
      <Input
        data-testid={`${idPrefix}-columns`}
        value={columns}
        onChange={(event) => setColumns(event.target.value)}
        placeholder="columns, comma separated"
        className="h-6 max-w-56 font-mono text-xs"
      />
      <div className="ml-auto flex items-center gap-1">
        <Button variant="ghost" size="sm" onClick={onDone}>
          Cancel
        </Button>
        <Button
          data-testid={`${idPrefix}-submit`}
          variant="default"
          size="sm"
          disabled={columnList.length === 0}
          onClick={() => {
            const shared = {
              schema: pg.schema,
              table: pg.table,
              name: name.trim() === "" ? null : name.trim(),
              columns: columnList,
            };
            pg.queueOp(
              kind === "unique"
                ? { op: "addUnique", ...shared }
                : { op: "addPrimaryKey", ...shared },
            );
            onDone();
          }}
        >
          Queue add
        </Button>
      </div>
    </InlineForm>
  );
}

function ConstraintRow({
  constraint,
  pg,
}: {
  constraint: ConstraintInfo;
  pg: PgStructureOps | null;
}) {
  return (
    <div className="flex flex-col gap-1 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        <Badge variant="outline" className="text-2xs uppercase">
          {constraint.kind}
        </Badge>
        <span className="font-mono text-foreground">{constraint.name}</span>
        {pg ? (
          <Button
            data-testid={`structure-drop-constraint-${constraint.name}`}
            variant="ghost"
            size="sm"
            className="ml-auto px-1.5 text-danger hover:text-danger/80"
            aria-label={`Drop constraint ${constraint.name}`}
            onClick={() =>
              pg.queueOp({
                op: "dropConstraint",
                schema: pg.schema,
                table: pg.table,
                name: constraint.name,
                cascade: false,
              })
            }
          >
            <IconTrash />
          </Button>
        ) : null}
      </div>
      <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs text-muted-foreground">
        {constraint.definition}
      </pre>
    </div>
  );
}
