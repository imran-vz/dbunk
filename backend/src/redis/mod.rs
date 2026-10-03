//! Redis-specific backend code.
//!
//! Scoped to the `StorageClass::KeyValue` half of the dispatch split
//! (ADR-0008). The relational engine modules (`postgres`, `clickhouse`)
//! live alongside this one; the dispatcher in `dispatch/keyvalue.rs`
//! calls into here.
//!
//! Phase 1.1 surface: URL builder, connection-manager cache,
//! capabilities probe, and the generated destructive-command list.
//! Phase 1.2+ adds keyspace browsing, per-type viewers, CLI, pub/sub,
//! and server-info modules.

pub mod capabilities;
pub mod cli;
pub mod connection;
pub mod destructive_commands;
pub mod key_inspector;
pub mod key_ops;
pub mod keyspace;
pub mod pubsub;
pub mod server_info;
pub mod url;
pub mod value;
