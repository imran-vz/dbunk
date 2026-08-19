import type { TableBrowseLoadStatus } from "@/lib/store/types";
import type {
  BrowseCount,
  BrowseExactCountResult,
  BrowseFilter,
  BrowseInspection,
  BrowsePageInfo,
  BrowseSortKey,
  TableBrowseError,
  TableBrowseFilterMode,
  TableBrowseHistoryEntry,
  TableBrowsePreset,
} from "@/lib/table-browse";

export type ServerBrowseGridModel = {
  typedFilters: BrowseFilter[];
  rawFilterText: string;
  filterMode: TableBrowseFilterMode;
  sort: BrowseSortKey[];
  pageSize: number;
  loadStatus: TableBrowseLoadStatus;
  error: TableBrowseError | null;
  inspection: BrowseInspection | null;
  omittedRows: number;
  truncatedCells: number;
  count: BrowseCount;
  exactCount: BrowseExactCountResult | null;
  countStatus: TableBrowseLoadStatus;
  pageInfo: BrowsePageInfo | null;
  history: TableBrowseHistoryEntry[];
  presets: TableBrowsePreset[];
  onApplyTypedFilter: (filter: BrowseFilter) => void;
  onRemoveTypedFilter: (column: string) => void;
  onClearTypedFilters: () => void;
  onRawFilterApply: (text: string) => void;
  onFilterModeChange: (mode: TableBrowseFilterMode) => void;
  onSortChange: (sort: BrowseSortKey[]) => void;
  onPageSizeChange: (pageSize: number) => void;
  onHeaderSort: (column: string, append: boolean) => void;
  onCountRows: () => void;
  onCancel: () => void;
  onApplyPreset: (name: string) => void;
  onSavePreset: (name: string) => void;
  onApplyHistory: (index: number) => void;
};
