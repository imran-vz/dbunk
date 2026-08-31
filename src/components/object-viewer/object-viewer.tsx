import { IconCopy, IconExternalLink } from "@tabler/icons-react";
import { useEffect, useMemo, useRef, useState } from "react";

import {
  DdlReviewDialog,
  DropImpactDialog,
  NewMaterializedViewDialog,
  NewViewDialog,
  ObjectActionDialog,
  type DdlReviewTerminal,
  type ObjectActionKind,
} from "@/components/object-ddl";
import { ObjectFactsPanel } from "@/components/object-viewer/object-facts";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  EmptyState,
  ErrorState,
  LoadingState,
} from "@/components/ui/state-panel";
import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import {
  canonicalPgObjectRefKey,
  displayPgObjectName,
} from "@/lib/pg-object-ref";
import {
  formatPgCatalogError,
  isConnectedStatus,
  pgObjectDescriptionKey,
  type DdlApplyResult,
  type PgObjectKind,
  type PgObjectOp,
  type PgObjectRef,
  useAppStore,
} from "@/lib/store";
import { errorToMessage } from "@/lib/tauri";
import { cn } from "@/lib/utils";

const KIND_LABELS = {
  schema: "Schema",
  table: "Table",
  view: "View",
  "materialized-view": "Materialized view",
  "foreign-table": "Foreign table",
  sequence: "Sequence",
  function: "Function",
  procedure: "Procedure",
  aggregate: "Aggregate",
  type: "Type",
  domain: "Domain",
  extension: "Extension",
} satisfies Record<PgObjectKind, string>;

type ViewerSection = "definition" | "facts";

export const canRenameObjectKind = (kind: PgObjectKind): boolean =>
  kind === "schema" ||
  kind === "table" ||
  kind === "view" ||
  kind === "materialized-view" ||
  kind === "sequence";

export const objectDefinitionTabLabel = (reference: PgObjectRef): string =>
  `${displayPgObjectName(reference)}_${reference.kind}_definition.sql`;

type ObjectViewerProps = {
  tabId: string;
  connectionId: string;
  reference: PgObjectRef;
};

/** Remount the stateful lifecycle surface whenever object identity changes. */
export function ObjectViewer(props: ObjectViewerProps) {
  const instanceKey = `${props.tabId}:${props.connectionId}:${canonicalPgObjectRefKey(props.reference)}`;
  return <ObjectViewerSession key={instanceKey} {...props} />;
}

