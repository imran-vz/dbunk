//! Native-only, profile-local map state. Legacy schema-map tables are untouched;
//! importing their name-keyed positions/preferences belongs to Plan030.
//! Every save/reset compares an observed scoped generation inside one SQLite
//! transaction. Reset retains a tombstone generation, preventing absence ABA.
use crate::backend::{
    data::{DataDocument, DataError},
    Backend,
};
mod storage;
mod types;
pub use types::*;
#[cfg(test)]
mod tests;

fn access(error: DataError) -> SchemaMapPreferencesError {
    match error {
        DataError::Document(_) => SchemaMapPreferencesError::Document,
        _ => SchemaMapPreferencesError::Unavailable,
    }
}
impl Backend {
    pub async fn load_schema_map_preferences(
        &self,
        document: &DataDocument,
        scope: SchemaMapPreferenceScope,
    ) -> Result<SchemaMapPreferencesCapture, SchemaMapPreferencesError> {
        scope
            .checked_heap_bytes()
            .ok_or(SchemaMapPreferencesError::Invalid)?;
        self.data_call(document, move |state, document, _admission| async move {
            Ok(storage::load(&state.pool, document.connection_id(), scope).await)
        })
        .await
        .map_err(access)?
    }
    /// The returned capture is the exact committed acknowledgement. The caller
    /// must not publish a changed preference/position as persisted before it.
    pub async fn save_schema_map_preferences(
        &self,
        document: &DataDocument,
        scope: SchemaMapPreferenceScope,
        expected: SchemaMapPreferencesRevision,
        value: SchemaMapPreferences,
    ) -> Result<SchemaMapPreferencesCapture, SchemaMapPreferencesError> {
        scope
            .checked_heap_bytes()
            .ok_or(SchemaMapPreferencesError::Invalid)?;
        value
            .checked_heap_bytes()
            .ok_or(SchemaMapPreferencesError::Invalid)?;
        self.data_call(document, move |state, document, _admission| async move {
            Ok(storage::save(
                &state.pool,
                document.connection_id(),
                scope,
                expected,
                Some(value),
            )
            .await)
        })
        .await
        .map_err(access)?
    }
    /// Clears only this exact scope; it cannot repair or replace a corrupt or
    /// future record, and cannot reuse an earlier observed absent generation.
    pub async fn reset_schema_map_preferences(
        &self,
        document: &DataDocument,
        scope: SchemaMapPreferenceScope,
        expected: SchemaMapPreferencesRevision,
    ) -> Result<SchemaMapPreferencesCapture, SchemaMapPreferencesError> {
        scope
            .checked_heap_bytes()
            .ok_or(SchemaMapPreferencesError::Invalid)?;
        self.data_call(document, move |state, document, _admission| async move {
            Ok(storage::save(&state.pool, document.connection_id(), scope, expected, None).await)
        })
        .await
        .map_err(access)?
    }
}
