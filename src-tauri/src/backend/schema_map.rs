//! Complete bounded schema-map metadata on the admitted document read lane.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_schema_map;
pub mod preferences;
pub use native_schema_map::*;
impl Backend {
    /// One read-only capture. No pooled reads, graph stubs, row data or DDL.
    /// The owned task keeps its document permit through socket cancellation/join
    /// even if the host drops its waiter. Refresh can require observed OIDs.
    pub async fn schema_map(
        &self,
        document: &DataDocument,
        request: SchemaMapRequest,
    ) -> Result<SchemaMapSnapshot, DataError> {
        request.validate().map_err(DataError::Catalog)?;
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_schema_map::read(&spec, &drivers, cancellation, request).await
        })
        .await
    }
}
#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod tests;
