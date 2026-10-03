//! Exact, bounded relation metadata for SQL completion. This does not analyze SQL.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_catalog::completion;

pub use completion::{
    CompletionColumn, CompletionColumns, CompletionRelationKind, MAX_COMPLETION_BYTES,
    MAX_COMPLETION_COLUMNS, MAX_COMPLETION_NAME_BYTES, MAX_COMPLETION_TYPE_BYTES,
};

impl Backend {
    /// Names are separate exact identifiers. Missing relations and capped reads
    /// are errors, distinct from a valid relation with no user columns.
    pub async fn completion_columns(
        &self,
        document: &DataDocument,
        schema: String,
        relation: String,
    ) -> Result<CompletionColumns, DataError> {
        completion::validate(&schema, &relation).map_err(DataError::Catalog)?;
        self.object_read(document, move |spec, drivers, cancellation| async move {
            completion::read(&spec, &drivers, cancellation, schema, relation).await
        })
        .await
    }
}

#[cfg(test)]
mod live_tests;
