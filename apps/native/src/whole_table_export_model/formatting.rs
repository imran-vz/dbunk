use super::*;
use crate::result_export::{
    self, Completeness, Encoding, ExportTable, Format, Options, Scope, SqlTarget,
};
use dbunk_lib::backend::{result_files as files, table_export::TableExportData};
/// Caller admits FILE_WORK_BYTES and holds the complete capture lease until the
/// joined file job finishes. Public metadata is checked before borrowed views.
pub fn prepare(
    data: &TableExportData,
    options: &ExportOptions,
    cancel: &files::Cancellation,
) -> Result<files::PreparedFile, String> {
    if cancel.is_cancelled() {
        return Err("Export cancelled".into());
    }
    data.checked_heap_bytes()
        .ok_or("Invalid or oversized complete table capture")?;
    if route(options)? {
        return Err("CSV requires the streaming transfer tool".into());
    }
    let columns = data
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();
    let rows = data.rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let scope = files::Source {
        completeness: files::Completeness::Complete,
        scope: files::Scope::CompleteResult,
        row_count: data.rows.len(),
    };
    let compression = if options.compression == ExportCompression::Gzip {
        files::Compression::Gzip
    } else {
        files::Compression::None
    };
    if options.format == ExportFormat::Xlsx {
        return files::prepare_xlsx(
            &files::XlsxTable {
                columns: &columns,
                rows: &rows,
                source_columns: None,
                sheet_name: "Export",
                null_as: &options.null_token,
                source: scope,
            },
            compression,
            cancel,
        )
        .map_err(|e| e.to_string());
    }
    let format = match options.format {
        ExportFormat::Json => Format::Json,
        ExportFormat::Sql => Format::Sql,
        ExportFormat::Html => Format::Html,
        ExportFormat::Markdown => Format::Markdown,
        ExportFormat::Txt => Format::Txt,
        _ => unreachable!("handled above"),
    };
    let table = ExportTable {
        columns: &columns,
        rows: &rows,
        source_columns: None,
        completeness: Completeness::Complete,
    };
    let mut formatting = Options::new(format);
    formatting.scope = Scope::CompleteResult;
    formatting.encoding = if options.encoding == ExportEncoding::Utf8 {
        Encoding::Utf8
    } else {
        Encoding::Utf16Le
    };
    formatting.null_as = &options.null_token;
    formatting.sql_target = Some(SqlTarget {
        schema: Some(&data.schema),
        table: &data.table,
    });
    let export = result_export::export(&table, &formatting).map_err(|e| e.to_string())?;
    files::prepare_bytes(export.bytes, compression, scope, cancel).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::table_export::{
        TableExportColumn, TableExportIdentity, TableExportKind,
    };
    fn data() -> TableExportData {
        TableExportData {
            connection_id: "connection".into(),
            database: "db".into(),
            schema: "schema.with.dot".into(),
            table: "table\"name".into(),
            identity: TableExportIdentity {
                database_oid: 1,
                relation_oid: 2,
            },
            schema_oid: 3,
            kind: TableExportKind::Table,
            row_security: false,
            captured_start: "2026-10-03T00:00:00Z".into(),
            captured_end: "2026-10-03T00:00:01Z".into(),
            columns: vec![TableExportColumn {
                name: "n".into(),
                attnum: 1,
                type_oid: 25,
                type_modifier: -1,
                collation_oid: 0,
            }],
            rows: vec![
                vec![Some("9223372036854775807".into())],
                vec![None],
                vec![Some("NULL".into())],
                vec![Some("雪".into())],
            ],
        }
    }
    #[test]
    fn complete_source_all_six_formats_and_exact_json_sql_values() {
        let data = data();
        let cancel = files::Cancellation::default();
        for format in [
            ExportFormat::Json,
            ExportFormat::Sql,
            ExportFormat::Html,
            ExportFormat::Markdown,
            ExportFormat::Txt,
            ExportFormat::Xlsx,
        ] {
            let options = ExportOptions {
                format,
                null_token: "NULL".into(),
                ..Default::default()
            };
            let output = prepare(&data, &options, &cancel).unwrap();
            assert_eq!(output.source().row_count, 4);
            assert_eq!(output.source().scope, files::Scope::CompleteResult);
            assert!(!output.is_empty());
            if format == ExportFormat::Json {
                let parsed: serde_json::Value = serde_json::from_slice(output.bytes()).unwrap();
                assert_eq!(parsed[0]["n"], "9223372036854775807");
                assert!(parsed[1]["n"].is_null());
                assert_eq!(parsed[2]["n"], "NULL");
            }
            if format == ExportFormat::Sql {
                let text = std::str::from_utf8(output.bytes()).unwrap();
                assert!(text.contains("\"schema.with.dot\".\"table\"\"name\""));
                assert!(text.contains("'9223372036854775807'"));
                assert!(text.contains("(NULL)"));
                assert!(text.contains("('NULL')"));
            }
        }
    }
    #[test]
    fn complete_empty_table_and_cancellation_remain_truthful() {
        let mut data = data();
        data.rows.clear();
        let options = ExportOptions {
            format: ExportFormat::Json,
            ..Default::default()
        };
        let cancel = files::Cancellation::default();
        assert_eq!(prepare(&data, &options, &cancel).unwrap().bytes(), b"[]");
        cancel.cancel();
        assert!(prepare(&data, &options, &cancel).is_err());
    }
    #[test]
    fn malformed_rows_and_csv_cannot_bypass_capture_or_stream_contract() {
        let mut data = data();
        assert!(
            prepare(
                &data,
                &ExportOptions::default(),
                &files::Cancellation::default()
            )
            .is_err()
        );
        data.rows[0].push(None);
        assert!(
            prepare(
                &data,
                &ExportOptions {
                    format: ExportFormat::Json,
                    ..Default::default()
                },
                &files::Cancellation::default()
            )
            .is_err()
        );
    }
}
