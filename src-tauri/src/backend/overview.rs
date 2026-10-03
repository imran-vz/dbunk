//! Explicit bounded overview/statistics reads on the admitted document lane.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_overview;
pub use native_overview::*;
impl Backend {
    pub async fn database_overview(
        &self,
        document: &DataDocument,
    ) -> Result<DatabaseOverviewSnapshot, DataError> {
        self.object_read(document, |spec, drivers, cancellation| async move {
            native_overview::database(&spec, &drivers, cancellation).await
        })
        .await
    }
    pub async fn relation_stats(
        &self,
        document: &DataDocument,
        request: RelationStatsRequest,
    ) -> Result<RelationStatsSnapshot, DataError> {
        request.validate().map_err(DataError::Catalog)?;
        request
            .bind_document(&document.0.manager_tab)
            .map_err(DataError::Catalog)?;
        let owner = document.0.manager_tab.clone();
        let connection = document.connection_id().to_owned();
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_overview::relations(&spec, &drivers, cancellation, request, connection, owner)
                .await
        })
        .await
    }
}
#[cfg(test)]
mod live_tests;
impl Backend {
    pub async fn overview(
        &self,
        document: &DataDocument,
        request: RelationStatsRequest,
    ) -> Result<OverviewSnapshot, DataError> {
        request.validate().map_err(DataError::Catalog)?;
        request
            .bind_document(&document.0.manager_tab)
            .map_err(DataError::Catalog)?;
        let owner = document.0.manager_tab.clone();
        let connection = document.connection_id().to_owned();
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_overview::overview(&spec, &drivers, cancellation, request, connection, owner)
                .await
        })
        .await
    }
}
