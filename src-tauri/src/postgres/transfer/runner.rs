use std::collections::HashSet;
use std::future::Future;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::csv::{self, CsvCell, CsvOptions, Parser};
use super::files::{self, FileFingerprint, PartialFile, SourceFile};
use super::manager::JobContext;
#[cfg(test)]
use super::protocol::TargetColumn;
use super::protocol::{
    ColumnMapping, Direction, InspectPayload, Inspection, SourceColumn, TransferError,
};
use crate::postgres::connect_spec::ResolvedPostgresConnectSpec;
use crate::postgres::dedicated::{self, DedicatedConnection, DedicatedError, NoticeSink};
use crate::StoredConnection;
use futures_util::{SinkExt, StreamExt};

mod catalog;
mod sql;

#[cfg(test)]
use catalog::CatalogColumn;
use catalog::RelationState;

const INSPECTION_BYTES: usize = 256 * 1024;
const SAMPLE_ROWS: usize = 50;
const SAMPLE_VALUE_BYTES: usize = 64 * 1024;
const DATABASE_FINALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const EXPORT_FIELD_LIMIT_SENTINEL: &str = "dbunk_csv_export_field_limit_0";
const EXPORT_RECORD_LIMIT_SENTINEL: &str = "dbunk_csv_export_record_limit_0";

#[derive(Clone)]
pub(crate) struct Review {
    pub(crate) payload: InspectPayload,
    pub(crate) inspection: Inspection,
    source_fingerprint: Option<FileFingerprint>,
    relation: RelationState,
}

pub(crate) enum RunRequest {
    Import { mapping: Vec<ColumnMapping> },
    Export { destination_path: String },
}

pub(crate) async fn inspect(
    connection: StoredConnection,
    payload: InspectPayload,
) -> Result<Review, TransferError> {
    validate_inspect_payload(&connection, &payload)?;
    payload
        .options
        .validate()
        .map_err(|error| TransferError::invalid("options", &error.reason))?;

    let spec = ResolvedPostgresConnectSpec::from_connection(&connection)
        .map_err(|_| TransferError::UnsupportedEngine)?;
    let dedicated = dedicated::connect(&spec, NoticeSink::Ignore)
        .await
        .map_err(map_dedicated_error)?;
    let result = inspect_connected(&dedicated, payload).await;
    dedicated.close().await;
    result
}

async fn inspect_connected(
    connection: &DedicatedConnection,
    payload: InspectPayload,
) -> Result<Review, TransferError> {
    let relation = catalog::inspect(connection, &payload.schema, &payload.table).await?;
    relation.ensure_supported(payload.direction)?;

    let (file_name, total_bytes, source_columns, sample_rows, sample_truncated, fingerprint) =
        match payload.direction {
            Direction::Import => {
                let path =
                    Path::new(payload.source_path.as_deref().ok_or_else(|| {
                        TransferError::invalid("sourcePath", "A source is required")
                    })?);
                let source = files::open_source(path).await?;
                let fingerprint = source.fingerprint.clone();
                let total_bytes = fingerprint.len();
                let (source_columns, sample_rows, sample_truncated) =
                    inspect_source(source, &payload.options).await?;
                (
                    Some(files::file_name(path)?),
                    Some(total_bytes),
                    source_columns,
                    sample_rows,
                    sample_truncated,
                    Some(fingerprint),
                )
            }
            Direction::Export => (None, None, Vec::new(), Vec::new(), false, None),
        };

    let target_columns = relation
        .columns
        .iter()
        .map(|column| column.public.clone())
        .collect();
    Ok(Review {
        inspection: Inspection {
            inspection_token: String::new(),
            connection_id: payload.connection_id.clone(),
            schema: payload.schema.clone(),
            table: payload.table.clone(),
            direction: payload.direction,
            file_name,
            total_bytes,
            source_columns,
            target_columns,
            sample_rows,
            sample_truncated,
            options: payload.options.clone(),
        },
        payload,
        source_fingerprint: fingerprint,
        relation,
    })
}

