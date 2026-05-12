import { pickRowIdentity } from "@/lib/row-identity";
import type { TableEditsCommitStatus, TableStructure } from "@/lib/store";

interface UseTableCapabilitiesArgs {
  structure: TableStructure | undefined;
  commitStatus: TableEditsCommitStatus | undefined;
  selectedCount: number;
}

export interface TableCapabilities {
  isReadOnly: boolean;
  isWriting: boolean;
  structureLoaded: boolean;
  canAddRow: boolean;
  canDeleteSelected: boolean;
  canEditCells: boolean;
}

// Mutation gates come from the per-table capability flags rather than
// an engine-name literal — a CH MergeTree table is editable and a CH
// Distributed/View table is not, with the same shape of code.
export function useTableCapabilities({
  structure,
  commitStatus,
  selectedCount,
}: UseTableCapabilitiesArgs): TableCapabilities {
  const structureLoaded = Boolean(structure);
  const isReadOnly = pickRowIdentity(structure) === null;
  const isWriting =
    commitStatus?.state === "running" || commitStatus?.state === "queued";

  const caps = structure?.capabilities;
  const canInsertRows = caps?.canInsertRows ?? false;
  const canUpdateRows = caps?.canUpdateRows ?? false;
  const canDeleteRows = caps?.canDeleteRows ?? false;

  return {
    isReadOnly,
    isWriting,
    structureLoaded,
    canAddRow: structureLoaded && canInsertRows && !isWriting,
    canDeleteSelected:
      selectedCount > 0 && canDeleteRows && !isReadOnly && !isWriting,
    canEditCells: canUpdateRows && !isReadOnly && !isWriting,
  };
}
