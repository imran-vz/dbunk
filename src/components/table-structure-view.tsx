import { IconAlertTriangle, IconKey, IconLink } from "@tabler/icons-react";
import { useEffect } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  type ColumnInfo,
  type ConstraintInfo,
  type DatabaseEngine,
  type ForeignKeyInfo,
  type IndexInfo,
  type StructureCapabilities,
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
  const loadTableStructure = useAppStore((state) => state.loadTableStructure);
  const structure = useAppStore(
    (state) =>
      state.tableStructure[tableStructureKey(connectionId, schema, tableName)],
  );
  const status = useAppStore(
    (state) =>
      state.tableStructureStatus[
        tableStructureKey(connectionId, schema, tableName)
      ],
  );
  const engine = useAppStore(
    (state) =>
      state.connections.find((connection) => connection.id === connectionId)
        ?.engine,
  );

  useEffect(() => {
    if (connectionId && schema && tableName) {
      void loadTableStructure(connectionId, schema, tableName);
    }
  }, [connectionId, schema, tableName, loadTableStructure]);

  const isLoading = status?.state === "loading";
  const errorMessage = status?.state === "error" ? status.error : null;
  const capabilities = structure?.capabilities ?? fallbackCapabilities;

  const handleRetry = () => {
    if (connectionId && schema && tableName) {
      void loadTableStructure(connectionId, schema, tableName);
    }
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

          <ColumnsSection columns={structure?.columns ?? []} />

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

function ColumnsSection({ columns }: { columns: ColumnInfo[] }) {
  return (
    <Section title="Columns" testId="structure-columns">
      {columns.length === 0 ? (
        <EmptyRow>No columns reported by the database.</EmptyRow>
      ) : (
        <div className="divide-y divide-border/50">
          {columns.map((column) => (
            <ColumnRow key={column.name} column={column} />
          ))}
        </div>
      )}
    </Section>
  );
}

function ColumnRow({ column }: { column: ColumnInfo }) {
  return (
    <div
      data-testid={`structure-column-${column.name}`}
      className="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto] items-center gap-3 px-4 py-3 text-sm"
    >
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
        {column.nullable ? (
          <span className="text-[0.625rem] uppercase tracking-wide text-muted-foreground">
            nullable
          </span>
        ) : (
          <span className="text-[0.625rem] uppercase tracking-wide text-rose-400">
            not null
          </span>
        )}
      </div>
      <div className="text-right text-xs text-muted-foreground">
        {column.defaultValue ? (
          <span className="font-mono" title={`Default: ${column.defaultValue}`}>
            {column.defaultValue}
          </span>
        ) : (
          <span className="text-muted-foreground/60">—</span>
        )}
      </div>
    </div>
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
  children,
}: {
  title: string;
  testId: string;
  children: React.ReactNode;
}) {
  return (
    <section data-testid={testId} className="space-y-2">
      <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
        {title}
      </h3>
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
