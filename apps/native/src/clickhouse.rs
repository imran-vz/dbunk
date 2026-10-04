//! Plan 031 step 4: native ClickHouse. Each connection's session, tree and
//! tabs are owned by its [`workspace::ClickHouseWorkspace`]; the sidebar tree lists every permitted database with tables,
//! views, materialized views and dictionaries kept apart; documents are a
//! query tab, a table's data and its structure. Backend:
//! `dbunk_lib::backend::clickhouse`.
pub mod document;
mod document_model;
pub mod session_model;
pub mod sessions;
pub mod tree_model;
pub mod tree_view;
pub mod workspace;
