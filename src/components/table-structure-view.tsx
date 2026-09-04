import { IconAlertTriangle, IconColumns3 } from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { ColumnsSection } from "@/components/table-structure/columns-section";
import { PendingChangesSection } from "@/components/table-structure/pending-changes-section";
import {
  ClickHousePhysicalLayout,
  ConstraintsSection,
  ForeignKeysSection,
  IndexesSection,
  PrimaryKeySection,
} from "@/components/table-structure/read-only-sections";
import {
  PrivilegesSection,
  RowLevelSecuritySection,
  TriggersSection,
  type TriggerFunctionOption,
} from "@/components/table-structure/table-security-sections";
import { useStructure } from "@/components/table-structure/use-structure";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { pgObjectDescriptionKey, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface TableStructureViewProps {
  connectionId: string;
  schema: string;
  tableName: string;
  className?: string;
}

export function TableStructureView({
  connectionId,
  schema,
  tableName,
  className,
}: TableStructureViewProps) {
  // Remount the whole editor when a reused workspace panel changes tables so
  // every open structure form and row control loses its previous-table state.
  return (
    <TableStructureViewForTable
      key={`${connectionId}\u0000${schema}\u0000${tableName}`}
      connectionId={connectionId}
      schema={schema}
      tableName={tableName}
      className={className}
    />
  );
}

function TableStructureViewForTable({
  connectionId,
  schema,
  tableName,
  className,
}: TableStructureViewProps) {
  const view = useStructure({ connectionId, schema, tableName });
  const [showPreview, setShowPreview] = useState(false);
  const catalogState = useAppStore(
    (state) => state.pgObjectCatalog[connectionId],
  );
  const descriptions = useAppStore((state) => state.pgObjectDescriptions);
  const roles =
    catalogState?.status === "ready" && catalogState.catalog
      ? catalogState.catalog.roles.map((role) => role.name)
      : [];
  const functions = useMemo<TriggerFunctionOption[]>(() => {
    if (catalogState?.status !== "ready" || !catalogState.catalog) return [];
    return catalogState.catalog.schemas.flatMap((schemaEntry) =>
      schemaEntry.functions
        .filter((fn) => (fn.identityArgs ?? "").trim() === "")
        .map((fn) => {
          const reference = {
            kind: "function",
            schema: schemaEntry.name,
            name: fn.name,
            identityArgs: "",
          } as const;
          const description =
            descriptions[pgObjectDescriptionKey(connectionId, reference)]
              ?.description;
          return {
            schema: schemaEntry.name,
            name: fn.name,
            identityArgs: "",
            returns:
              description?.facts.kind === "routine"
                ? description.facts.returns
                : null,
          };
        }),
    );
  }, [catalogState, connectionId, descriptions]);

  return (
    <div
      className={cn(
        "flex h-full min-h-0 flex-col overflow-hidden bg-surface-app",
        className,
      )}
    >
      {view.isLoading ? (
        <div
          data-testid="structure-loading"
          className="h-0.5 w-full animate-pulse bg-primary"
        />
      ) : null}
      {view.errorMessage ? (
        <ErrorBanner message={view.errorMessage} onRetry={view.retry} />
      ) : null}
      <div className="flex-1 overflow-auto p-3">
        <StructureContent
          schema={schema}
          tableName={tableName}
          view={view}
          showPreview={showPreview}
          onTogglePreview={() => setShowPreview((value) => !value)}
          roles={roles}
          functions={functions}
        />
      </div>
    </div>
  );
}

function StructureContent({
  schema,
  tableName,
  view,
  showPreview,
  onTogglePreview,
  roles,
  functions,
}: {
  schema: string;
  tableName: string;
  view: ReturnType<typeof useStructure>;
  showPreview: boolean;
  onTogglePreview: () => void;
  roles: string[];
  functions: TriggerFunctionOption[];
}) {
  const { capabilities, policy, editable } = view;
  const pg = view.queuePgOp
    ? {
        schema,
        table: tableName,
        queueOp: view.queuePgOp,
        hasPrimaryKey: (view.primaryKey?.length ?? 0) > 0,
      }
    : null;
  return (
    <div className="mx-auto max-w-5xl space-y-3">
      <Header
        schema={schema}
        tableName={tableName}
        tableEngine={view.tableEngine}
        primaryKey={view.primaryKey}
        primaryKeyBadge={policy.labels.primaryKeyBadge}
      />

      {view.showPhysicalLayout ? (
        <ClickHousePhysicalLayout
          partitionBy={view.partitionBy}
          sampleBy={view.sampleBy}
        />
      ) : null}

      <ColumnsSection
        columns={view.columns}
        editable={editable}
        pg={pg}
        onQueueChange={view.queueChange}
      />

      {editable ? (
        <PendingChangesSection
          pending={view.pending}
          previewSql={view.previewSql}
          pgPreview={view.pgPreview}
          showPreview={showPreview}
          commitStatus={view.commitStatus}
          commitDisabled={view.commitDisabled}
          lastOutcome={view.lastOutcome}
          onTogglePreview={onTogglePreview}
          onRemove={view.removePending}
          onCommit={() => {
            void view.commit();
          }}
        />
      ) : null}

      <PrimaryKeySection
        primaryKey={view.primaryKey}
        supported={capabilities.primaryKey}
        policy={policy}
      />

      <ForeignKeysSection
        foreignKeys={view.foreignKeys}
        supported={capabilities.foreignKeys}
        engine={view.engine}
        policy={policy}
        pg={pg}
      />

      <IndexesSection
        indexes={view.indexes}
        supported={capabilities.indexes}
        engine={view.engine}
        policy={policy}
        pg={pg}
      />

      <ConstraintsSection
        constraints={view.constraints}
        supported={capabilities.constraints}
        engine={view.engine}
        pg={pg}
      />

      <TriggersSection
        triggers={view.triggers}
        supported={capabilities.triggers}
        pg={pg}
        functions={functions}
      />

      <RowLevelSecuritySection
        rowSecurity={view.rowSecurity}
        policies={view.policies}
        supported={capabilities.policies}
        pg={pg}
        roles={roles}
      />

      <PrivilegesSection
        privileges={view.privileges}
        supported={capabilities.privileges}
        pg={pg}
        roles={roles}
      />
    </div>
  );
}

function Header({
  schema,
  tableName,
  tableEngine,
  primaryKey,
  primaryKeyBadge,
}: {
  schema: string;
  tableName: string;
  tableEngine: string | undefined;
  primaryKey: string[] | null;
  primaryKeyBadge: string;
}) {
  return (
    <header className="flex items-center justify-between gap-3 rounded-sm border border-border-subtle bg-surface-panel px-3 py-2">
      <div className="flex min-w-0 items-center gap-2">
        <div className="flex size-7 shrink-0 items-center justify-center rounded-sm border border-primary/25 bg-primary/10">
          <IconColumns3 className="size-3.5 text-primary" />
        </div>
        <div className="min-w-0">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <span>{schema}</span>
            <span>·</span>
            <span>{tableName}</span>
          </div>
          <h2 className="truncate font-mono text-sm font-semibold text-foreground">
            {tableName}
          </h2>
        </div>
      </div>
      <div className="flex items-center gap-1.5">
        {tableEngine ? (
          <Badge variant="outline" className="text-2xs">
            {tableEngine}
          </Badge>
        ) : null}
        {primaryKey && primaryKey.length > 0 ? (
          <Badge variant="secondary" className="text-2xs">
            {primaryKeyBadge} {primaryKey.join(", ")}
          </Badge>
        ) : null}
      </div>
    </header>
  );
}

function ErrorBanner({
  message,
  onRetry,
}: {
  message: string;
  onRetry: () => void;
}) {
  return (
    <div
      data-testid="structure-error"
      role="alert"
      className="flex items-center gap-2 border-b border-danger/40 bg-danger/10 px-4 py-2 text-xs text-danger"
    >
      <IconAlertTriangle className="size-4" />
      <span>Failed to load structure: {message}</span>
      <Button variant="ghost" size="sm" className="ml-auto" onClick={onRetry}>
        Retry
      </Button>
    </div>
  );
}
