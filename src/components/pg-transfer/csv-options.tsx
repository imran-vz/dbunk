import type {
  PgCsvOptions,
  PgTransferDirection,
} from "@/lib/pg-transfer/client";

const inputClass =
  "h-(--control-h) rounded-sm border border-border-strong bg-surface-input px-2 text-xs outline-none focus-visible:ring-1 focus-visible:ring-accent";

export function CsvOptionsFields({
  direction,
  options,
  disabled,
  onChange,
}: {
  direction: PgTransferDirection;
  options: PgCsvOptions;
  disabled: boolean;
  onChange: (next: PgCsvOptions) => void;
}) {
  return (
    <fieldset disabled={disabled} className="space-y-3">
      <div className="grid grid-cols-[6rem_minmax(0,1fr)] items-start gap-3 text-xs">
        <span className="pt-1 text-text-secondary">CSV options</span>
        <div className="grid grid-cols-2 gap-2 xl:grid-cols-4">
          <label className="min-w-0">
            <span className="mb-1 block text-2xs text-text-muted">
              Delimiter
            </span>
            <select
              aria-label="CSV delimiter"
              className={`${inputClass} w-full`}
              value={options.delimiter}
              onChange={(event) =>
                onChange({ ...options, delimiter: event.target.value })
              }
            >
              <option value=",">Comma (,)</option>
              <option value=";">Semicolon (;)</option>
              <option value="\t">Tab</option>
              <option value="|">Pipe (|)</option>
            </select>
          </label>
          <label className="min-w-0">
            <span className="mb-1 block text-2xs text-text-muted">Quote</span>
            <input
              aria-label="CSV quote"
              className={`${inputClass} w-full font-mono`}
              value={options.quote}
              maxLength={1}
              onChange={(event) =>
                onChange({ ...options, quote: event.target.value })
              }
            />
          </label>
          <label className="min-w-0">
            <span className="mb-1 block text-2xs text-text-muted">Escape</span>
            <input
              aria-label="CSV escape"
              className={`${inputClass} w-full font-mono`}
              value={options.escape}
              maxLength={1}
              onChange={(event) =>
                onChange({ ...options, escape: event.target.value })
              }
            />
          </label>
          <label className="min-w-0">
            <span className="mb-1 block text-2xs text-text-muted">
              NULL token
            </span>
            <input
              aria-label="CSV NULL token"
              className={`${inputClass} w-full font-mono`}
              value={options.nullToken}
              maxLength={64}
              onChange={(event) =>
                onChange({ ...options, nullToken: event.target.value })
              }
            />
          </label>
        </div>
      </div>
      <label className="grid grid-cols-[6rem_minmax(0,1fr)] items-start gap-3 text-xs">
        <span className="text-text-secondary">Header</span>
        <span className="flex items-center gap-2">
          <input
            aria-label={
              direction === "import"
                ? "First row contains column names"
                : "Include column names"
            }
            type="checkbox"
            checked={options.header}
            onChange={(event) =>
              onChange({ ...options, header: event.target.checked })
            }
          />
          {direction === "import"
            ? "First row contains column names"
            : "Include column names"}
        </span>
      </label>
      <div className="grid grid-cols-[6rem_minmax(0,1fr)] gap-3 text-xs">
        <span className="text-text-secondary">Encoding</span>
        <span>UTF-8</span>
      </div>
    </fieldset>
  );
}