function ObjectViewerSession({
  tabId,
  connectionId,
  reference,
}: ObjectViewerProps) {
  const [section, setSection] = useState<ViewerSection>("definition");
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const [reviewOps, setReviewOps] = useState<PgObjectOp[] | null>(null);
  const [reviewTerminal, setReviewTerminal] =
    useState<DdlReviewTerminal | null>(null);
  const [commentEditing, setCommentEditing] = useState(false);
  const [commentDraft, setCommentDraft] = useState("");
  const [actionDialog, setActionDialog] = useState<ObjectActionKind | null>(
    null,
  );
  const [definitionDialog, setDefinitionDialog] = useState<
    "view" | "materialized-view" | null
  >(null);
  const [dropAppendOps, setDropAppendOps] = useState<PgObjectOp[] | null>(null);
  const [refreshState, setRefreshState] = useState<
    "idle" | "refreshing" | "success" | "error"
  >("idle");
  const connection = useAppStore((state) =>
    state.connections.find((candidate) => candidate.id === connectionId),
  );
  const cacheKey = useMemo(
    () => pgObjectDescriptionKey(connectionId, reference),
    [connectionId, reference],
  );
  const descriptionState = useAppStore(
    (state) => state.pgObjectDescriptions[cacheKey],
  );
  const loadDescription = useAppStore((state) => state.loadPgObjectDescription);
  const referenceRef = useRef(reference);
  referenceRef.current = reference;
  const connected =
    connection !== undefined && isConnectedStatus(connection.status);
  const beginReview = (ops: PgObjectOp[]) => {
    setReviewTerminal(null);
    setReviewOps(ops);
  };

  useEffect(() => {
    if (
      !commentEditing &&
      descriptionState?.status === "ready" &&
      descriptionState.description
    ) {
      setCommentDraft(descriptionState.description.comment ?? "");
    }
  }, [commentEditing, descriptionState]);

  useEffect(() => {
    if (!connected) return;
    const revalidate = () => {
      void loadDescription(connectionId, referenceRef.current);
    };

    // A cached viewer can outlive the database object. Revalidate on mount and
    // whenever the app regains focus so an external drop reaches the typed
    // objectNotFound state instead of leaving stale metadata on screen.
    revalidate();
    window.addEventListener("focus", revalidate);
    return () => window.removeEventListener("focus", revalidate);
  }, [cacheKey, connected, connectionId, loadDescription]);

  if (!connection) {
    return (
      <ErrorState
        message="This object's connection no longer exists."
        className="m-4"
      />
    );
  }

  if (!connected) {
    return (
      <div
        data-testid="object-awaiting-connection"
        className="flex h-full flex-col items-center justify-center gap-3 bg-surface-app p-6 text-center"
      >
        <div className="text-sm font-semibold text-foreground">
          “{connection.name}” is disconnected
        </div>
        <p className="max-w-sm text-xs text-text-muted">
          Connect to load{" "}
          <span className="font-mono">{displayPgObjectName(reference)}</span>.
          Nothing is fetched until you do.
        </p>
        <Button
          type="button"
          size="sm"
          onClick={() => {
            void useAppStore.getState().connectConnection(connection.id);
          }}
        >
          Connect &amp; load
        </Button>
      </div>
    );
  }

  if (
    !descriptionState ||
    (descriptionState.status === "loading" && !descriptionState.description)
  ) {
    return <LoadingState label="Loading object…" />;
  }

  if (descriptionState.status === "idle") {
    return <LoadingState label="Loading object…" />;
  }

  if (descriptionState.status === "error") {
    if (descriptionState.error?.kind === "objectNotFound") {
      return (
        <EmptyState
          title={`${displayPgObjectName(reference)} no longer exists. It may have been dropped outside dbunk.`}
          action={
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => {
                void useAppStore.getState().closeTab(tabId);
              }}
            >
              Close tab
            </Button>
          }
        />
      );
    }
    return (
      <ErrorState
        message={
          descriptionState.error
            ? formatPgCatalogError(descriptionState.error)
            : "Failed to load the object."
        }
        onRetry={() => {
          void loadDescription(connectionId, reference);
        }}
        className="m-4"
      />
    );
  }

  const description = descriptionState.description;
  if (!description) {
    return (
      <ErrorState message="The object description was empty." className="m-4" />
    );
  }
  const definition = description.definitionSql;

  const copyDefinition = async () => {
    if (!definition) return;
    try {
      await navigator.clipboard.writeText(definition);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  };

  const openDefinition = () => {
    if (!definition) return;
    useAppStore.getState().openWorkspaceTab({
      kind: "query",
      label: objectDefinitionTabLabel(reference),
      connectionId,
      schema: reference.schema ?? "",
      query: definition,
    });
  };

  const refreshMaterializedView = async () => {
    if (reference.kind !== "materialized-view" || !reference.schema) return;
    const expectedGeneration =
      useAppStore.getState().pgObjectCatalog[connectionId]?.generation ?? 0;
    const expectedConnectionEpoch =
      useAppStore.getState().connectionEpochs[connectionId] ?? 0;
    const isConnectionCurrent = () =>
      (useAppStore.getState().connectionEpochs[connectionId] ?? 0) ===
      expectedConnectionEpoch;
    setRefreshState("refreshing");
    try {
      // Declared lifecycle exception: REFRESH MATERIALIZED VIEW is not DDL and
      // remains on the existing safety-confirmed command path.
      await invokeWithSafetyConfirmation<{ runtimeMs: number }>({
        command: "refresh_materialized_view",
        payload: {
          connectionId,
          schema: reference.schema,
          view: reference.name,
          concurrently: false,
        },
        connection,
        isConnectionCurrent,
      });
      if (!isConnectionCurrent()) {
        setRefreshState("error");
        return;
      }
      const descriptionResult = await loadDescription(
        connectionId,
        reference,
        expectedGeneration,
      );
      if (descriptionResult !== "ready" || !isConnectionCurrent()) {
        setRefreshState("error");
        return;
      }
      setRefreshState("success");
    } catch (error) {
      console.error(
        "Failed to refresh materialized view",
        errorToMessage(error),
      );
      setRefreshState("error");
    }
  };

  const facts = description.facts;
  const isEnum = facts.kind === "type" && facts.class === "enum";
  const enumLabels = isEnum ? (facts.enumLabels ?? []) : [];
  const editableViewDefinition =
    facts.kind === "view" || facts.kind === "materializedView"
      ? facts.definition
      : "";
  const materializedViewPopulated =
    facts.kind === "materializedView" ? facts.populated : true;
  const lifecycleReviewOpen = reviewOps !== null || commentEditing;

  const handleApplied = async (
    _result: DdlApplyResult,
    expectedGeneration: number,
  ) => {
    const targetsCurrentReference = (operation: PgObjectOp) =>
      (operation.op === "renameObject" || operation.op === "dropObject") &&
      canonicalPgObjectRefKey(operation.reference) ===
        canonicalPgObjectRefKey(reference);
    const rename = reviewOps?.find(
      (operation) =>
        operation.op === "renameObject" && targetsCurrentReference(operation),
    );
    const dropped = reviewOps?.some(
      (operation) =>
        operation.op === "dropObject" && targetsCurrentReference(operation),
    );
    if ((!rename || rename.op !== "renameObject") && !dropped) return;

    const store = useAppStore.getState();
    if (
      (store.pgObjectCatalog[connectionId]?.generation ?? 0) !==
      expectedGeneration
    ) {
      return;
    }

    if (reference.kind === "schema") {
      store.dropPgObjectDescriptionsForSchema(connectionId, reference.name);
      if (!rename || rename.op !== "renameObject") {
        await useAppStore
          .getState()
          .loadPgObjectDescription(connectionId, reference, expectedGeneration);
        return;
      }

      const renamedSchema = rename.newName;
      const renamedReference = { ...reference, name: renamedSchema };
      store.setWorkspaceTabs((tabs) =>
        tabs.map((tab) => {
          if (tab.kind !== "object" || tab.connectionId !== connectionId) {
            return tab;
          }
          const tabReference = tab.objectRef;
          if (!tabReference) return tab;
          const nextReference =
            tabReference.kind === "schema" &&
            tabReference.name === reference.name
              ? { ...tabReference, name: renamedSchema }
              : tabReference.schema === reference.name
                ? { ...tabReference, schema: renamedSchema }
                : null;
          return nextReference
            ? {
                ...tab,
                label: displayPgObjectName(nextReference),
                schema: nextReference.schema ?? "",
                objectRef: nextReference,
              }
            : tab;
        }),
      );
      await useAppStore
        .getState()
        .loadPgObjectDescription(
          connectionId,
          renamedReference,
          expectedGeneration,
        );
      return;
    }

    if (!rename || rename.op !== "renameObject") return;
    const renamedReference = { ...reference, name: rename.newName };
    store.setWorkspaceTabs((tabs) =>
      tabs.map((tab) =>
        tab.id === tabId
          ? {
              ...tab,
              label: displayPgObjectName(renamedReference),
              schema: renamedReference.schema ?? "",
              objectRef: renamedReference,
            }
          : tab,
      ),
    );
    await useAppStore
      .getState()
      .loadPgObjectDescription(
        connectionId,
        renamedReference,
        expectedGeneration,
      );
  };

  const reviewComment = () => {
    beginReview([
      {
        op: "setComment",
        target: { kind: "object", reference },
        comment: commentDraft.trim() || null,
      },
    ]);
    setCommentEditing(false);
  };

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface-app">
      <header className="shrink-0 border-b border-border-subtle px-4 py-3">
        <div className="flex min-w-0 items-baseline gap-3">
          <span className="shrink-0 font-mono text-2xs font-semibold uppercase tracking-wide text-accent">
            {KIND_LABELS[reference.kind]}
          </span>
          <h1 className="truncate font-mono text-base font-semibold text-foreground">
            {displayPgObjectName(reference)}
          </h1>
        </div>
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 text-xs text-text-muted">
          <span>Owner {description.owner ?? "unknown"}</span>
          <span aria-hidden="true">·</span>
          {commentEditing ? (
            <>
              <Input
                aria-label="Object comment"
                value={commentDraft}
                onChange={(event) => setCommentDraft(event.target.value)}
                className="h-7 min-w-48 flex-1 text-xs"
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    setCommentDraft(description.comment ?? "");
                    setCommentEditing(false);
                  }
                }}
              />
              <Button type="button" size="xs" onClick={reviewComment}>
                Review DDL
              </Button>
              <Button
                type="button"
                size="xs"
                variant="ghost"
                onClick={() => {
                  setCommentDraft(description.comment ?? "");
                  setCommentEditing(false);
                }}
              >
                Cancel
              </Button>
            </>
          ) : (
            <>
              <span>{description.comment ?? "No comment"}</span>
              <Button
                type="button"
                size="xs"
                variant="ghost"
                disabled={lifecycleReviewOpen}
                onClick={() => setCommentEditing(true)}
              >
                Edit comment
              </Button>
            </>
          )}
        </div>
      </header>

      <div className="flex h-9 shrink-0 items-stretch border-b border-border-subtle px-4">
        <div
          role="tablist"
          aria-label="Object viewer sections"
          className="flex"
        >
          {(["definition", "facts"] as const).map((candidate) => (
            <button
              key={candidate}
              type="button"
              role="tab"
              aria-selected={section === candidate}
              onClick={() => setSection(candidate)}
              className={cn(
                "border-b-2 px-3 text-xs capitalize",
                section === candidate
                  ? "border-accent text-foreground"
                  : "border-transparent text-text-muted hover:text-foreground",
              )}
            >
              {candidate}
            </button>
          ))}
        </div>
        {section === "definition" ? (
          <div className="ml-auto flex items-center gap-1">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={!definition}
              onClick={() => void copyDefinition()}
            >
              <IconCopy />
              {copyState === "copied"
                ? "Copied"
                : copyState === "failed"
                  ? "Copy failed"
                  : "Copy"}
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={!definition}
              onClick={openDefinition}
            >
              <IconExternalLink />
              Open in SQL editor
            </Button>
          </div>
        ) : null}
      </div>

      <div
        role="tabpanel"
        aria-label={section === "definition" ? "Definition" : "Facts"}
        className="min-h-0 flex-1 overflow-auto"
      >
        {section === "facts" ? (
          <ObjectFactsPanel facts={description.facts} />
        ) : definition ? (
          <pre className="min-h-full overflow-auto p-4 font-mono text-xs leading-5 whitespace-pre-wrap text-text-secondary">
            <code>{definition}</code>
          </pre>
        ) : (
          <EmptyState
            title={
              reference.kind === "aggregate"
                ? "Definition rendering is not supported for aggregates."
                : "Definition is not available for this object."
            }
          />
        )}
      </div>
      {reviewOps ? (
        <DdlReviewDialog
          open
          variant="inline"
          connectionId={connectionId}
          ops={reviewOps}
          onApplied={handleApplied}
          initialTerminal={reviewTerminal}
          onTerminalChange={setReviewTerminal}
          onOpenChange={(next) => {
            if (!next) {
              setReviewOps(null);
              setReviewTerminal(null);
            }
          }}
        />
      ) : null}
      <footer className="flex min-h-11 shrink-0 items-center gap-1 border-t border-border-subtle px-4 py-2">
        {canRenameObjectKind(reference.kind) ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={lifecycleReviewOpen}
            onClick={() => setActionDialog("rename")}
          >
            Rename
          </Button>
        ) : null}
        {reference.kind === "sequence" ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={lifecycleReviewOpen}
            onClick={() => setActionDialog("alter-sequence")}
          >
            Alter
          </Button>
        ) : null}
        {isEnum ? (
          <>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={lifecycleReviewOpen}
              onClick={() => setActionDialog("add-enum-value")}
            >
              Add value
            </Button>
            {enumLabels.length > 0 ? (
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={lifecycleReviewOpen}
                onClick={() => setActionDialog("rename-enum-value")}
              >
                Rename value
              </Button>
            ) : null}
          </>
        ) : null}
        {reference.kind === "view" ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            disabled={lifecycleReviewOpen}
            onClick={() => setDefinitionDialog("view")}
          >
            Edit definition
          </Button>
        ) : null}
        {reference.kind === "materialized-view" ? (
          <>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={lifecycleReviewOpen}
              onClick={() => setDefinitionDialog("materialized-view")}
            >
              Edit definition
            </Button>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              disabled={refreshState === "refreshing" || lifecycleReviewOpen}
              onClick={() => void refreshMaterializedView()}
            >
              {refreshState === "refreshing" ? "Refreshing…" : "Refresh"}
            </Button>
          </>
        ) : null}
        {refreshState === "success" ? (
          <output className="ml-2 text-xs text-success">Refreshed</output>
        ) : refreshState === "error" ? (
          <output className="ml-2 text-xs text-danger">Refresh failed</output>
        ) : null}
        <span className="flex-1" />
        {reference.kind !== "extension" ? (
          <Button
            type="button"
            size="sm"
            variant="destructive"
            disabled={lifecycleReviewOpen}
            onClick={() => setDropAppendOps([])}
          >
            Drop
          </Button>
        ) : null}
      </footer>

      {actionDialog ? (
        <ObjectActionDialog
          open
          action={actionDialog}
          reference={reference}
          currentComment={description.comment}
          enumLabels={enumLabels}
          onOpenChange={(next) => {
            if (!next) setActionDialog(null);
          }}
          onOps={beginReview}
        />
      ) : null}
      {definitionDialog === "view" ? (
        <NewViewDialog
          open
          connectionId={connectionId}
          schema={reference.schema ?? ""}
          initialName={reference.name}
          initialSqlBody={editableViewDefinition}
          orReplace
          onOpenChange={(next) => {
            if (!next) setDefinitionDialog(null);
          }}
          onOps={beginReview}
        />
      ) : null}
      {definitionDialog === "materialized-view" ? (
        <NewMaterializedViewDialog
          open
          connectionId={connectionId}
          schema={reference.schema ?? ""}
          initialName={reference.name}
          initialSqlBody={editableViewDefinition}
          initialWithData={materializedViewPopulated}
          onOpenChange={(next) => {
            if (!next) setDefinitionDialog(null);
          }}
          onOps={(ops) => setDropAppendOps(ops)}
        />
      ) : null}
      {dropAppendOps ? (
        <DropImpactDialog
          open
          connectionId={connectionId}
          reference={reference}
          appendOps={dropAppendOps}
          onOpenChange={(next) => {
            if (!next) setDropAppendOps(null);
          }}
          onReview={beginReview}
        />
      ) : null}
    </section>
  );
}
