import { useEffect, useMemo, useState } from "react";

import {
  type ColumnChangeKind,
  classifyDestructive,
  generateDdlForEngine,
  type PendingChange,
} from "@/lib/ddl";
import { type RelationalPolicy, relationalPolicy } from "@/lib/engine-policy";
import {
  type ColumnInfo,
  type ConstraintInfo,
  type DatabaseEngine,
  type DDLOutcome,
  type ForeignKeyInfo,
  type IndexInfo,
  type StructureCapabilities,
  type StructureCommitStatus,
  type TableStructure,
  type TableStructureStatus,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";

import { describeChange } from "./shared";

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
  const pending = pendingChanges ?? [];
  const policy = relationalPolicy(engine ?? "PostgreSQL");
  const isLoading = status?.state === "loading";
  const errorMessage = status?.state === "error" ? status.error : null;

  const previewSql = useMemo(
    () =>
      generateDdlForEngine(
        engine ?? "PostgreSQL",
        schema,
        tableName,
        pending.map((entry) => entry.change),
        snapshot.columns,
      ),
    [engine, schema, tableName, pending, snapshot.columns],
  );

  const commit = async () => {
    const outcome = await runDestructiveCommit(pending, () =>
      commitStructureChanges(key),
    );
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
      addPendingStructureChange(key, { schema, table: tableName, change }),
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
  pending: PendingChange[],
  commit: () => Promise<T>,
): Promise<T | undefined> {
  if (pending.length === 0) return undefined;
  const { destructive } = classifyDestructive(
    pending.map((entry) => entry.change),
  );
  if (destructive.length > 0) {
    const summary = destructive
      .map((change) => describeChange(change))
      .join("\n");
    const ok = window.confirm(
      `These changes are destructive and may lose data:\n\n${summary}\n\nProceed?`,
    );
    if (!ok) return undefined;
  }
  return commit();
}
