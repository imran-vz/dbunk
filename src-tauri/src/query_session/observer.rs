use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

use super::postgres;
use super::protocol::{QuerySessionError, QueryTransactionStatus};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;

struct Request {
    pid: i32,
    backend_start: String,
    response: oneshot::Sender<QueryTransactionStatus>,
}

pub(crate) struct Observer {
    _connection: postgres::SessionConnection,
    requests: mpsc::Sender<Request>,
}

impl Observer {
    pub(crate) async fn connect(
        spec: &ResolvedPostgresConnectSpec,
    ) -> Result<Arc<Self>, QuerySessionError> {
        let connection = postgres::connect(spec).await?;
        let client = connection.client.clone();
        let (requests, mut receiver) = mpsc::channel::<Request>(128);
        tokio::spawn(async move {
            while let Some(first) = receiver.recv().await {
                tokio::time::sleep(Duration::from_millis(5)).await;
                let mut batch = vec![first];
                while let Ok(request) = receiver.try_recv() {
                    batch.push(request);
                }
                let pids = batch.iter().map(|request| request.pid).collect::<Vec<_>>();
                let parameters: [&(dyn tokio_postgres::types::ToSql + Sync); 1] = [&pids];
                let query = client.query("SELECT pid, backend_start::text, state FROM pg_stat_activity WHERE pid = ANY($1)", &parameters);
                let rows = tokio::time::timeout(Duration::from_secs(2), query)
                    .await
                    .ok()
                    .and_then(Result::ok);
                for request in batch {
                    let status = rows
                        .as_ref()
                        .and_then(|rows| {
                            rows.iter().find(|row| row.get::<_, i32>(0) == request.pid)
                        })
                        .map(|row| {
                            let start: String = row.get(1);
                            let state: String = row.get(2);
                            if start != request.backend_start {
                                QueryTransactionStatus::Unknown
                            } else {
                                map_state(&state)
                            }
                        })
                        .unwrap_or(QueryTransactionStatus::Unknown);
                    let _ = request.response.send(status);
                }
            }
        });
        Ok(Arc::new(Self {
            _connection: connection,
            requests,
        }))
    }

    pub(crate) async fn observe(&self, pid: i32, backend_start: String) -> QueryTransactionStatus {
        let (response, receiver) = oneshot::channel();
        let send = tokio::time::timeout(
            Duration::from_secs(2),
            self.requests.send(Request {
                pid,
                backend_start,
                response,
            }),
        )
        .await;
        if !matches!(send, Ok(Ok(()))) {
            return QueryTransactionStatus::Unknown;
        }
        tokio::time::timeout(Duration::from_secs(2), receiver)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or(QueryTransactionStatus::Unknown)
    }
}

fn map_state(state: &str) -> QueryTransactionStatus {
    match state {
        "idle" => QueryTransactionStatus::Idle,
        "idle in transaction" => QueryTransactionStatus::Active,
        "idle in transaction (aborted)" => QueryTransactionStatus::Failed,
        _ => QueryTransactionStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_only_authoritative_idle_states() {
        assert_eq!(map_state("idle"), QueryTransactionStatus::Idle);
        assert_eq!(map_state("active"), QueryTransactionStatus::Unknown);
        assert_eq!(
            map_state("idle in transaction (aborted)"),
            QueryTransactionStatus::Failed
        );
    }
}
