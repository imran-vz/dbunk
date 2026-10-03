//! Local saved metadata only. No credential reader or PostgreSQL operation.
use dbunk_lib::backend::DevelopmentConnection;
use std::{
    cell::Cell,
    fmt::{self, Write},
    rc::Rc,
};
const SHARED_BYTES: usize = 128 * 1024 * 1024;
const TEXT_BYTES: usize = 64 * 1024;
const MAX_ROWS: usize = 100;
struct Row {
    label: &'static str,
    value: String,
}
pub struct Capture {
    id: String,
    rows: Vec<Row>,
    editable: bool,
    budget: Rc<Cell<usize>>,
    retained: usize,
}
/// Rechecked against the latest record when dispatching Edit, not just when
/// displaying a previously captured row. Unsupported engines never get defaults.
pub fn editable_connection<'a>(
    connections: &'a [DevelopmentConnection],
    id: &str,
) -> Option<&'a DevelopmentConnection> {
    connections.iter().find(|c| {
        c.id == id
            && c.engine == "PostgreSQL"
            && c.postgres.is_some()
            && c.unsupported_reason.is_none()
    })
}
impl Capture {
    pub fn new(
        connection: &DevelopmentConnection,
        expected_id: &str,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, &'static str> {
        if connection.id != expected_id || connection.id.len() > 256 {
            return Err("Saved connection identity does not match this tab");
        }
        let mut total = 0usize;
        let mut largest = 0usize;
        let mut lines = 0usize;
        let mut count = 0usize;
        visit(connection, |label, value| {
            let mut counter = Counter::default();
            write!(&mut counter, "{label}\n{value}")?;
            total = total.checked_add(counter.bytes).ok_or(fmt::Error)?;
            largest = largest.max(counter.bytes);
            lines = lines.max(counter.lines + 1);
            count += 1;
            if total > TEXT_BYTES || count > MAX_ROWS {
                return Err(fmt::Error);
            }
            Ok(())
        })
        .map_err(|_| "Saved connection metadata exceeds display bounds")?;
        // Current/replacement editor text and logical AX runs, with soft wrap
        // disabled. Conservative payload reservation, not a process RSS bound.
        let retained = total
            .checked_mul(2)
            .and_then(|n| n.checked_add(largest.checked_mul(64)?))
            .and_then(|n| n.checked_add(lines.checked_mul(4096)?))
            .and_then(|n| {
                n.checked_add(64 * 1024 + connection.id.len() + count * std::mem::size_of::<Row>())
            })
            .ok_or("Saved connection metadata exceeds display bounds")?;
        if retained > SHARED_BYTES.saturating_sub(budget.get()) {
            return Err("Connection settings need shared memory; previous capture retained");
        }
        budget.set(budget.get() + retained);
        let mut capture = Self {
            id: connection.id.clone(),
            rows: Vec::with_capacity(count),
            editable: editable_connection(std::slice::from_ref(connection), expected_id).is_some(),
            budget,
            retained,
        };
        visit(connection, |label, value| {
            capture.rows.push(Row {
                label,
                value: value.to_string(),
            });
            Ok(())
        })
        .map_err(|_| "Could not format saved connection metadata")?;
        let actual = capture
            .rows
            .iter()
            .try_fold(
                std::mem::size_of::<Self>()
                    + capture.id.capacity()
                    + capture.rows.capacity() * std::mem::size_of::<Row>(),
                |n, row| n.checked_add(row.value.capacity()),
            )
            .ok_or("Saved connection metadata exceeds display bounds")?;
        if actual > 2 * total + 64 * 1024 + connection.id.len() + count * std::mem::size_of::<Row>()
        {
            return Err("Saved connection metadata exceeds display bounds");
        }
        Ok(capture)
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn editable(&self) -> bool {
        self.editable
    }
    pub fn count(&self) -> usize {
        self.rows.len()
    }
    pub fn label(&self, index: usize) -> Option<&'static str> {
        self.rows.get(index).map(|r| r.label)
    }
    pub fn details(&self, index: usize) -> Option<String> {
        self.rows
            .get(index)
            .map(|r| format!("{}\n{}", r.label, r.value))
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.retained));
    }
}
#[derive(Default)]
struct Counter {
    bytes: usize,
    lines: usize,
}
impl Write for Counter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        self.bytes = self.bytes.checked_add(value.len()).ok_or(fmt::Error)?;
        if self.bytes > TEXT_BYTES {
            return Err(fmt::Error);
        }
        self.lines = self
            .lines
            .checked_add(value.bytes().filter(|b| *b == b'\n' || *b == b'\r').count())
            .ok_or(fmt::Error)?;
        Ok(())
    }
}
struct Optional<'a, T>(&'a Option<T>);
impl<T: fmt::Display> fmt::Display for Optional<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(value) => write!(f, "Configured: {value}"),
            None => f.write_str("Not configured"),
        }
    }
}
fn visit(
    c: &DevelopmentConnection,
    mut row: impl FnMut(&'static str, fmt::Arguments<'_>) -> fmt::Result,
) -> fmt::Result {
    row("Connection ID", format_args!("{}", c.id))?;
    row("Name", format_args!("{}", c.name))?;
    row("Engine", format_args!("{}", c.engine))?;
    row("Folder", format_args!("{}", c.organization.folder))?;
    row("Favorite", format_args!("{}", c.organization.is_favorite))?;
    row("Color", format_args!("{}", c.organization.color))?;
    if let Some(reason) = &c.unsupported_reason {
        row("Unavailable", format_args!("{reason}"))?;
    }
    let Some(p) = &c.postgres else {
        return row(
            "PostgreSQL settings",
            format_args!("Unavailable; this record cannot be edited through the PostgreSQL form"),
        );
    };
    row("Host", format_args!("{}", p.host))?;
    row("Port", format_args!("{}", p.port))?;
    row("Database", format_args!("{}", p.database))?;
    row("User", format_args!("{}", p.user))?;
    row("Environment", format_args!("{:?}", p.environment))?;
    row("Configured safe mode", format_args!("{:?}", p.safe_mode))?;
    row("Read only", format_args!("{}", p.read_only))?;
    row("TLS mode", format_args!("{:?}", p.tls.mode))?;
    row(
        "TLS server name",
        format_args!("{}", Optional(&p.tls.server_name)),
    )?;
    row(
        "Root certificate path",
        format_args!("{}", Optional(&p.tls.root_cert_path)),
    )?;
    row(
        "Client certificate path",
        format_args!("{}", Optional(&p.tls.client_cert_path)),
    )?;
    row(
        "Client key path",
        format_args!("{}", Optional(&p.tls.client_key_path)),
    )?;
    let d = &p.driver_options;
    row(
        "Statement timeout (ms)",
        format_args!("{}", Optional(&d.statement_timeout_ms)),
    )?;
    row(
        "Idle transaction timeout (ms)",
        format_args!("{}", Optional(&d.idle_in_transaction_timeout_ms)),
    )?;
    row(
        "Connect timeout (ms)",
        format_args!("{}", Optional(&d.connect_timeout_ms)),
    )?;
    row(
        "Keepalive (seconds)",
        format_args!("{}", Optional(&d.keepalive_seconds)),
    )?;
    row(
        "Default role",
        format_args!("{}", Optional(&d.default_role)),
    )?;
    match &d.default_search_path {
        None => row("Default search path", format_args!("Not configured"))?,
        Some(paths) => {
            if paths.len() > 64 {
                return Err(fmt::Error);
            }
            row(
                "Default search path",
                format_args!(
                    "Configured: {} entries, in the following order",
                    paths.len()
                ),
            )?;
            for (index, path) in paths.iter().enumerate() {
                row(
                    "Search path entry",
                    format_args!("Position {}\n{}", index + 1, path),
                )?;
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
