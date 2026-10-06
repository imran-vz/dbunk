//! Bounded ClickHouse HTTP reads for native sessions (Plan 031 step 4).
//!
//! [`super::run_query`] buffers the whole `JSONCompact` body with no deadline.
//! Native documents instead stream `JSONCompactEachRowWithNamesAndTypes`, one
//! JSON array per line, and stop reading at a row cap, a byte cap or a
//! deadline. Stopping drops the HTTP connection; nothing is retried.
//!
//! No server setting is sent besides the output format and an optional
//! `query_id`, so a server profile with `readonly=1` (which forbids changing
//! settings over HTTP) still works. Bounds are enforced by this client.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::StoredConnection;

/// Client-side bounds for one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    pub max_rows: usize,
    pub max_bytes: usize,
    pub timeout: Duration,
}

/// Why a result stopped before the server finished sending it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClickHouseTruncation {
    Rows,
    Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseColumn {
    pub name: String,
    /// ClickHouse type name, e.g. `Nullable(String)`; empty when unknown.
    pub type_name: String,
}

/// One bounded result. Cells are `None` for SQL `NULL`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseRows {
    pub columns: Vec<ClickHouseColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub truncated: Option<ClickHouseTruncation>,
    pub runtime_ms: u64,
    /// From `X-ClickHouse-Summary` when the server reports writes.
    pub written_rows: Option<u64>,
    /// Set on a table-browse page that has no stable order (no chosen sort
    /// and no sorting key): offset pages may repeat or skip rows.
    pub approximate_order: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClickHouseErrorKind {
    /// The endpoint could not be reached or the connection dropped. The
    /// session is no longer trusted; reconnecting is an explicit user action.
    Lost,
    Timeout,
    /// ClickHouse answered with an exception.
    Server,
    /// Refused before any request: closed session, policy, invalid input.
    Refused,
    /// The response could not be decoded.
    Protocol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClickHouseError {
    pub kind: ClickHouseErrorKind,
    pub message: String,
}

impl ClickHouseError {
    pub(crate) fn new(kind: ClickHouseErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
    pub(crate) fn refused(message: impl Into<String>) -> Self {
        Self::new(ClickHouseErrorKind::Refused, message)
    }
}

impl std::fmt::Display for ClickHouseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Longest server error body kept for display.
const ERROR_BODY_LIMIT: usize = 16 * 1024;
const FORMAT: &str = "JSONCompactEachRowWithNamesAndTypes";

fn transport(error: reqwest::Error) -> ClickHouseError {
    // `without_url` keeps hosts and query ids out of the message; credentials
    // travel in a header and never appear in either.
    if error.is_timeout() {
        return ClickHouseError::new(ClickHouseErrorKind::Timeout, "ClickHouse request timed out");
    }
    ClickHouseError::new(
        ClickHouseErrorKind::Lost,
        format!("ClickHouse connection failed: {}", error.without_url()),
    )
}

/// Runs one statement and reads at most `limits` of its result.
pub(crate) async fn run(
    connection: &StoredConnection,
    query: &str,
    query_id: Option<&str>,
    limits: Limits,
) -> Result<ClickHouseRows, ClickHouseError> {
    let ch = super::as_ch(connection).map_err(ClickHouseError::refused)?;
    let mut url = super::url(connection).map_err(ClickHouseError::refused)?;
    let pairs = url
        .query_pairs()
        .filter(|(key, _)| key != "default_format")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    {
        let mut query_pairs = url.query_pairs_mut();
        query_pairs.clear();
        query_pairs.extend_pairs(pairs);
        query_pairs.append_pair("default_format", FORMAT);
        if let Some(id) = query_id {
            query_pairs.append_pair("query_id", id);
        }
    }
    let mut request = super::shared_client()
        .post(url)
        .timeout(limits.timeout)
        .body(query.to_owned());
    if !ch.user.is_empty() {
        request = request.basic_auth(ch.user.clone(), Some(ch.password.clone()));
    }
    let started = Instant::now();
    let mut response = request.send().await.map_err(transport)?;
    let status = response.status();
    let written_rows = response
        .headers()
        .get("x-clickhouse-summary")
        .and_then(|value| value.to_str().ok())
        .and_then(written_rows);
    if !status.is_success() {
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport)? {
            body.extend_from_slice(&chunk);
            if body.len() >= ERROR_BODY_LIMIT {
                body.truncate(ERROR_BODY_LIMIT);
                break;
            }
        }
        let text = String::from_utf8_lossy(&body).trim().to_string();
        return Err(ClickHouseError::new(
            ClickHouseErrorKind::Server,
            if text.is_empty() {
                format!("ClickHouse returned HTTP {status}")
            } else {
                text
            },
        ));
    }
    let mut reader = RowReader::new(limits);
    while let Some(chunk) = response.chunk().await.map_err(transport)? {
        if reader.push(&chunk)? == Flow::Stop {
            break;
        }
    }
    let mut rows = reader.finish()?;
    rows.runtime_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    rows.written_rows = written_rows;
    Ok(rows)
}

/// Best-effort `KILL QUERY` for a cancelled request, bounded and never retried.
pub(crate) async fn kill(connection: &StoredConnection, query_id: &str) {
    let statement = format!(
        "KILL QUERY WHERE query_id = '{}' ASYNC",
        super::escape(query_id)
    );
    let limits = Limits {
        max_rows: 16,
        max_bytes: 64 * 1024,
        timeout: Duration::from_secs(5),
    };
    if let Err(error) = run(connection, &statement, None, limits).await {
        log::debug!(
            "ClickHouse KILL QUERY was not accepted: {}",
            error.kind_label()
        );
    }
}

impl ClickHouseError {
    fn kind_label(&self) -> &'static str {
        match self.kind {
            ClickHouseErrorKind::Lost => "lost",
            ClickHouseErrorKind::Timeout => "timeout",
            ClickHouseErrorKind::Server => "server",
            ClickHouseErrorKind::Refused => "refused",
            ClickHouseErrorKind::Protocol => "protocol",
        }
    }
}