pub(crate) async fn run(
    context: JobContext,
    connection: StoredConnection,
    review: Review,
    request: RunRequest,
) -> Result<(), TransferError> {
    validate_run_request(&connection, &review, &request)?;
    let spec = ResolvedPostgresConnectSpec::from_connection(&connection)
        .map_err(|_| TransferError::UnsupportedEngine)?;
    let dedicated = tokio::select! {
        biased;
        _ = context.cancelled() => return Err(TransferError::Cancelled),
        result = dedicated::connect(&spec, NoticeSink::Ignore) => {
            result.map_err(map_dedicated_error)?
        }
    };
    let result = match request {
        RunRequest::Import { mapping } => run_import(&context, &dedicated, &review, &mapping).await,
        RunRequest::Export { destination_path } => {
            run_export(
                &context,
                &dedicated,
                &review,
                PathBuf::from(destination_path),
            )
            .await
        }
    };
    dedicated.close().await;
    result
}

async fn inspect_source(
    source: SourceFile,
    options: &CsvOptions,
) -> Result<(Vec<SourceColumn>, Vec<Vec<Option<String>>>, bool), TransferError> {
    let mut parser = Parser::new(options).map_err(csv_error)?;
    let mut bytes_read = 0_usize;
    let mut records = Vec::new();
    let mut source_columns = None;
    let mut retained_bytes = 0_usize;
    let mut discarded_sample = false;
    let mut reached_eof = false;

    while bytes_read < INSPECTION_BYTES {
        let remaining = INSPECTION_BYTES - bytes_read;
        let bytes = source
            .read(remaining.min(csv::CSV_OUTPUT_CHUNK_BYTES))
            .await?;
        if bytes.is_empty() {
            reached_eof = true;
            break;
        }
        bytes_read += bytes.len();
        for byte in bytes {
            if let Some(record) = parser.push(byte).map_err(csv_error)? {
                let record_number = parser.next_record_number().saturating_sub(1);
                retain_inspection_record(
                    record,
                    record_number,
                    options,
                    &mut source_columns,
                    &mut records,
                    &mut retained_bytes,
                    &mut discarded_sample,
                )?;
            }
        }
    }
    if reached_eof {
        if let Some(record) = parser.finish().map_err(csv_error)? {
            let record_number = parser.next_record_number().saturating_sub(1);
            retain_inspection_record(
                record,
                record_number,
                options,
                &mut source_columns,
                &mut records,
                &mut retained_bytes,
                &mut discarded_sample,
            )?;
        }
    }

    if !reached_eof && source_columns.is_none() {
        return Err(TransferError::Csv {
            record: 1,
            column: None,
            reason: "The first record exceeds the 256 KiB inspection limit".into(),
        });
    }

    let incomplete = !reached_eof || parser.has_pending_input();
    let columns = source_columns.unwrap_or_default();
    Ok((columns, records, discarded_sample || incomplete))
}

fn retain_inspection_record(
    record: Vec<CsvCell>,
    record_number: u64,
    options: &CsvOptions,
    source_columns: &mut Option<Vec<SourceColumn>>,
    records: &mut Vec<Vec<Option<String>>>,
    retained_bytes: &mut usize,
    discarded_sample: &mut bool,
) -> Result<(), TransferError> {
    if source_columns.is_none() {
        if options.header
            && record.iter().map(|cell| cell.value.len()).sum::<usize>() > SAMPLE_VALUE_BYTES
        {
            return Err(TransferError::Csv {
                record: record_number,
                column: None,
                reason: "Header labels exceed the 64 KiB inspection limit".into(),
            });
        }
        let columns = if options.header {
            record
                .iter()
                .enumerate()
                .map(|(index, cell)| SourceColumn {
                    index,
                    name: cell.value.clone(),
                })
                .collect()
        } else {
            (0..record.len())
                .map(|index| SourceColumn {
                    index,
                    name: format!("Column {}", index + 1),
                })
                .collect()
        };
        *source_columns = Some(columns);
        if options.header {
            return Ok(());
        }
    }

    let expected = source_columns.as_ref().map_or(0, Vec::len);
    if record.len() != expected {
        return Err(TransferError::Csv {
            record: record_number,
            column: None,
            reason: format!("Expected {expected} columns but found {}", record.len()),
        });
    }
    let value_bytes = record.iter().map(|cell| cell.value.len()).sum::<usize>();
    if records.len() >= SAMPLE_ROWS
        || retained_bytes.saturating_add(value_bytes) > SAMPLE_VALUE_BYTES
    {
        *discarded_sample = true;
        return Ok(());
    }
    *retained_bytes += value_bytes;
    records.push(
        record
            .into_iter()
            .map(|cell| (!cell.is_null(options)).then_some(cell.value))
            .collect(),
    );
    Ok(())
}

