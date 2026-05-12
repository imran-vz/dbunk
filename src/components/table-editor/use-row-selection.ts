import type { RowSelectionState } from "@tanstack/react-table";
import { useMemo, useState } from "react";

export interface RowSelection<T> {
  rowSelection: RowSelectionState;
  setRowSelection: (next: RowSelectionState) => void;
  selectedIndices: number[];
  selectedIndex: number | null;
  selectedRow: T | undefined;
  selectedCount: number;
  hasMultiple: boolean;
  clear: () => void;
}

export function useRowSelection<T>(rows: T[]): RowSelection<T> {
  const [rowSelection, setRowSelection] = useState<RowSelectionState>({});

  const selectedIndices = useMemo(
    () =>
      Object.entries(rowSelection)
        .filter(([, selected]) => selected)
        .map(([rowId]) => Number.parseInt(rowId, 10))
        .filter((n) => Number.isFinite(n)),
    [rowSelection],
  );

  const selectedCount = selectedIndices.length;
  const selectedIndex = selectedCount === 1 ? selectedIndices[0] : null;
  const selectedRow = selectedIndex !== null ? rows[selectedIndex] : rows[0];

  return {
    rowSelection,
    setRowSelection,
    selectedIndices,
    selectedIndex,
    selectedRow,
    selectedCount,
    hasMultiple: selectedCount > 1,
    clear: () => setRowSelection({}),
  };
}
