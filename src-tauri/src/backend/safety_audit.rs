//! Read-only, profile-local retained successful safety overrides. This is not a
//! complete server audit: storage retains only the newest 1,000 rows globally.
mod storage;
mod types;
use super::Backend;
use std::sync::Arc;
pub use types::*;

impl Backend {
    /// Available disconnected. Never hydrates credentials, resolves an endpoint,
    /// executes PostgreSQL SQL or changes audit records. A cursor is valid only
    /// on this backend/profile owner and its original exact connection filter.
    /// Each page has a SQLite snapshot; global retention can remove later pages.
    pub async fn load_safety_audit(
        &self,
        connection_id: String,
        cursor: Option<SafetyAuditCursor>,
    ) -> Result<SafetyAuditPage, SafetyAuditError> {
        if !types::connection_valid(&connection_id) {
            return Err(SafetyAuditError::InvalidRequest);
        }
        let owner = Arc::downgrade(&self.0);
        if let Some(cursor) = &cursor {
            if !cursor.owner.ptr_eq(&owner) || cursor.connection_id != connection_id {
                return Err(SafetyAuditError::ForeignCursor);
            }
            if cursor.checked_heap_bytes().is_none() {
                return Err(SafetyAuditError::InvalidRequest);
            }
        }
        let connection_id = connection_id.into_boxed_str().into_string();
        self.call(move |state| async move {
            Ok(storage::load(&state.pool, owner, connection_id, cursor).await)
        })
        .await
        .map_err(|_| SafetyAuditError::Unavailable)?
    }
}

#[cfg(test)]
mod tests;