async fn run_import(
    context: &JobContext,
    connection: &DedicatedConnection,
    review: &Review,
    mapping: &[ColumnMapping],
) -> Result<(), TransferError> {
    let validated = validate_mapping(review, mapping)?;
    let path = Path::new(
        review
            .payload
            .source_path
            .as_deref()
            .ok_or(TransferError::InspectionExpired)?,
    );
    let source = files::open_source(path)
        .await
        .map_err(|_| TransferError::SourceChanged)?;
    let expected_fingerprint = review
        .source_fingerprint
        .as_ref()
        .ok_or(TransferError::InspectionExpired)?;
    if &source.fingerprint != expected_fingerprint {
        return Err(TransferError::SourceChanged);
    }
    context.progress(0, Some(0));

    cancellable_pg(
        context,
        connection,
        connection.client.batch_execute("BEGIN"),
    )
    .await?;
    let result = import_transaction(context, connection, review, validated, source, path).await;
    if result.is_err() && !matches!(result, Err(TransferError::OutcomeUnknown)) {
        let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
        rollback(connection).await;
    }
    result
}

async fn import_transaction(
    context: &JobContext,
    connection: &DedicatedConnection,
    review: &Review,
    mapping: Vec<(usize, String)>,
    source: SourceFile,
    source_path: &Path,
) -> Result<(), TransferError> {
    set_session(context, connection).await?;
    catalog::lock_and_validate(context, connection, review).await?;

    let statement = sql::import(review, &mapping);
    let sink = cancellable_pg(context, connection, connection.client.copy_in(&statement)).await?;
    tokio::pin!(sink);
    let mut parser = Parser::new(&review.payload.options).map_err(csv_error)?;
    let mut bytes_read = 0_u64;
    let mut rows_sent = 0_u64;
    let mut saw_header = !review.payload.options.header;
    let expected_width = review.inspection.source_columns.len();

    loop {
        let bytes = tokio::select! {
            biased;
            _ = context.cancelled() => {
                let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
                return Err(TransferError::Cancelled);
            },
            result = source.read(csv::CSV_OUTPUT_CHUNK_BYTES) => result?,
        };
        if bytes.is_empty() {
            break;
        }
        bytes_read = checked_counter(bytes_read, bytes.len() as u64, "source")?;
        for byte in bytes {
            if let Some(record) = parser.push(byte).map_err(csv_error)? {
                if !saw_header {
                    saw_header = true;
                    validate_width(&record, expected_width, parser.next_record_number() - 1)?;
                    continue;
                }
                validate_width(&record, expected_width, parser.next_record_number() - 1)?;
                send_record(
                    context,
                    connection,
                    sink.as_mut(),
                    &record,
                    &mapping,
                    &review.payload.options,
                )
                .await?;
                rows_sent = checked_counter(rows_sent, 1, "rows")?;
            }
        }
        context.progress(bytes_read, Some(rows_sent));
    }
    if let Some(record) = parser.finish().map_err(csv_error)? {
        if !saw_header {
            validate_width(&record, expected_width, parser.next_record_number() - 1)?;
        } else {
            validate_width(&record, expected_width, parser.next_record_number() - 1)?;
            send_record(
                context,
                connection,
                sink.as_mut(),
                &record,
                &mapping,
                &review.payload.options,
            )
            .await?;
            rows_sent = checked_counter(rows_sent, 1, "rows")?;
        }
    }
    context.progress(bytes_read, Some(rows_sent));

    // BEFORE INSERT triggers may suppress or route rows. COPY's processed count
    // can therefore differ from rows sent, and becomes committed only after COMMIT.
    let committed_rows = cancellable_pg(context, connection, sink.as_mut().finish()).await?;

    let handle_fingerprint = source
        .current_fingerprint()
        .await
        .map_err(|_| TransferError::SourceChanged)?;
    let path_fingerprint = files::path_fingerprint(source_path)
        .await
        .map_err(|_| TransferError::SourceChanged)?;
    if &handle_fingerprint != review.source_fingerprint.as_ref().unwrap()
        || path_fingerprint != handle_fingerprint
    {
        return Err(TransferError::SourceChanged);
    }
    catalog::validate(context, connection, review).await?;
    if !context.begin_finalizing() {
        return Err(TransferError::Cancelled);
    }

    match tokio::time::timeout(
        DATABASE_FINALIZE_TIMEOUT,
        connection.client.batch_execute("COMMIT"),
    )
    .await
    {
        Ok(Ok(())) => {
            context.succeeded(Some(committed_rows));
            Ok(())
        }
        Ok(Err(error)) if error.as_db_error().is_some() => {
            rollback(connection).await;
            Err(TransferError::database(&error))
        }
        Ok(Err(_)) | Err(_) => Err(TransferError::OutcomeUnknown),
    }
}

