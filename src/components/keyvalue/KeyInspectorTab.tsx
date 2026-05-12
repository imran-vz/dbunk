/**
 * Three-region key inspector (Q10):
 *  - sticky header: name, type badge, TTL, encoding, action row
 *  - center: type-specific viewer
 *  - right drawer: object info + metadata (collapsible)
 *
 * Phase 1.2 renders read-only viewers for all seven supported types.
 * Editors land in Phase 1.4 for string/hash (the rest stay read-only;
 * see deferrals).
 */

import {
  IconChevronRight,
  IconClock,
  IconCopy,
  IconEdit,
  IconRefresh,
  IconTrash,
} from "@tabler/icons-react";
import { useEffect, useState } from "react";

import { HashValueView } from "@/components/keyvalue/viewers/HashValueView";
import { JsonValueView } from "@/components/keyvalue/viewers/JsonValueView";
import { ListValueView } from "@/components/keyvalue/viewers/ListValueView";
import { SetValueView } from "@/components/keyvalue/viewers/SetValueView";
import { SortedSetValueView } from "@/components/keyvalue/viewers/SortedSetValueView";
import { StreamValueView } from "@/components/keyvalue/viewers/StreamValueView";
import { StringValueView } from "@/components/keyvalue/viewers/StringValueView";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  delRedisKeys,
  fetchKeyMetadata,
  type KeyMetadata,
  renameRedisKey,
  setRedisExpire,
} from "@/lib/redis/api";
import { cn } from "@/lib/utils";

interface KeyInspectorTabProps {
  connectionId: string;
  keyName: string;
  onKeyDeleted?: (key: string) => void;
  onKeyRenamed?: (oldKey: string, newKey: string) => void;
}

