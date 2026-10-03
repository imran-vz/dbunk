//! Complete, bounded, read-only table metadata on the caller's admitted document.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_catalog::table_structure;
pub use table_structure::*;
impl Backend {
    pub async fn table_structure(
        &self,
        document: &DataDocument,
        request: TableStructureRequest,
    ) -> Result<TableStructureSnapshot, DataError> {
        request.validate().map_err(DataError::Catalog)?;
        self.object_read(document, move |spec, drivers, cancellation| async move {
            table_structure::read(&spec, &drivers, cancellation, request).await
        })
        .await
    }
}
#[cfg(test)]
mod live_tests;
