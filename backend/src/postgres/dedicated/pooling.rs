//! Detects a connection pooler between dbunk and PostgreSQL. Behind a
//! transaction pooler, session state set in one transaction's server process
//! is invisible to the next transaction and visible to other clients, so
//! session-level settings must not be sent and multi-request sequences must
//! stay inside one transaction.
//!
//! Known managed endpoints are classified from their address. Otherwise the
//! process ID in BackendKeyData is compared with `pg_backend_pid()`: a direct
//! server reports its own process, a pooler substitutes its own cancel key.
//! A pooler's mode cannot be proven from the client: a small or round-robin
//! pool can keep a transaction pooler's client on one server process for as
//! long as it is observed. An unknown pooler is therefore treated as one that
//! pools by transaction. Results are cached per endpoint for the process.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use tokio::sync::OnceCell;
use tokio_postgres::{Client, SimpleQueryMessage};

use super::wire::WireState;
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::ConnectionPooling;

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

type Key = (String, String, u16, String, String);
static CACHE: LazyLock<Mutex<HashMap<Key, Arc<OnceCell<ConnectionPooling>>>>> =
    LazyLock::new(Default::default);

/// Classifies the started session `client`, whose protocol `wire` observes.
/// An inconclusive observation is reported as `Direct` and is not cached, so
/// a later connect observes again.
pub(super) async fn classify(
    spec: &ResolvedPostgresConnectSpec,
    client: &Client,
    wire: &WireState,
) -> ConnectionPooling {
    if let Some(known) = known_endpoint(&spec.tls.server_name, spec.port) {
        return known;
    }
    let cell = CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .entry(key(spec))
        .or_default()
        .clone();
    cell.get_or_try_init(|| async { observe(client, wire).await.ok_or(()) })
        .await
        .copied()
        .unwrap_or_else(|()| {
            log::warn!("Connection pooling could not be determined; treating as direct");
            ConnectionPooling::Direct
        })
}

/// What is already known without connecting: a managed endpoint's address,
/// or an earlier observation of the same endpoint.
pub(crate) fn known(spec: &ResolvedPostgresConnectSpec) -> Option<ConnectionPooling> {
    known_endpoint(&spec.tls.server_name, spec.port).or_else(|| {
        CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&key(spec))
            .and_then(|cell| cell.get().copied())
    })
}

fn key(spec: &ResolvedPostgresConnectSpec) -> Key {
    (
        spec.tls.server_name.to_ascii_lowercase(),
        spec.host.to_ascii_lowercase(),
        spec.port,
        spec.database.clone(),
        spec.user.clone(),
    )
}

/// Managed services whose pooling mode is fixed by the address.
fn known_endpoint(host: &str, port: u16) -> Option<ConnectionPooling> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.ends_with(".pooler.supabase.com") {
        return match port {
            6543 => Some(ConnectionPooling::TransactionPooler),
            5432 => Some(ConnectionPooling::SessionPooler),
            _ => None,
        };
    }
    if host.ends_with(".neon.tech") {
        // Neon's `-pooler` endpoints are PgBouncer in transaction mode; its
        // other endpoints proxy one client to one server process.
        let first_label = host.split('.').next().unwrap_or_default();
        return Some(if first_label.ends_with("-pooler") {
            ConnectionPooling::TransactionPooler
        } else {
            ConnectionPooling::SessionPooler
        });
    }
    None
}

async fn observe(client: &Client, wire: &WireState) -> Option<ConnectionPooling> {
    let key_pid = wire.key_pid()?;
    let messages = tokio::time::timeout(
        PROBE_TIMEOUT,
        client.simple_query("SELECT pg_backend_pid()"),
    )
    .await
    .ok()?
    .ok()?;
    let backend_pid: i32 = messages.iter().find_map(|message| match message {
        SimpleQueryMessage::Row(row) => row.get(0)?.parse().ok(),
        _ => None,
    })?;
    Some(if key_pid == backend_pid {
        ConnectionPooling::Direct
    } else {
        ConnectionPooling::Pooler
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_endpoints_are_classified_by_address() {
        let host = "aws-1-ap-south-1.pooler.supabase.com";
        assert_eq!(
            known_endpoint(host, 6543),
            Some(ConnectionPooling::TransactionPooler)
        );
        assert_eq!(
            known_endpoint("AWS-0-EU-WEST-1.POOLER.SUPABASE.COM.", 5432),
            Some(ConnectionPooling::SessionPooler)
        );
        assert_eq!(known_endpoint(host, 7000), None);
        assert_eq!(known_endpoint("db.abcdefgh.supabase.co", 5432), None);
        assert_eq!(
            known_endpoint(
                "ep-cool-darkness-123456-pooler.us-east-2.aws.neon.tech",
                5432
            ),
            Some(ConnectionPooling::TransactionPooler)
        );
        assert_eq!(
            known_endpoint("ep-cool-darkness-123456.us-east-2.aws.neon.tech", 5432),
            Some(ConnectionPooling::SessionPooler)
        );
        assert_eq!(
            known_endpoint("pooler.supabase.com.evil.example", 6543),
            None
        );
        assert_eq!(known_endpoint("neon.tech.evil.example", 5432), None);
    }
}
