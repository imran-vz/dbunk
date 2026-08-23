import { IconGitBranch } from "@tabler/icons-react";

import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import { Button } from "@/components/ui/button";
import { SchemaMapGlossaryButton } from "@/components/workspace-overview/schema-map-glossary";
import { tableSchemaMapScope } from "@/lib/schema-graph";
import { useAppStore } from "@/lib/store";

interface SchemaMapSubTabProps {
  connectionId: string;
  schema: string;
  tableName: string;
}

/**
 * Table-Level Schema Map subtab — the current Table Card plus its
 * directly referencing and directly referenced Table Cards, reusing
 * the global Schema Map's notation, Relationship Detail Popover,
 * focus/dimming, dragging, and glossary behavior. Layout persists
 * under a dedicated `(connection, table scope)` key.
 */
export function SchemaMapSubTab({
  connectionId,
  schema,
  tableName,
}: SchemaMapSubTabProps) {
  const resetSchemaMapPositions = useAppStore(
    (state) => state.resetSchemaMapPositions,
  );

  return (
    <div
      data-testid="table-schema-map-subtab"
      className="flex h-full min-h-0 flex-col overflow-hidden bg-[#080c10]"
    >
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-2">
        <span className="text-2xs font-medium text-text-muted">
          Direct relationships of {schema}.{tableName}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() =>
              void resetSchemaMapPositions(
                connectionId,
                tableSchemaMapScope(schema, tableName),
              )
            }
          >
            <IconGitBranch className="size-3" />
            Reset
          </Button>
          <SchemaMapGlossaryButton />
        </div>
      </div>
      <div className="min-h-0 flex-1">
        <SchemaRelationshipMap
          connectionId={connectionId}
          schema={schema}
          activeTable={tableName}
          tableScope={{ schema, table: tableName }}
        />
      </div>
    </div>
  );
}