fn written_rows(summary: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(summary).ok()?;
    let written = value.get("written_rows")?;
    let count = written
        .as_u64()
        .or_else(|| written.as_str().and_then(|text| text.parse().ok()))?;
    (count > 0).then_some(count)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    Continue,
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
enum Stage {
    Names,
    Types,
    Rows,
    /// The statement chose its own `FORMAT`; lines are shown verbatim.
    Text,
}

/// Incremental line decoder. Pure, so the bounds are unit-tested without a
/// server.
pub(crate) struct RowReader {
    limits: Limits,
    stage: Stage,
    pending: Vec<u8>,
    /// Bytes of `pending` already known to hold no newline, so a long line
    /// split across many chunks is scanned once, not once per chunk.
    scanned: usize,
    consumed: usize,
    result: ClickHouseRows,
    done: bool,
}

impl RowReader {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            stage: Stage::Names,
            pending: Vec::new(),
            scanned: 0,
            consumed: 0,
            result: ClickHouseRows::default(),
            done: false,
        }
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<Flow, ClickHouseError> {
        if self.done {
            return Ok(Flow::Stop);
        }
        self.pending.extend_from_slice(chunk);
        let mut start = 0;
        let mut search = self.scanned;
        while let Some(offset) = self.pending[search..].iter().position(|&b| b == b'\n') {
            let end = search + offset;
            if self.consumed + end + 1 > self.limits.max_bytes {
                return Ok(self.stop(ClickHouseTruncation::Bytes));
            }
            let line = std::mem::take(&mut self.pending);
            let flow = self.line(&line[start..end]);
            self.pending = line;
            if flow? == Flow::Stop {
                return Ok(Flow::Stop);
            }
            start = end + 1;
            search = start;
        }
        self.consumed += start;
        self.pending.drain(..start);
        self.scanned = self.pending.len();
        if self.consumed + self.pending.len() > self.limits.max_bytes {
            return Ok(self.stop(ClickHouseTruncation::Bytes));
        }
        Ok(Flow::Continue)
    }

    pub(crate) fn finish(mut self) -> Result<ClickHouseRows, ClickHouseError> {
        if !self.done && !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.line(&line)?;
        }
        Ok(self.result)
    }

    fn stop(&mut self, reason: ClickHouseTruncation) -> Flow {
        self.done = true;
        self.result.truncated = Some(reason);
        self.pending.clear();
        self.scanned = 0;
        Flow::Stop
    }

    fn line(&mut self, raw: &[u8]) -> Result<Flow, ClickHouseError> {
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        if raw.iter().all(u8::is_ascii_whitespace) && self.stage != Stage::Text {
            return Ok(Flow::Continue);
        }
        let parsed = serde_json::from_slice::<Vec<serde_json::Value>>(raw).ok();
        let Some(values) = parsed.filter(|_| self.stage != Stage::Text) else {
            let text = String::from_utf8_lossy(raw);
            if text.contains("DB::Exception") || text.trim_start().starts_with("Code:") {
                return Err(ClickHouseError::new(
                    ClickHouseErrorKind::Server,
                    text.trim().to_string(),
                ));
            }
            return match self.stage {
                Stage::Names | Stage::Text => {
                    if self.stage == Stage::Names {
                        self.stage = Stage::Text;
                        self.result.columns = vec![ClickHouseColumn {
                            name: "result".into(),
                            type_name: String::new(),
                        }];
                    }
                    self.row(vec![Some(text.into_owned())])
                }
                Stage::Types | Stage::Rows => Err(ClickHouseError::new(
                    ClickHouseErrorKind::Protocol,
                    "ClickHouse sent a row that is not valid JSON",
                )),
            };
        };
        match self.stage {
            Stage::Names => {
                self.result.columns = values
                    .iter()
                    .map(|value| ClickHouseColumn {
                        name: cell(value).unwrap_or_default(),
                        type_name: String::new(),
                    })
                    .collect();
                self.stage = Stage::Types;
                Ok(Flow::Continue)
            }
            Stage::Types => {
                for (column, value) in self.result.columns.iter_mut().zip(&values) {
                    column.type_name = cell(value).unwrap_or_default();
                }
                self.stage = Stage::Rows;
                Ok(Flow::Continue)
            }
            Stage::Rows => self.row(values.iter().map(cell).collect()),
            Stage::Text => unreachable!("text lines never parse as rows"),
        }
    }

    fn row(&mut self, row: Vec<Option<String>>) -> Result<Flow, ClickHouseError> {
        if self.result.rows.len() >= self.limits.max_rows {
            return Ok(self.stop(ClickHouseTruncation::Rows));
        }
        self.result.rows.push(row);
        Ok(Flow::Continue)
    }
}

