/**
 * Database navigator: one keyboard-navigable tree over schemas, typed
 * PostgreSQL catalog groups, and database-scoped list-only entries.
 *
 * PostgreSQL renders directly from the Object Catalog. Other relational
 * engines keep the legacy schema/table adapter until they gain typed object
 * contracts of their own.
 */

import {
  IconChevronDown,
  IconChevronRight,
  IconDatabase,
  IconDots,
  IconEye,
  IconPlus,
  IconRefresh,
  IconServer,
  IconTable,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { EmptyState } from "@/components/ui/state-panel";
import {
  buildNavigatorRows,
  createKindForGroup,
  type CreatableObjectKind,
  isBrowseRelation,
  type NavigatorRow,
  SCHEMA_GROUPS,
  tableKey,
} from "@/components/workbench/database-navigator-model";
import { useShortcutHandler } from "@/lib/shortcuts";
import {
  formatPgCatalogError,
  isConnectedStatus,
  type PgObjectRef,
  type SchemaExplorer,
  useAppStore,
} from "@/lib/store";
import { cn } from "@/lib/utils";

interface DatabaseNavigatorProps {
  connectionId: string;
  schemas: SchemaExplorer[];
  activeTableKey: string | null;
  onOpenTable: (schema: string, table: string) => void;
  onOpenView?: (schema: string, view: string) => void;
  onOpenObject?: (reference: PgObjectRef) => void;
  onCreateObject?: (kind: CreatableObjectKind, schema?: string) => void;
  className?: string;
}

const TYPE_AHEAD_RESET_MS = 700;

export function DatabaseNavigator({
  connectionId,
  schemas,
  activeTableKey,
  onOpenTable,
  onOpenView,
  onOpenObject,
  onCreateObject,
  className,
}: DatabaseNavigatorProps) {
  const [filter, setFilter] = useState("");
  const [expandedLimits, setExpandedLimits] = useState<Set<string>>(
    () => new Set(),
  );
  const expandedSchemas = useAppStore((state) => state.expandedSchemas);
  const expandedNavigatorGroups = useAppStore(
    (state) => state.expandedNavigatorGroups,
  );
  const toggleSchema = useAppStore((state) => state.toggleSchema);
  const toggleNavigatorGroup = useAppStore(
    (state) => state.toggleNavigatorGroup,
  );
  const loadPgObjectCatalog = useAppStore((state) => state.loadPgObjectCatalog);
  const catalogState = useAppStore(
    (state) => state.pgObjectCatalog[connectionId],
  );
  const connection = useAppStore((state) =>
    state.connections.find((candidate) => candidate.id === connectionId),
  );
  const connectionTransitioning = useAppStore((state) =>
    state.connectionTransitionIds.includes(connectionId),
  );
  const filterInputRef = useRef<HTMLInputElement>(null);
  const treeRef = useRef<HTMLDivElement>(null);
  const [focusedIndex, setFocusedIndex] = useState(0);
  const typeAheadRef = useRef({ buffer: "", at: 0 });

  const isPostgres = connection?.engine === "PostgreSQL";
  const createObject =
    isPostgres &&
    isConnectedStatus(connection.status) &&
    !connectionTransitioning
      ? onCreateObject
      : undefined;
  const needle = filter.trim().toLowerCase();

  useShortcutHandler(
    "focus-navigator-filter",
    useCallback(() => filterInputRef.current?.focus(), []),
  );

  const rows = useMemo(
    () =>
      buildNavigatorRows({
        connectionId,
        isPostgres,
        schemas,
        catalog: catalogState?.catalog,
        needle,
        expandedSchemas,
        expandedNavigatorGroups,
        expandedLimits,
      }),
    [
      catalogState?.catalog,
      connectionId,
      expandedLimits,
      expandedNavigatorGroups,
      expandedSchemas,
      isPostgres,
      needle,
      schemas,
    ],
  );

  useEffect(() => {
    setFocusedIndex((current) =>
      rows.length === 0 ? 0 : Math.min(current, rows.length - 1),
    );
  }, [rows.length]);

  const focusRow = useCallback((index: number) => {
    const element = treeRef.current?.querySelector<HTMLElement>(
      `[data-nav-row="${index}"]`,
    );
    element?.focus();
    element?.scrollIntoView?.({ block: "nearest" });
  }, []);

  const moveFocus = useCallback(
    (index: number) => {
      const clamped = Math.max(0, Math.min(rows.length - 1, index));
      setFocusedIndex(clamped);
      requestAnimationFrame(() => focusRow(clamped));
    },
    [focusRow, rows.length],
  );

  const activateRow = useCallback(
    (row: NavigatorRow) => {
      switch (row.kind) {
        case "schema":
          toggleSchema(row.id);
          return;
        case "group":
          toggleNavigatorGroup(row.id);
          return;
        case "legacy-table":
          onOpenTable(row.schema, row.name);
          return;
        case "object":
          if (row.reference.kind === "table") {
            onOpenTable(row.reference.schema ?? "", row.reference.name);
          } else if (isBrowseRelation(row.reference.kind)) {
            onOpenView?.(row.reference.schema ?? "", row.reference.name);
          } else {
            onOpenObject?.(row.reference);
          }
          return;
        case "show-more":
          setExpandedLimits((current) => {
            const next = new Set(current);
            next.add(row.parentId);
            return next;
          });
          return;
        case "database":
        case "list-only":
        case "truncated":
          return;
      }
    },
    [onOpenObject, onOpenTable, onOpenView, toggleNavigatorGroup, toggleSchema],
  );

  const moveToParent = useCallback(
    (row: NavigatorRow) => {
      if (row.kind === "schema" || row.kind === "database") return;
      const parent = rows.findIndex(
        (candidate) => candidate.id === row.parentId,
      );
      if (parent !== -1) moveFocus(parent);
    },
    [moveFocus, rows],
  );

  const activateSecondary = useCallback((index: number): boolean => {
    const secondary = treeRef.current?.querySelector<HTMLElement>(
      `[data-nav-row-container="${index}"] [data-nav-secondary]`,
    );
    if (!secondary) return false;
    secondary.focus();
    secondary.click();
    return true;
  }, []);

  const handleTreeKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (rows.length === 0) return;
      let activeIndex = focusedIndex;
      const secondaryTarget =
        event.target instanceof HTMLElement &&
        event.target.closest<HTMLElement>("[data-nav-secondary]");
      if (secondaryTarget) {
        const container = secondaryTarget.closest<HTMLElement>(
          "[data-nav-row-container]",
        );
        const parsedIndex = Number(container?.dataset.navRowContainer);
        if (Number.isInteger(parsedIndex)) activeIndex = parsedIndex;
        if (event.key === "Escape") {
          event.preventDefault();
          moveFocus(activeIndex);
          return;
        }
        // Keep native activation/menu keys on the secondary control. Tree
        // navigation and typeahead below recover from that control's focus.
        if (
          event.key === "Enter" ||
          event.key === " " ||
          event.key === "ContextMenu" ||
          (event.key === "F10" && event.shiftKey)
        ) {
          return;
        }
      }
      if (
        ((event.key === "Enter" && event.shiftKey) ||
          event.key === "ContextMenu" ||
          (event.key === "F10" && event.shiftKey)) &&
        activateSecondary(activeIndex)
      ) {
        event.preventDefault();
        return;
      }
      const row = rows[activeIndex];
      switch (event.key) {
        case "ArrowDown":
          event.preventDefault();
          moveFocus(activeIndex + 1);
          return;
        case "ArrowUp":
          event.preventDefault();
          moveFocus(activeIndex - 1);
          return;
        case "ArrowRight":
          event.preventDefault();
          if (
            row &&
            (row.kind === "schema" || row.kind === "group") &&
            !row.expanded
          ) {
            activateRow(row);
          } else {
            moveFocus(activeIndex + 1);
          }
          return;
        case "ArrowLeft":
          event.preventDefault();
          if (
            row &&
            (row.kind === "schema" || row.kind === "group") &&
            row.expanded
          ) {
            activateRow(row);
          } else if (row) {
            moveToParent(row);
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
      if (
        event.key.length === 1 &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.altKey
      ) {
        const now = Date.now();
        const current = typeAheadRef.current;
        const buffer =
          now - current.at > TYPE_AHEAD_RESET_MS
            ? event.key.toLowerCase()
            : current.buffer + event.key.toLowerCase();
        typeAheadRef.current = { buffer, at: now };
        const start = buffer.length === 1 ? activeIndex + 1 : activeIndex;
        for (let step = 0; step < rows.length; step += 1) {
          const index = (start + step) % rows.length;
          if (rows[index]?.name.toLowerCase().startsWith(buffer)) {
            moveFocus(index);
            return;
          }
        }
      }
    },
    [
      activateRow,
      activateSecondary,
      focusedIndex,
      moveFocus,
      moveToParent,
      rows,
    ],
  );

  const renderRow = (row: NavigatorRow, index: number) => {
    const focused = focusedIndex === index;
    const rowProps = {
      "data-nav-row": index,
      tabIndex: focused ? 0 : -1,
      onFocus: () => setFocusedIndex(index),
      onClick: () => activateRow(row),
    } as const;

    if (row.kind === "schema") {
      return (
        <div
          key={row.id}
          data-nav-row-container={index}
          className="group relative"
        >
          <button
            {...rowProps}
            type="button"
            role="treeitem"
            aria-label={row.name}
            aria-expanded={row.expanded}
            aria-level={1}
            className="flex min-h-7 w-full items-center gap-1.5 rounded py-1 pr-8 pl-2 text-2xs font-medium text-text-secondary hover:bg-surface-panel"
          >
            {row.expanded ? (
              <IconChevronDown className="size-3 shrink-0" />
            ) : (
              <IconChevronRight className="size-3 shrink-0" />
            )}
            <IconServer className="size-3.5 shrink-0 text-text-disabled" />
            <span className="truncate">{row.name}</span>
            <span className="ml-auto shrink-0 text-2xs text-text-disabled">
              {row.count}
            </span>
          </button>
          {isPostgres && (createObject || onOpenObject) ? (
            <DropdownMenu
              onOpenChange={(open) => {
                if (!open) moveFocus(index);
              }}
            >
              <DropdownMenuTrigger
                data-nav-secondary
                tabIndex={-1}
                aria-label={`Actions for schema ${row.name}`}
                title="Schema actions (Shift+Enter)"
                onClick={(event) => event.stopPropagation()}
                className="absolute top-1/2 right-1 grid size-6 -translate-y-1/2 place-items-center rounded text-text-disabled opacity-0 hover:bg-surface-panel-elevated hover:text-foreground focus:opacity-100 group-hover:opacity-100"
              >
                <IconDots className="size-3.5" />
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-44">
                {onOpenObject ? (
                  <DropdownMenuItem
                    onClick={() =>
                      onOpenObject({
                        kind: "schema",
                        schema: null,
                        name: row.name,
                        identityArgs: null,
                      })
                    }
                  >
                    Open schema viewer
                  </DropdownMenuItem>
                ) : null}
                {createObject
                  ? (
                      [
                        ["view", "View"],
                        ["materialized-view", "Materialized view"],
                        ["sequence", "Sequence"],
                        ["enum", "Enum"],
                      ] as const
                    ).map(([kind, label]) => (
                      <DropdownMenuItem
                        key={kind}
                        onClick={() => createObject(kind, row.name)}
                      >
                        New {label.toLowerCase()}
                      </DropdownMenuItem>
                    ))
                  : null}
              </DropdownMenuContent>
            </DropdownMenu>
          ) : null}
        </div>
      );
    }

    if (row.kind === "database") {
      return (
        <button
          {...rowProps}
          key={row.id}
          type="button"
          role="treeitem"
          aria-level={1}
          aria-disabled="true"
          className="mt-2 flex min-h-7 w-full items-center gap-1.5 border-t border-border-subtle px-2 pt-2 pb-1 text-2xs font-medium text-text-secondary"
        >
          <span className="size-3 shrink-0" />
          <IconDatabase className="size-3.5 shrink-0 text-text-disabled" />
          <span className="truncate">{row.name}</span>
          <span className="ml-auto shrink-0 text-2xs text-text-disabled">
            {row.count}
          </span>
        </button>
      );
    }

    if (row.kind === "group") {
      const createKind = createKindForGroup(row.group);
      return (
        <div
          key={row.id}
          data-nav-row-container={index}
          className="group relative"
        >
          <button
            {...rowProps}
            type="button"
            role="treeitem"
            aria-label={row.name}
            aria-expanded={row.expanded}
            aria-level={2}
            className="flex min-h-7 w-full items-center gap-1.5 rounded py-1 pr-8 pl-5 text-2xs font-medium text-text-secondary hover:bg-surface-panel"
          >
            {row.expanded ? (
              <IconChevronDown className="size-3 shrink-0" />
            ) : (
              <IconChevronRight className="size-3 shrink-0" />
            )}
            <span
              aria-hidden="true"
              className="w-7 shrink-0 font-mono text-2xs text-text-disabled"
            >
              {row.abbreviation}
            </span>
            <span className="truncate">{row.name}</span>
            <span className="ml-auto shrink-0 text-2xs text-text-disabled">
              {row.count}
            </span>
          </button>
          {createKind && createObject ? (
            <button
              type="button"
              data-nav-secondary
              tabIndex={-1}
              aria-label={`New ${createKind === "materialized-view" ? "materialized view" : createKind}`}
              title={`New ${createKind}`}
              onClick={(event) => {
                event.stopPropagation();
                createObject(createKind, row.schema);
                moveFocus(index);
              }}
              className="absolute top-1/2 right-1 grid size-6 -translate-y-1/2 place-items-center rounded text-text-disabled opacity-0 hover:bg-surface-panel-elevated hover:text-foreground focus:opacity-100 group-hover:opacity-100"
            >
              <IconPlus className="size-3.5" />
            </button>
          ) : null}
        </div>
      );
    }

    if (row.kind === "legacy-table") {
      const active = activeTableKey === tableKey(row.schema, row.name);
      return (
        <button
          {...rowProps}
          key={row.id}
          type="button"
          role="treeitem"
          aria-level={2}
          aria-selected={active || undefined}
          className={cn(
            "flex min-h-7 w-full items-center gap-2 rounded py-1 pr-2 pl-6 text-left text-xs transition-colors",
            active
              ? "bg-accent-subdued text-foreground"
              : "text-text-muted hover:bg-surface-panel hover:text-foreground",
          )}
        >
          <IconTable
            className={cn(
              "size-3.5 shrink-0",
              active ? "text-accent" : "text-text-disabled",
            )}
          />
          <span className="truncate">{row.name}</span>
        </button>
      );
    }

    if (row.kind === "object") {
      const active =
        row.reference.kind === "table" &&
        activeTableKey ===
          tableKey(row.reference.schema ?? "", row.reference.name);
      const showViewerAction =
        (row.reference.kind === "table" ||
          isBrowseRelation(row.reference.kind)) &&
        onOpenObject !== undefined;
      return (
        <div
          key={row.id}
          data-nav-row-container={index}
          className="group relative"
        >
          <button
            {...rowProps}
            type="button"
            role="treeitem"
            aria-label={row.displayName}
            aria-level={3}
            aria-selected={active || undefined}
            className={cn(
              "flex min-h-7 w-full items-center gap-2 rounded py-1 pr-7 pl-10 text-left text-xs transition-colors",
              active
                ? "bg-accent-subdued text-foreground"
                : "text-text-muted hover:bg-surface-panel hover:text-foreground",
            )}
          >
            {row.reference.kind === "table" ? (
              <IconTable
                className={cn(
                  "size-3.5 shrink-0",
                  active ? "text-accent" : "text-text-disabled",
                )}
              />
            ) : (
              <span
                aria-hidden="true"
                className="w-5 shrink-0 font-mono text-2xs text-text-disabled"
              >
                {SCHEMA_GROUPS.find(
                  (group) => group.objectKind === row.reference.kind,
                )?.abbreviation ?? "OBJ"}
              </span>
            )}
            <span className="truncate">{row.displayName}</span>
            {row.typeClass ? (
              <span className="ml-auto text-2xs text-text-disabled">
                {row.typeClass}
              </span>
            ) : null}
          </button>
          {showViewerAction ? (
            <button
              type="button"
              data-nav-secondary
              tabIndex={-1}
              aria-label={`Open object viewer for ${row.displayName}`}
              title="Open object viewer"
              onClick={(event) => {
                event.stopPropagation();
                onOpenObject(row.reference);
                moveFocus(index);
              }}
              className="absolute top-1/2 right-1 grid size-6 -translate-y-1/2 place-items-center rounded text-text-disabled opacity-0 hover:bg-surface-panel-elevated hover:text-foreground focus:opacity-100 group-hover:opacity-100"
            >
              <IconEye className="size-3.5" />
            </button>
          ) : null}
        </div>
      );
    }

    if (row.kind === "list-only") {
      return (
        <button
          {...rowProps}
          key={row.id}
          type="button"
          role="treeitem"
          aria-label={row.name}
          aria-level={3}
          aria-disabled="true"
          title="List only. Description and lifecycle actions are deferred."
          className="flex min-h-7 w-full cursor-not-allowed items-center gap-2 rounded py-1 pr-2 pl-10 text-left text-xs text-text-disabled"
        >
          <span aria-hidden="true" className="w-5 shrink-0 font-mono text-2xs">
            {row.abbreviation}
          </span>
          <span className="truncate">{row.name}</span>
        </button>
      );
    }

    if (row.kind === "show-more") {
      return (
        <button
          {...rowProps}
          key={row.id}
          type="button"
          role="treeitem"
          aria-label={`Show ${row.remaining} more`}
          aria-level={3}
          className="flex min-h-7 w-full items-center gap-2 rounded py-1 pr-2 pl-10 text-left text-2xs text-semantic-info hover:bg-surface-panel"
        >
          <span aria-hidden="true" className="w-5 shrink-0 font-mono">
            ···
          </span>
          Show {row.remaining} more
        </button>
      );
    }

    return (
      <button
        {...rowProps}
        key={row.id}
        type="button"
        role="treeitem"
        aria-level={3}
        aria-disabled="true"
        className="flex min-h-7 w-full cursor-default items-start gap-2 py-1.5 pr-2 pl-10 text-left text-2xs leading-relaxed text-semantic-warning"
      >
        <span className="w-5 shrink-0 font-mono">!</span>
        <span>{row.name}</span>
      </button>
    );
  };

  const catalogError = isPostgres && catalogState?.status === "error";
  const catalogLoading =
    isPostgres &&
    (catalogState?.status === "loading" ||
      (isConnectedStatus(connection?.status) && !catalogState));

  return (
    <aside
      aria-label="Database navigator"
      className={cn(
        "flex w-56 shrink-0 flex-col border-r border-border-subtle bg-surface-sidebar",
        className,
      )}
    >
      <div className="flex min-h-9 items-center border-b border-border-subtle px-3 py-2 text-2xs font-semibold uppercase tracking-wider text-text-muted">
        <span>Database navigator</span>
        {isPostgres && createObject ? (
          <button
            type="button"
            aria-label="New schema"
            title="New schema"
            onClick={() => createObject("schema")}
            className="ml-auto grid size-6 place-items-center rounded text-text-disabled hover:bg-surface-panel hover:text-foreground"
          >
            <IconPlus className="size-3.5" />
          </button>
        ) : null}
      </div>
      <div className="px-2 py-2">
        <Input
          ref={filterInputRef}
          value={filter}
          onChange={(event) => setFilter(event.target.value)}
          placeholder="Filter objects"
          aria-label="Filter objects"
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
      {catalogError ? (
        <div
          role="alert"
          className="mx-2 mb-2 border border-semantic-danger/40 bg-semantic-danger/10 px-2.5 py-2 text-2xs leading-relaxed text-semantic-danger"
        >
          <div className="flex items-start gap-2">
            <span className="min-w-0 flex-1">
              <strong className="block font-semibold">
                Catalog unavailable
              </strong>
              {catalogState.error
                ? formatPgCatalogError(catalogState.error)
                : "Object metadata could not be loaded."}
            </span>
            <button
              type="button"
              onClick={() => void loadPgObjectCatalog(connectionId)}
              className="flex shrink-0 items-center gap-1 text-2xs font-medium text-foreground hover:underline"
            >
              <IconRefresh className="size-3" />
              Retry
            </button>
          </div>
        </div>
      ) : null}
      <div
        ref={treeRef}
        role="tree"
        aria-label={
          isPostgres ? "Schemas and database objects" : "Schemas and tables"
        }
        data-slot="navigator-tree"
        tabIndex={rows.length > 0 ? -1 : undefined}
        onKeyDown={handleTreeKeyDown}
        className="min-h-0 flex-1 overflow-auto p-1.5"
      >
        {catalogLoading ? (
          <div className="px-2 py-4 text-center text-xs text-text-muted">
            Loading object catalog…
          </div>
        ) : null}
        {!catalogLoading && !catalogError && rows.length === 0 ? (
          <EmptyState
            title={
              isConnectedStatus(connection?.status)
                ? filter
                  ? "No objects match"
                  : "No database objects"
                : "Connect to load schemas"
            }
            className="h-auto px-2 py-4"
          />
        ) : null}
        {rows.map(renderRow)}
      </div>
    </aside>
  );
}
