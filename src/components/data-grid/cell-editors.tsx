/**
 * Specialized cell editors per ADR-0014. The registry maps a
 * Postgres column data type to an overlay editor component. When a
 * column has no matching editor, the grid falls back to the default
 * single-line inline editor.
 *
 * Each editor:
 * - Receives the cell's current literal as `initialValue`.
 * - Calls `onSave(literal)` with a Postgres-compatible literal on
 *   commit. The pending-mutations pipeline is unchanged from the
 *   inline-editor path.
 * - Calls `onCancel()` on Escape / outside click without writing.
 */
import {
  IconCheck,
  IconCode,
  IconPlus,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

export type SpecializedCellKind = "json" | "array" | "geometry";

export type CellEditorProps = {
  initialValue: string;
  columnName: string;
  onSave: (literal: string) => void;
  onCancel: () => void;
};

/**
 * Classify a Postgres column type into the registry key, or return
 * `null` to mean "use the default inline editor". Lower-cased and
 * trimmed so callers don't need to normalize.
 */
export function specializedCellKind(
  dataType: string | undefined,
): SpecializedCellKind | null {
  if (!dataType) return null;
  const t = dataType.toLowerCase().trim();
  if (t === "json" || t === "jsonb") return "json";
  if (t.endsWith("[]")) return "array";
  if (
    t === "geometry" ||
    t === "geography" ||
    t.startsWith("geometry(") ||
    t.startsWith("geography(")
  ) {
    return "geometry";
  }
  return null;
}

export const CELL_EDITORS: Record<
  SpecializedCellKind,
  React.ComponentType<CellEditorProps>
  // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
> = {
  json: JsonCellEditor,
  array: ArrayCellEditor,
  geometry: GeometryCellEditor,
};

// ---------------------------------------------------------------------------
// JSON / JSONB
// ---------------------------------------------------------------------------

function JsonCellEditor({
  initialValue,
  columnName,
  onSave,
  onCancel,
}: CellEditorProps) {
  const [text, setText] = useState(initialValue);
  const [error, setError] = useState<string | null>(null);
  const helperId = useId();

  const validate = (value: string): string | null => {
    if (value === "" || value.toUpperCase() === "NULL") return null;
    try {
      JSON.parse(value);
      return null;
    } catch (e) {
      return e instanceof Error ? e.message : "Invalid JSON";
    }
  };

  const handleSave = () => {
    const message = validate(text);
    if (message !== null) {
      setError(message);
      return;
    }
    onSave(text);
  };

  const handlePretty = () => {
    try {
      const parsed = JSON.parse(text);
      setText(JSON.stringify(parsed, null, 2));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Invalid JSON");
    }
  };

  return (
    <EditorShell
      title={`Edit JSON · ${columnName}`}
      onCancel={onCancel}
      onSave={handleSave}
      footerExtra={
        <Button size="sm" variant="ghost" onClick={handlePretty}>
          <IconCode className="size-3.5" /> Pretty
        </Button>
      }
    >
      <Textarea
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          if (error) setError(null);
        }}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            handleSave();
          }
        }}
        className="min-h-48 flex-1 font-mono text-xs"
        placeholder='{"key": "value"}'
        aria-describedby={helperId}
      />
      <div
        id={helperId}
        className={cn(
          "min-h-4 text-2xs",
          error ? "text-danger" : "text-text-muted",
        )}
      >
        {error ??
          "JSON.parse must succeed before saving. Cmd/Ctrl+Enter to save."}
      </div>
    </EditorShell>
  );
}

// ---------------------------------------------------------------------------
// Array (text[], int[], etc.)
// ---------------------------------------------------------------------------

