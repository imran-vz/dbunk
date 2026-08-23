import type * as React from "react";

import type { ColumnChangeKind } from "@/lib/ddl";
import type { DatabaseEngine } from "@/lib/store";

export function Section({
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
      <div className="overflow-hidden rounded-sm border border-border-subtle bg-surface-panel">
        {children}
      </div>
    </section>
  );
}

export function EmptyRow({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-3 py-2 text-xs text-muted-foreground">{children}</div>
  );
}

export function UnsupportedNotice({
  engine,
  feature,
}: {
  engine: DatabaseEngine | undefined;
  feature: string;
}) {
  const engineLabel = engine ?? "this engine";
  return (
    <div className="px-3 py-2 text-xs text-muted-foreground">
      {feature} are not supported on {engineLabel}.
    </div>
  );
}

export function describeChange(change: ColumnChangeKind): string {
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
