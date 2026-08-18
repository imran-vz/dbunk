import {
  IconDatabase,
  IconDownload,
  IconGitBranch,
  IconRoute,
  IconSchema,
  IconTypography,
} from "@tabler/icons-react";
import type * as React from "react";

import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { SchemaMapGlossaryButton } from "@/components/workspace-overview/schema-map-glossary";
import { ALL_SCHEMAS_SENTINEL, isAllSchemas } from "@/lib/schema-graph";
import type {
  SchemaMapAttrMode,
  SchemaMapPrefs,
  SchemaMapRouting,
} from "@/lib/store";
import { cn } from "@/lib/utils";

export type SchemaMapExportFormat = "png" | "svg";

type SchemaMapToolbarProps = {
  schemas: string[];
  selectedSchema: string;
  /**
   * Label shown when the "all schemas" option is selected (typically the
   * connection's database name, falling back to "Database").
   */
  databaseLabel?: string;
  prefs: SchemaMapPrefs;
  isBusy?: boolean;
  exportError?: string | null;
  onSchemaChange: (schema: string) => void;
  onPrefsChange: (patch: Partial<SchemaMapPrefs>) => void;
  onResetLayout: () => void;
  onExport: (format: SchemaMapExportFormat) => void;
};

const modeOptions: Array<{ id: SchemaMapAttrMode; label: string }> = [
  { id: "all", label: "All" },
  { id: "keys-only", label: "Keys" },
  { id: "none", label: "None" },
];

export const safeSchemaMapSlug = (value: string): string => {
  const slug = value
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .replace(/-{2,}/g, "-");
  return slug || "schema";
};

export const schemaMapExportFilename = (
  connectionName: string,
  schema: string,
  format: SchemaMapExportFormat,
): string =>
  `${safeSchemaMapSlug(connectionName)}-${safeSchemaMapSlug(schema)}-schema.${format}`;

export function SchemaMapToolbar({
  schemas,
  selectedSchema,
  databaseLabel,
  prefs,
  isBusy = false,
  exportError,
  onSchemaChange,
  onPrefsChange,
  onResetLayout,
  onExport,
}: SchemaMapToolbarProps) {
  const allMode = isAllSchemas(selectedSchema);
  const triggerIcon = allMode ? (
    <IconDatabase className="size-3 text-text-muted" />
  ) : (
    <IconSchema className="size-3 text-text-muted" />
  );
  const allLabel = databaseLabel?.trim()
    ? `${databaseLabel} · all schemas`
    : "Database (all schemas)";

  return (
    <div className="flex shrink-0 flex-col gap-2 border-b border-border-subtle bg-surface-window px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <Select
          value={selectedSchema}
          onValueChange={(value) => {
            if (value) {
              onSchemaChange(value);
            }
          }}
        >
          <SelectTrigger
            size="sm"
            aria-label="Schema"
            disabled={schemas.length === 0 && !allMode}
            className="min-w-36"
          >
            {triggerIcon}
            <SelectValue placeholder="Schema">
              {allMode ? allLabel : selectedSchema}
            </SelectValue>
          </SelectTrigger>
          <SelectContent align="start">
            <SelectItem value={ALL_SCHEMAS_SENTINEL}>
              <span className="inline-flex items-center gap-1.5">
                <IconDatabase className="size-3 text-text-muted" />
                {allLabel}
              </span>
            </SelectItem>
            {schemas.length > 0 ? <SelectSeparator /> : null}
            {schemas.map((schema) => (
              <SelectItem key={schema} value={schema}>
                {schema}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <fieldset
          aria-label="Attribute mode"
          className="m-0 inline-flex h-6 overflow-hidden rounded-sm border border-border-subtle bg-surface-panel p-0"
        >
          {modeOptions.map((option) => (
            <button
              key={option.id}
              type="button"
              aria-pressed={prefs.attrMode === option.id}
              onClick={() => onPrefsChange({ attrMode: option.id })}
              className={cn(
                "px-2 text-[0.6875rem] font-medium transition-colors",
                prefs.attrMode === option.id
                  ? "bg-accent/15 text-accent-hover"
                  : "text-text-muted hover:bg-surface-panel-elevated hover:text-foreground",
              )}
            >
              {option.label}
            </button>
          ))}
        </fieldset>

        <ToggleChip
          active={prefs.showTypes}
          icon={<IconTypography className="size-3" />}
          label="Types"
          onClick={() => onPrefsChange({ showTypes: !prefs.showTypes })}
        />
        <ToggleChip
          active={prefs.showNulls}
          label="NULL"
          onClick={() => onPrefsChange({ showNulls: !prefs.showNulls })}
        />
        <ToggleChip
          active={prefs.showComments}
          label="Comments"
          onClick={() => onPrefsChange({ showComments: !prefs.showComments })}
        />

        <fieldset
          aria-label="Routing"
          className="m-0 inline-flex h-6 overflow-hidden rounded-sm border border-border-subtle bg-surface-panel p-0"
        >
          {/* oxlint-disable-next-line anti-slop/require-safety-comment-for-type-assertion -- This literal tuple is the complete SchemaMapRouting domain. */}
          {(["bezier", "step"] as SchemaMapRouting[]).map((routing) => (
            <button
              key={routing}
              type="button"
              aria-pressed={prefs.routing === routing}
              onClick={() => onPrefsChange({ routing })}
              className={cn(
                "inline-flex items-center gap-1 px-2 text-[0.6875rem] font-medium transition-colors",
                prefs.routing === routing
                  ? "bg-accent/15 text-accent-hover"
                  : "text-text-muted hover:bg-surface-panel-elevated hover:text-foreground",
              )}
            >
              <IconRoute className="size-3" />
              {routing === "bezier" ? "Curve" : "Step"}
            </button>
          ))}
        </fieldset>

        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={isBusy || !selectedSchema}
          onClick={onResetLayout}
        >
          <IconGitBranch className="size-3" />
          Reset
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={isBusy || !selectedSchema}
          onClick={() => onExport("png")}
        >
          <IconDownload className="size-3" />
          PNG
        </Button>
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={isBusy || !selectedSchema}
          onClick={() => onExport("svg")}
        >
          <IconDownload className="size-3" />
          SVG
        </Button>
        <SchemaMapGlossaryButton />
      </div>
      {exportError ? (
        <div
          role="alert"
          className="rounded-sm border border-danger/40 bg-danger/10 px-2 py-1 text-[0.6875rem] text-danger"
        >
          {exportError}
        </div>
      ) : null}
    </div>
  );
}

function ToggleChip({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon?: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "inline-flex h-6 items-center gap-1 rounded-sm border px-2 text-[0.6875rem] font-medium transition-colors",
        active
          ? "border-accent/40 bg-accent/10 text-accent-hover"
          : "border-border-subtle bg-surface-panel text-text-muted hover:bg-surface-panel-elevated hover:text-foreground",
      )}
    >
      {icon}
      {label}
    </button>
  );
}