function ArrayCellEditor({
  initialValue,
  columnName,
  onSave,
  onCancel,
}: CellEditorProps) {
  const [items, setItems] = useState(() => parsePgArrayLiteral(initialValue));

  const handleSave = () => {
    onSave(formatPgArrayLiteral(items));
  };

  return (
    <EditorShell
      title={`Edit array · ${columnName}`}
      onCancel={onCancel}
      onSave={handleSave}
      footerExtra={
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setItems((current) => [...current, ""])}
        >
          <IconPlus className="size-3.5" /> Add element
        </Button>
      }
    >
      {items.length === 0 ? (
        <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-xs text-text-muted">
          Empty array — click "Add element" to start.
        </div>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {items.map((item, index) => (
            // oxlint-disable-next-line react/no-array-index-key -- order is the identity
            <li key={index} className="flex items-center gap-1.5">
              <span className="w-6 text-right font-mono text-2xs text-text-muted">
                {index}
              </span>
              <Input
                value={item}
                onChange={(event) =>
                  setItems((current) => {
                    const next = current.slice();
                    next[index] = event.target.value;
                    return next;
                  })
                }
                className="h-7 flex-1 font-mono text-xs"
              />
              <Button
                size="icon-sm"
                variant="ghost"
                aria-label={`Remove element ${index}`}
                onClick={() =>
                  setItems((current) => current.filter((_, i) => i !== index))
                }
              >
                <IconTrash className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="font-mono text-2xs text-text-muted">
        Postgres literal: {formatPgArrayLiteral(items) || "{}"}
      </div>
    </EditorShell>
  );
}

// ---------------------------------------------------------------------------
// Geometry / Geography (WKT)
// ---------------------------------------------------------------------------

function GeometryCellEditor({
  initialValue,
  columnName,
  onSave,
  onCancel,
}: CellEditorProps) {
  const [text, setText] = useState(initialValue);
  const [error, setError] = useState<string | null>(null);

  const handleSave = () => {
    const trimmed = text.trim();
    if (trimmed === "" || trimmed.toUpperCase() === "NULL") {
      onSave("");
      return;
    }
    if (!isLikelyWkt(trimmed)) {
      setError(
        "Expected WKT (e.g., POINT(10 20), LINESTRING(...), POLYGON((...))).",
      );
      return;
    }
    onSave(trimmed);
  };

  return (
    <EditorShell
      title={`Edit geometry · ${columnName}`}
      onCancel={onCancel}
      onSave={handleSave}
    >
      <Textarea
        value={text}
        onChange={(event) => {
          setText(event.target.value);
          if (error) setError(null);
        }}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault();
            handleSave();
          }
        }}
        className="min-h-32 flex-1 font-mono text-xs"
        placeholder="POINT(10 20)"
      />
      <div
        className={cn(
          "min-h-4 text-2xs",
          error ? "text-danger" : "text-text-muted",
        )}
      >
        {error ?? "Well-Known Text per OGC SFS. Cmd/Ctrl+Enter to save."}
      </div>
    </EditorShell>
  );
}

// ---------------------------------------------------------------------------
// Shell (modal frame)
// ---------------------------------------------------------------------------

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- This custom modal overlay uses application-managed open and backdrop behavior. */
function EditorShell({
  title,
  onCancel,
  onSave,
  footerExtra,
  children,
}: {
  title: string;
  onCancel: () => void;
  onSave: () => void;
  footerExtra?: React.ReactNode;
  children: React.ReactNode;
}) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    /* oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions, jsx-a11y/prefer-tag-over-role -- This custom modal overlay handles backdrop clicks without changing native dialog lifecycle behavior. */
    <div
      role="dialog"
      aria-modal="true"
      aria-label={title}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div className="flex w-full max-w-2xl flex-col gap-3 rounded-lg border border-border-subtle bg-surface-window p-4 shadow-xl">
        <div className="flex items-center justify-between gap-2">
          <h2 className="text-sm font-semibold text-foreground">{title}</h2>
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label="Close"
            onClick={onCancel}
          >
            <IconX className="size-3.5" />
          </Button>
        </div>
        <div className="flex max-h-[60vh] flex-col gap-2 overflow-auto">
          {children}
        </div>
        <div className="flex items-center justify-end gap-1.5">
          {footerExtra}
          <Button size="sm" variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button size="sm" onClick={onSave}>
            <IconCheck className="size-3.5" /> Save
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// PG array literal helpers
// ---------------------------------------------------------------------------

/**
 * Parse a Postgres array literal like `{a,b,"c d"}` into a JS array.
 * Tolerant of:
 * - Empty `{}` → `[]`.
 * - Unquoted scalars (numbers, booleans, identifiers).
 * - Quoted strings with backslash escapes.
 * - Leading/trailing whitespace.
 *
 * Not a full Postgres array parser — multi-dimensional arrays and
 * nested record types fall through to the raw value rendered as a
 * single element. Good enough for the common case of `text[]` /
 * `int[]` editing.
 */
/* oxlint-enable jsx-a11y/prefer-tag-over-role */
export function parsePgArrayLiteral(value: string): string[] {
  const trimmed = value.trim();
  if (!trimmed) return [];
  if (!trimmed.startsWith("{") || !trimmed.endsWith("}")) {
    return [trimmed];
  }
  const body = trimmed.slice(1, -1);
  if (!body) return [];
  const result: string[] = [];
  let current = "";
  let inQuotes = false;
  let escaped = false;
  for (const ch of body) {
    if (escaped) {
      current += ch;
      escaped = false;
      continue;
    }
    if (ch === "\\") {
      escaped = true;
      continue;
    }
    if (ch === '"') {
      inQuotes = !inQuotes;
      continue;
    }
    if (ch === "," && !inQuotes) {
      result.push(current);
      current = "";
      continue;
    }
    current += ch;
  }
  result.push(current);
  return result.map((item) => item.trim());
}

/**
 * Format a JS array as a Postgres array literal. Elements that
 * contain `,`, `"`, `\` or whitespace get double-quoted with
 * backslash-escaped inner `"` and `\`. Empty input renders as `{}`.
 */
export function formatPgArrayLiteral(items: string[]): string {
  if (items.length === 0) return "{}";
  const encoded = items.map((item) => {
    if (item === "") return '""';
    if (/[\s,"\\{}]/.test(item)) {
      const escaped = item.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
      return `"${escaped}"`;
    }
    return item;
  });
  return `{${encoded.join(",")}}`;
}

function isLikelyWkt(value: string): boolean {
  return /^\s*(SRID=\d+;)?\s*(POINT|LINESTRING|POLYGON|MULTIPOINT|MULTILINESTRING|MULTIPOLYGON|GEOMETRYCOLLECTION|CIRCULARSTRING|COMPOUNDCURVE|CURVEPOLYGON|MULTICURVE|MULTISURFACE|POLYHEDRALSURFACE|TIN|TRIANGLE)\s*\(/i.test(
    value,
  );
}
