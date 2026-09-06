//! Bounded PG16 catalog capture. The job manager (Step 5) will own admission,
//! endpoint resolution/generations and this future. Nothing registers commands.
mod data;
mod queries;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;

use super::{budget::*, protocol::*};
use crate::postgres::{connect_spec::ResolvedPostgresConnectSpec, dedicated};
pub use data::{CapturedEndpoint, CapturedField, CapturedValue, ExcludedCount};
use std::{future::Future, time::Duration};
use tokio::{sync::watch, time::Instant};
use tokio_postgres::{types::ToSql, Client, Row};

const BATCH_ROWS: i64 = 64;
const BATCH_BYTES: i64 = 2 * 1024 * 1024;
// Covers bounded driver response buffers, row envelopes, JSON decoder scratch,
// query/parameter storage and transfer into retained, separately charged fields.
const CAPTURE_SCRATCH: usize = 32 * 1024 * 1024;
const TABLE_BATCH: usize = 32;
const CLEANUP_GRACE: Duration = Duration::from_secs(5);

pub struct CaptureControl {
    deadline: Instant,
    cancel: watch::Receiver<bool>,
}

impl CaptureControl {
    /// The owner supplies its original admission deadline, including resolution.
    pub fn new(deadline: Instant, cancel: watch::Receiver<bool>) -> Self {
        Self { deadline, cancel }
    }

    pub fn check(&self) -> Result<(), CompareError> {
        if *self.cancel.borrow() {
            Err(CompareError::Cancelled)
        } else if Instant::now() >= self.deadline {
            Err(CompareError::DeadlineExceeded)
        } else {
            Ok(())
        }
    }

    async fn wait<T>(&self, future: impl Future<Output = T>) -> Result<T, CompareError> {
        self.check()?;
        let mut cancel = self.cancel.clone();
        tokio::select! {
            biased;
            _ = async { loop {
                if *cancel.borrow_and_update() { break; }
                if cancel.changed().await.is_err() { std::future::pending::<()>().await; }
            }} => Err(CompareError::Cancelled),
            _ = tokio::time::sleep_until(self.deadline) => Err(CompareError::DeadlineExceeded),
            result = future => Ok(result),
        }
    }

    fn statement_ms(&self, configured: Option<u32>) -> Result<u64, CompareError> {
        self.check()?;
        Ok(self
            .deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(10_000)
            .min(u128::from(configured.filter(|n| *n > 0).unwrap_or(10_000)))
            .max(1) as u64)
    }
}

/// Uses exactly one resolved native transport for one stored connection. Two
/// same-connection schemas share discovery, pre-snapshot locks and transaction.
/// Independent connections call this separately and retain independent times.
#[allow(dead_code)] // Dark entry point until Plan 021 Step 5 registers its owner.
pub(crate) async fn capture_resolved(
    spec: &ResolvedPostgresConnectSpec,
    endpoints: &[(Side, &Endpoint)],
    control: &CaptureControl,
    budget: &Budget,
) -> Result<Vec<CapturedEndpoint>, CompareError> {
    validate_endpoints(&spec.connection_id, endpoints)?;
    let budget = budget.result_scope();
    let _scratch = budget.scratch(CAPTURE_SCRATCH)?;
    let connection = control
        .wait(dedicated::connect(spec, dedicated::NoticeSink::Ignore))
        .await?
        .map_err(|_| CompareError::Unavailable)?;
    let result = capture(
        &connection.client,
        endpoints,
        control,
        &budget,
        spec.driver_options.statement_timeout_ms,
    )
    .await;
    // The connection stays owned throughout cancellation/rollback. Closing
    // joins (or aborts and joins) the dedicated driver before returning capacity.
    let cleanup_deadline = Instant::now() + CLEANUP_GRACE - Duration::from_secs(2);
    if result.is_err() {
        let _ = tokio::time::timeout_at(
            cleanup_deadline,
            dedicated::cancel(connection.cancel.clone(), connection.tls.clone()),
        )
        .await;
        let _ = tokio::time::timeout_at(
            cleanup_deadline,
            connection.client.batch_execute("ROLLBACK"),
        )
        .await;
    }
    connection.close().await;
    result
}

