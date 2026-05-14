import {
  IconAlertTriangle,
  IconArrowsRightLeft,
  IconLink,
  IconPlus,
} from "@tabler/icons-react";
import { useEffect, useMemo } from "react";

import { ForeignKeysSection } from "@/components/table-structure/read-only-sections";
import { UnsupportedNotice } from "@/components/table-structure/shared";
import { useStructure } from "@/components/table-structure/use-structure";
import { Button } from "@/components/ui/button";
import type { SchemaForeignKey } from "@/lib/schema-graph";
import { schemaRelationshipsKey, useAppStore } from "@/lib/store";

import type { SubTab } from "./header";

interface RelationsSubTabProps {
  connectionId: string;
  schema: string;
  tableName: string;
  onOpenTable: (schema: string, tableName: string) => void;
  onOpenSpecialized: (subTab: SubTab) => void;
}

export function RelationsSubTab({
  connectionId,
  schema,
  tableName,
  onOpenTable,
  onOpenSpecialized,
}: RelationsSubTabProps) {
  const view = useStructure({ connectionId, schema, tableName });
  const loadSchemaRelationships = useAppStore(
    (state) => state.loadSchemaRelationships,
  );
  const relationshipsKey = schemaRelationshipsKey(connectionId, schema);
  const relationships = useAppStore(
    (state) => state.schemaRelationships[relationshipsKey],
  );
  const relationshipsStatus = useAppStore(
    (state) => state.schemaRelationshipsStatus[relationshipsKey],
  );

  useEffect(() => {
    if (!connectionId) return;
    if (relationships || relationshipsStatus?.state === "loading") return;
    void loadSchemaRelationships(connectionId, schema);
  }, [
    connectionId,
    schema,
    relationships,
    relationshipsStatus?.state,
    loadSchemaRelationships,
  ]);

  const inbound = useMemo<SchemaForeignKey[]>(
    () =>
      (relationships?.foreignKeys ?? []).filter(
        (fk) => fk.toSchema === schema && fk.toTable === tableName,
      ),
    [relationships, schema, tableName],
  );

  const isLoadingRelations = relationshipsStatus?.state === "loading";
  const relationshipsError =
    relationshipsStatus?.state === "error" ? relationshipsStatus.error : null;

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-[#080c10]">
      {view.isLoading || isLoadingRelations ? (
        <div className="h-0.5 w-full animate-pulse bg-primary" />
      ) : null}
      {view.errorMessage ? (
        <Banner message={view.errorMessage} onRetry={view.retry} />
      ) : null}
      {relationshipsError ? (
        <Banner
          message={relationshipsError}
          onRetry={() => {
            void loadSchemaRelationships(connectionId, schema);
          }}
        />
      ) : null}
      <div className="flex-1 overflow-auto p-3">
        <div className="mx-auto flex max-w-5xl flex-col gap-3">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="flex size-7 items-center justify-center rounded-sm border border-border-subtle bg-surface-panel text-text-secondary">
                <IconArrowsRightLeft className="size-3.5" />
              </span>
              <div>
                <h2 className="text-sm font-semibold text-foreground">
                  Relations
                </h2>
                <p className="text-[0.6875rem] text-text-muted">
                  {schema}.{tableName}
                </p>
              </div>
            </div>
            <Button
              size="sm"
              variant="outline"
              onClick={() => onOpenSpecialized("specialized")}
              aria-label="Open the foreign-key builder"
            >
              <IconPlus className="size-3.5" />
              New foreign key
            </Button>
          </div>

          {view.capabilities.foreignKeys ? (
            <ForeignKeysSection
              foreignKeys={view.foreignKeys}
              supported
              engine={view.engine}
              policy={view.policy}
            />
          ) : (
            <UnsupportedNotice
              engine={view.engine}
              feature="Outbound relations"
            />
          )}

          <InboundSection
            inbound={inbound}
            isLoading={isLoadingRelations && !relationships}
            onOpenTable={onOpenTable}
          />
        </div>
      </div>
    </div>
  );
}

function InboundSection({
  inbound,
  isLoading,
  onOpenTable,
}: {
  inbound: SchemaForeignKey[];
  isLoading: boolean;
  onOpenTable: (schema: string, tableName: string) => void;
}) {
  return (
    <section
      data-testid="relations-inbound"
      className="rounded-sm border border-border-subtle bg-surface-window"
    >
      <header className="border-b border-border-subtle px-3 py-2">
        <h3 className="text-xs font-semibold text-foreground">Referenced by</h3>
        <p className="text-[0.6875rem] text-text-muted">
          Tables whose foreign keys point at this one.
        </p>
      </header>
      {isLoading ? (
        <div className="px-3 py-2 text-xs text-text-muted">Loading…</div>
      ) : inbound.length === 0 ? (
        <div className="px-3 py-2 text-xs text-text-muted">
          No tables reference this one.
        </div>
      ) : (
        <ul className="divide-y divide-white/8">
          {inbound.map((fk) => (
            <li
              key={`${fk.fromSchema}.${fk.fromTable}.${fk.constraintName}`}
              className="flex flex-wrap items-center gap-2 px-3 py-2 text-xs"
            >
              <IconLink className="size-3.5 text-text-muted" />
              <button
                type="button"
                className="font-mono text-foreground hover:underline"
                onClick={() => onOpenTable(fk.fromSchema, fk.fromTable)}
              >
                {fk.fromSchema}.{fk.fromTable}
              </button>
              <span className="text-text-muted">
                ({fk.fromColumns.join(", ")})
              </span>
              <span className="text-text-muted">→</span>
              <span className="font-mono text-foreground">
                ({fk.toColumns.join(", ")})
              </span>
              <span className="ml-auto font-mono text-[0.625rem] text-text-muted">
                {fk.constraintName}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function Banner({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div
      role="alert"
      className="flex items-start gap-2 border-b border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
    >
      <IconAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
      <div className="flex-1 whitespace-pre-wrap font-mono">{message}</div>
      <Button size="sm" variant="ghost" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}