async fn send_record(
    context: &JobContext,
    connection: &DedicatedConnection,
    mut sink: std::pin::Pin<&mut tokio_postgres::CopyInSink<Cursor<Vec<u8>>>>,
    record: &[CsvCell],
    mapping: &[(usize, String)],
    options: &CsvOptions,
) -> Result<(), TransferError> {
    for (output_index, (source_index, _)) in mapping.iter().enumerate() {
        if output_index > 0 {
            send_copy_chunk(
                context,
                connection,
                sink.as_mut(),
                vec![options.delimiter.as_bytes()[0]],
            )
            .await?;
        }
        let cell = &record[*source_index];
        let value = (!cell.is_null(options)).then_some(cell.value.as_str());
        for chunk in csv::encode_cell(value, options).map_err(csv_error)? {
            send_copy_chunk(context, connection, sink.as_mut(), chunk).await?;
        }
    }
    send_copy_chunk(context, connection, sink, vec![b'\n']).await
}

async fn send_copy_chunk(
    context: &JobContext,
    connection: &DedicatedConnection,
    mut sink: std::pin::Pin<&mut tokio_postgres::CopyInSink<Cursor<Vec<u8>>>>,
    chunk: Vec<u8>,
) -> Result<(), TransferError> {
    debug_assert!(chunk.len() <= csv::CSV_OUTPUT_CHUNK_BYTES);
    cancellable_pg(context, connection, sink.as_mut().send(Cursor::new(chunk))).await
}

async fn run_export(
    context: &JobContext,
    connection: &DedicatedConnection,
    review: &Review,
    destination: PathBuf,
) -> Result<(), TransferError> {
    let mut partial = PartialFile::create(destination, &context.job_id).await?;
    context.progress(0, None);
    cancellable_pg(
        context,
        connection,
        connection.client.batch_execute("BEGIN READ ONLY"),
    )
    .await?;
    let result = export_transaction(context, connection, review, &mut partial).await;
    if result.is_err() {
        let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
        rollback(connection).await;
    }
    result?;

    partial.finish_writing().await?;
    if !context.begin_finalizing() {
        return Err(TransferError::Cancelled);
    }
    partial.publish().await?;
    context.succeeded(None);
    Ok(())
}

async fn export_transaction(
    context: &JobContext,
    connection: &DedicatedConnection,
    review: &Review,
    partial: &mut PartialFile,
) -> Result<(), TransferError> {
    set_session(context, connection).await?;
    catalog::lock_and_validate(context, connection, review).await?;
    let statement = sql::export(review);
    let stream =
        cancellable_export_pg(context, connection, connection.client.copy_out(&statement)).await?;
    tokio::pin!(stream);
    let mut bytes_written = 0_u64;
    loop {
        let next = tokio::select! {
            biased;
            _ = context.cancelled() => {
                let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
                return Err(TransferError::Cancelled);
            },
            next = stream.next() => next,
        };
        let Some(chunk) = next else { break };
        let chunk = chunk.map_err(|error| map_export_pg_error(context, error))?;
        for bounded in chunk.chunks(csv::CSV_OUTPUT_CHUNK_BYTES) {
            tokio::select! {
                biased;
                _ = context.cancelled() => {
                    let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
                    return Err(TransferError::Cancelled);
                },
                result = partial.write_all(bounded) => result?,
            }
            bytes_written = checked_counter(bytes_written, bounded.len() as u64, "output")?;
            context.progress(bytes_written, None);
        }
    }
    catalog::validate(context, connection, review).await?;
    cancellable_pg(
        context,
        connection,
        connection.client.batch_execute("COMMIT"),
    )
    .await
}

