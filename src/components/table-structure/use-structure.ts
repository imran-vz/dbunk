import { useEffect, useMemo, useState } from "react";

import { requestConfirm } from "@/lib/confirm";
import {
  type ColumnChangeKind,
  classifyDestructive,
  generateDdlForEngine,
} from "@/lib/ddl";
import { type RelationalPolicy, relationalPolicy } from "@/lib/engine-policy";
import { formatObjectDdlError, previewObjectDdl } from "@/lib/object-ddl";
import {
  type ColumnInfo,
  type ConstraintInfo,
  type DatabaseEngine,
  type DdlPlanPreview,
  type DDLOutcome,
  type ForeignKeyInfo,
  type IndexInfo,
  type PendingChange,
  type PgObjectOp,
  type StructureCapabilities,
  type StructureCommitStatus,
  type TableStructure,
  type TableStructureStatus,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { pendingStructureBatch } from "@/lib/structure-changes";

import { describeChange } from "./shared";

/**
 * Async preview state of a queued PostgreSQL `pg-op` batch. `idle` when
 * the pending list is empty or legacy-column shaped. The loaded preview
 * is the single source of per-statement summaries and destructiveness —
 * PG rows never go through `describeChange` / `classifyDestructive`.
 */
export type StructurePgPreview =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready"; preview: DdlPlanPreview }
  | { state: "error"; message: string };

const FALLBACK_CAPABILITIES: StructureCapabilities = {
  columns: false,
  primaryKey: false,
  foreignKeys: false,
  indexes: false,
  constraints: false,
  canInsertRows: false,
  canUpdateRows: false,
  canDeleteRows: false,
  canAlterSchema: false,
  uniquenessGuarantee: "best-effort",
  triggers: false,
  policies: false,
  privileges: false,
};

interface StructureSnapshot {
  columns: ColumnInfo[];
  primaryKey: string[] | null;
  foreignKeys: ForeignKeyInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  capabilities: StructureCapabilities;
  tableEngine: string | undefined;
  partitionBy: string | null;
  sampleBy: string | null;
}

export function snapshotFromStructure(
  structure: TableStructure | undefined,
): StructureSnapshot {
  return {
    columns: structure?.columns ?? [],
    primaryKey: structure?.primaryKey ?? null,
    foreignKeys: structure?.foreignKeys ?? [],
    indexes: structure?.indexes ?? [],
    constraints: structure?.constraints ?? [],
    capabilities: structure?.capabilities ?? FALLBACK_CAPABILITIES,
    tableEngine: structure?.tableEngine,
    partitionBy: structure?.partitionBy ?? null,
    sampleBy: structure?.sampleBy ?? null,
  };
}

interface UseStructureArgs {
  connectionId: string;
  schema: string;
  tableName: string;
}

export interface StructureView {
  key: string;
  structure: TableStructure | undefined;
  status: TableStructureStatus | undefined;
  pending: PendingChange[];
  commitStatus: StructureCommitStatus | undefined;
  engine: DatabaseEngine | undefined;
  policy: RelationalPolicy;
  capabilities: StructureCapabilities;
  editable: boolean;
  isLoading: boolean;
  errorMessage: string | null;
  previewSql: string;
  pgPreview: StructurePgPreview;
  commitDisabled: boolean;
  lastOutcome: DDLOutcome | null;
  // Pre-defaulted values so the parent JSX doesn't need `?? []` everywhere
  columns: ColumnInfo[];
  primaryKey: string[] | null;
  foreignKeys: ForeignKeyInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  tableEngine: string | undefined;
  partitionBy: string | null;
  sampleBy: string | null;
  showPhysicalLayout: boolean;
  queueChange: (change: ColumnChangeKind) => void;
  /** Non-null only for editable PostgreSQL tables — forms produce typed
   * ops directly; there is no `ColumnChangeKind -> PgObjectOp` mapper. */
  queuePgOp: ((op: PgObjectOp) => void) | null;
  removePending: (id: string) => void;
  retry: () => void;
  commit: () => Promise<void>;
}

export function useStructure({
  connectionId,
  schema,
  tableName,
}: UseStructureArgs): StructureView {
  const key = tableStructureKey(connectionId, schema, tableName);
  const loadTableStructure = useAppStore((state) => state.loadTableStructure);
  const addPendingStructureChange = useAppStore(
    (state) => state.addPendingStructureChange,
  );
  const removePendingStructureChange = useAppStore(
    (state) => state.removePendingStructureChange,
  );
  const commitStructureChanges = useAppStore(
    (state) => state.commitStructureChanges,
  );
  const structure = useAppStore((state) => state.tableStructure[key]);
  const status = useAppStore((state) => state.tableStructureStatus[key]);
  const pendingChanges = useAppStore(
    (state) => state.pendingStructureChanges[key],
  );
  const commitStatus = useAppStore((state) => state.structureCommitStatus[key]);
  const engine = useAppStore(
    (state) =>
      state.connections.find((connection) => connection.id === connectionId)
        ?.engine,
  );

  // Terminal outcome lives component-local. Disappears on table switch
  // (the effect below resets it). See CONTEXT.md — DDL Outcome.
  const [lastOutcome, setLastOutcome] = useState<DDLOutcome | null>(null);

  useEffect(() => {
    setLastOutcome(null);
    if (connectionId && schema && tableName) {
      void loadTableStructure(connectionId, schema, tableName);
    }
  }, [connectionId, schema, tableName, loadTableStructure]);

  const snapshot = snapshotFromStructure(structure);
  const editable = snapshot.capabilities.canAlterSchema;
  const pending = useMemo(() => pendingChanges ?? [], [pendingChanges]);
  const policy = relationalPolicy(engine ?? "PostgreSQL");
  const isLoading = status?.state === "loading";
  const errorMessage = status?.state === "error" ? status.error : null;
  const pendingBatch = useMemo(() => pendingStructureBatch(pending), [pending]);

  const previewSql = useMemo(() => {
    if (pendingBatch.kind !== "column") return "";
    return generateDdlForEngine(
      engine ?? "PostgreSQL",
      schema,
      tableName,
      pendingBatch.changes,
      snapshot.columns,
    );
  }, [engine, schema, tableName, snapshot.columns, pendingBatch]);

  // Execution identity of the queued PostgreSQL batch. Any pending-list
  // edit yields a new key; any DDL applied elsewhere bumps the version;
  // a reconnect bumps the epoch. All three invalidate a loaded preview.
  const pgOpsKey = useMemo(
    () =>
      pendingBatch.kind === "pg-op" ? JSON.stringify(pendingBatch.ops) : null,
    [pendingBatch],
  );
  const ddlVersion = useAppStore((state) => state.pgObjectDdlVersion);
  const connectionEpoch = useAppStore(
    (state) => state.connectionEpochs[connectionId] ?? 0,
  );
  const [loadedPreview, setLoadedPreview] = useState<
    | { state: "loading" }
    | { state: "ready"; preview: DdlPlanPreview; reviewedKey: string }
    | { state: "error"; message: string; reviewedKey: string }
  >({ state: "loading" });

  // Everything the reviewed preview must still match at commit time.
  const previewIdentity =
    pgOpsKey === null ? null : `${connectionEpoch}:${ddlVersion}:${pgOpsKey}`;

  useEffect(() => {
    if (previewIdentity === null || pgOpsKey === null) return;
    let stale = false;
    setLoadedPreview({ state: "loading" });
    // SAFETY: pgOpsKey is JSON.stringify of the pending batch's PgObjectOp[].
    const ops = JSON.parse(pgOpsKey) as PgObjectOp[];
    void previewObjectDdl({ connectionId, ops }).then((result) => {
      if (stale) return;
      if (result.kind === "ok") {
        setLoadedPreview({
          state: "ready",
          preview: result.value,
          reviewedKey: previewIdentity,
        });
      } else if (result.kind === "error") {
        setLoadedPreview({
          state: "error",
          message: formatObjectDdlError(result.error),
          reviewedKey: previewIdentity,
        });
      }
    });
    return () => {
      stale = true;
    };
  }, [previewIdentity, pgOpsKey, connectionId]);

  // Derived synchronously so a pending-list edit can never render or
  // commit against the previous batch's preview — a `ready`/`error`
  // result for a stale identity reads as loading until the effect
  // catches up (Plan 015 review finding).
  const pgPreview: StructurePgPreview = useMemo(() => {
    if (previewIdentity === null) return { state: "idle" };
    if (loadedPreview.state === "ready") {
      return loadedPreview.reviewedKey === previewIdentity
        ? { state: "ready", preview: loadedPreview.preview }
        : { state: "loading" };
    }
    if (loadedPreview.state === "error") {
      return loadedPreview.reviewedKey === previewIdentity
        ? { state: "error", message: loadedPreview.message }
        : { state: "loading" };
    }
    return { state: "loading" };
  }, [previewIdentity, loadedPreview]);

  const isRunning = commitStatus?.state === "running";
  const commitDisabled =
    pending.length === 0 ||
    isRunning ||
    (pendingBatch.kind === "pg-op" && pgPreview.state !== "ready");

  // True only while the reviewed preview still matches the live pending
  // batch, DDL version, and connection epoch. Re-checked after every
  // await inside commit — the render-time closure is not trusted.
  const reviewedPreviewIsCurrent = () => {
    if (loadedPreview.state !== "ready") return false;
    const current = useAppStore.getState();
    const currentBatch = pendingStructureBatch(
      current.pendingStructureChanges[key] ?? [],
    );
    if (currentBatch.kind !== "pg-op") return false;
    const currentIdentity = `${current.connectionEpochs[connectionId] ?? 0}:${current.pgObjectDdlVersion}:${JSON.stringify(currentBatch.ops)}`;
    return loadedPreview.reviewedKey === currentIdentity;
  };

  const commit = async () => {
    if (pendingBatch.kind === "pg-op") {
      if (
        loadedPreview.state !== "ready" ||
        isRunning ||
        !reviewedPreviewIsCurrent()
      ) {
        return;
      }
      // Destructiveness comes from the reviewed preview, never from a
      // frontend classifier (Plan 015 invariant).
      const destructive = loadedPreview.preview.statements.filter(
        (statement) => statement.destructive,
      );
      if (destructive.length > 0) {
        const ok = await requestConfirm({
          title: "Apply destructive schema changes?",
          message: "These statements are destructive and may lose data.",
          detail: destructive.map((statement) => statement.summary).join("\n"),
          confirmLabel: "Apply changes",
          danger: true,
        });
        if (!ok) return;
      }
      // The confirm dialog can stay open for a while; anything reviewed
      // may have drifted underneath it.
      if (!reviewedPreviewIsCurrent()) return;
      const outcome = await commitStructureChanges(key, {
        statements: loadedPreview.preview.statements,
      });
      if (outcome) setLastOutcome(outcome);
      return;
    }
    const outcome =
      pendingBatch.kind === "column"
        ? await runDestructiveCommit(pendingBatch.changes, () =>
            commitStructureChanges(key),
          )
        : await commitStructureChanges(key);
    if (outcome) setLastOutcome(outcome);
  };

  return {
    key,
    structure,
    status,
    pending,
    commitStatus,
    engine,
    policy,
    capabilities: snapshot.capabilities,
    editable,
    isLoading,
    errorMessage,
    previewSql,
    pgPreview,
    commitDisabled,
    lastOutcome,
    columns: snapshot.columns,
    primaryKey: snapshot.primaryKey,
    foreignKeys: snapshot.foreignKeys,
    indexes: snapshot.indexes,
    constraints: snapshot.constraints,
    tableEngine: snapshot.tableEngine,
    partitionBy: snapshot.partitionBy,
    sampleBy: snapshot.sampleBy,
    showPhysicalLayout: Boolean(snapshot.partitionBy || snapshot.sampleBy),
    queueChange: (change: ColumnChangeKind) =>
      addPendingStructureChange(key, {
        schema,
        table: tableName,
        change: { kind: "column", change },
      }),
    queuePgOp:
      engine === "PostgreSQL" && editable
        ? (op: PgObjectOp) =>
            addPendingStructureChange(key, {
              schema,
              table: tableName,
              change: { kind: "pg-op", op },
            })
        : null,
    removePending: (id: string) => removePendingStructureChange(key, id),
    retry: () => {
      if (connectionId && schema && tableName) {
        void loadTableStructure(connectionId, schema, tableName);
      }
    },
    commit,
  };
}

async function runDestructiveCommit<T>(
  changes: ColumnChangeKind[],
  commit: () => Promise<T>,
): Promise<T | undefined> {
  if (changes.length === 0) return undefined;
  const { destructive } = classifyDestructive(changes);
  if (destructive.length > 0) {
    const summary = destructive
      .map((change) => describeChange(change))
      .join("\n");
    const ok = await requestConfirm({
      title: "Apply destructive schema changes?",
      message: "These changes are destructive and may lose data.",
      detail: summary,
      confirmLabel: "Apply changes",
      danger: true,
    });
    if (!ok) return undefined;
  }
  return commit();
}
