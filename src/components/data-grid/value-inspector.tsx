/**
 * Value inspector (DESIGN-SYSTEM §5.4) — opened with `Space` or
 * `Shift+Enter` on a focused cell. Shows the full untruncated value as
 * text, pretty-printed JSON (when it parses), and a hex dump.
 */

import { useMemo, useState } from "react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogBody,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Segmented } from "@/components/ui/segmented";
import { GRID_NULL_SENTINEL } from "@/lib/table-browse";

export type InspectedValue = {
  column: string;
  value: string;
};

type InspectorView = "text" | "json" | "hex";

const tryPrettyJson = (value: string): string | null => {
  const trimmed = value.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return null;
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return null;
  }
};

const HEX_BYTES_PER_LINE = 16;
const HEX_MAX_BYTES = 4096;

const toHexDump = (value: string): string => {
  const bytes = new TextEncoder().encode(value).slice(0, HEX_MAX_BYTES);
  const lines: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += HEX_BYTES_PER_LINE) {
    const chunk = bytes.slice(offset, offset + HEX_BYTES_PER_LINE);
    const hex = [...chunk]
      .map((byte) => byte.toString(16).padStart(2, "0"))
      .join(" ");
    const ascii = [...chunk]
      .map((byte) =>
        byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : "·",
      )
      .join("");
    lines.push(
      `${offset.toString(16).padStart(8, "0")}  ${hex.padEnd(HEX_BYTES_PER_LINE * 3 - 1)}  ${ascii}`,
    );
  }
  return lines.join("\n") || "(empty)";
};

export function ValueInspector({
  inspected,
  onClose,
}: {
  inspected: InspectedValue | null;
  onClose: () => void;
}) {
  const [view, setView] = useState<InspectorView>("text");
  const value = inspected?.value ?? "";
  const isNull = value === GRID_NULL_SENTINEL;
  const prettyJson = useMemo(
    () => (inspected && !isNull ? tryPrettyJson(value) : null),
    [inspected, isNull, value],
  );

  const body = isNull
    ? "NULL"
    : view === "json"
      ? (prettyJson ?? "Not valid JSON.")
      : view === "hex"
        ? toHexDump(value)
        : value || "(empty string)";

  return (
    <Dialog
      open={inspected !== null}
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <DialogContent size="lg">
        <DialogHeader>
          <DialogTitle className="font-mono text-sm">
            {inspected?.column}
          </DialogTitle>
        </DialogHeader>
        <DialogBody className="flex min-h-0 flex-col gap-2">
          <Segmented<InspectorView>
            value={view}
            onChange={setView}
            options={[
              { id: "text", label: "Text" },
              { id: "json", label: "JSON" },
              { id: "hex", label: "Hex" },
            ]}
          />
          <pre className="max-h-96 min-h-24 overflow-auto rounded-sm border border-border-subtle bg-surface-window p-3 font-mono text-xs whitespace-pre-wrap break-all text-foreground">
            {body}
          </pre>
        </DialogBody>
        <DialogFooter>
          <Button
            size="sm"
            variant="outline"
            disabled={isNull}
            onClick={async () => {
              try {
                await navigator.clipboard.writeText(value);
                toast.success("Value copied.");
              } catch {
                toast.error("Copy failed.");
              }
            }}
          >
            Copy value
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
