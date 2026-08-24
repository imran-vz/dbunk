/**
 * Organization fields (Plan 010, mock A): folder, favorite, and the
 * user-picked identity color. Engine-independent — rendered for every
 * engine in the main form body. The color vocabulary and CSS-variable
 * mapping live in `src/lib/connection-colors.ts`; swatches consume the
 * variables via inline style so no raw hex lands in TSX.
 */

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  CONNECTION_COLORS,
  type ConnectionColor,
  connectionColorVar,
} from "@/lib/connection-colors";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

import { ToggleSwitchRow } from "./field-helpers";
import type { ConnectionFormApi } from "./use-connection-form";

export function OrganizationFields({ form }: { form: ConnectionFormApi }) {
  // Existing folder names feed the datalist so groups stay consistent
  // without a dedicated folder manager.
  const connections = useAppStore((state) => state.connections);
  const knownFolders = [
    ...new Set(
      connections
        .map((connection) => connection.folder?.trim() ?? "")
        .filter(Boolean),
    ),
  ].sort((a, b) => a.localeCompare(b, undefined, { sensitivity: "base" }));

  return (
    <div className="grid gap-3">
      <div className="grid gap-3 sm:grid-cols-2">
        <form.Field name="folder">
          {(field) => (
            <div className="grid gap-1.5">
              <Label htmlFor="connection-folder">Folder</Label>
              <Input
                id="connection-folder"
                list="connection-folder-options"
                placeholder="Ungrouped"
                value={field.state.value ?? ""}
                onChange={(event) => field.handleChange(event.target.value)}
                onBlur={field.handleBlur}
              />
              <datalist id="connection-folder-options">
                {knownFolders.map((folder) => (
                  <option key={folder} value={folder}>
                    {folder}
                  </option>
                ))}
              </datalist>
            </div>
          )}
        </form.Field>
        <form.Field name="color">
          {(field) => (
            <div className="grid gap-1.5">
              <Label id="connection-color-label">Color</Label>
              <div
                role="radiogroup"
                aria-labelledby="connection-color-label"
                className="flex h-9 flex-wrap items-center gap-2"
              >
                <ColorSwatch
                  color={undefined}
                  selected={!field.state.value}
                  onSelect={() => field.handleChange(undefined)}
                />
                {CONNECTION_COLORS.map((color) => (
                  <ColorSwatch
                    key={color}
                    color={color}
                    selected={field.state.value === color}
                    onSelect={() => field.handleChange(color)}
                  />
                ))}
              </div>
            </div>
          )}
        </form.Field>
      </div>
      <form.Field name="isFavorite">
        {(field) => (
          <ToggleSwitchRow
            id="connection-favorite"
            title="Favorite"
            description="Pinned to the top of its folder in the connections list."
            checked={field.state.value ?? false}
            onCheckedChange={(checked) => field.handleChange(checked)}
          />
        )}
      </form.Field>
    </div>
  );
}

function ColorSwatch({
  color,
  selected,
  onSelect,
}: {
  color: ConnectionColor | undefined;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <input
      type="radio"
      name="connection-color"
      checked={selected}
      onChange={onSelect}
      aria-label={color ?? "No color"}
      className={cn(
        "size-5 cursor-pointer appearance-none rounded-full border transition-shadow",
        color ? "border-transparent" : "border-dashed border-border-strong",
        selected &&
          "ring-2 ring-ring ring-offset-1 ring-offset-surface-window",
      )}
      style={
        color ? { backgroundColor: connectionColorVar(color) } : undefined
      }
    />
  );
}
