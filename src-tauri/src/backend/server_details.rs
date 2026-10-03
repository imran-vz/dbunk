//! Explicit read-only reader-session facts, settings and extensions.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_catalog::server_details;
pub use server_details::{
    ReaderContext, ServerDetailsSnapshot, ServerExtension, ServerFacts, ServerLimit, ServerRows,
    ServerSection, ServerSetting, ServerText, MAX_SERVER_DETAILS_BYTES, MAX_SERVER_EXTENSIONS,
    MAX_SERVER_IDENTITY_BYTES, MAX_SERVER_SETTINGS, MAX_SERVER_TEXT_BYTES,
};

impl Backend {
    pub async fn server_details(
        &self,
        document: &DataDocument,
    ) -> Result<ServerDetailsSnapshot, DataError> {
        self.object_read(document, |spec, drivers, cancellation| async move {
            server_details::read(&spec, &drivers, cancellation).await
        })
        .await
    }
}

#[cfg(test)]
mod live_tests;
