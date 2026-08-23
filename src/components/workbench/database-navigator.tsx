import {
  IconChevronDown,
  IconChevronRight,
  IconServer,
  IconTable,
} from "@tabler/icons-react";
import { useMemo, useState } from "react";

import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/ui/state-panel";
import { type SchemaExplorer, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

interface DatabaseNavigatorProps {
  connectionId: string;
  schemas: SchemaExplorer[];
  activeTableKey: string | null;
  onOpenTable: (schema: string, table: string) => void;
  className?: string;
}

function tableKey(schema: string, table: string): string {
  return `${schema}.${table}`;
}

export function DatabaseNavigator({
  connectionId,
  schemas,
  activeTableKey,
  onOpenTable,
  className,
}: DatabaseNavigatorProps) {
  const [filter, setFilter] = useState("");
  const expandedSchemas = useAppStore((state) => state.expandedSchemas);
  const toggleSchema = useAppStore((state) => state.toggleSchema);

  const filteredSchemas = useMemo(() => {
    const needle = filter.trim().toLowerCase();
    if (!needle) return schemas;
    return schemas
      .map((schema) => ({
        ...schema,
        tables: schema.tables.filter((name) =>
          name.toLowerCase().includes(needle),
        ),
      }))
      .filter((schema) => schema.tables.length > 0);
  }, [filter, schemas]);

  return (
    <aside
      aria-label="Database navigator"
      className={cn(
        "flex w-56 shrink-0 flex-col border-r border-border-subtle bg-surface-sidebar",
        className,
      )}
    >
      <div className="border-b border-border-subtle px-3 py-2 text-2xs font-semibold uppercase tracking-wider text-text-muted">
        Database navigator
      </div>
      <div className="px-2 py-2">
        <Input
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Filter tables"
          aria-label="Filter tables"
          className="h-7 text-xs"
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-1.5">
        {filteredSchemas.length === 0 ? (
          <EmptyState
            title={
              schemas.length === 0
                ? "Connect to load schemas"
                : "No tables match"
            }
            className="h-auto px-2 py-4"
          />
        ) : null}
        {filteredSchemas.map((schema) => {
          const schemaId = `${connectionId}:${schema.name}`;
          const isExpanded = expandedSchemas.includes(schemaId);
          return (
            <div key={schemaId} className="mb-1">
              <button
                type="button"
                onClick={() => toggleSchema(schemaId)}
                className="flex w-full items-center gap-1.5 rounded px-2 py-1 text-2xs font-medium text-text-secondary hover:bg-surface-panel"
              >
                {isExpanded ? (
                  <IconChevronDown className="size-3 shrink-0" />
                ) : (
                  <IconChevronRight className="size-3 shrink-0" />
                )}
                <IconServer className="size-3.5 shrink-0 text-text-disabled" />
                <span className="truncate">{schema.name}</span>
              </button>
              {isExpanded
                ? schema.tables.map((table) => {
                    const key = tableKey(schema.name, table);
                    const on = activeTableKey === key;
                    return (
                      <button
                        key={key}
                        type="button"
                        onClick={() => onOpenTable(schema.name, table)}
                        className={cn(
                          "flex w-full items-center gap-2 rounded py-1.5 pr-2 pl-6 text-left text-xs transition-colors",
                          on
                            ? "bg-accent-subdued text-foreground"
                            : "text-text-muted hover:bg-surface-panel hover:text-foreground",
                        )}
                      >
                        <IconTable
                          className={cn(
                            "size-3.5 shrink-0",
                            on ? "text-accent" : "text-text-disabled",
                          )}
                        />
                        <span className="truncate">{table}</span>
                      </button>
                    );
                  })
                : null}
            </div>
          );
        })}
      </div>
    </aside>
  );
}
