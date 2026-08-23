/* oxlint-disable anti-slop/require-safety-comment-for-type-assertion -- Select values are constrained to the operator and nulls unions at this UI boundary. */
import { IconCopy, IconX } from "@tabler/icons-react";
import { useCallback, useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  type BrowseDraftOperator,
  type BrowseFilter,
  type BrowseNulls,
  type BrowseSortDirection,
  type ComparisonOperator,
  type TextMatchOperator,
  browseOperatorNeedsValue,
  buildBrowseFilter,
} from "@/lib/table-browse";
import { formatTableBrowseError } from "@/lib/table-browse-error";
import { cn } from "@/lib/utils";

import type { ServerBrowseGridModel } from "./browse-model";

const COMPARISON_OPS: Array<{
  id: ComparisonOperator;
  label: string;
  symbol: string;
}> = [
  { id: "eq", label: "equals", symbol: "=" },
  { id: "neq", label: "not equals", symbol: "<>" },
  { id: "gt", label: "greater", symbol: ">" },
  { id: "gte", label: "greater or equals", symbol: ">=" },
  { id: "lt", label: "less", symbol: "<" },
  { id: "lte", label: "less or equals", symbol: "<=" },
];

const TEXT_OPS: Array<{
  id: TextMatchOperator;
  label: string;
  symbol: string;
}> = [
  { id: "contains", label: "contains", symbol: "LIKE" },
  { id: "notContains", label: "not contains", symbol: "NOT LIKE" },
  { id: "startsWith", label: "starts with", symbol: "STARTS" },
  { id: "endsWith", label: "ends with", symbol: "ENDS" },
];

const NULL_OPS = [
  { id: "isNull", label: "is null", symbol: "IS NULL" },
  { id: "isNotNull", label: "is not null", symbol: "IS NOT NULL" },
] as const;

const IN_LIST_OP = { id: "inList", label: "in list", symbol: "IN" } as const;

type DraftOperator = BrowseDraftOperator;

const ALL_OPS: Array<{ id: DraftOperator; label: string; symbol: string }> = [
  ...COMPARISON_OPS,
  ...TEXT_OPS,
  IN_LIST_OP,
  ...NULL_OPS,
];

const filterChipLabel = (filter: BrowseFilter): string => {
  switch (filter.kind) {
    case "comparison":
      return `${filter.column} ${COMPARISON_OPS.find((op) => op.id === filter.operator)?.symbol ?? filter.operator} ${filter.value}`;
    case "textMatch":
      return `${filter.column} ${TEXT_OPS.find((op) => op.id === filter.operator)?.label ?? filter.operator} ${filter.value}`;
    case "isNull":
      return `${filter.column} IS NULL`;
    case "isNotNull":
      return `${filter.column} IS NOT NULL`;
    case "inList":
      return `${filter.column} IN (${filter.values.join(", ")})`;
    case "rawSql":
      return filter.text;
  }
};

