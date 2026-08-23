import { IconAlertTriangle, IconKey, IconPlus } from "@tabler/icons-react";

import { IndexesSection } from "@/components/table-structure/read-only-sections";
import { UnsupportedNotice } from "@/components/table-structure/shared";
import { useStructure } from "@/components/table-structure/use-structure";
import { Button } from "@/components/ui/button";

import type { SubTab } from "./header";

interface IndexesSubTabProps {
  connectionId: string;
  schema: string;
  tableName: string;
  onOpenSpecialized: (subTab: SubTab) => void;
}

export function IndexesSubTab({
  connectionId,
  schema,
  tableName,
  onOpenSpecialized,
}: IndexesSubTabProps) {
  const view = useStructure({ connectionId, schema, tableName });

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-surface-app">
      {view.isLoading ? (
        <div className="h-0.5 w-full animate-pulse bg-primary" />
      ) : null}
      {view.errorMessage ? (
        <div
          role="alert"
          className="flex items-start gap-2 border-b border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
        >
          <IconAlertTriangle className="mt-0.5 size-3.5 shrink-0" />
          <div className="flex-1 whitespace-pre-wrap font-mono">
            {view.errorMessage}
          </div>
          <Button size="sm" variant="ghost" onClick={view.retry}>
            Retry
          </Button>
        </div>
      ) : null}
      <div className="flex-1 overflow-auto p-3">
        <div className="mx-auto flex max-w-5xl flex-col gap-3">
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <span className="flex size-7 items-center justify-center rounded-sm border border-border-subtle bg-surface-panel text-text-secondary">
                <IconKey className="size-3.5" />
              </span>
              <div>
                <h2 className="text-sm font-semibold text-foreground">
                  Indexes
                </h2>
                <p className="text-2xs text-text-muted">
                  {schema}.{tableName}
                </p>
              </div>
            </div>
            <Button
              size="sm"
              variant="outline"
              onClick={() => onOpenSpecialized("specialized")}
              aria-label="Open the index builder"
            >
              <IconPlus />
              New index
            </Button>
          </div>

          {view.capabilities.indexes ? (
            <IndexesSection
              indexes={view.indexes}
              supported
              engine={view.engine}
              policy={view.policy}
            />
          ) : (
            <UnsupportedNotice engine={view.engine} feature="Indexes" />
          )}
        </div>
      </div>
    </div>
  );
}
