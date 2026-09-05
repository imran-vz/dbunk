import type {
  PgTransferColumnMapping,
  PgTransferInspection,
} from "@/lib/pg-transfer/client";

const selectClass =
  "h-(--control-h) min-w-36 rounded-sm border border-border-strong bg-surface-input px-2 text-xs outline-none focus-visible:ring-1 focus-visible:ring-accent";

export function PgCsvImportReview({
  inspection,
  mapping,
  validation,
  disabled,
  onChange,
}: {
  inspection: PgTransferInspection;
  mapping: PgTransferColumnMapping[];
  validation: string | null;
  disabled: boolean;
  onChange: (next: PgTransferColumnMapping[]) => void;
}) {
  const targetColumns = inspection.targetColumns.filter(
    (column) => !column.generated && !column.identity,
  );
  const mappingBySource = new Map(
    mapping.map((entry) => [entry.sourceIndex, entry.targetColumn]),
  );
  const setTarget = (sourceIndex: number, targetColumn: string) => {
    onChange(
      inspection.sourceColumns.map((source) => ({
        sourceIndex: source.index,
        targetColumn:
          source.index === sourceIndex
            ? targetColumn
            : (mappingBySource.get(source.index) ?? ""),
      })),
    );
  };

  return (
    <div className="mt-4">
      <div className="mb-1 flex items-center gap-3 text-xs">
        <h3 className="font-semibold">Column mapping</h3>
        <span className="ml-auto text-2xs text-text-muted">
          Source positions remain distinct when headers are blank or duplicated
        </span>
      </div>
      <div className="max-h-64 overflow-auto border border-border-subtle">
        <table className="w-full text-left text-xs">
          <thead className="sticky top-0 bg-surface-sidebar text-2xs text-text-muted">
            <tr>
              <th className="px-2 py-1 font-normal">#</th>
              <th className="px-2 py-1 font-normal">CSV column</th>
              <th className="px-2 py-1 font-normal">Table column</th>
            </tr>
          </thead>
          <tbody>
            {inspection.sourceColumns.map((source) => (
              <tr key={source.index} className="border-t border-border-subtle">
                <td className="px-2 py-1 text-text-muted">
                  {source.index + 1}
                </td>
                <td className="max-w-48 truncate px-2 py-1 font-mono">
                  {source.name || (
                    <span className="text-text-muted">blank</span>
                  )}
                </td>
                <td className="px-2 py-1">
                  <select
                    aria-label={`Map source column ${source.index + 1}`}
                    className={selectClass}
                    disabled={disabled}
                    value={mappingBySource.get(source.index) ?? ""}
                    onChange={(event) =>
                      setTarget(source.index, event.target.value)
                    }
                  >
                    <option value="">Skip column</option>
                    {targetColumns.map((target) => (
                      <option key={target.name} value={target.name}>
                        {target.name} · {target.dataType}
                      </option>
                    ))}
                  </select>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p
        role={validation ? "alert" : undefined}
        className={`mt-2 text-2xs ${validation ? "text-danger" : "text-text-muted"}`}
      >
        {validation ?? "Unmapped columns use database defaults."}
      </p>
    </div>
  );
}

export function PgCsvSample({
  inspection,
}: {
  inspection: PgTransferInspection;
}) {
  if (!inspection.sourceColumns.length) return null;
  const occurrences = new Map<string, number>();
  const rows = inspection.sampleRows.map((row) => {
    const content = JSON.stringify(row);
    const occurrence = (occurrences.get(content) ?? 0) + 1;
    occurrences.set(content, occurrence);
    return { row, key: `${content}:${occurrence}` };
  });
  return (
    <section className="mt-4">
      <div className="mb-1 flex items-center gap-3 text-xs">
        <h3 className="font-semibold">Preview</h3>
        <span className="ml-auto text-2xs text-text-muted">
          {inspection.sampleRows.length} sampled record
          {inspection.sampleRows.length === 1 ? "" : "s"}
          {inspection.sampleTruncated ? " · preview truncated" : ""}
        </span>
      </div>
      <div className="max-h-48 overflow-auto border border-border-subtle">
        <table className="min-w-full text-left text-xs">
          <thead className="sticky top-0 bg-surface-sidebar text-2xs text-text-muted">
            <tr>
              {inspection.sourceColumns.map((source) => (
                <th
                  className="max-w-48 truncate px-2 py-1 font-normal"
                  key={source.index}
                >
                  {source.name || `Column ${source.index + 1}`}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map(({ row, key }) => (
              <tr key={key} className="border-t border-border-subtle">
                {inspection.sourceColumns.map((source) => (
                  <td
                    className="max-w-48 truncate px-2 py-1 font-mono"
                    key={source.index}
                  >
                    {row[source.index] === null ? (
                      <span className="text-text-muted">NULL</span>
                    ) : (
                      (row[source.index] ?? "")
                    )}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="mt-1 text-2xs text-text-muted">
        Total rows are unknown. Preview reads at most 256 KiB and 50 records.
      </p>
    </section>
  );
}
