use super::*;
use std::fmt::Write;
const MAX_SQL_BYTES: usize = 512 * 1024;
/// The always-NULL whole-row expression preserves the actual FROM row type
/// without returning or detoasting a composite payload. OFFSET 0 keeps canonical
/// type output stable between the guard and returned fields (ADR-0029).
pub(super) fn capture(
    request: &TableExportRequest,
    columns: &[TableExportColumn],
) -> Result<String, CatalogError> {
    request.validate()?;
    // Bound every input before building scratch strings. The independent final
    // SQL cap can refuse a wide projection without growing with server data.
    if columns.len() > MAX_TABLE_EXPORT_COLUMNS
        || columns.iter().any(|column| !bounds::name(&column.name))
    {
        return Err(CatalogError::TableExportLimit);
    }
    let mut raw =
        String::from("SELECT CASE WHEN false THEN t ELSE NULL END AS whole, TRUE AS present");
    let mut checks = Vec::with_capacity(columns.len());
    let mut lengths = Vec::with_capacity(columns.len());
    let mut projection = String::from("SELECT pg_catalog.pg_typeof(g.whole)::oid AS witness, g.present, COALESCE(g.oversized,FALSE) AS oversized");
    for (index, column) in columns.iter().enumerate() {
        let name = crate::quote_double(&column.name);
        write!(raw, ", CASE WHEN t.{name} IS NOT DISTINCT FROM NULL THEN NULL::text ELSE pg_catalog.format('%s',t.{name}) END AS c{index}").unwrap();
        let length = format!(
            "COALESCE(pg_catalog.octet_length(pg_catalog.convert_to(s.c{index},'UTF8')),0)::bigint"
        );
        checks.push(format!("({length}>{MAX_TABLE_EXPORT_FIELD_BYTES})"));
        lengths.push(length);
        write!(
            projection,
            ", CASE WHEN g.oversized THEN NULL::text ELSE g.c{index} END AS c{index}"
        )
        .unwrap();
    }
    write!(
        raw,
        " FROM {}.{} AS t LIMIT {} OFFSET 0",
        crate::quote_double(&request.schema),
        crate::quote_double(&request.table),
        bounds::row_limit(columns.len()) + 1
    )
    .unwrap();
    let guard = if columns.is_empty() {
        "FALSE".into()
    } else {
        format!(
            "({}) OR (({})>{MAX_TABLE_EXPORT_TEXT_BYTES})",
            checks.join(" OR "),
            lengths.join("+")
        )
    };
    let sql = format!("{projection} FROM (SELECT s.*,({guard}) AS oversized FROM ({raw}) AS s OFFSET 0) AS g RIGHT JOIN (VALUES(1)) AS empty_source(dummy) ON TRUE");
    if sql.len() > MAX_SQL_BYTES {
        return Err(CatalogError::TableExportLimit);
    }
    Ok(sql)
}