fn validate_endpoints(
    connection_id: &str,
    endpoints: &[(Side, &Endpoint)],
) -> Result<(), CompareError> {
    if endpoints.is_empty()
        || endpoints.len() > 2
        || endpoints.iter().any(|(_, e)| {
            e.connection_id != connection_id
                || e.connection_id.is_empty()
                || e.connection_id.len() > 128
                || e.schema.is_empty()
                || e.schema.len() > 63
                || e.schema.contains('\0')
        })
        || (endpoints.len() == 2 && endpoints[0].0 == endpoints[1].0)
    {
        return Err(CompareError::InvalidRequest);
    }
    Ok(())
}

async fn execute(client: &Client, control: &CaptureControl, sql: &str) -> Result<(), CompareError> {
    control
        .wait(client.batch_execute(sql))
        .await?
        .map_err(database_error)
}

async fn query(
    client: &Client,
    control: &CaptureControl,
    sql: &str,
    params: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, CompareError> {
    control
        .wait(client.query(sql, params))
        .await?
        .map_err(database_error)
}

fn database_error(error: tokio_postgres::Error) -> CompareError {
    // Required reads never become empty inventories. These catalog/lock races
    // can be retried once, with a fresh transaction and the original deadline.
    match error.code().map(|c| c.code()) {
        Some("42P01" | "42704" | "42809" | "55P03" | "40001") => CompareError::CaptureChanged,
        _ => CompareError::Unavailable,
    }
}

async fn settings(
    client: &Client,
    control: &CaptureControl,
    configured: Option<u32>,
    local: bool,
) -> Result<(), CompareError> {
    let ms = control.statement_ms(configured)?;
    let local = if local { "LOCAL " } else { "" };
    execute(
        client,
        control,
        &format!(
            "SET {local}statement_timeout = '{ms}ms'; SET {local}lock_timeout = '{}ms'",
            ms.min(2000)
        ),
    )
    .await
}

async fn capture(
    client: &Client,
    endpoints: &[(Side, &Endpoint)],
    control: &CaptureControl,
    budget: &Budget,
    configured: Option<u32>,
) -> Result<Vec<CapturedEndpoint>, CompareError> {
    execute(client, control, "SET search_path = pg_catalog").await?;
    settings(client, control, configured, false).await?;
    // SHOW is version independent. Limit the version display before retaining it.
    let version = query(client, control, "SELECT current_setting('server_version_num')::integer, left(current_setting('server_version'),128)", &[]).await?;
    let number: i32 = version[0]
        .try_get(0)
        .map_err(|_| CompareError::Unavailable)?;
    let display: &str = version[0]
        .try_get(1)
        .map_err(|_| CompareError::Unavailable)?;
    if number / 10_000 != 16 {
        return Err(CompareError::UnsupportedVersion {
            side: endpoints[0].0,
            version: display.into(),
        });
    }
    for attempt in 0..2 {
        let result = attempt_capture(
            client,
            endpoints,
            control,
            budget,
            configured,
            number as u32,
            display,
        )
        .await;
        // No capture can escape while its transaction or locks remain active.
        control
            .wait(client.batch_execute("ROLLBACK"))
            .await?
            .map_err(|_| CompareError::Unavailable)?;
        match result {
            Err(CompareError::CaptureChanged) if attempt == 0 => continue,
            result => return result,
        }
    }
    unreachable!("at most one fresh retry")
}

async fn schema_oid(
    client: &Client,
    control: &CaptureControl,
    schema: &str,
) -> Result<u32, CompareError> {
    let rows = query(client, control, queries::SCHEMA, &[&schema]).await?;
    let Some(row) = rows.first() else {
        return Err(CompareError::Unavailable);
    };
    if !row
        .try_get::<_, bool>(1)
        .map_err(|_| CompareError::Unavailable)?
    {
        return Err(CompareError::Unavailable);
    }
    row.try_get(0).map_err(|_| CompareError::Unavailable)
}

async fn inventory(
    client: &Client,
    control: &CaptureControl,
    schema: u32,
) -> Result<Vec<Row>, CompareError> {
    let rows = query(
        client,
        control,
        queries::INVENTORY,
        &[&schema, &((INVENTORY_ENTRIES + 1) as i64)],
    )
    .await?;
    if rows.len() > INVENTORY_ENTRIES {
        return Err(CompareError::LimitExceeded {
            limit: Limit::Inventory,
        });
    }
    let mut tables = 0;
    for row in &rows {
        if eligibility(row)? == Eligibility::Eligible {
            tables += 1;
        }
    }
    if tables > TABLE_ENTRIES {
        return Err(CompareError::LimitExceeded {
            limit: Limit::Tables,
        });
    }
    Ok(rows)
}

fn eligibility(row: &Row) -> Result<Eligibility, CompareError> {
    let kind: &str = row.try_get("kind").map_err(|_| CompareError::Unavailable)?;
    let partition: bool = row
        .try_get("partition_member")
        .map_err(|_| CompareError::Unavailable)?;
    let inherited: bool = row
        .try_get("inherited")
        .map_err(|_| CompareError::Unavailable)?;
    let extension: bool = row
        .try_get("extension_owned")
        .map_err(|_| CompareError::Unavailable)?;
    let reason = if extension {
        Some(Exclusion::ExtensionOwned)
    } else {
        match kind {
            "p" => Some(Exclusion::Partitioned),
            "r" if partition => Some(Exclusion::Partitioned),
            "f" => Some(Exclusion::Foreign),
            "r" if inherited => Some(Exclusion::Inherited),
            "r" => None,
            "v" | "m" | "S" | "c" => Some(Exclusion::OtherKind),
            _ => return Err(CompareError::Unavailable),
        }
    };
    Ok(
        reason.map_or(Eligibility::Eligible, |reason| Eligibility::Excluded {
            reason,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
async fn attempt_capture(
    client: &Client,
    endpoints: &[(Side, &Endpoint)],
    control: &CaptureControl,
    budget: &Budget,
    configured: Option<u32>,
    version: u32,
    display: &str,
) -> Result<Vec<CapturedEndpoint>, CompareError> {
    let mut discovered = Vec::with_capacity(2);
    for (_, endpoint) in endpoints {
        settings(client, control, configured, false).await?;
        let schema = schema_oid(client, control, &endpoint.schema).await?;
        discovered.push(inventory(client, control, schema).await?);
    }
    execute(client, control, queries::BEGIN).await?;
    // No SELECT, set_config(), prepared expression or snapshot-producing query
    // is permitted between BEGIN and the final table lock.
    for ((_, endpoint), rows) in endpoints.iter().zip(&discovered) {
        for chunk in rows.chunks(TABLE_BATCH) {
            settings(client, control, configured, true).await?;
            let mut sql = String::from("LOCK TABLE ");
            let mut count = 0;
            for row in chunk {
                if eligibility(row)? != Eligibility::Eligible {
                    continue;
                }
                if count > 0 {
                    sql.push_str(", ");
                }
                let name: &str = row.try_get("name").map_err(|_| CompareError::Unavailable)?;
                sql.push_str("ONLY ");
                sql.push_str(&crate::quote_double(&endpoint.schema));
                sql.push('.');
                sql.push_str(&crate::quote_double(name));
                count += 1;
            }
            if count > 0 {
                sql.push_str(" IN ACCESS SHARE MODE");
                execute(client, control, &sql).await?;
            }
        }
    }
    let mut captures = Vec::with_capacity(endpoints.len());
    let snapshot = std::sync::Arc::new(());
    for (_, endpoint) in endpoints {
        settings(client, control, configured, true).await?;
        let schema = schema_oid(client, control, &endpoint.schema).await?;
        let rows = inventory(client, control, schema).await?;
        let time = query(client, control, "SELECT transaction_timestamp()::text", &[]).await?;
        let captured_at: String = time[0].try_get(0).map_err(|_| CompareError::Unavailable)?;
        let mut captured = CapturedEndpoint::new(
            CaptureMetadata {
                endpoint: (*endpoint).clone(),
                server_version: display.into(),
                server_version_num: version,
                captured_at,
            },
            budget,
        )?;
        captured.snapshot = snapshot.clone();
        for row in rows {
            control.check()?;
            let eligible = eligibility(&row)?;
            if eligible == Eligibility::Eligible {
                let locked: bool = row
                    .try_get("locked")
                    .map_err(|_| CompareError::Unavailable)?;
                let readable: bool = row
                    .try_get("readable")
                    .map_err(|_| CompareError::Unavailable)?;
                if !locked {
                    return Err(CompareError::CaptureChanged);
                }
                if !readable {
                    return Err(CompareError::Unavailable);
                }
            } else {
                captured.coverage.excluded_relations += 1;
            }
            let kind = match row
                .try_get::<_, &str>("kind")
                .map_err(|_| CompareError::Unavailable)?
            {
                "r" => RelationKind::Table,
                "p" => RelationKind::PartitionedTable,
                "f" => RelationKind::ForeignTable,
                "v" => RelationKind::View,
                "m" => RelationKind::MaterializedView,
                "S" => RelationKind::Sequence,
                "c" => RelationKind::Composite,
                _ => return Err(CompareError::Unavailable),
            };
            captured
                .oids
                .push(row.try_get("oid").map_err(|_| CompareError::Unavailable)?);
            let name: &str = row.try_get("name").map_err(|_| CompareError::Unavailable)?;
            captured.inventory.push(InventoryEntry {
                identity: RelationIdentity {
                    kind,
                    name: name.into(),
                },
                eligibility: eligible,
            });
        }
        capture_fields(client, control, configured, &mut captured).await?;
        queries::excluded_counts(client, control, schema, &mut captured).await?;
        captured.certify(|| control.check())?;
        captures.push(captured);
    }
    Ok(captures)
}

async fn capture_fields(
    client: &Client,
    control: &CaptureControl,
    configured: Option<u32>,
    captured: &mut CapturedEndpoint,
) -> Result<(), CompareError> {
    let tables: Vec<_> = captured
        .oids
        .iter()
        .zip(&captured.inventory)
        .filter_map(|(oid, row)| (row.eligibility == Eligibility::Eligible).then_some(*oid))
        .collect();
    for tables in tables.chunks(TABLE_BATCH) {
        let tables = tables.to_vec();
        let mut offset = 0_i64;
        loop {
            settings(client, control, configured, true).await?;
            let rows = query(
                client,
                control,
                include_str!("facts.sql"),
                &[
                    &tables,
                    &captured.metadata.endpoint.schema,
                    &offset,
                    &BATCH_ROWS,
                    &(FIELD_BYTES as i32),
                    &BATCH_BYTES,
                ],
            )
            .await?;
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                control.check()?;
                if row
                    .try_get::<_, bool>(0)
                    .map_err(|_| CompareError::Unavailable)?
                {
                    return Err(CompareError::LimitExceeded {
                        limit: Limit::FieldBytes,
                    });
                }
                if row
                    .try_get::<_, bool>(2)
                    .map_err(|_| CompareError::Unavailable)?
                {
                    return Err(CompareError::Unavailable);
                }
                let text: &str = row.try_get(1).map_err(|_| CompareError::Unavailable)?;
                if text.len() > BATCH_BYTES as usize {
                    return Err(CompareError::LimitExceeded {
                        limit: Limit::FieldBytes,
                    });
                }
                let wire = serde_json::from_str(text).map_err(|_| CompareError::Unavailable)?;
                captured.push(wire)?;
            }
            offset += rows.len() as i64;
        }
    }
    Ok(())
}