fn cell(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => Some(value.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(max_rows: usize, max_bytes: usize) -> Limits {
        Limits {
            max_rows,
            max_bytes,
            timeout: Duration::from_secs(1),
        }
    }

    fn read(chunks: &[&[u8]], limits: Limits) -> Result<ClickHouseRows, ClickHouseError> {
        let mut reader = RowReader::new(limits);
        for chunk in chunks {
            if reader.push(chunk)? == Flow::Stop {
                break;
            }
        }
        reader.finish()
    }

    #[test]
    fn decodes_names_types_and_nulls_across_split_chunks() {
        let rows = read(
            &[
                b"[\"id\",\"name\",\"tags\"]\n[\"UInt64\",\"Nullable(Str",
                b"ing)\",\"Array(String)\"]\n[\"1\",null,[\"a\"]]\n[2,\"x\",[]]",
            ],
            limits(10, 1 << 20),
        )
        .unwrap();
        assert_eq!(rows.columns[1].name, "name");
        assert_eq!(rows.columns[1].type_name, "Nullable(String)");
        assert_eq!(
            rows.rows,
            vec![
                vec![Some("1".into()), None, Some("[\"a\"]".into())],
                vec![Some("2".into()), Some("x".into()), Some("[]".into())],
            ]
        );
        assert_eq!(rows.truncated, None);
    }

    #[test]
    fn row_cap_stops_reading_and_marks_truncation() {
        let mut body = b"[\"n\"]\n[\"UInt8\"]\n".to_vec();
        for n in 0..10 {
            body.extend_from_slice(format!("[{n}]\n").as_bytes());
        }
        let rows = read(&[&body], limits(3, 1 << 20)).unwrap();
        assert_eq!(rows.rows.len(), 3);
        assert_eq!(rows.truncated, Some(ClickHouseTruncation::Rows));
        // Exactly the cap is not a truncation.
        let exact = read(&[b"[\"n\"]\n[\"UInt8\"]\n[1]\n[2]\n"], limits(2, 1 << 20)).unwrap();
        assert_eq!(exact.rows.len(), 2);
        assert_eq!(exact.truncated, None);
    }

    #[test]
    fn byte_cap_holds_even_without_a_newline() {
        let header = b"[\"s\"]\n[\"String\"]\n[\"short\"]\n";
        let huge = vec![b'x'; 4096];
        let rows = read(&[header, b"[\"", &huge, &huge], limits(100, 1024)).unwrap();
        assert_eq!(rows.rows, vec![vec![Some("short".into())]]);
        assert_eq!(rows.truncated, Some(ClickHouseTruncation::Bytes));
    }

    #[test]
    fn a_line_split_across_many_chunks_is_scanned_once() {
        let value = "v".repeat(5_000);
        let body = format!("[\"s\"]\n[\"String\"]\n[\"{value}\"]\n[\"tail\"]\n");
        let mut reader = RowReader::new(limits(10, 1 << 20));
        for byte in body.as_bytes() {
            assert_eq!(
                reader.push(std::slice::from_ref(byte)).unwrap(),
                Flow::Continue
            );
            // Everything still pending was already searched for a newline.
            assert_eq!(reader.scanned, reader.pending.len());
        }
        let rows = reader.finish().unwrap();
        assert_eq!(
            rows.rows,
            vec![vec![Some(value)], vec![Some("tail".into())]]
        );
        assert_eq!(rows.truncated, None);

        // Uneven chunks that end mid-line and mid-newline-run still decode.
        let mut reader = RowReader::new(limits(10, 1 << 20));
        for chunk in body.as_bytes().chunks(7) {
            reader.push(chunk).unwrap();
        }
        assert_eq!(reader.finish().unwrap().rows.len(), 2);
    }

    #[test]
    fn exceptions_in_the_stream_are_server_errors() {
        let error = read(
            &[b"[\"n\"]\n[\"UInt8\"]\n[1]\nCode: 395. DB::Exception: boom\n"],
            limits(10, 1 << 20),
        )
        .unwrap_err();
        assert_eq!(error.kind, ClickHouseErrorKind::Server);
        assert!(error.message.contains("boom"));
    }

    #[test]
    fn explicit_formats_and_empty_bodies_are_kept_verbatim() {
        let text = read(&[b"a,b\n1,2\n"], limits(10, 1 << 20)).unwrap();
        assert_eq!(text.columns[0].name, "result");
        assert_eq!(
            text.rows,
            vec![vec![Some("a,b".into())], vec![Some("1,2".into())]]
        );
        let empty = read(&[], limits(10, 1 << 20)).unwrap();
        assert!(empty.columns.is_empty() && empty.rows.is_empty());
    }

    #[test]
    fn summary_reports_only_positive_writes() {
        assert_eq!(written_rows(r#"{"written_rows":"12"}"#), Some(12));
        assert_eq!(written_rows(r#"{"written_rows":"0"}"#), None);
        assert_eq!(written_rows("nope"), None);
    }
}