export function BrowseFilterBar({
  columnNames,
  browse,
}: {
  columnNames: string[];
  browse: ServerBrowseGridModel;
}) {
  const [draftColumn, setDraftColumn] = useState(columnNames[0] ?? "");
  const [draftOperator, setDraftOperator] = useState<DraftOperator>("eq");
  const [draftValue, setDraftValue] = useState("");
  const [rawDraft, setRawDraft] = useState(browse.rawFilterText);

  useEffect(() => {
    if (columnNames.length === 0) return;
    setDraftColumn((prev) =>
      columnNames.includes(prev) ? prev : (columnNames[0] ?? ""),
    );
  }, [columnNames]);

  useEffect(() => {
    setRawDraft(browse.rawFilterText);
  }, [browse.rawFilterText]);

  const needsValue = browseOperatorNeedsValue(draftOperator);
  const canApply =
    buildBrowseFilter(draftColumn, draftOperator, draftValue) !== null;

  const applyDraft = useCallback(() => {
    const filter = buildBrowseFilter(draftColumn, draftOperator, draftValue);
    if (!filter) return;
    browse.onApplyTypedFilter(filter);
    setDraftValue("");
  }, [browse, draftColumn, draftOperator, draftValue]);

  const databaseError = browse.error?.kind === "database" ? browse.error : null;

  return (
    <div className="flex min-h-10 flex-wrap items-center gap-1.5 border-b border-border-subtle bg-surface-window px-3 py-1.5">
      <div className="flex items-center gap-1">
        <Button
          size="sm"
          variant={browse.filterMode === "typed" ? "secondary" : "ghost"}
          onClick={() => browse.onFilterModeChange("typed")}
        >
          Typed
        </Button>
        <Button
          size="sm"
          variant={browse.filterMode === "raw" ? "secondary" : "ghost"}
          onClick={() => browse.onFilterModeChange("raw")}
        >
          WHERE
        </Button>
      </div>

      {browse.typedFilters.length > 0 ? (
        <div className="flex flex-wrap items-center gap-1">
          {browse.typedFilters.map((filter) => (
            <div
              key={filterChipLabel(filter)}
              className="flex h-6 items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel px-2 text-xs"
            >
              <span className="max-w-64 truncate">
                {filterChipLabel(filter)}
              </span>
              {filter.kind !== "rawSql" ? (
                <button
                  type="button"
                  className="rounded-sm text-muted-foreground hover:text-foreground"
                  onClick={() =>
                    browse.onRemoveTypedFilter(
                      "column" in filter ? filter.column : "",
                    )
                  }
                  aria-label={`Remove filter ${filterChipLabel(filter)}`}
                >
                  <IconX className="size-3" />
                </button>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}

      {browse.filterMode === "raw" ? (
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5">
          <Input
            className="h-6 min-w-56 flex-1 font-mono text-xs"
            aria-label="Raw WHERE fragment"
            placeholder="email ILIKE '%@dbunk.dev'"
            value={rawDraft}
            onChange={(event) => setRawDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                browse.onRawFilterApply(rawDraft);
              }
            }}
          />
          <Button
            size="sm"
            variant="secondary"
            onClick={() => browse.onRawFilterApply(rawDraft)}
          >
            Apply
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-1.5 rounded-sm border border-border-subtle bg-surface-panel p-1">
          <Select
            value={draftColumn}
            onValueChange={(val) => setDraftColumn(val ?? "")}
          >
            <SelectTrigger
              className="h-6 w-auto min-w-24 border-none bg-transparent px-1.5 text-xs shadow-none"
              aria-label="Filter column"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {columnNames.map((col) => (
                <SelectItem key={col} value={col}>
                  {col}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select
            value={draftOperator}
            onValueChange={(val) =>
              setDraftOperator((val as DraftOperator) ?? "eq")
            }
          >
            <SelectTrigger
              className="h-6 w-auto min-w-28 border-none bg-transparent px-1.5 text-xs shadow-none"
              aria-label="Filter operator"
            >
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {ALL_OPS.map((op) => (
                <SelectItem key={op.id} value={op.id}>
                  {op.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          {needsValue ? (
            <Input
              className="h-6 w-36 border-none bg-surface-app px-2 text-xs shadow-none sm:w-56"
              placeholder={draftOperator === "inList" ? "a, b, c" : "value"}
              value={draftValue}
              onChange={(event) => setDraftValue(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  applyDraft();
                }
              }}
            />
          ) : null}
          <Button
            size="sm"
            variant="secondary"
            onClick={applyDraft}
            disabled={!canApply}
          >
            Apply
          </Button>
        </div>
      )}

      <p className="text-2xs text-muted-foreground">
        Typed filters and a WHERE fragment AND-combine.
      </p>

      {databaseError ? (
        <p role="alert" className="w-full font-mono text-2xs text-danger">
          {formatTableBrowseError(databaseError)}
          {databaseError.position !== null
            ? ` at position ${databaseError.position}`
            : ""}
        </p>
      ) : null}

      <div className="ml-auto flex items-center gap-1.5">
        <HistoryMenu browse={browse} />
        <Button
          variant="ghost"
          size="sm"
          onClick={browse.onClearTypedFilters}
          disabled={
            browse.typedFilters.length === 0 && browse.rawFilterText === ""
          }
        >
          Clear filters
        </Button>
      </div>
    </div>
  );
}

function HistoryMenu({ browse }: { browse: ServerBrowseGridModel }) {
  const [presetName, setPresetName] = useState("");
  return (
    <div className="flex items-center gap-1">
      <Select
        value=""
        onValueChange={(value) => {
          if (value?.startsWith("h:")) {
            browse.onApplyHistory(Number.parseInt(value.slice(2), 10));
          } else if (value?.startsWith("p:")) {
            browse.onApplyPreset(value.slice(2));
          }
        }}
      >
        <SelectTrigger
          className="h-6 w-28 text-xs"
          aria-label="Filter history and presets"
        >
          <SelectValue placeholder="History" />
        </SelectTrigger>
        <SelectContent>
          {browse.history.map((entry, index) => (
            <SelectItem
              key={`${entry.appliedAt}:${entry.filterMode}:${JSON.stringify(entry.typedFilters)}:${JSON.stringify(entry.sort)}`}
              value={`h:${index}`}
            >
              {entry.filterMode} · {entry.sort.length} sort
            </SelectItem>
          ))}
          {browse.presets.map((preset) => (
            <SelectItem key={preset.name} value={`p:${preset.name}`}>
              {preset.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      <Input
        className="h-6 w-28 text-xs"
        aria-label="Preset name"
        placeholder="Preset name"
        value={presetName}
        onChange={(event) => setPresetName(event.target.value)}
      />
      <Button
        size="sm"
        variant="ghost"
        disabled={presetName.trim().length === 0}
        onClick={() => {
          browse.onSavePreset(presetName.trim());
          setPresetName("");
        }}
      >
        Save
      </Button>
    </div>
  );
}

export function BrowseSortEditor({
  columnNames,
  browse,
}: {
  columnNames: string[];
  browse: ServerBrowseGridModel;
}) {
  return (
    <div className="flex min-h-10 flex-wrap items-center gap-1.5 border-b border-border-subtle bg-surface-window px-3 py-1.5">
      {browse.sort.map((key, index) => (
        <div
          key={key.column}
          className="flex h-6 items-center gap-1 rounded-sm border border-border-subtle bg-surface-panel px-2 text-xs"
        >
          <span className="tabular-nums text-muted-foreground">
            {index + 1}
          </span>
          <span>{key.column}</span>
          <button
            type="button"
            aria-label={`Toggle ${key.column} direction`}
            onClick={() =>
              browse.onSortChange(
                browse.sort.map((item) =>
                  item.column === key.column
                    ? {
                        ...item,
                        direction: item.direction === "asc" ? "desc" : "asc",
                      }
                    : item,
                ),
              )
            }
          >
            {key.direction}
          </button>
          <Select
            value={key.nulls}
            onValueChange={(value) =>
              browse.onSortChange(
                browse.sort.map((item) =>
                  item.column === key.column
                    ? { ...item, nulls: (value as BrowseNulls) ?? "default" }
                    : item,
                ),
              )
            }
          >
            <SelectTrigger className="h-5 w-20 border-none bg-transparent px-1 text-2xs">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="default">nulls default</SelectItem>
              <SelectItem value="first">nulls first</SelectItem>
              <SelectItem value="last">nulls last</SelectItem>
            </SelectContent>
          </Select>
          <button
            type="button"
            aria-label={`Remove sort on ${key.column}`}
            onClick={() =>
              browse.onSortChange(
                browse.sort.filter((item) => item.column !== key.column),
              )
            }
          >
            <IconX className="size-3" />
          </button>
        </div>
      ))}
      <Select
        value=""
        onValueChange={(column) => {
          if (!column) return;
          browse.onSortChange([
            ...browse.sort,
            {
              column,
              direction: "asc" as BrowseSortDirection,
              nulls: "default",
            },
          ]);
        }}
      >
        <SelectTrigger className="h-6 w-32 text-xs" aria-label="Add sort key">
          <SelectValue placeholder="Add sort" />
        </SelectTrigger>
        <SelectContent>
          {columnNames
            .filter(
              (column) => !browse.sort.some((key) => key.column === column),
            )
            .map((column) => (
              <SelectItem key={column} value={column}>
                {column}
              </SelectItem>
            ))}
        </SelectContent>
      </Select>
    </div>
  );
}

export function BrowseInspectionPanel({
  browse,
}: {
  browse: ServerBrowseGridModel;
}) {
  const inspection = browse.inspection;
  if (!inspection) return null;
  const copy = async (text: string) => {
    await navigator.clipboard.writeText(text);
  };
  return (
    <div
      data-testid="browse-inspection"
      className="max-w-xl rounded-sm border border-border-subtle bg-surface-panel p-3 text-xs"
    >
      <div className="mb-2 flex items-center justify-between">
        <span className="text-2xs uppercase tracking-wide text-muted-foreground">
          Executed SQL
        </span>
        <Button
          size="sm"
          variant="ghost"
          aria-label="Copy SQL"
          onClick={() => void copy(inspection.sql)}
        >
          <IconCopy />
        </Button>
      </div>
      <pre className="max-h-40 overflow-auto whitespace-pre-wrap font-mono text-2xs">
        {inspection.sql}
      </pre>
      <div className="mt-2 flex items-center justify-between">
        <span className="text-2xs uppercase tracking-wide text-muted-foreground">
          Parameters
        </span>
        <Button
          size="sm"
          variant="ghost"
          aria-label="Copy parameters"
          onClick={() =>
            void copy(
              inspection.params
                .map((param, index) =>
                  param.kind === "text"
                    ? `$${index + 1}=${param.value}`
                    : `$${index + 1}={${param.values.join(",")}}`,
                )
                .join("\n"),
            )
          }
        >
          <IconCopy />
        </Button>
      </div>
      <ol className="mt-1 space-y-0.5 font-mono text-2xs text-muted-foreground">
        {inspection.params.map((param, index) => (
          <li key={`$${index + 1}:${param.kind}`}>
            ${index + 1}{" "}
            {param.kind === "text" ? param.value : param.values.join(", ")}
          </li>
        ))}
      </ol>
      {browse.omittedRows > 0 || browse.truncatedCells > 0 ? (
        <output className="mt-2 block text-warning">
          Partial result: {browse.omittedRows.toLocaleString()} omitted rows,{" "}
          {browse.truncatedCells.toLocaleString()} truncated cells.
        </output>
      ) : null}
    </div>
  );
}

export function browseStatusText(browse: ServerBrowseGridModel): string {
  if (browse.loadStatus.state === "loading") return "Loading table rows";
  if (browse.loadStatus.state === "error") {
    return formatTableBrowseError(browse.loadStatus.error);
  }
  return "";
}

export function BrowseLiveRegion({
  browse,
}: {
  browse: ServerBrowseGridModel;
}) {
  return (
    <div className={cn("sr-only")} aria-live="polite">
      {browseStatusText(browse)}
    </div>
  );
}

export function BrowsePartialResultNotice({
  browse,
}: {
  browse: ServerBrowseGridModel;
}) {
  if (browse.omittedRows === 0 && browse.truncatedCells === 0) return null;
  return (
    <output className="block border-b border-border-subtle bg-surface-window px-3 py-1 text-2xs text-warning">
      Partial result: {browse.omittedRows.toLocaleString()} omitted rows,{" "}
      {browse.truncatedCells.toLocaleString()} truncated cells.
    </output>
  );
}
