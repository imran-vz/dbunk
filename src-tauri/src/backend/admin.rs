//! Bounded administration observations and consumed, policy-gated signal reviews.
use super::{
    data::{DataDocument, DataError},
    Backend,
};
use crate::postgres::native_catalog::admin;

mod capture;
mod control;
mod control_types;
pub(crate) use super::data_documents::ControlPermit;
pub use capture::AdminCapture;
pub use control_types::*;

pub use admin::{
    AdminLock, AdminMetric, AdminSession, AdminSnapshot, AdminStats, MAX_ADMIN_BLOCKERS,
    MAX_ADMIN_BYTES, MAX_ADMIN_QUERY_CHARS, MAX_ADMIN_ROWS, MAX_ADMIN_TEXT_BYTES,
};

impl Backend {
    /// Captures immutable row identity for optional native control review.
    pub async fn admin_capture(&self, document: &DataDocument) -> Result<AdminCapture, DataError> {
        AdminCapture::new(document.clone(), self.admin_snapshot(document).await?)
    }
    /// Bound to the admitted document connection. Statistics are collected over
    /// an interval, not an atomic cluster snapshot. No polling is installed.
    pub async fn admin_snapshot(
        &self,
        document: &DataDocument,
    ) -> Result<AdminSnapshot, DataError> {
        self.object_read(document, |spec, drivers, cancellation| async move {
            admin::read(&spec, &drivers, cancellation).await
        })
        .await
    }
}

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod control_tests;

#[cfg(test)]
mod control_live_tests;