async fn set_session(
    context: &JobContext,
    connection: &DedicatedConnection,
) -> Result<(), TransferError> {
    cancellable_pg(
        context,
        connection,
        connection
            .client
            .batch_execute("SET LOCAL DateStyle TO ISO; SET LOCAL TIME ZONE 'UTC'"),
    )
    .await
}

fn validate_mapping(
    review: &Review,
    mapping: &[ColumnMapping],
) -> Result<Vec<(usize, String)>, TransferError> {
    if mapping.is_empty() {
        return Err(TransferError::invalid(
            "mapping",
            "At least one target column must be mapped",
        ));
    }
    if mapping.len() > csv::MAX_COLUMNS {
        return Err(TransferError::invalid("mapping", "Too many mapped columns"));
    }
    let mut targets = HashSet::new();
    let mut validated = Vec::with_capacity(mapping.len());
    for entry in mapping {
        if entry.source_index >= review.inspection.source_columns.len() {
            return Err(TransferError::invalid(
                "mapping",
                "A source column index is out of range",
            ));
        }
        if !targets.insert(entry.target_column.as_str()) {
            return Err(TransferError::invalid(
                "mapping",
                "A target column is mapped more than once",
            ));
        }
        let target = review
            .relation
            .columns
            .iter()
            .find(|column| column.public.name == entry.target_column)
            .ok_or_else(|| TransferError::invalid("mapping", "A target column does not exist"))?;
        if target.public.generated || target.public.identity {
            return Err(TransferError::UnsupportedTarget {
                reason: "Generated and identity columns cannot be mapped".into(),
            });
        }
        validated.push((entry.source_index, entry.target_column.clone()));
    }
    for column in &review.relation.columns {
        if !targets.contains(column.public.name.as_str())
            && !column.public.nullable
            && !column.public.has_default
            && !column.public.generated
            && !column.public.identity
        {
            return Err(TransferError::UnsupportedTarget {
                reason: format!(
                    "Required target column {} needs a mapping",
                    column.public.name
                ),
            });
        }
    }
    Ok(validated)
}

fn validate_width(record: &[CsvCell], expected: usize, number: u64) -> Result<(), TransferError> {
    if record.len() == expected {
        Ok(())
    } else {
        Err(TransferError::Csv {
            record: number,
            column: None,
            reason: format!("Expected {expected} columns but found {}", record.len()),
        })
    }
}

fn validate_inspect_payload(
    connection: &StoredConnection,
    payload: &InspectPayload,
) -> Result<(), TransferError> {
    let StoredConnection::PostgreSQL(pg) = connection else {
        return Err(TransferError::UnsupportedEngine);
    };
    if payload.connection_id != pg.id {
        return Err(TransferError::invalid(
            "connectionId",
            "The connection does not match the request",
        ));
    }
    validate_identifier("schema", &payload.schema)?;
    validate_identifier("table", &payload.table)?;
    match payload.direction {
        Direction::Import if payload.source_path.is_none() => Err(TransferError::invalid(
            "sourcePath",
            "A source file is required",
        )),
        Direction::Export if payload.source_path.is_some() => Err(TransferError::invalid(
            "sourcePath",
            "Exports do not accept a source file",
        )),
        _ => Ok(()),
    }
}

fn validate_run_request(
    connection: &StoredConnection,
    review: &Review,
    request: &RunRequest,
) -> Result<(), TransferError> {
    validate_inspect_payload(connection, &review.payload)?;
    match (&review.payload.direction, request) {
        (Direction::Import, RunRequest::Import { .. })
        | (Direction::Export, RunRequest::Export { .. }) => Ok(()),
        _ => Err(TransferError::invalid(
            "direction",
            "The run request does not match its inspection",
        )),
    }
}

fn validate_identifier(field: &str, value: &str) -> Result<(), TransferError> {
    if value.is_empty() || value.len() > 1_024 || value.contains('\0') {
        Err(TransferError::invalid(field, "Invalid relation identifier"))
    } else {
        Ok(())
    }
}

pub(super) async fn cancellable_pg<T>(
    context: &JobContext,
    connection: &DedicatedConnection,
    operation: impl Future<Output = Result<T, tokio_postgres::Error>>,
) -> Result<T, TransferError> {
    tokio::select! {
        biased;
        _ = context.cancelled() => {
            let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
            Err(TransferError::Cancelled)
        }
        result = operation => result.map_err(|error| map_pg_error(context, error)),
    }
}

