use super::*;
use crate::{quote_double, quote_literal};
pub(super) fn render(
    target: &SchemaAlterDescription,
    intent: &SchemaAlterIntent,
    timeout: Option<u32>,
) -> Result<SchemaAlterPreview, SchemaAlterError> {
    target
        .checked_heap_bytes()
        .ok_or(SchemaAlterError::InvalidTarget)?;
    intent
        .checked_heap_bytes()
        .ok_or(SchemaAlterError::InvalidTarget)?;
    // Target+intent also form the receipt. Reserve escaped routing, attempt,
    // outcome fields and SQLSTATE before any executable review exists.
    let receipt_bound = target
        .encoded_bytes()
        .saturating_add(super::super::schema_ddl::encoded_bytes(intent))
        .saturating_add(256 * 6 + 1024);
    if receipt_bound > MAX_SCHEMA_ALTER_RECEIPT_BYTES {
        return Err(SchemaAlterError::InvalidTarget);
    }
    let schema = quote_double(&target.schema);
    // Two fixed typed forms, equivalent to object_ddl's schema renderers
    // without its trim-based validation of legal quoted whitespace names.
    let (sql, summary) = match intent {
        SchemaAlterIntent::SetComment { comment } => (
            format!(
                "COMMENT ON SCHEMA {schema} IS {};",
                comment
                    .as_deref()
                    .map(quote_literal)
                    .unwrap_or_else(|| "NULL".into())
            ),
            format!(
                "{} comment on SCHEMA {schema}",
                if comment.as_ref().is_none_or(|text| text.is_empty()) {
                    "Clear"
                } else {
                    "Set"
                }
            ),
        ),
        SchemaAlterIntent::Rename { new_name } => (
            format!(
                "ALTER SCHEMA {schema} RENAME TO {};",
                quote_double(new_name)
            ),
            format!("Rename schema {schema} to {}", quote_double(new_name)),
        ),
    };
    let preview = SchemaAlterPreview {
        sql: sql.into_boxed_str().into_string(),
        summary: summary.into_boxed_str().into_string(),
        statement_timeout_ms: timeout,
        operation_timeout_ms: SCHEMA_ALTER_OPERATION_TIMEOUT_MS,
    };
    preview
        .checked_heap_bytes()
        .ok_or(SchemaAlterError::InvalidTarget)?;
    Ok(preview)
}
