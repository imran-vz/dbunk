use serde::Deserialize;
use sqlx::Connection;
use tauri::State;

use crate::postgres::object_ddl::{
    derived_index_name, generate_object_ddl, statement_summaries, CreateIndexOp, DdlApplyResult,
    DdlPlanPreview, DdlResidue, DropIndexOp, PgObjectError, PgObjectOp, StatementGroup,
};
use crate::postgres::objects::{PgDropImpact, PgObjectCatalog, PgObjectDescription, PgObjectRef};
use crate::safety::policy::{assert_permitted, AuditDisposition, SafetyAuthorization, WriteIntent};
use crate::{AppState, ConnectionPayload};

use super::{find_connection, touch_connection_activity};

#[tauri::command]
pub async fn load_pg_object_catalog(
    state: State<'_, AppState>,
    payload: ConnectionPayload,
) -> Result<PgObjectCatalog, PgObjectError> {
    load_pg_object_catalog_inner(state.inner(), &payload.connection_id).await
}

pub(crate) async fn load_pg_object_catalog_inner(
    state: &AppState,
    connection_id: &str,
) -> Result<PgObjectCatalog, PgObjectError> {
    let connection = find_connection(state, connection_id)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let catalog = crate::postgres::objects::load_pg_object_catalog(&connection).await?;
    touch_connection_activity(state, connection_id).await;
    Ok(catalog)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PgObjectReferencePayload {
    connection_id: String,
    reference: PgObjectRef,
}

#[tauri::command]
pub async fn describe_pg_object(
    state: State<'_, AppState>,
    payload: PgObjectReferencePayload,
) -> Result<PgObjectDescription, PgObjectError> {
    let connection = find_connection(state.inner(), &payload.connection_id)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let description =
        crate::postgres::objects::describe_pg_object(&connection, payload.reference).await?;
    touch_connection_activity(state.inner(), &payload.connection_id).await;
    Ok(description)
}

#[tauri::command]
pub async fn load_pg_drop_impact(
    state: State<'_, AppState>,
    payload: PgObjectReferencePayload,
) -> Result<PgDropImpact, PgObjectError> {
    let connection = find_connection(state.inner(), &payload.connection_id)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let impact =
        crate::postgres::objects::load_pg_drop_impact(&connection, payload.reference).await?;
    touch_connection_activity(state.inner(), &payload.connection_id).await;
    Ok(impact)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreviewObjectDdlPayload {
    connection_id: String,
    ops: Vec<PgObjectOp>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyObjectDdlPayload {
    pub(crate) connection_id: String,
    pub(crate) ops: Vec<PgObjectOp>,
    pub(crate) confirmed: bool,
}

#[tauri::command]
pub async fn preview_object_ddl(
    state: State<'_, AppState>,
    payload: PreviewObjectDdlPayload,
) -> Result<DdlPlanPreview, PgObjectError> {
    preview_object_ddl_inner(state.inner(), &payload.connection_id, &payload.ops).await
}

pub(crate) async fn preview_object_ddl_inner(
    state: &AppState,
    connection_id: &str,
    ops: &[PgObjectOp],
) -> Result<DdlPlanPreview, PgObjectError> {
    // Preview is pure. Read only the persisted record so credential stores and
    // SSH tunnel resolution cannot turn SQL generation into an I/O operation.
    let connection = crate::storage::read_connection_by_id(&state.pool, connection_id)
        .await
        .map_err(|message| PgObjectError::Connection { message })?
        .ok_or_else(|| PgObjectError::Connection {
            message: "Connection not found".into(),
        })?;
    ensure_postgres(&connection)?;
    generate_object_ddl(ops)
}

#[tauri::command]
pub async fn apply_object_ddl(
    state: State<'_, AppState>,
    payload: ApplyObjectDdlPayload,
) -> Result<DdlApplyResult, PgObjectError> {
    apply_object_ddl_inner(state.inner(), payload).await
}

pub(crate) async fn apply_object_ddl_inner(
    state: &AppState,
    payload: ApplyObjectDdlPayload,
) -> Result<DdlApplyResult, PgObjectError> {
    let connection = find_connection(state, &payload.connection_id)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    ensure_postgres(&connection)?;
    // Apply regenerates from operations instead of trusting a preview or SQL
    // statement supplied by the frontend.
    let preview = generate_object_ddl(&payload.ops)?;
    let authorization = authorize_object_ddl(&connection, &preview, payload.confirmed)?;
    let started = std::time::Instant::now();
    let outcome = execute_ddl_plan(&connection, &payload.ops, &preview).await;

    // Groups commit independently, so a failure after the first committed
    // group is still a schema change made under this override. Audit whenever
    // anything was applied, not only when the whole plan succeeded.
    let applied_statements = match &outcome {
        Ok(applied_statements) => *applied_statements,
        Err(error) => error.applied_statements(),
    };
    if outcome.is_ok() || applied_statements > 0 {
        if authorization.audit_disposition() == AuditDisposition::RequiredAfterSuccess {
            super::safety::record_override(
                &state.pool,
                &payload.connection_id,
                "apply_object_ddl",
                &WriteIntent::Ddl,
            )
            .await;
        }
        touch_connection_activity(state, &payload.connection_id).await;
    }
    let applied_statements = outcome?;
    Ok(DdlApplyResult {
        applied_statements,
        runtime_ms: started.elapsed().as_millis() as u64,
    })
}

fn ensure_postgres(connection: &crate::StoredConnection) -> Result<(), PgObjectError> {
    if connection.engine() == crate::DatabaseEngine::PostgreSQL {
        return Ok(());
    }
    Err(PgObjectError::UnsupportedEngine {
        engine: format!("{:?}", connection.engine()),
    })
}

fn authorize_object_ddl(
    connection: &crate::StoredConnection,
    preview: &DdlPlanPreview,
    confirmed: bool,
) -> Result<SafetyAuthorization, PgObjectError> {
    let summaries = statement_summaries(preview);
    assert_permitted(
        &super::safety::resolved_policy(connection),
        &WriteIntent::Ddl,
        confirmed,
    )
    .map_err(|refusal| {
        refusal.fold(
            |reason, _| PgObjectError::PolicyBlocked {
                reason: reason.to_string(),
            },
            |_| PgObjectError::PolicyNeedsConfirmation {
                statements: summaries,
            },
        )
    })
}

async fn execute_ddl_plan(
    connection: &crate::StoredConnection,
    ops: &[PgObjectOp],
    preview: &DdlPlanPreview,
) -> Result<usize, PgObjectError> {
    let pooled = crate::postgres::connect(connection)
        .await
        .map_err(|message| PgObjectError::Connection { message })?;
    let mut conn = pooled.detach();
    let result = async {
        // The pool's after_connect hook already applied the connection's
        // driver options, including any user-configured statement_timeout;
        // that bound is honoured here exactly as the legacy DDL path does.
        sqlx::query("SET lock_timeout = '10s'")
            .execute(&mut conn)
            .await
            .map_err(|error| apply_database_error(error, None, 0, None, None))?;

        let mut applied_statements = 0usize;
        for group in &preview.groups {
            match group {
                StatementGroup::Atomic { statement_indexes } => {
                    let first_index = statement_indexes.first().copied().unwrap_or_default();
                    let mut transaction = conn.begin().await.map_err(|error| {
                        apply_database_error(
                            error,
                            Some(first_index),
                            applied_statements,
                            None,
                            None,
                        )
                    })?;
                    for statement_index in statement_indexes {
                        let statement = &preview.statements[*statement_index];
                        if let Err(error) =
                            sqlx::query(&statement.sql).execute(&mut *transaction).await
                        {
                            let mapped = apply_database_error(
                                error,
                                Some(*statement_index),
                                applied_statements,
                                Some(&statement.sql),
                                None,
                            );
                            let _ = transaction.rollback().await;
                            return Err(mapped);
                        }
                    }
                    transaction.commit().await.map_err(|error| {
                        apply_database_error(
                            error,
                            statement_indexes.last().copied(),
                            applied_statements,
                            None,
                            None,
                        )
                    })?;
                    applied_statements += statement_indexes.len();
                }
                StatementGroup::Standalone { statement_index } => {
                    let statement = &preview.statements[*statement_index];
                    if let Err(error) = sqlx::query(&statement.sql).execute(&mut conn).await {
                        let residue =
                            concurrent_index_residue(&mut conn, ops.get(*statement_index)).await;
                        return Err(apply_database_error(
                            error,
                            Some(*statement_index),
                            applied_statements,
                            Some(&statement.sql),
                            residue,
                        ));
                    }
                    applied_statements += 1;
                }
            }
        }
        Ok(applied_statements)
    }
    .await;
    if let Err(error) = conn.close().await {
        log::warn!("Failed to close detached object-DDL connection: {error}");
    }
    result
}

async fn concurrent_index_residue(
    conn: &mut sqlx::PgConnection,
    op: Option<&PgObjectOp>,
) -> Option<DdlResidue> {
    // Both concurrent index operations leave an INVALID index behind when
    // they fail after their first phase: a build that errored, or a drop that
    // marked the index invalid and then timed out waiting for transactions.
    let (schema, index_name) = match op {
        Some(PgObjectOp::CreateIndex(CreateIndexOp {
            schema,
            table,
            name,
            columns,
            concurrently: true,
            ..
        })) => (
            schema,
            name.clone()
                .unwrap_or_else(|| derived_index_name(table, columns)),
        ),
        Some(PgObjectOp::DropIndex(DropIndexOp {
            schema,
            name,
            concurrently: true,
            ..
        })) => (schema, name.clone()),
        _ => return None,
    };
    let invalid = sqlx::query_scalar::<_, bool>(
        r#"
SELECT NOT index_state.indisvalid
FROM pg_index index_state
JOIN pg_class index_relation ON index_relation.oid = index_state.indexrelid
JOIN pg_namespace namespace ON namespace.oid = index_relation.relnamespace
WHERE namespace.nspname = $1 AND index_relation.relname = $2
"#,
    )
    .bind(schema)
    .bind(&index_name)
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten()
    .unwrap_or(false);
    invalid.then(|| DdlResidue::InvalidIndex {
        schema: schema.clone(),
        name: index_name,
    })
}

fn apply_database_error(
    error: sqlx::Error,
    statement_index: Option<usize>,
    applied_statements: usize,
    statement_sql: Option<&str>,
    residue: Option<DdlResidue>,
) -> PgObjectError {
    let (code, message, position) = match error.as_database_error() {
        Some(database) => {
            let position = database
                .try_downcast_ref::<sqlx::postgres::PgDatabaseError>()
                .and_then(|database| match database.position() {
                    Some(sqlx::postgres::PgErrorPosition::Original(position)) => statement_sql
                        .and_then(|sql| postgres_position_to_byte_offset(sql, position)),
                    _ => None,
                });
            (
                database.code().map(|code| code.into_owned()),
                database.message().to_string(),
                position,
            )
        }
        None => (None, error.to_string(), None),
    };
    map_apply_database_error(
        code,
        message,
        position,
        statement_index,
        applied_statements,
        residue,
    )
}

fn map_apply_database_error(
    code: Option<String>,
    message: String,
    position: Option<u32>,
    statement_index: Option<usize>,
    applied_statements: usize,
    residue: Option<DdlResidue>,
) -> PgObjectError {
    if code.as_deref() == Some("55P03") {
        return PgObjectError::LockTimeout {
            statement_index: statement_index.unwrap_or_default(),
            applied_statements,
            residue: residue.map(Box::new),
        };
    }
    PgObjectError::Database {
        statement_index,
        code,
        message,
        position,
        applied_statements,
        residue: residue.map(Box::new),
    }
}

fn postgres_position_to_byte_offset(sql: &str, position: usize) -> Option<u32> {
    let character_offset = position.checked_sub(1)?;
    let byte_offset = sql
        .char_indices()
        .nth(character_offset)
        .map(|(byte_offset, _)| byte_offset)
        .or_else(|| (character_offset == sql.chars().count()).then_some(sql.len()))?;
    u32::try_from(byte_offset).ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::postgres::object_ddl::*;
    use crate::{
        ConnectionOrganization, Environment, PgStoredConnection, SafeMode, SshTunnelConfig,
        StoredConnection,
    };

    pub(crate) fn connection(id: &str, safe_mode: SafeMode, read_only: bool) -> StoredConnection {
        let port = std::env::var("DBUNK_OBJECT_TEST_PORT")
            .ok()
            .and_then(|port| port.parse().ok())
            .unwrap_or(15432);
        StoredConnection::PostgreSQL(PgStoredConnection {
            organization: ConnectionOrganization::default(),
            id: id.into(),
            name: "Object DDL policy".into(),
            database: "dbunk_demo".into(),
            host: "127.0.0.1".into(),
            port,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: "read/write".into(),
            environment: Environment::Development,
            safe_mode,
            read_only,
            last_activity_at: None,
            ssl: port == 15433,
            tls_options: None,
            driver_options: None,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    fn ops() -> Vec<PgObjectOp> {
        vec![PgObjectOp::CreateSchema(CreateSchemaOp {
            name: "object_policy_test".into(),
        })]
    }

    #[test]
    fn postgres_character_positions_become_zero_based_utf8_byte_offsets() {
        let sql = "é🙂x";
        assert_eq!(postgres_position_to_byte_offset(sql, 1), Some(0));
        assert_eq!(postgres_position_to_byte_offset(sql, 2), Some(2));
        assert_eq!(postgres_position_to_byte_offset(sql, 3), Some(6));
        assert_eq!(postgres_position_to_byte_offset(sql, 4), Some(7));
        assert_eq!(postgres_position_to_byte_offset(sql, 0), None);
        assert_eq!(postgres_position_to_byte_offset(sql, 5), None);
    }

    #[test]
    fn lock_timeout_preserves_concurrent_index_residue() {
        let residue = DdlResidue::InvalidIndex {
            schema: "lifecycle".into(),
            name: "orders_idx".into(),
        };
        assert_eq!(
            map_apply_database_error(
                Some("55P03".into()),
                "canceling statement due to lock timeout".into(),
                Some(4),
                Some(2),
                1,
                Some(residue.clone()),
            ),
            PgObjectError::LockTimeout {
                statement_index: 2,
                applied_statements: 1,
                residue: Some(Box::new(residue)),
            }
        );
    }

    #[tokio::test]
    async fn object_ddl_preview_does_not_resolve_credentials_or_tunnels() {
        let (_directory, state) = crate::test_app_state().await;
        let mut stored = connection("pure-preview", SafeMode::Disabled, false);
        let StoredConnection::PostgreSQL(postgres) = &mut stored else {
            unreachable!("test connection is PostgreSQL")
        };
        postgres.password.clear();
        postgres.ssh_tunnel.enabled = true;
        postgres.ssh_tunnel.bastion_server_id = Some("missing-bastion".into());
        crate::storage::upsert_connection(&state.pool, &stored)
            .await
            .expect("persist unresolved connection record");

        let preview = preview_object_ddl_inner(&state, "pure-preview", &ops())
            .await
            .expect("pure preview");
        assert_eq!(preview.statements.len(), 1);
    }

    #[test]
    fn object_ddl_gate_returns_preview_summaries_and_honors_read_only() {
        let preview = generate_object_ddl(&ops()).expect("preview");
        assert!(matches!(
            authorize_object_ddl(
                &connection("read-only", SafeMode::Disabled, true),
                &preview,
                true
            ),
            Err(PgObjectError::PolicyBlocked { .. })
        ));

        let strict = connection("strict", SafeMode::Strict, false);
        assert_eq!(
            authorize_object_ddl(&strict, &preview, false),
            Err(PgObjectError::PolicyNeedsConfirmation {
                statements: statement_summaries(&preview)
            })
        );
        assert!(!statement_summaries(&preview).is_empty());
        assert!(matches!(
            authorize_object_ddl(&strict, &preview, true)
                .expect("confirmed strict authorization")
                .audit_disposition(),
            AuditDisposition::RequiredAfterSuccess
        ));
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn object_ddl_refusals_never_audit() {
        let (_directory, state) = crate::test_app_state().await;
        for (connection_id, safe_mode, read_only, expected) in [
            (
                "object-read-only",
                SafeMode::Disabled,
                true,
                "policyBlocked",
            ),
            (
                "object-strict",
                SafeMode::Strict,
                false,
                "policyNeedsConfirmation",
            ),
        ] {
            crate::commands::connections::save_connection_inner(
                &state,
                connection(connection_id, safe_mode, read_only),
            )
            .await
            .expect("save policy connection");
            let result = apply_object_ddl_inner(
                &state,
                ApplyObjectDdlPayload {
                    connection_id: connection_id.into(),
                    ops: ops(),
                    confirmed: false,
                },
            )
            .await;
            assert_eq!(
                serde_json::to_value(result.expect_err("gate refusal")).expect("serialize refusal")
                    ["kind"],
                expected
            );
            assert!(
                crate::storage::read_safety_overrides(&state.pool, connection_id)
                    .await
                    .expect("read audits")
                    .is_empty()
            );
        }
    }
}
