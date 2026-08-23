/**
 * Database navigator (DESIGN-SYSTEM §5.5, P7): filter pinned at top,
 * schema/table tree with full keyboard support — the tree is one tab
 * stop with roving focus, arrows navigate/expand/collapse, type-ahead
 * jumps by name, Home/End, Enter opens. The palette's "Filter tables…"
 * command focuses the filter input.
 */

import {
  IconChevronDown,
  IconChevronRight,
  IconServer,
  IconTable,
} from "@tabler/icons-react";
import { useCallback, useMemo, useRef, useState } from "react";

import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/ui/state-panel";
import { useShortcutHandler } from "@/lib/shortcuts";
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

type NavigatorRow =
  | { kind: "schema"; id: string; name: string; expanded: boolean }
  | { kind: "table"; schemaId: string; schema: string; table: string };

const TYPE_AHEAD_RESET_MS = 700;

export function DatabaseNavigator({
  connectionId,
  schemas,
  activeTableKey,
  onOpenTable,
  className,
}: DatabaseNavigatorProps) {
  const [filter, setFilter] = useState("");
  const { expandedSchemas, toggleSchema } = useAppStore();
  const filterInputRef = useRef<HTMLInputElement>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  const [focusedIndex, setFocusedIndex] = useState(0);
  const typeAheadRef = useRef({ buffer: "", at: 0 });

  useShortcutHandler(
    "focus-navigator-filter",
    useCallback(() => filterInputRef.current?.focus(), []),
  );

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

  // Flat list of visible rows — the roving-focus model walks this.
  const rows = useMemo((): NavigatorRow[] => {
    const list: NavigatorRow[] = [];
    const filtering = filter.trim() !== "";
    for (const schema of filteredSchemas) {
      const schemaId = `${connectionId}:${schema.name}`;
      // While filtering, matches show with their ancestor chain open.
      const expanded = filtering || expandedSchemas.includes(schemaId);
      list.push({ kind: "schema", id: schemaId, name: schema.name, expanded });
      if (expanded) {
        for (const table of schema.tables) {
          list.push({
            kind: "table",
            schemaId,
            schema: schema.name,
            table,
          });
        }
      }
    }
    return list;
  }, [filteredSchemas, expandedSchemas, connectionId, filter]);

  const focusRow = useCallback((index: number) => {
    const el = treeRef.current?.querySelector<HTMLElement>(
      `[data-nav-row="${index}"]`,
    );
    el?.focus();
    el?.scrollIntoView({ block: "nearest" });
  }, []);

  const moveFocus = useCallback(
    (index: number) => {
      const clamped = Math.max(0, Math.min(rows.length - 1, index));
      setFocusedIndex(clamped);
      // Focus after state settles — the row is already rendered.
      requestAnimationFrame(() => focusRow(clamped));
    },
    [rows.length, focusRow],
  );

  const activateRow = useCallback(
    (row: NavigatorRow) => {
      if (row.kind === "schema") {
        toggleSchema(row.id);
      } else {
        onOpenTable(row.schema, row.table);
      }
    },
    [toggleSchema, onOpenTable],
  );

  const handleTreeKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (rows.length === 0) return;
      const row = rows[focusedIndex];
      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          moveFocus(focusedIndex + 1);
          return;
        case "ArrowUp":
          event.preventDefault();
          moveFocus(focusedIndex - 1);
          return;
        case "ArrowRight":
          event.preventDefault();
          if (row?.kind === "schema" && !row.expanded) toggleSchema(row.id);
          else moveFocus(focusedIndex + 1);
          return;
        case "ArrowLeft":
          event.preventDefault();
          if (row?.kind === "schema" && row.expanded) {
            toggleSchema(row.id);
          } else if (row?.kind === "table") {
            // Jump to the parent schema row.
            const parent = rows.findIndex(
              (candidate) =>
                candidate.kind === "schema" && candidate.id === row.schemaId,
            );
            if (parent !== -1) moveFocus(parent);
          }
          return;
        case "Home":
          event.preventDefault();
          moveFocus(0);
          return;
        case "End":
          event.preventDefault();
          moveFocus(rows.length - 1);
          return;
        case "Enter":
        case " ":
          event.preventDefault();
          if (row) activateRow(row);
          return;
        default:
          break;
      }
      // Type-ahead jump (§5.5).
      if (
        event.key.length === 1 &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.altKey
      ) {
        const now = Date.now();
        const state = typeAheadRef.current;
        const buffer =
          now - state.at > TYPE_AHEAD_RESET_MS
            ? event.key.toLowerCase()
            : state.buffer + event.key.toLowerCase();
        typeAheadRef.current = { buffer, at: now };
        const nameOf = (candidate: NavigatorRow) =>
          candidate.kind === "schema" ? candidate.name : candidate.table;
        const start = buffer.length === 1 ? focusedIndex + 1 : focusedIndex;
        for (let step = 0; step < rows.length; step += 1) {
          const index = (start + step) % rows.length;
          if (nameOf(rows[index]).toLowerCase().startsWith(buffer)) {
            moveFocus(index);
            return;
          }
        }
      }
    },
    [rows, focusedIndex, moveFocus, toggleSchema, activateRow],
  );

  const indexByKey = useMemo(() => {
    const map = new Map<string, number>();
    rows.forEach((row, index) => {
      map.set(
        row.kind === "schema" ? row.id : `${row.schemaId}::${row.table}`,
        index,
      );
    });
    return map;
  }, [rows]);

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
          ref={filterInputRef}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Filter tables"
          aria-label="Filter tables"
          className="h-7 text-xs"
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              moveFocus(0);
            } else if (event.key === "Escape" && filter) {
              event.stopPropagation();
              setFilter("");
            }
          }}
        />
      </div>
      <div
        ref={treeRef}
        role="tree"
        aria-label="Schemas and tables"
        data-slot="navigator-tree"
        tabIndex={rows.length > 0 ? -1 : undefined}
        onKeyDown={handleTreeKeyDown}
        className="min-h-0 flex-1 overflow-auto p-1.5"
      >
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
          const isExpanded =
            filter.trim() !== "" || expandedSchemas.includes(schemaId);
          const schemaRowIndex = indexByKey.get(schemaId) ?? 0;
          return (
            <div key={schemaId} className="mb-1">
              <button
                type="button"
                role="treeitem"
                aria-expanded={isExpanded}
                aria-level={1}
                data-nav-row={schemaRowIndex}
                tabIndex={focusedIndex === schemaRowIndex ? 0 : -1}
                onFocus={() => setFocusedIndex(schemaRowIndex)}
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
                <span className="ml-auto shrink-0 text-2xs text-text-disabled">
                  {schema.tables.length}
                </span>
              </button>
              {isExpanded
                ? schema.tables.map((table) => {
                    const key = tableKey(schema.name, table);
                    const on = activeTableKey === key;
                    const tableRowIndex =
                      indexByKey.get(`${schemaId}::${table}`) ?? 0;
                    return (
                      <button
                        key={key}
                        type="button"
                        role="treeitem"
                        aria-level={2}
                        aria-selected={on || undefined}
                        data-nav-row={tableRowIndex}
                        tabIndex={focusedIndex === tableRowIndex ? 0 : -1}
                        onFocus={() => setFocusedIndex(tableRowIndex)}
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
