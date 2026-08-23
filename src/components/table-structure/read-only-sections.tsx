import { IconKey, IconLink } from "@tabler/icons-react";

import { Badge } from "@/components/ui/badge";
import type { RelationalPolicy } from "@/lib/engine-policy";
import type {
  ConstraintInfo,
  DatabaseEngine,
  ForeignKeyInfo,
  IndexInfo,
} from "@/lib/store";

import { EmptyRow, Section, UnsupportedNotice } from "./shared";

export function ClickHousePhysicalLayout({
  partitionBy,
  sampleBy,
}: {
  partitionBy: string | null;
  sampleBy: string | null;
}) {
  return (
    <Section title="Physical layout" testId="structure-physical-layout">
      <div className="divide-y divide-white/8">
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
}: {
  foreignKeys: ForeignKeyInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
}) {
  return (
    <Section title="Foreign keys" testId="structure-foreign-keys">
      <ForeignKeysBody
        foreignKeys={foreignKeys}
        supported={supported}
        engine={engine}
        policy={policy}
      />
    </Section>
  );
}

function ForeignKeysBody({
  foreignKeys,
  supported,
  engine,
  policy,
}: {
  foreignKeys: ForeignKeyInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
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
    <div className="divide-y divide-white/8">
      {foreignKeys.map((fk) => (
        <ForeignKeyRow key={fk.name} fk={fk} />
      ))}
    </div>
  );
}

function ForeignKeyRow({ fk }: { fk: ForeignKeyInfo }) {
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
    </div>
  );
}

export function IndexesSection({
  indexes,
  supported,
  engine,
  policy,
}: {
  indexes: IndexInfo[];
  supported: boolean;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
}) {
  const title = policy.labels.indexes;
  return (
    <Section title={title} testId="structure-indexes">
      {!supported ? (
        <UnsupportedNotice engine={engine} feature={title} />
      ) : indexes.length === 0 ? (
        <EmptyRow>{policy.labels.noIndexes}</EmptyRow>
      ) : (
        <div className="divide-y divide-white/8">
          {indexes.map((index) => (
            <IndexRow key={index.name} index={index} />
          ))}
        </div>
      )}
    </Section>
  );
}

function IndexRow({ index }: { index: IndexInfo }) {
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 py-2 text-xs">
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
    </div>
  );
}

export function ConstraintsSection({
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
        <div className="divide-y divide-white/8">
          {constraints.map((constraint) => (
            <ConstraintRow key={constraint.name} constraint={constraint} />
          ))}
        </div>
      )}
    </Section>
  );
}

function ConstraintRow({ constraint }: { constraint: ConstraintInfo }) {
  return (
    <div className="flex flex-col gap-1 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        <Badge variant="outline" className="text-2xs uppercase">
          {constraint.kind}
        </Badge>
        <span className="font-mono text-foreground">{constraint.name}</span>
      </div>
      <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs text-muted-foreground">
        {constraint.definition}
      </pre>
    </div>
  );
}
