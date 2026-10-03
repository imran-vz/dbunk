//! Read-only native catalog and bounded descriptions on an admitted document.
//! No pooled reader or DDL execution is exposed here.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::dedicated::DriverJoins;
use crate::postgres::{connect_spec::ResolvedPostgresConnectSpec, native_catalog};
use std::{future::Future, time::Duration};
use tokio::sync::watch;

pub use crate::postgres::objects::{
    PgCatalogEntry, PgCatalogTruncation, PgDropDependent, PgDropImpact, PgObjectCatalog,
    PgObjectDescription, PgObjectFacts, PgObjectKind, PgObjectRef, PgSchemaObjects,
    PgTypeAttribute, PgTypeClass,
};
pub use native_catalog::dependencies::{MAX_DROP_IMPACT_BYTES, MAX_DROP_IMPACT_RESULTS};
pub use native_catalog::description::{
    MAX_DESCRIPTION_BYTES, MAX_DESCRIPTION_COMPONENTS, MAX_DESCRIPTION_TEXT_BYTES,
};
pub use native_catalog::foreign_keys::{
    ForeignKey, MAX_FOREIGN_KEYS, MAX_FOREIGN_KEY_BYTES, MAX_FOREIGN_KEY_PAIRS,
};
pub use native_catalog::{CatalogError, MAX_CATALOG_BYTES, MAX_CATALOG_NODES};

impl Backend {
    /// Read-only downstream drop impact, not a DDL preview or permission to drop.
    /// Traverses silent internal/automatic dependencies as well as reported ones.
    /// A capped walk returns `truncated`, including when its visible list is empty.
    /// Exact identity/text failures remain errors. Uses the document's owned
    /// socket, cancellation epoch, deadline and joined teardown.
    pub async fn load_object_drop_impact(
        &self,
        document: &DataDocument,
        reference: PgObjectRef,
    ) -> Result<PgDropImpact, DataError> {
        native_catalog::description::validate(&reference).map_err(DataError::Catalog)?;
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_catalog::dependencies::read(&spec, &drivers, cancellation, reference).await
        })
        .await
    }

    /// Retains baseline per-kind truncation markers. Native reads additionally
    /// refuse the whole result above 10,000 scanned schemas/objects, 8 MiB JSON,
    /// or 8 KiB in any catalog text field. Routine identity is never truncated.
    /// Dropping this future does not abandon its admitted task or socket join.
    pub async fn load_object_catalog(
        &self,
        document: &DataDocument,
    ) -> Result<PgObjectCatalog, DataError> {
        self.object_read(document, |spec, drivers, cancellation| async move {
            native_catalog::read(&spec, &drivers, cancellation, Duration::from_secs(30)).await
        })
        .await
    }

    /// Supports every baseline PgObjectKind through bounded dedicated reads.
    /// Table SQL is a limited reconstruction, not a canonical schema dump.
    /// Metadata is exact or refused: 8 KiB metadata/reference fields, 1 MiB
    /// body/definition/arguments fields, 8 MiB serialized result. Reconstructed
    /// SQL is read-only text, never dispatched for execution.
    pub async fn describe_object(
        &self,
        document: &DataDocument,
        reference: PgObjectRef,
    ) -> Result<PgObjectDescription, DataError> {
        native_catalog::description::validate(&reference).map_err(DataError::Catalog)?;
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_catalog::description::read(&spec, &drivers, cancellation, reference).await
        })
        .await
    }

    /// Full ordered composite metadata; no foreign rows or server routes are read.
    pub async fn load_foreign_keys(
        &self,
        document: &DataDocument,
        schema: String,
        table: String,
    ) -> Result<Vec<ForeignKey>, DataError> {
        native_catalog::foreign_keys::validate(&schema, &table).map_err(DataError::Catalog)?;
        self.object_read(document, move |spec, drivers, cancellation| async move {
            native_catalog::foreign_keys::read(&spec, &drivers, cancellation, schema, table).await
        })
        .await
    }

    pub(super) async fn object_read<T, F, Fut>(
        &self,
        document: &DataDocument,
        read: F,
    ) -> Result<T, DataError>
    where
        T: Send + 'static,
        F: FnOnce(ResolvedPostgresConnectSpec, DriverJoins, watch::Receiver<u64>) -> Fut
            + Send
            + 'static,
        Fut: Future<Output = Result<T, CatalogError>> + Send + 'static,
    {
        let drivers = self.0.tasks.child();
        self.data_call(document, move |state, document, admission| async move {
            // data_call verifies profile endpoint and generation before this
            // credential hydration. The document permit and cancellation epoch
            // own the subsequent socket startup after the snapshot gate opens.
            let cancellation = document.0.read_cancellation();
            let connection = crate::app::find_connection(&state, &document.0.connection)
                .await
                .map_err(|_| DataError::Catalog(CatalogError::Connection))?;
            let spec = ResolvedPostgresConnectSpec::from_connection(&connection)
                .map_err(|_| DataError::Catalog(CatalogError::UnsupportedEngine))?;
            // The dedicated driver registers with this host-owned child group.
            // The document permit remains owned through cancellation and drain.
            drop(admission);
            let result = read(spec, drivers, cancellation).await;
            if result.is_ok() {
                crate::app::touch_connection_activity(&state, &document.0.connection).await;
            }
            document.0.check_open().map_err(DataError::Document)?;
            result.map_err(DataError::Catalog)
        })
        .await
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod foreign_key_live_tests;

#[cfg(test)]
mod dependency_live_tests;

#[cfg(test)]
mod remaining_live_tests;
