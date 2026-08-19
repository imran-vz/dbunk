use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_util::{future::poll_fn, StreamExt};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, Error as RustlsError, SignatureScheme};
use tokio::sync::{mpsc, Mutex};
use tokio_postgres::{config::SslMode, AsyncMessage, Client, NoTls, SimpleQueryMessage};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::protocol::{QueryDatabaseError, QuerySessionError};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::options::driver_option_sql;

#[derive(Debug)]
pub(crate) struct Notice {
    pub severity: String,
    pub message: String,
}
pub(crate) struct SessionConnection {
    pub client: Arc<Client>,
    pub cancel: tokio_postgres::CancelToken,
    pub pid: i32,
    pub backend_start: String,
    pub notices: Arc<Mutex<mpsc::Receiver<Notice>>>,
    dropped_notices: Arc<AtomicU32>,
    _driver: tokio::task::JoinHandle<()>,
}

pub(crate) async fn connect(
    spec: &ResolvedPostgresConnectSpec,
) -> Result<SessionConnection, QuerySessionError> {
    let mut config = spec.tokio_config();
    config.ssl_mode(if spec.tls_prefer {
        SslMode::Prefer
    } else {
        SslMode::Disable
    });
    let (notice_tx, notice_rx) = mpsc::channel(500);
    let dropped_notices = Arc::new(AtomicU32::new(0));
    let (client, driver) = if spec.tls_prefer {
        let tls = MakeRustlsConnect::new(permissive_tls_config());
        let connect = config.connect(tls);
        let (client, mut connection) = with_deadline(spec, connect).await?;
        let dropped = dropped_notices.clone();
        let driver = tokio::spawn(async move {
            while let Some(Ok(message)) = poll_fn(|cx| connection.poll_message(cx)).await {
                if let AsyncMessage::Notice(notice) = message {
                    if notice_tx
                        .try_send(Notice {
                            severity: notice.severity().to_string(),
                            message: notice.message().to_string(),
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        (client, driver)
    } else {
        let connect = config.connect(NoTls);
        let (client, mut connection) = with_deadline(spec, connect).await?;
        let dropped = dropped_notices.clone();
        let driver = tokio::spawn(async move {
            while let Some(Ok(message)) = poll_fn(|cx| connection.poll_message(cx)).await {
                if let AsyncMessage::Notice(notice) = message {
                    if notice_tx
                        .try_send(Notice {
                            severity: notice.severity().to_string(),
                            message: notice.message().to_string(),
                        })
                        .is_err()
                    {
                        dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        (client, driver)
    };
    for statement in driver_option_sql(&spec.driver_options) {
        client
            .batch_execute(&statement)
            .await
            .map_err(database_error)?;
    }
    let identity = client.query_one("SELECT pg_backend_pid(), backend_start::text FROM pg_stat_activity WHERE pid = pg_backend_pid()", &[]).await.map_err(database_error)?;
    let cancel = client.cancel_token();
    Ok(SessionConnection {
        client: Arc::new(client),
        cancel,
        pid: identity.get(0),
        backend_start: identity.get(1),
        notices: Arc::new(Mutex::new(notice_rx)),
        dropped_notices,
        _driver: driver,
    })
}

impl SessionConnection {
    pub(crate) fn take_dropped_notices(&self) -> u32 {
        self.dropped_notices.swap(0, Ordering::Relaxed)
    }
}

async fn with_deadline<T, E>(
    spec: &ResolvedPostgresConnectSpec,
    future: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T, QuerySessionError> {
    match spec.connect_timeout {
        Some(limit) => tokio::time::timeout(limit, future)
            .await
            .map_err(|_| QuerySessionError::Timeout {
                operation: "connect".into(),
            })?
            .map_err(|_| QuerySessionError::ConnectionLost),
        None => future.await.map_err(|_| QuerySessionError::ConnectionLost),
    }
}

#[derive(Debug)]
struct AcceptAllVerifier;
impl ServerCertVerifier for AcceptAllVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PKCS1_SHA512,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::ED25519,
        ]
    }
}
fn permissive_tls_config() -> rustls::ClientConfig {
    let mut config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("ring protocol versions")
    .with_root_certificates(rustls::RootCertStore::empty())
    .with_no_client_auth();
    config
        .dangerous()
        .set_certificate_verifier(Arc::new(AcceptAllVerifier));
    config
}

#[derive(Debug, Default)]
pub(crate) struct ExecutionTotals {
    pub omitted_rows: u64,
    pub omitted_result_sets: u32,
    pub omitted_notices: u32,
    pub omitted_metadata_bytes: u64,
    pub truncation_reasons: Vec<String>,
    retained_metadata_bytes: usize,
}

#[derive(Debug)]
pub(crate) enum DriverEvent {
    ResultStarted {
        result_set_index: u32,
        columns: Vec<Option<String>>,
    },
    RowBatch {
        result_set_index: u32,
        rows: Vec<Vec<Option<String>>>,
    },
    ResultCompleted {
        result_set_index: u32,
        row_count: u64,
    },
    ResultAborted {
        result_set_index: u32,
        row_count: u64,
    },
    Notice(Notice),
    Finished(Result<ExecutionTotals, (QuerySessionError, ExecutionTotals)>),
}

/// A capacity-one handoff makes frontend credit apply directly to polling the
/// SimpleQueryStream: when the actor pauses, the driver and TCP socket pause.
pub(crate) fn execute_stream(
    client: Arc<Client>,
    notices: Arc<Mutex<mpsc::Receiver<Notice>>>,
    sql: String,
) -> mpsc::Receiver<DriverEvent> {
    let (sender, receiver) = mpsc::channel(1);
    tokio::spawn(async move {
        reduce_stream(&client, notices, &sql, sender).await;
    });
    receiver
}

async fn reduce_stream(
    client: &Client,
    notices: Arc<Mutex<mpsc::Receiver<Notice>>>,
    sql: &str,
    sender: mpsc::Sender<DriverEvent>,
) {
    let mut stream = match client.simple_query_raw(sql).await {
        Ok(stream) => Box::pin(stream),
        Err(error) => {
            let _ = sender
                .send(DriverEvent::Finished(Err((
                    database_error(error),
                    ExecutionTotals::default(),
                ))))
                .await;
            return;
        }
    };
    let mut totals = ExecutionTotals::default();
    let mut result_set_index = 0_u32;
    let mut columns = Vec::new();
    let mut batch = Vec::new();
    let mut batch_bytes = 0_usize;
    let mut total = 0_u64;
    let mut retained_in_set = 0_usize;
    let mut retained_total = 0_usize;
    let mut retained_bytes = 0_usize;
    let mut result_open = false;
    let mut retained_notices = 0_u32;
    let mut notices = notices.lock().await;
    let mut notices_open = true;
    loop {
        let message = tokio::select! {
            biased;
            notice = notices.recv(), if notices_open => {
                if let Some(notice) = notice {
                    if send_notice(&sender, notice, &mut totals, &mut retained_notices).await.is_err() {
                        return;
                    }
                } else {
                    notices_open = false;
                }
                continue;
            }
            message = stream.next() => match message {
                Some(message) => message,
                None => break,
            },
        };
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                flush_batch(&sender, result_set_index, &mut batch, &mut batch_bytes).await;
                if drain_notices(&sender, &mut notices, &mut totals, &mut retained_notices)
                    .await
                    .is_err()
                {
                    return;
                }
                if result_open {
                    if result_set_index < 64 {
                        if sender
                            .send(DriverEvent::ResultAborted {
                                result_set_index,
                                row_count: total,
                            })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    } else {
                        totals.omitted_result_sets += 1;
                        totals.truncation_reasons.push("resultSets".into());
                    }
                }
                totals.truncation_reasons.sort();
                totals.truncation_reasons.dedup();
                let _ = sender
                    .send(DriverEvent::Finished(Err((database_error(error), totals))))
                    .await;
                return;
            }
        };
        match message {
            SimpleQueryMessage::RowDescription(description) => {
                result_open = true;
                columns = description
                    .iter()
                    .map(|column| Some(column.name().to_string()))
                    .collect();
                bound_metadata(&mut columns, &mut totals);
                if result_set_index < 64
                    && sender
                        .send(DriverEvent::ResultStarted {
                            result_set_index,
                            columns: columns.clone(),
                        })
                        .await
                        .is_err()
                {
                    return;
                }
            }
            SimpleQueryMessage::Row(row) => {
                total += 1;
                if retained_total >= 50_000
                    || retained_in_set >= 10_000
                    || retained_bytes >= 32 * 1024 * 1024
                {
                    totals.omitted_rows += 1;
                    totals.truncation_reasons.push("rowCount".into());
                    continue;
                }
                let mut values = (0..row.len())
                    .map(|index| {
                        row.get(index).map(|value| {
                            truncate_utf8(value, 1024 * 1024, &mut totals.truncation_reasons)
                        })
                    })
                    .collect::<Vec<_>>();
                shrink_row(&mut values, &mut totals.truncation_reasons);
                let bytes = serde_json::to_vec(&values)
                    .map(|json| json.len())
                    .unwrap_or(usize::MAX);
                if bytes > 2 * 1024 * 1024 || retained_bytes + bytes > 32 * 1024 * 1024 {
                    totals.omitted_rows += 1;
                    totals.truncation_reasons.push("rowBytes".into());
                } else {
                    if !batch.is_empty() && (batch.len() >= 200 || batch_bytes + bytes > 256 * 1024)
                    {
                        flush_batch(&sender, result_set_index, &mut batch, &mut batch_bytes).await;
                    }
                    retained_total += 1;
                    retained_in_set += 1;
                    retained_bytes += bytes;
                    batch_bytes += bytes;
                    batch.push(values);
                }
            }
            SimpleQueryMessage::CommandComplete(count) => {
                if result_set_index < 64 {
                    flush_batch(&sender, result_set_index, &mut batch, &mut batch_bytes).await;
                    if columns.is_empty() {
                        let _ = sender
                            .send(DriverEvent::ResultStarted {
                                result_set_index,
                                columns: Vec::new(),
                            })
                            .await;
                    }
                    if sender
                        .send(DriverEvent::ResultCompleted {
                            result_set_index,
                            row_count: total.max(count),
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                } else {
                    totals.omitted_result_sets += 1;
                    totals.truncation_reasons.push("resultSets".into());
                    batch.clear();
                    batch_bytes = 0;
                }
                total = 0;
                retained_in_set = 0;
                columns.clear();
                result_open = false;
                result_set_index += 1;
            }
            _ => {}
        }
    }
    if drain_notices(&sender, &mut notices, &mut totals, &mut retained_notices)
        .await
        .is_err()
    {
        return;
    }
    totals.truncation_reasons.sort();
    totals.truncation_reasons.dedup();
    let _ = sender.send(DriverEvent::Finished(Ok(totals))).await;
}

async fn drain_notices(
    sender: &mpsc::Sender<DriverEvent>,
    notices: &mut mpsc::Receiver<Notice>,
    totals: &mut ExecutionTotals,
    retained_notices: &mut u32,
) -> Result<(), ()> {
    while let Ok(notice) = notices.try_recv() {
        send_notice(sender, notice, totals, retained_notices).await?;
    }
    Ok(())
}

async fn send_notice(
    sender: &mpsc::Sender<DriverEvent>,
    notice: Notice,
    totals: &mut ExecutionTotals,
    retained_notices: &mut u32,
) -> Result<(), ()> {
    if *retained_notices >= 500 {
        totals.omitted_notices += 1;
        totals.truncation_reasons.push("notices".into());
        return Ok(());
    }
    let bytes = serde_json::to_vec(&(&notice.severity, &notice.message))
        .map(|json| json.len())
        .unwrap_or(usize::MAX);
    if totals.retained_metadata_bytes.saturating_add(bytes) > 1024 * 1024 {
        totals.omitted_notices += 1;
        totals.omitted_metadata_bytes = totals.omitted_metadata_bytes.saturating_add(bytes as u64);
        totals.truncation_reasons.push("metadataBytes".into());
        return Ok(());
    }
    totals.retained_metadata_bytes += bytes;
    *retained_notices += 1;
    sender
        .send(DriverEvent::Notice(notice))
        .await
        .map_err(|_| ())
}

async fn flush_batch(
    sender: &mpsc::Sender<DriverEvent>,
    result_set_index: u32,
    batch: &mut Vec<Vec<Option<String>>>,
    bytes: &mut usize,
) {
    if batch.is_empty() {
        return;
    }
    let rows = std::mem::take(batch);
    *bytes = 0;
    let _ = sender
        .send(DriverEvent::RowBatch {
            result_set_index,
            rows,
        })
        .await;
}

fn bound_metadata(columns: &mut [Option<String>], result: &mut ExecutionTotals) {
    let mut remaining = (1024 * 1024_usize).saturating_sub(result.retained_metadata_bytes);
    for column in columns {
        let Some(name) = column else { continue };
        let bytes = name.len();
        if bytes <= remaining {
            remaining -= bytes;
            result.retained_metadata_bytes += bytes;
        } else {
            result.omitted_metadata_bytes += bytes as u64;
            *column = None;
            result.truncation_reasons.push("metadataBytes".into());
        }
    }
}
pub(crate) fn truncate_utf8(value: &str, max: usize, reasons: &mut Vec<String>) -> String {
    if value.len() <= max {
        return value.to_string();
    }
    let mut end = max;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    reasons.push("cellBytes".into());
    value[..end].to_string()
}
pub(crate) fn shrink_row(values: &mut [Option<String>], reasons: &mut Vec<String>) {
    while serde_json::to_vec(values)
        .map(|json| json.len())
        .unwrap_or(usize::MAX)
        > 2 * 1024 * 1024
    {
        let Some(value) = values
            .iter_mut()
            .rev()
            .find_map(Option::as_mut)
            .filter(|value| !value.is_empty())
        else {
            break;
        };
        let mut end = value.len() / 2;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        reasons.push("rowBytes".into());
    }
}

pub(crate) async fn cancel(cancel: tokio_postgres::CancelToken, tls: bool) -> bool {
    let future = async move {
        if tls {
            cancel
                .cancel_query(MakeRustlsConnect::new(permissive_tls_config()))
                .await
        } else {
            cancel.cancel_query(NoTls).await
        }
    };
    tokio::time::timeout(Duration::from_secs(2), future)
        .await
        .is_ok_and(|result| result.is_ok())
}
pub(crate) fn database_error(error: tokio_postgres::Error) -> QuerySessionError {
    if let Some(db) = error.as_db_error() {
        QuerySessionError::Database {
            code: Some(db.code().code().into()),
            message: db.message().into(),
            severity: Some(db.severity().into()),
            position: None,
        }
    } else {
        QuerySessionError::ConnectionLost
    }
}
pub(crate) fn display_error(error: &QuerySessionError) -> Option<QueryDatabaseError> {
    match error {
        QuerySessionError::Database {
            code,
            message,
            severity,
            position,
        } => Some(QueryDatabaseError {
            code: code.clone(),
            message: message.clone(),
            severity: severity.clone(),
            position: *position,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PgDriverOptions;
    #[test]
    fn utf8_truncation_stays_on_boundary() {
        let mut reasons = Vec::new();
        assert_eq!(truncate_utf8("éé", 3, &mut reasons), "é");
        assert_eq!(reasons, ["cellBytes"]);
    }
    #[test]
    fn oversized_rows_shrink_later_cells_first() {
        let mut values = vec![Some("a".repeat(1024 * 1024)), Some("b".repeat(1024 * 1024))];
        let mut reasons = Vec::new();
        shrink_row(&mut values, &mut reasons);
        assert!(serde_json::to_vec(&values).unwrap().len() <= 2 * 1024 * 1024);
    }

    #[tokio::test]
    async fn notices_share_the_metadata_budget_and_count_limit() {
        let (sender, receiver) = mpsc::channel(501);
        let mut totals = ExecutionTotals::default();
        let mut retained = 0;
        for _ in 0..501 {
            send_notice(
                &sender,
                Notice {
                    severity: "NOTICE".into(),
                    message: "message".into(),
                },
                &mut totals,
                &mut retained,
            )
            .await
            .unwrap();
        }
        assert_eq!(retained, 500);
        assert_eq!(totals.omitted_notices, 1);
        assert_eq!(receiver.len(), 500);

        totals.retained_metadata_bytes = 1024 * 1024;
        retained = 0;
        send_notice(
            &sender,
            Notice {
                severity: "NOTICE".into(),
                message: "too large".into(),
            },
            &mut totals,
            &mut retained,
        )
        .await
        .unwrap();
        assert_eq!(totals.omitted_notices, 2);
        assert!(totals.omitted_metadata_bytes > 0);
        assert_eq!(receiver.len(), 500);
    }

    #[tokio::test]
    async fn queued_notices_are_drained_before_a_terminal_event() {
        let (notice_sender, mut notices) = mpsc::channel(1);
        notice_sender
            .send(Notice {
                severity: "NOTICE".into(),
                message: "before terminal".into(),
            })
            .await
            .unwrap();
        let (event_sender, mut events) = mpsc::channel(2);
        let mut totals = ExecutionTotals::default();
        let mut retained = 0;

        drain_notices(&event_sender, &mut notices, &mut totals, &mut retained)
            .await
            .unwrap();
        event_sender
            .send(DriverEvent::Finished(Ok(totals)))
            .await
            .unwrap();

        assert!(matches!(events.recv().await, Some(DriverEvent::Notice(_))));
        assert!(matches!(
            events.recv().await,
            Some(DriverEvent::Finished(Ok(_)))
        ));
    }

    fn live_spec(port: u16, tls_prefer: bool) -> ResolvedPostgresConnectSpec {
        ResolvedPostgresConnectSpec {
            connection_id: format!("live-{port}"),
            host: "127.0.0.1".into(),
            port,
            database: "dbunk_demo".into(),
            user: "dbunk".into(),
            password: "dbunk".into(),
            tls_prefer,
            connect_timeout: Some(Duration::from_secs(5)),
            driver_options: PgDriverOptions::default(),
        }
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres"]
    async fn query_session_live_plaintext_refusal_results_temp_state_and_cancel() {
        let connection = connect(&live_spec(15432, true))
            .await
            .expect("Prefer falls back when TLS is refused");
        connection.client.batch_execute("CREATE TEMP TABLE query_session_live_temp(value int); INSERT INTO query_session_live_temp VALUES (1)").await.expect("temp state");
        let mut events = execute_stream(
            connection.client.clone(),
            connection.notices.clone(),
            "SELECT * FROM query_session_live_temp; SELECT 1 WHERE false".into(),
        );
        let mut starts = Vec::new();
        let mut rows = 0;
        while let Some(event) = events.recv().await {
            match event {
                DriverEvent::ResultStarted { columns, .. } => starts.push(columns),
                DriverEvent::RowBatch { rows: batch, .. } => rows += batch.len(),
                DriverEvent::Finished(result) => {
                    result.expect("multi result");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(starts.len(), 2);
        assert_eq!(rows, 1);
        assert_eq!(starts[1].len(), 1);
        let cancel = connection.cancel.clone();
        let query = connection.client.simple_query("SELECT pg_sleep(30)");
        let (result, requested) = tokio::join!(query, cancel_query_for_test(cancel, false));
        assert!(requested);
        assert!(result.is_err());
    }

    #[tokio::test]
    #[ignore = "requires pnpm db:postgres-tls"]
    async fn query_session_live_tls_connects_and_cancels() {
        let connection = connect(&live_spec(15433, true))
            .await
            .expect("self-signed TLS");
        let cancel = connection.cancel.clone();
        let query = connection.client.simple_query("SELECT pg_sleep(30)");
        let (result, requested) = tokio::join!(query, cancel_query_for_test(cancel, true));
        assert!(requested);
        assert!(result.is_err());
    }

    async fn cancel_query_for_test(token: tokio_postgres::CancelToken, tls: bool) -> bool {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel(token, tls).await
    }
}
