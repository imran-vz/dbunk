//! Complete bounded relation captures for non-CSV export. No execution of user
//! SQL, file publication, stored paths, or fallback to retained grid rows.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_table_export;
pub use native_table_export::*;
impl Backend {
    pub async fn capture_table_export(
        &self,
        document: &DataDocument,
        request: TableExportRequest,
    ) -> Result<TableExportCapture, DataError> {
        request.validate().map_err(DataError::Catalog)?;
        let connection = document.connection_id().to_owned();
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_table_export::read(&spec, &drivers, cancellation, connection, request).await
        })
        .await
    }
}
#[cfg(test)]
mod live_tests;
#[cfg(test)]
mod tests;