export function KeyInspectorTab({
  connectionId,
  keyName,
  onKeyDeleted,
  onKeyRenamed,
}: KeyInspectorTabProps) {
  const [metadata, setMetadata] = useState<KeyMetadata | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshTick, setRefreshTick] = useState(0);
  const [drawerOpen, setDrawerOpen] = useState(true);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleteConfirm, setDeleteConfirm] = useState("");
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameValue, setRenameValue] = useState("");
  const [expireOpen, setExpireOpen] = useState(false);
  const [expireValue, setExpireValue] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: refreshTick is an intentional re-trigger
  useEffect(() => {
    let cancelled = false;
    setError(null);
    fetchKeyMetadata({ connectionId, key: keyName })
      .then((result) => {
        if (!cancelled) setMetadata(result);
      })
      .catch((err) => {
        if (!cancelled)
          setError(err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [connectionId, keyName, refreshTick]);

  const handleCopyName = () => {
    void navigator.clipboard?.writeText(keyName);
  };

  return (
    <div className="flex h-full min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-panel/80 px-4 py-2">
          <button
            type="button"
            onClick={handleCopyName}
            className="flex items-center gap-1 truncate font-mono text-xs text-foreground hover:text-primary"
            title="Copy key name"
          >
            <IconCopy className="size-3 text-text-muted" />
            <span className="truncate">{keyName}</span>
          </button>
          {metadata ? (
            <>
              <Badge variant="secondary" className="text-[0.625rem]">
                {labelForType(metadata.type)}
              </Badge>
              <TtlBadge ttl={metadata.ttlSeconds} />
              {metadata.encoding ? (
                <Badge variant="outline" className="text-[0.625rem]">
                  {metadata.encoding}
                </Badge>
              ) : null}
              {metadata.elementCount !== undefined ? (
                <span className="text-[0.65rem] text-text-muted">
                  {metadata.elementCount.toLocaleString()}{" "}
                  {metadata.type === "string" ? "bytes" : "elements"}
                </span>
              ) : null}
            </>
          ) : null}
          <div className="ml-auto flex items-center gap-1">
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-[0.65rem]"
              onClick={() => setRefreshTick((t) => t + 1)}
            >
              <IconRefresh className="size-3" />
              Refresh
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-[0.65rem]"
              onClick={() => {
                setExpireValue("");
                setExpireOpen(true);
              }}
            >
              <IconClock className="size-3" />
              TTL
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-[0.65rem]"
              onClick={() => {
                setRenameValue(keyName);
                setRenameOpen(true);
              }}
            >
              <IconEdit className="size-3" />
              Rename
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="h-7 px-2 text-[0.65rem] text-destructive"
              onClick={() => {
                setDeleteConfirm("");
                setDeleteOpen(true);
              }}
            >
              <IconTrash className="size-3" />
              Delete
            </Button>
            <button
              type="button"
              onClick={() => setDrawerOpen((prev) => !prev)}
              className={cn(
                "rounded-md border border-border-subtle px-2 py-1 text-[0.65rem] text-text-muted hover:text-foreground",
                drawerOpen && "bg-surface-panel text-foreground",
              )}
            >
              <IconChevronRight
                className={cn(
                  "inline size-3 transition-transform",
                  drawerOpen && "rotate-180",
                )}
              />{" "}
              Details
            </button>
          </div>
        </header>
        {error ? (
          <div
            role="alert"
            className="mx-4 mt-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
          >
            {error}
          </div>
        ) : null}
        <div className="flex min-h-0 flex-1 flex-col">
          {metadata ? (
            <ValueRouter
              metadata={metadata}
              connectionId={connectionId}
              keyName={keyName}
            />
          ) : (
            <p className="p-4 text-xs text-text-muted">Loading metadata…</p>
          )}
        </div>
      </div>
      {actionError ? (
        <div
          role="alert"
          className="absolute top-12 left-1/2 z-10 -translate-x-1/2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          {actionError}
        </div>
      ) : null}
      <DeleteDialog
        open={deleteOpen}
        keyName={keyName}
        confirmText={deleteConfirm}
        onConfirmTextChange={setDeleteConfirm}
        onOpenChange={setDeleteOpen}
        onConfirm={async () => {
          setActionError(null);
          try {
            await delRedisKeys({ connectionId, keys: [keyName] });
            setDeleteOpen(false);
            onKeyDeleted?.(keyName);
          } catch (err) {
            setActionError(err instanceof Error ? err.message : String(err));
          }
        }}
      />
      <RenameDialog
        open={renameOpen}
        keyName={keyName}
        value={renameValue}
        onValueChange={setRenameValue}
        onOpenChange={setRenameOpen}
        onConfirm={async () => {
          setActionError(null);
          try {
            await renameRedisKey({
              connectionId,
              from: keyName,
              to: renameValue,
            });
            setRenameOpen(false);
            onKeyRenamed?.(keyName, renameValue);
          } catch (err) {
            setActionError(err instanceof Error ? err.message : String(err));
          }
        }}
      />
      <ExpireDialog
        open={expireOpen}
        keyName={keyName}
        value={expireValue}
        onValueChange={setExpireValue}
        onOpenChange={setExpireOpen}
        onConfirm={async () => {
          setActionError(null);
          const seconds = expireValue.trim() ? Number(expireValue) : null;
          try {
            await setRedisExpire({
              connectionId,
              key: keyName,
              ttlSeconds:
                Number.isFinite(seconds) && seconds !== null ? seconds : null,
            });
            setExpireOpen(false);
            setRefreshTick((t) => t + 1);
          } catch (err) {
            setActionError(err instanceof Error ? err.message : String(err));
          }
        }}
      />
      {drawerOpen && metadata ? (
        <aside className="w-72 shrink-0 border-l border-border-subtle bg-surface-panel/70 px-4 py-3 text-[0.65rem]">
          <h3 className="mb-2 font-semibold uppercase tracking-wide text-text-muted">
            Metadata
          </h3>
          <dl className="space-y-1.5">
            <MetadataRow label="Type" value={metadata.type} />
            <MetadataRow
              label="TTL"
              value={
                metadata.ttlSeconds === -1
                  ? "never"
                  : metadata.ttlSeconds === -2
                    ? "missing"
                    : `${metadata.ttlSeconds}s`
              }
            />
            {metadata.encoding ? (
              <MetadataRow label="Encoding" value={metadata.encoding} />
            ) : null}
            {metadata.elementCount !== undefined ? (
              <MetadataRow
                label="Elements"
                value={metadata.elementCount.toLocaleString()}
              />
            ) : null}
            {metadata.memoryBytes !== undefined ? (
              <MetadataRow
                label="Memory"
                value={`${metadata.memoryBytes.toLocaleString()} bytes`}
              />
            ) : null}
          </dl>
          <p className="mt-4 text-text-muted">
            Editors for string/hash land in Phase 1.4. Live-refresh / keyspace
            notifications are tracked in FOLLOWUPS.md.
          </p>
        </aside>
      ) : null}
    </div>
  );
}

function ValueRouter({
  metadata,
  connectionId,
  keyName,
}: {
  metadata: KeyMetadata;
  connectionId: string;
  keyName: string;
}) {
  switch (metadata.type) {
    case "string":
      return <StringValueView connectionId={connectionId} keyName={keyName} />;
    case "hash":
      return (
        <HashValueView
          connectionId={connectionId}
          keyName={keyName}
          elementCount={metadata.elementCount}
        />
      );
    case "list":
      return (
        <ListValueView
          connectionId={connectionId}
          keyName={keyName}
          elementCount={metadata.elementCount}
        />
      );
    case "set":
      return (
        <SetValueView
          connectionId={connectionId}
          keyName={keyName}
          elementCount={metadata.elementCount}
        />
      );
    case "zset":
      return (
        <SortedSetValueView
          connectionId={connectionId}
          keyName={keyName}
          elementCount={metadata.elementCount}
        />
      );
    case "stream":
      return (
        <StreamValueView
          connectionId={connectionId}
          keyName={keyName}
          elementCount={metadata.elementCount}
        />
      );
    case "ReJSON-RL":
      return <JsonValueView connectionId={connectionId} keyName={keyName} />;
    case "none":
      return (
        <p className="p-4 text-xs text-text-muted">
          Key not found — it may have expired or been deleted.
        </p>
      );
    default:
      return (
        <p className="p-4 text-xs text-text-muted">
          No viewer for type {metadata.type}. Detected as an unsupported or
          module-specific type; raw inspection via CLI tab (Phase 1.3) will
          help.
        </p>
      );
  }
}

function TtlBadge({ ttl }: { ttl: number }) {
  if (ttl === -1) {
    return (
      <Badge variant="outline" className="text-[0.625rem]">
        TTL: never
      </Badge>
    );
  }
  if (ttl === -2) {
    return (
      <Badge variant="destructive" className="text-[0.625rem]">
        Missing
      </Badge>
    );
  }
  return (
    <Badge variant="outline" className="text-[0.625rem]">
      TTL: {ttl}s
    </Badge>
  );
}

function MetadataRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-2 border-b border-border-subtle pb-1">
      <dt className="text-text-muted">{label}</dt>
      <dd className="font-mono text-foreground">{value}</dd>
    </div>
  );
}

function labelForType(type: string): string {
  switch (type) {
    case "ReJSON-RL":
      return "JSON";
    case "none":
      return "missing";
    default:
      return type;
  }
}

interface DialogShellProps {
  open: boolean;
  keyName: string;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => Promise<void> | void;
}

function DeleteDialog({
  open,
  keyName,
  confirmText,
  onConfirmTextChange,
  onOpenChange,
  onConfirm,
}: DialogShellProps & {
  confirmText: string;
  onConfirmTextChange: (text: string) => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Delete key</AlertDialogTitle>
          <AlertDialogDescription>
            Type{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {keyName}
            </code>{" "}
            to confirm. This is irreversible.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <Input
          value={confirmText}
          onChange={(event) => onConfirmTextChange(event.target.value)}
          className="font-mono text-xs"
        />
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            variant="destructive"
            disabled={confirmText !== keyName}
            onClick={() => {
              void onConfirm();
            }}
          >
            Delete
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function RenameDialog({
  open,
  keyName,
  value,
  onValueChange,
  onOpenChange,
  onConfirm,
}: DialogShellProps & {
  value: string;
  onValueChange: (value: string) => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Rename key</AlertDialogTitle>
          <AlertDialogDescription>
            Rename{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {keyName}
            </code>{" "}
            — issues `RENAME old new`, overwriting any existing key at the new
            name.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-1">
          <Label htmlFor="rename-key-input" className="text-xs">
            New name
          </Label>
          <Input
            id="rename-key-input"
            value={value}
            onChange={(event) => onValueChange(event.target.value)}
            className="font-mono text-xs"
          />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            disabled={!value.trim() || value === keyName}
            onClick={() => {
              void onConfirm();
            }}
          >
            Rename
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function ExpireDialog({
  open,
  keyName,
  value,
  onValueChange,
  onOpenChange,
  onConfirm,
}: DialogShellProps & {
  value: string;
  onValueChange: (value: string) => void;
}) {
  return (
    <AlertDialog open={open} onOpenChange={onOpenChange}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>Set TTL</AlertDialogTitle>
          <AlertDialogDescription>
            Set TTL for{" "}
            <code className="rounded bg-surface-panel px-1 py-0.5 font-mono">
              {keyName}
            </code>
            . Leave blank to remove TTL (PERSIST).
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-1">
          <Label htmlFor="expire-key-input" className="text-xs">
            Seconds
          </Label>
          <Input
            id="expire-key-input"
            type="number"
            value={value}
            onChange={(event) => onValueChange(event.target.value)}
            placeholder="e.g. 3600 — leave empty to PERSIST"
            className="font-mono text-xs"
          />
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
          <Button
            onClick={() => {
              void onConfirm();
            }}
          >
            Apply
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
