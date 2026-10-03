//! Bounded relation-oriented SQL artifacts. This facade has no execute path.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_ddl_export;
pub use native_ddl_export::*;
impl Backend {
    pub async fn export_ddl(
        &self,
        document: &DataDocument,
        request: DdlExportRequest,
    ) -> Result<DdlExportArtifact, DataError> {
        request.validate().map_err(DataError::Catalog)?;
        let connection = document.connection_id().to_owned();
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_ddl_export::read(&spec, &drivers, cancellation, connection, request).await
        })
        .await
    }
}

#[cfg(test)]
mod live_tests;
