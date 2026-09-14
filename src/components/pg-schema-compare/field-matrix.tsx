import { Button } from "@/components/ui/button";
import { LoadingBar } from "@/components/ui/state-panel";
import {
  DIFFERENCE_LABEL,
  describeFieldPath,
  EXCLUSION_REASON,
  formatByteRange,
  formatFieldPath,
  formatPagePosition,
  INCOMPARABLE_REASON,
  SIDE_LABEL,
} from "@/lib/pg-schema-compare/presentation";
import {
  type SchemaCompareFieldSummary,
  type SchemaCompareObjectSummary,
  type SchemaCompareValueRef,
} from "@/lib/pg-schema-compare/protocol";
import {
  fieldSides,
  isExcludedObject,
  objectIdentity,
  objectSides,
  type SchemaCompareEligibilityDetail,
  type SchemaCompareFieldPage,
  type SchemaCompareSide,
  type SchemaCompareValueChunk,
} from "@/lib/pg-schema-compare/reader";
import { cn } from "@/lib/utils";

import { DifferenceBadge } from "./coverage";
import { PagerControls } from "./object-list";

const VALUE_KIND_LABEL = {
  null: "null",
  text: "text",
  boolean: "boolean",
  integer: "integer",
  qualifiedName: "qualified name",
  orderedNames: "ordered names",
  operatorSignatures: "operator signatures",
} satisfies Record<SchemaCompareValueRef["valueKind"], string>;

const SIDES = ["source", "target"] as const;
type Eligibility = Record<
  SchemaCompareSide,
  SchemaCompareEligibilityDetail | null
>;

const samePath = (
  a: SchemaCompareFieldSummary | null,
  b: SchemaCompareFieldSummary,
) => a !== null && formatFieldPath(a.path) === formatFieldPath(b.path);

/**
 * What one side of a field shows. A missing side is Absent unless that side's
 * definition is excluded from scope, which is not directional absence. An
 * observed NULL, an empty string and a value not yet read each say what they are.
 */
type ValueState =
  | { kind: "absent" }
  | { kind: "excluded"; reason: string | null }
  | { kind: "null" }
  | { kind: "empty" }
  | { kind: "pending" }
  | { kind: "text"; text: string };

function valueState(
  value: SchemaCompareValueRef | null,
  chunk: SchemaCompareValueChunk | null,
  excluded: { reason: string | null } | null,
): ValueState {
  if (!value)
    return excluded ? { kind: "excluded", ...excluded } : { kind: "absent" };
  if (value.valueKind === "null") return { kind: "null" };
  if (!chunk) return { kind: "pending" };
  if (chunk.text === "" && chunk.complete) return { kind: "empty" };
  return { kind: "text", text: chunk.text };
}

/**
 * The side's excluded definition. A loaded eligibility read is authoritative;
 * until then only a non-table relation kind is known to be excluded.
 */
function excludedSide(
  object: SchemaCompareObjectSummary,
  eligibility: Eligibility,
  side: SchemaCompareSide,
): { reason: string | null } | null {
  const detail = eligibility[side];
  if (detail?.kind === "excluded") {
    return { reason: EXCLUSION_REASON[detail.reason] };
  }
  if (detail?.kind === "eligible") return null;
  const identity = objectSides(object)[side];
  return isExcludedObject(object) &&
    identity !== null &&
    identity.kind !== "table"
    ? { reason: null }
    : null;
}

function ValueText({
  state,
  nullLabel,
  pendingLabel,
  block = false,
}: {
  state: ValueState;
  nullLabel: string;
  pendingLabel: string;
  block?: boolean;
}) {
  switch (state.kind) {
    case "absent":
      return <span className="text-text-muted italic">Absent</span>;
    case "excluded":
      return (
        <span className="text-warning italic">
          Excluded{state.reason ? ` (${state.reason})` : ""}
        </span>
      );
    case "null":
      return <span className="text-text-muted">{nullLabel}</span>;
    case "empty":
      return <span className="text-text-muted">Empty string</span>;
    case "pending":
      return <span className="text-text-muted">{pendingLabel}</span>;
    case "text":
      return block ? (
        <pre className="whitespace-pre-wrap break-all">{state.text}</pre>
      ) : (
        state.text
      );
  }
}

function FieldPathLabel({ field }: { field: SchemaCompareFieldSummary }) {
  const parts = describeFieldPath(field.path);
  return (
    <span className="inline-flex min-w-0 flex-wrap items-baseline gap-x-1.5">
      <span className="text-text-muted">{parts.group}</span>
      {parts.name !== null ? (
        <span className="font-mono">{parts.name}</span>
      ) : null}
      {parts.owner !== null ? (
        <span className="text-text-muted">
          (constraint <span className="font-mono">{parts.owner}</span>)
        </span>
      ) : null}
      {parts.position !== null ? (
        <span className="font-mono text-text-muted">#{parts.position}</span>
      ) : null}
      <span>{parts.field}</span>
    </span>
  );
}

