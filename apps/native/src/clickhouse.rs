//! Plan 031 step 4: native ClickHouse. Sessions are explicit and owned by the
//! workspace; the sidebar tree lists every permitted database with tables,
//! views, materialized views and dictionaries kept apart; documents are a
//! query tab, a table's data and its structure. Backend:
//! `dbunk_lib::backend::clickhouse`.
pub mod document;
mod document_model;
pub mod session_model;
pub mod sessions;
pub mod tree_model;
pub mod tree_view;

use dbunk_lib::backend::{DevelopmentConnection, DevelopmentEngineConnection};

/// A saved, supported ClickHouse connection that can open a session.
pub fn is_clickhouse(connection: &DevelopmentConnection) -> bool {
    connection.unsupported_reason.is_none()
        && matches!(
            connection.settings,
            Some(DevelopmentEngineConnection::ClickHouse(_))
        )
}
