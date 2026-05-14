# ADR-0017 — Parquet and XML format support

**Status**: Proposed (2026-05-14)

## Context

Export already covers CSV / JSON / SQL INSERTs / HTML / Markdown /
TXT / XLSX (see commit `48c39a8` and `src/lib/export.ts`). The two
remaining DBeaver-parity formats are **Parquet** (columnar binary)
and **XML** (text). They have wildly different implementation costs.

**Parquet** requires a compliant writer:

- Pure-JS writers exist (`parquetjs`, `hyparquet`) but support a
  subset of types and don't stream well for large tables.
- The Rust ecosystem has `parquet` + `arrow-rs` which write to a
  streaming `File` target with full type coverage and Snappy/Zstd
  compression.

XLSX taught us a lesson: shipping a hand-rolled binary format
(`toXlsx` in `src/lib/export.ts`) costs maintenance churn (no
shared-strings table, single sheet, fragile CRC math). Parquet is
strictly less hand-rollable than XLSX, so a real writer is needed.

**XML** is small but design-heavy: there's no single canonical
schema mapping. DBeaver emits `<table><row><col name="x">v</col>...`;
JetBrains emits `<row x="v" .../>`; some users want XSD-driven.
Without a target, an XML implementation invites complaints.

ROADMAP.md §6 marks both `❌`. ROADMAP.md §7 also asks for XML
import (XML → table).

## Decision

### Parquet — Rust-side streaming

- Implement export entirely in Rust. New command:
  `export_table_to_parquet(connection_id, schema, table, file_path,
  options: { compression: "snappy" | "zstd" | "uncompressed" })`.
- Paginate `load_table_data`-style 1000 rows at a time; convert each
  batch to Arrow `RecordBatch`; write through `ArrowWriter` to the
  output `File`. Schema inference uses the existing column-type
  classifier in `dispatch::relational`.
- Renderer triggers a save dialog (Tauri dialog plugin) and shows a
  progress indicator while Rust writes. No bytes cross the IPC
  boundary — the file lands on disk directly.
- Import: out of scope for v1. Schema-match preview is its own
  follow-up ADR.

### XML — TypeScript, DBeaver-shaped

- Pick the DBeaver shape as the canonical wire format:
  ```xml
  <data table="public.users">
    <row>
      <col name="id">1</col>
      <col name="email" null="true"/>
    </row>
  </data>
  ```
- Implement export in `src/lib/export.ts` next to the existing
  `toHtml`/`toMarkdown` helpers. No new dependency — DOM strings.
- Implement import as a streaming SAX parser using the
  `htmlparser2` package (already a transitive dep). New
  `parseXmlSheet(input)` returns `{ columns, rows }` matching the
  CSV / XLSX shape so the import wizard can reuse its mapping UI.

### Configurable column null marker

Already implemented for CSV / SQL exports. XML uses `null="true"`
attribute (not a value) which is a different mechanism — document
the asymmetry in the export options dialog.

## Consequences

- Parquet adds an `arrow`/`parquet` dependency to `src-tauri`,
  ~3 MB compressed when statically linked. Acceptable; it's the
  smallest reasonable Rust Parquet writer.
- Parquet ships *write* but not *read* in v1. Users importing
  Parquet are routed to the docs.
- XML round-trips cleanly between dbunk-export and dbunk-import.
  Compatibility with other tools is a "supported but not
  guaranteed" contract — the wire format is documented in the
  export dialog's tooltip.
- `src/lib/export.ts` grows two functions (`toXml`, `fromXml`). The
  in-renderer write surface doesn't change.
- Whole-table XML export reuses the existing pagination loop in
  `table-editor-panel.tsx` (commit `48c39a8`).

## Alternatives considered

1. **Parquet in TypeScript via `parquetjs`.** Rejected — type
   coverage gaps for `numeric`, `interval`, `jsonb`; non-streaming
   memory profile breaks on tables over ~50k rows.
2. **Bundle DuckDB and use its Parquet writer.** Rejected — DuckDB
   pulls in a full SQL engine for a write-only need; ~30 MB.
3. **XML in Rust.** Rejected — XML is text, no streaming benefit,
   and keeping the export path in TypeScript matches CSV / SQL /
   HTML / Markdown / TXT (all TS today).
4. **Avro / Iceberg / ORC.** Adjacent formats; not requested by
   ROADMAP. Defer.

## Open questions

- Parquet schema inference for `jsonb`: write as Parquet `BYTE_ARRAY`
  with UTF-8 logical type, or as Parquet `STRUCT`? v1: `BYTE_ARRAY`
  to match what every existing reader expects.
- XML attribute vs. element representation of NULL: chose `null="true"`
  attribute. Pre-empt the bikeshed.
- Compression default for Parquet: `zstd` (better ratio) vs `snappy`
  (DBeaver / Spark default). Pick `zstd` and document the difference.

## Related

- ROADMAP.md §6 and §7 — gaps this closes.
- `src/lib/export.ts` — the TS-side XML/Parquet entry points slot
  next to existing format helpers.
- ADR-0008 — storage-class fork. Both new formats live in the
  relational export path; key-value is unaffected.