/** One side of the value inspector, with explicit chunk navigation. */
function ValuePane({
  side,
  value,
  chunk,
  excluded,
  loading,
  onChunk,
}: {
  side: SchemaCompareSide;
  value: SchemaCompareValueRef | null;
  chunk: SchemaCompareValueChunk | null;
  excluded: { reason: string | null } | null;
  loading: boolean;
  onChunk: (side: SchemaCompareSide, direction: "next" | "previous") => void;
}) {
  const label = SIDE_LABEL[side];
  return (
    <section
      aria-label={`${label} value`}
      className="relative flex min-h-0 min-w-0 flex-col border border-border-subtle"
    >
      <div className="flex h-(--control-h) shrink-0 items-center gap-2 border-b border-border-subtle px-2 text-2xs">
        <span className="font-semibold tracking-wide text-text-muted uppercase">
          {label}
        </span>
        {value ? (
          <span className="text-text-muted">
            {VALUE_KIND_LABEL[value.valueKind]} · {value.rawBytes} B
          </span>
        ) : null}
        {value && chunk && !chunk.complete ? (
          <span className="ml-auto text-warning">
            Partial · {formatByteRange(chunk)} of {value.rawBytes}
          </span>
        ) : null}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-2 font-mono text-xs">
        <ValueText
          state={valueState(value, chunk, excluded)}
          nullLabel="NULL (observed)"
          pendingLabel={loading ? "Loading…" : "Not loaded"}
          block
        />
      </div>
      {value && chunk && (!chunk.complete || chunk.offset > 0) ? (
        <div className="flex h-(--control-h) shrink-0 items-center gap-1 border-t border-border-subtle px-1">
          <Button
            variant="ghost"
            size="sm"
            disabled={loading || chunk.offset === 0}
            onClick={() => onChunk(side, "previous")}
          >
            Previous chunk
          </Button>
          <Button
            variant="ghost"
            size="sm"
            disabled={loading || chunk.complete}
            onClick={() => onChunk(side, "next")}
          >
            Next chunk
          </Button>
          {!chunk.complete ? (
            <span className="ml-auto text-2xs text-text-muted">
              Raw text is shown as captured; partial values are never parsed.
            </span>
          ) : null}
        </div>
      ) : null}
    </section>
  );
}

/** Per-side eligibility for an object excluded on at least one side. */
function ExclusionNote({
  object,
  eligibility,
}: {
  object: SchemaCompareObjectSummary;
  eligibility: Eligibility;
}) {
  const sides = objectSides(object);
  return (
    <div className="space-y-1 border-b border-border-subtle p-(--pad-panel) text-xs">
      {SIDES.map((side) => {
        const identity = sides[side];
        const detail = eligibility[side];
        return (
          <p key={side} className="text-text-secondary">
            <span className="font-semibold text-foreground">
              {SIDE_LABEL[side]}:
            </span>{" "}
            {!identity ? (
              "not observed."
            ) : detail === null ? (
              "eligibility not loaded."
            ) : detail.kind === "eligible" ? (
              "eligible ordinary table."
            ) : (
              <>
                excluded, {EXCLUSION_REASON[detail.reason]} (
                <span className="font-mono">{identity.kind}</span>).
              </>
            )}
          </p>
        );
      })}
    </div>
  );
}

