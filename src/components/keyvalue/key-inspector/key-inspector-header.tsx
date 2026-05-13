import {
  IconChevronRight,
  IconClock,
  IconCopy,
  IconEdit,
  IconRefresh,
  IconTrash,
} from "@tabler/icons-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import type { KeyMetadata } from "@/lib/redis/api";
import { cn } from "@/lib/utils";

interface KeyInspectorHeaderProps {
  keyName: string;
  metadata: KeyMetadata | null;
  drawerOpen: boolean;
  onCopyName: () => void;
  onRefresh: () => void;
  onOpenExpire: () => void;
  onOpenRename: () => void;
  onOpenDelete: () => void;
  onToggleDrawer: () => void;
}

export function KeyInspectorHeader({
  keyName,
  metadata,
  drawerOpen,
  onCopyName,
  onRefresh,
  onOpenExpire,
  onOpenRename,
  onOpenDelete,
  onToggleDrawer,
}: KeyInspectorHeaderProps) {
  return (
    <header className="flex flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-panel/80 px-4 py-2">
      <button
        type="button"
        onClick={onCopyName}
        className="flex items-center gap-1 truncate font-mono text-xs text-foreground hover:text-primary"
        title="Copy key name"
      >
        <IconCopy className="size-3 text-text-muted" />
        <span className="truncate">{keyName}</span>
      </button>
      {metadata ? <HeaderMetadataBadges metadata={metadata} /> : null}
      <div className="ml-auto flex items-center gap-1">
        <Button
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-[0.65rem]"
          onClick={onRefresh}
        >
          <IconRefresh className="size-3" />
          Refresh
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-[0.65rem]"
          onClick={onOpenExpire}
        >
          <IconClock className="size-3" />
          TTL
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-[0.65rem]"
          onClick={onOpenRename}
        >
          <IconEdit className="size-3" />
          Rename
        </Button>
        <Button
          size="sm"
          variant="ghost"
          className="h-7 px-2 text-[0.65rem] text-destructive"
          onClick={onOpenDelete}
        >
          <IconTrash className="size-3" />
          Delete
        </Button>
        <button
          type="button"
          onClick={onToggleDrawer}
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
  );
}

function HeaderMetadataBadges({ metadata }: { metadata: KeyMetadata }) {
  return (
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
  );
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

export function labelForType(type: string): string {
  if (type === "ReJSON-RL") return "JSON";
  if (type === "none") return "missing";
  return type;
}