async fn cancellable_export_pg<T>(
    context: &JobContext,
    connection: &DedicatedConnection,
    operation: impl Future<Output = Result<T, tokio_postgres::Error>>,
) -> Result<T, TransferError> {
    tokio::select! {
        biased;
        _ = context.cancelled() => {
            let _ = dedicated::cancel(connection.cancel.clone(), connection.tls.clone()).await;
            Err(TransferError::Cancelled)
        }
        result = operation => result.map_err(|error| map_export_pg_error(context, error)),
    }
}

fn map_pg_error(context: &JobContext, error: tokio_postgres::Error) -> TransferError {
    if context.is_cancelled() {
        TransferError::Cancelled
    } else {
        TransferError::database(&error)
    }
}

fn map_export_pg_error(context: &JobContext, error: tokio_postgres::Error) -> TransferError {
    if context.is_cancelled() {
        return TransferError::Cancelled;
    }
    if error.code().is_some_and(|code| code.code() == "22P02") {
        let message = error
            .as_db_error()
            .map(tokio_postgres::error::DbError::message);
        if message.is_some_and(|message| has_quoted_value(message, EXPORT_FIELD_LIMIT_SENTINEL)) {
            return TransferError::ExportLimitExceeded {
                limit: super::protocol::ExportLimit::Field,
            };
        }
        if message.is_some_and(|message| has_quoted_value(message, EXPORT_RECORD_LIMIT_SENTINEL)) {
            return TransferError::ExportLimitExceeded {
                limit: super::protocol::ExportLimit::Record,
            };
        }
    }
    TransferError::database(&error)
}

fn has_quoted_value(message: &str, value: &str) -> bool {
    message.split('"').any(|part| part == value)
}

async fn rollback(connection: &DedicatedConnection) {
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        connection.client.batch_execute("ROLLBACK"),
    )
    .await;
}

fn map_dedicated_error(error: DedicatedError) -> TransferError {
    match error {
        DedicatedError::Timeout { operation } => TransferError::Timeout { operation },
        DedicatedError::Database { code, .. } => TransferError::Database {
            code,
            reason: "The database rejected the connection".into(),
        },
        DedicatedError::Tls { .. } => TransferError::Database {
            code: None,
            reason: "The secure database connection failed".into(),
        },
        DedicatedError::ConnectionLost => TransferError::Database {
            code: None,
            reason: "The database connection was lost".into(),
        },
    }
}

fn csv_error(error: csv::CsvError) -> TransferError {
    TransferError::Csv {
        record: error.record,
        column: error.column,
        reason: error.reason,
    }
}

fn checked_counter(current: u64, amount: u64, name: &str) -> Result<u64, TransferError> {
    current
        .checked_add(amount)
        .filter(|value| *value <= files::SAFE_INTEGER)
        .ok_or_else(|| TransferError::invalid(name, "The transfer is too large to report safely"))
}

#[cfg(test)]
pub(crate) fn test_review(connection_id: &str) -> Review {
    let options = CsvOptions::default();
    let target = TargetColumn {
        name: "value".into(),
        data_type: "text".into(),
        nullable: true,
        has_default: false,
        generated: false,
        identity: false,
    };
    let payload = InspectPayload {
        connection_id: connection_id.into(),
        schema: "public".into(),
        table: "transfer_test".into(),
        direction: Direction::Export,
        source_path: None,
        options: options.clone(),
    };
    Review {
        inspection: Inspection {
            inspection_token: String::new(),
            connection_id: connection_id.into(),
            schema: payload.schema.clone(),
            table: payload.table.clone(),
            direction: Direction::Export,
            file_name: None,
            total_bytes: None,
            source_columns: Vec::new(),
            target_columns: vec![target.clone()],
            sample_rows: Vec::new(),
            sample_truncated: false,
            options,
        },
        payload,
        source_fingerprint: None,
        relation: RelationState {
            oid: 1,
            kind: "r".into(),
            row_security: false,
            force_row_security: false,
            populated: true,
            columns: vec![CatalogColumn {
                number: 1,
                type_oid: 25,
                type_modifier: -1,
                collation_oid: 100,
                default_fingerprint: None,
                public: target,
            }],
        },
    }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