/** The selected table: its field page, then the lazy source/target inspector. */
export function FieldMatrix({
  schema,
  object,
  page,
  eligibility,
  selected,
  values,
  loading,
  onSelect,
  onPage,
  onChunk,
}: {
  schema: { source: string; target: string };
  object: SchemaCompareObjectSummary;
  page: SchemaCompareFieldPage | null;
  eligibility: Eligibility;
  selected: SchemaCompareFieldSummary | null;
  values: Record<SchemaCompareSide, SchemaCompareValueChunk | null>;
  loading: "fields" | "eligibility" | "values" | null;
  onSelect: (field: SchemaCompareFieldSummary) => void;
  onPage: (direction: "next" | "previous") => void;
  onChunk: (side: SchemaCompareSide, direction: "next" | "previous") => void;
}) {
  const identity = objectIdentity(object);
  const sides = objectSides(object);
  const selectedSides = selected ? fieldSides(selected) : null;
  const excluded = {
    source: excludedSide(object, eligibility, "source"),
    target: excludedSide(object, eligibility, "target"),
  };
  return (
    <section
      aria-label="Selected object fields"
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
    >
      {loading === "fields" || loading === "eligibility" ? (
        <LoadingBar />
      ) : null}
      <div className="flex min-h-(--h-toolbar) shrink-0 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border-subtle px-(--pad-panel) py-1 text-xs">
        <span className="min-w-0 truncate font-mono font-semibold">
          {sides.source ? schema.source : schema.target}.{identity.name}
        </span>
        <DifferenceBadge kind={object.kind} />
        {object.kind === "changed" || object.kind === "notComparable" ? (
          <span className="text-text-secondary">
            {object.changedFields} changed · {object.incomparableFields} not
            comparable
          </span>
        ) : null}
        {object.kind === "notComparable" ? (
          <span
            className="min-w-0 basis-full text-warning"
            data-testid="object-reason"
          >
            {INCOMPARABLE_REASON[object.reason]}
          </span>
        ) : null}
      </div>
      {isExcludedObject(object) ? (
        <ExclusionNote object={object} eligibility={eligibility} />
      ) : null}
      {object.fieldCount === 0 ? (
        <p className="min-h-0 flex-1 p-(--pad-panel) text-xs text-text-secondary">
          {isExcludedObject(object)
            ? "No fields are compared for an excluded definition."
            : "No comparable fields in scope for this object."}
        </p>
      ) : (
        <>
          <div className="min-h-0 flex-1 overflow-auto">
            {page ? (
              <table className="w-max min-w-full text-left text-xs">
                <thead className="sticky top-0 bg-surface-sidebar text-2xs text-text-muted">
                  <tr>
                    {["Field", "Source", "Target", "Result"].map((title) => (
                      <th
                        key={title}
                        className="h-(--row-grid) px-(--pad-panel) font-normal"
                      >
                        {title}
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {page.items.map((field) => {
                    const active = samePath(selected, field);
                    const fieldValues = fieldSides(field);
                    return (
                      <tr
                        key={formatFieldPath(field.path)}
                        className={cn(
                          "border-b border-border-subtle",
                          active
                            ? "bg-accent-subdued"
                            : "hover:bg-surface-row-hover",
                        )}
                      >
                        <td className="h-(--row-grid) px-(--pad-panel)">
                          <button
                            type="button"
                            aria-pressed={active}
                            onClick={() => onSelect(field)}
                            className={cn(
                              "text-left outline-none focus-visible:ring-1 focus-visible:ring-accent",
                              active && "text-accent",
                            )}
                          >
                            <FieldPathLabel field={field} />
                          </button>
                        </td>
                        {SIDES.map((side) => (
                          <td
                            key={side}
                            className="h-(--row-grid) max-w-64 truncate px-(--pad-panel) font-mono"
                          >
                            <ValueText
                              state={valueState(
                                fieldValues[side],
                                active ? values[side] : null,
                                excluded[side],
                              )}
                              nullLabel="NULL"
                              pendingLabel={
                                active ? "Loading…" : "Select to inspect"
                              }
                            />
                          </td>
                        ))}
                        <td className="h-(--row-grid) max-w-md truncate px-(--pad-panel)">
                          <DifferenceBadge kind={field.kind} />
                          {field.kind === "notComparable" ? (
                            <span className="ml-2 text-text-muted">
                              {INCOMPARABLE_REASON[field.reason]}
                            </span>
                          ) : null}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            ) : (
              <p className="p-(--pad-panel) text-xs text-text-muted">
                {loading === "fields"
                  ? "Reading fields…"
                  : "Fields not loaded."}
              </p>
            )}
          </div>
          <div className="flex h-(--h-toolbar) shrink-0 items-center justify-between gap-2 border-t border-border-subtle px-(--pad-panel)">
            <span className="text-2xs text-text-muted">
              Captured definitions · Values load on selection
            </span>
            {page ? (
              <PagerControls
                label="field"
                position={`Fields ${formatPagePosition(
                  page.offset,
                  page.items.length,
                  object.fieldCount,
                )}`}
                hasPrevious={page.offset > 0}
                hasNext={page.nextOffset !== null}
                onPrevious={() => onPage("previous")}
                onNext={() => onPage("next")}
              />
            ) : null}
          </div>
          {selected ? (
            <section
              aria-label="Value inspector"
              className="relative flex max-h-[45%] min-h-32 shrink-0 flex-col border-t border-border-subtle"
            >
              {loading === "values" ? <LoadingBar /> : null}
              <div className="flex min-h-(--control-h) shrink-0 flex-wrap items-center gap-x-3 gap-y-0.5 px-(--pad-panel) py-0.5 text-xs">
                <span className="font-semibold">Value inspector</span>
                <FieldPathLabel field={selected} />
                <span className="text-text-muted">
                  {DIFFERENCE_LABEL[selected.kind]}
                </span>
                {selected.kind === "notComparable" ? (
                  <span className="min-w-0 basis-full text-warning">
                    {INCOMPARABLE_REASON[selected.reason]}
                  </span>
                ) : null}
              </div>
              <div className="grid min-h-0 flex-1 gap-2 px-(--pad-panel) pb-(--pad-panel) @xl:grid-cols-2">
                {SIDES.map((side) => (
                  <ValuePane
                    key={side}
                    side={side}
                    value={selectedSides?.[side] ?? null}
                    chunk={values[side]}
                    excluded={excluded[side]}
                    loading={loading === "values"}
                    onChunk={onChunk}
                  />
                ))}
              </div>
            </section>
          ) : null}
        </>
      )}
    </section>
  );
}
