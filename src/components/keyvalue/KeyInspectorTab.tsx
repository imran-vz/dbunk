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

import { useCallback } from "react";

import {
  DeleteDialog,
  ExpireDialog,
  RenameDialog,
} from "@/components/keyvalue/key-inspector/key-inspector-dialogs";
import { KeyInspectorHeader } from "@/components/keyvalue/key-inspector/key-inspector-header";
import { MetadataDrawer } from "@/components/keyvalue/key-inspector/metadata-drawer";
import { useDrawerVisibility } from "@/components/keyvalue/key-inspector/use-drawer-visibility";
import { useInspectorDialogs } from "@/components/keyvalue/key-inspector/use-inspector-dialogs";
import { useKeyActions } from "@/components/keyvalue/key-inspector/use-key-actions";
import { useKeyMetadata } from "@/components/keyvalue/key-inspector/use-key-metadata";
import { ValueRouter } from "@/components/keyvalue/key-inspector/value-router";
import { useContainerWidth } from "@/lib/use-resizable-width";

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
  const [containerRef, containerWidth] = useContainerWidth<HTMLDivElement>();
  const drawer = useDrawerVisibility(containerWidth);
  const { metadata, error, refresh } = useKeyMetadata(connectionId, keyName);
  const dialogs = useInspectorDialogs(keyName);
  const actions = useKeyActions({
    connectionId,
    keyName,
    onKeyDeleted,
    onKeyRenamed,
    onAfterExpire: refresh,
  });

  const handleCopyName = useCallback(() => {
    void navigator.clipboard?.writeText(keyName);
  }, [keyName]);

  const handleConfirmDelete = async () => {
    if (await actions.deleteKey()) dialogs.delete.setOpen(false);
  };

  const handleConfirmRename = async () => {
    if (await actions.renameKey(dialogs.rename.value))
      dialogs.rename.setOpen(false);
  };

  const handleConfirmExpire = async () => {
    if (await actions.setExpire(dialogs.expire.value))
      dialogs.expire.setOpen(false);
  };

  return (
    <div ref={containerRef} className="flex h-full min-h-0 flex-1">
      <div className="flex min-w-0 flex-1 flex-col">
        <KeyInspectorHeader
          keyName={keyName}
          metadata={metadata}
          drawerOpen={drawer.open}
          onCopyName={handleCopyName}
          onRefresh={refresh}
          onOpenExpire={dialogs.expire.request}
          onOpenRename={dialogs.rename.request}
          onOpenDelete={dialogs.delete.request}
          onToggleDrawer={drawer.toggle}
        />
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
      {actions.actionError ? (
        <div
          role="alert"
          className="absolute top-12 left-1/2 z-10 -translate-x-1/2 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          {actions.actionError}
        </div>
      ) : null}
      <DeleteDialog
        open={dialogs.delete.open}
        keyName={keyName}
        confirmText={dialogs.delete.confirmText}
        onConfirmTextChange={dialogs.delete.setConfirmText}
        onOpenChange={dialogs.delete.setOpen}
        onConfirm={handleConfirmDelete}
      />
      <RenameDialog
        open={dialogs.rename.open}
        keyName={keyName}
        value={dialogs.rename.value}
        onValueChange={dialogs.rename.setValue}
        onOpenChange={dialogs.rename.setOpen}
        onConfirm={handleConfirmRename}
      />
      <ExpireDialog
        open={dialogs.expire.open}
        keyName={keyName}
        value={dialogs.expire.value}
        onValueChange={dialogs.expire.setValue}
        onOpenChange={dialogs.expire.setOpen}
        onConfirm={handleConfirmExpire}
      />
      {drawer.open && metadata ? <MetadataDrawer metadata={metadata} /> : null}
    </div>
  );
}
