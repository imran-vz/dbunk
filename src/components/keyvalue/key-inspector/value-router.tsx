import type { JSX } from "react";

import { HashValueView } from "@/components/keyvalue/viewers/HashValueView";
import { JsonValueView } from "@/components/keyvalue/viewers/JsonValueView";
import { ListValueView } from "@/components/keyvalue/viewers/ListValueView";
import { SetValueView } from "@/components/keyvalue/viewers/SetValueView";
import { SortedSetValueView } from "@/components/keyvalue/viewers/SortedSetValueView";
import { StreamValueView } from "@/components/keyvalue/viewers/StreamValueView";
import { StringValueView } from "@/components/keyvalue/viewers/StringValueView";
import type { KeyMetadata } from "@/lib/redis/api";

interface ValueRouterProps {
  metadata: KeyMetadata;
  connectionId: string;
  keyName: string;
}

interface ViewerProps {
  connectionId: string;
  keyName: string;
  elementCount?: number;
}

type ViewerRenderer = (props: ViewerProps) => JSX.Element;

/**
 * Per-type viewer lookup. Keeping this as data (not a switch) is how we keep
 * the inspector itself below fallow's cognitive threshold — adding a new type
 * is a one-line map entry, not another branch.
 */
// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const VIEWERS: Record<string, ViewerRenderer> = {
  string: ({ connectionId, keyName }) => (
    <StringValueView connectionId={connectionId} keyName={keyName} />
  ),
  hash: ({ connectionId, keyName, elementCount }) => (
    <HashValueView
      connectionId={connectionId}
      keyName={keyName}
      elementCount={elementCount}
    />
  ),
  list: ({ connectionId, keyName, elementCount }) => (
    <ListValueView
      connectionId={connectionId}
      keyName={keyName}
      elementCount={elementCount}
    />
  ),
  set: ({ connectionId, keyName, elementCount }) => (
    <SetValueView
      connectionId={connectionId}
      keyName={keyName}
      elementCount={elementCount}
    />
  ),
  zset: ({ connectionId, keyName, elementCount }) => (
    <SortedSetValueView
      connectionId={connectionId}
      keyName={keyName}
      elementCount={elementCount}
    />
  ),
  stream: ({ connectionId, keyName, elementCount }) => (
    <StreamValueView
      connectionId={connectionId}
      keyName={keyName}
      elementCount={elementCount}
    />
  ),
  "ReJSON-RL": ({ connectionId, keyName }) => (
    <JsonValueView connectionId={connectionId} keyName={keyName} />
  ),
  none: () => (
    <p className="p-4 text-xs text-text-muted">
      Key not found — it may have expired or been deleted.
    </p>
  ),
};

export function ValueRouter({
  metadata,
  connectionId,
  keyName,
}: ValueRouterProps) {
  const render = VIEWERS[metadata.type];
  if (render) {
    return render({
      connectionId,
      keyName,
      elementCount: metadata.elementCount,
    });
  }
  return (
    <p className="p-4 text-xs text-text-muted">
      No viewer for type {metadata.type}. Detected as an unsupported or
      module-specific type; raw inspection via CLI tab (Phase 1.3) will help.
    </p>
  );
}
