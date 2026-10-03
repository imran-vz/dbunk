use super::*;
use crate::{quote_double, quote_literal};
pub(super) fn render(
    target: &TableDdlDescription,
    intent: &TableDdlIntent,
    timeout: Option<u32>,
) -> Result<TableDdlPreview, TableDdlError> {
    target
        .checked_heap_bytes()
        .ok_or(TableDdlError::InvalidTarget)?;
    intent
        .checked_heap_bytes()
        .ok_or(TableDdlError::InvalidTarget)?;
    // Target+intent also form the receipt. Reserve escaped 256-byte routing,
    // attempt/outcome fields and SQLSTATE before any executable review exists.
    let receipt_bound = target
        .encoded_bytes()
        .saturating_add(super::super::schema_ddl::encoded_bytes(intent))
        .saturating_add(256 * 6 + 1024);
    if receipt_bound > MAX_TABLE_DDL_RECEIPT_BYTES {
        return Err(TableDdlError::InvalidTarget);
    }
    let qualified = format!(
        "{}.{}",
        quote_double(&target.schema),
        quote_double(&target.table)
    );
    // Four fixed typed forms, equivalent to object_ddl's renderers. Its legacy
    // trim-based validation rejects legal quoted whitespace identifiers; this
    // boundary validates byte length/NUL without changing the spelling.
    let (sql, summary) = match intent {
        TableDdlIntent::SetComment { comment } => {
            let identity = match &target.column {
                Some(column) => format!("COLUMN {qualified}.{}", quote_double(&column.name)),
                None => format!("TABLE {qualified}"),
            };
            (
                format!(
                    "COMMENT ON {identity} IS {};",
                    comment
                        .as_deref()
                        .map(quote_literal)
                        .unwrap_or_else(|| "NULL".into())
                ),
                format!(
                    "{} comment on {identity}",
                    if comment.as_ref().is_none_or(|text| text.is_empty()) {
                        "Clear"
                    } else {
                        "Set"
                    }
                ),
            )
        }
        TableDdlIntent::Rename { new_name } => match &target.column {
            Some(column) => (
                format!(
                    "ALTER TABLE {qualified} RENAME COLUMN {} TO {};",
                    quote_double(&column.name),
                    quote_double(new_name)
                ),
                format!(
                    "Rename column {} on {qualified} to {}",
                    quote_double(&column.name),
                    quote_double(new_name)
                ),
            ),
            None => (
                format!(
                    "ALTER TABLE {qualified} RENAME TO {};",
                    quote_double(new_name)
                ),
                format!("Rename table {qualified} to {}", quote_double(new_name)),
            ),
        },
    };
    let preview = TableDdlPreview {
        sql: sql.into_boxed_str().into_string(),
        summary: summary.into_boxed_str().into_string(),
        statement_timeout_ms: timeout,
        operation_timeout_ms: TABLE_DDL_OPERATION_TIMEOUT_MS,
    };
    preview
        .checked_heap_bytes()
        .ok_or(TableDdlError::InvalidTarget)?;
    Ok(preview)
}
