import { pickRowIdentity } from "@/lib/row-identity";
import type {
  TableDataState,
  TableEditsCommitStatus,
  TableLoadStatus,
  TableRef,
  TableSessionCapabilities,
  TableSessionSnapshot,
  TableStructure,
  TableStructureStatus,
} from "@/lib/store/types";
import { tableSessionKey, tableStructureKey } from "@/lib/store/types";
import {
  type BrowseIdentityKind,
  identityIsEditable,
} from "@/lib/table-browse";

export const EMPTY_TABLE_SESSION_CAPABILITIES: TableSessionCapabilities = {
  structureLoaded: false,
  isReadOnly: true,
  isWriting: false,
  canAddRow: false,
  canDeleteRows: false,
  canEditCells: false,
};

export const deriveTableSessionCapabilities = (
  structure: TableStructure | undefined,
  writeStatus: TableEditsCommitStatus | undefined,
  browseIdentityKind?: BrowseIdentityKind,
): TableSessionCapabilities => {
  const structureLoaded = Boolean(structure);
  const isWriting =
    writeStatus?.state === "running" || writeStatus?.state === "queued";
  const isReadOnly = browseIdentityKind
    ? !identityIsEditable(browseIdentityKind)
    : pickRowIdentity(structure) === null;
  const caps = structure?.capabilities;

  if (!caps) return EMPTY_TABLE_SESSION_CAPABILITIES;

  return {
    structureLoaded,
    isReadOnly,
    isWriting,
    canAddRow: structureLoaded && caps.canInsertRows && !isWriting,
    canDeleteRows: caps.canDeleteRows && !isReadOnly && !isWriting,
    canEditCells: caps.canUpdateRows && !isReadOnly && !isWriting,
  };
};

export type SelectedTableSessionCapabilities = TableSessionCapabilities & {
  canDeleteSelected: boolean;
};

export const deriveSelectedTableSessionCapabilities = (
  capabilities: TableSessionCapabilities | undefined,
  selectedCount: number,
): SelectedTableSessionCapabilities => {
  const caps = capabilities ?? EMPTY_TABLE_SESSION_CAPABILITIES;
  return {
    ...caps,
    canDeleteSelected: selectedCount > 0 && caps.canDeleteRows,
  };
};

export const tableSessionStructureKey = (ref: TableRef) =>
  tableStructureKey(ref.connectionId, ref.schema, ref.table);

export type TableRefResolution =
  | { ok: true; ref: TableRef }
  | { ok: false; reason: string; missing: boolean };

export const resolveTableRefByName = (
  tableData: Record<
    string,
    Pick<TableDataState, "connectionId" | "schema" | "table">
  >,
  tableName: string,
): TableRefResolution => {
  const matches = Object.values(tableData).filter(
    (entry) => entry.table === tableName,
  );
  if (matches.length === 0) {
    return {
      ok: false,
      missing: true,
      reason: `Table data for ${tableName} is not loaded.`,
    };
  }
  if (matches.length > 1) {
    return {
      ok: false,
      missing: false,
      reason: `Ambiguous table name "${tableName}"; use a Table Session action with connectionId, schema, and table.`,
    };
  }
  const [data] = matches;
  return {
    ok: true,
    ref: {
      connectionId: data.connectionId,
      schema: data.schema,
      table: data.table,
    },
  };
};

export const buildTableSessionSnapshot = ({
  ref,
  data,
  structure,
  loadStatus,
  structureStatus,
  writeStatus,
  edits,
  browseIdentityKind,
}: {
  ref: TableRef;
  data: TableDataState | undefined;
  structure: TableStructure | undefined;
  loadStatus: TableLoadStatus | undefined;
  structureStatus: TableStructureStatus | undefined;
  writeStatus: TableEditsCommitStatus | undefined;
  edits: Record<number, Record<number, string>> | undefined;
  browseIdentityKind?: BrowseIdentityKind;
}): TableSessionSnapshot => ({
  ref,
  key: tableSessionKey(ref),
  data,
  structure,
  loadStatus,
  structureStatus,
  writeStatus,
  edits: edits ?? {},
  capabilities: deriveTableSessionCapabilities(
    structure,
    writeStatus,
    browseIdentityKind,
  ),
});
