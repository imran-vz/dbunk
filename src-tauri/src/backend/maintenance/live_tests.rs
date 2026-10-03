//! Serial opt-in against the exact owned stage03 fixture. Each external helper
//! verifies the fixture before SQL; cleanup checks OID/owner/comment and RESTRICT.
use super::*;
use crate::{
    backend::profile,
    postgres::dedicated::{self, DedicatedConnection, DriverJoins, NoticeSink},
};
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
const SOURCE: &str = "source\"字";
const REPLACE: &str = "replace\"字";
const VIEW: &str = "view\"字";
const EMPTY: &str = "empty\"字";
const FIXTURE_HELPER: &str = r#"import sys, signal
def deadline(_signal, _frame):
    raise TimeoutError('owned fixture helper deadline')
signal.signal(signal.SIGALRM, deadline)
signal.alarm(25)
sys.path.insert(0, sys.argv[1])
import fixture
owned, target = fixture.check()
assert owned['instance'] == sys.argv[2], 'foreign fixture'
print(fixture.sql(target, sys.stdin.read()))
"#;

struct Probe {
    schema: String,
    comment: String,
    application: String,
    lock_application: String,
    helpers: DriverJoins,
}
impl Probe {
    fn qualified(&self, name: &str) -> String {
        format!(
            "{}.{}",
            crate::quote_double(&self.schema),
            crate::quote_double(name)
        )
    }
    fn reference(&self, name: &str, kind: PgObjectKind) -> PgObjectRef {
        PgObjectRef {
            kind,
            schema: Some(self.schema.clone()),
            name: name.into(),
            identity_args: None,
        }
    }
    async fn sql(&self, query: String) -> Result<String, String> {
        let application = self.application.clone();
        let (send, receive) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
            let mut child = std::process::Command::new("python3").arg("-c")
                .arg(FIXTURE_HELPER)
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
            let guarded = format!("SET application_name={}; SET statement_timeout='10s'; SET lock_timeout='3s'; {query}", crate::quote_literal(&application));
            child.stdin.take().unwrap().write_all(guarded.as_bytes()).map_err(|e| e.to_string())?;
            let output = child.wait_with_output().map_err(|e| e.to_string())?;
            if !output.status.success() { return Err(String::from_utf8_lossy(&output.stderr).into_owned()); }
            String::from_utf8(output.stdout).map(|s| s.trim().to_owned()).map_err(|e| e.to_string())
            }).await.map_err(|error| error.to_string()).and_then(|result| result);
            let _ = send.send(result);
        });
        self.helpers.track_task(task);
        receive.await.map_err(|error| error.to_string())?
    }
    async fn guard(&self) -> u64 {
        self.sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid();".into()).await.unwrap().parse().unwrap()
    }
    async fn observe(
        &self,
        backend: &Backend,
        document: &DataDocument,
        name: &str,
        kind: PgObjectKind,
    ) -> ObservedMaintenanceTarget {
        self.guard().await;
        backend
            .observe_maintenance_target(document, self.reference(name, kind))
            .await
            .unwrap()
    }
    async fn confirm(
        &self,
        backend: &Backend,
        review: MaintenanceReview,
    ) -> MaintenanceConfirmation {
        let attempt = review.attempt_id().to_owned();
        let target = review.target().clone();
        let preview = review.preview().clone();
        assert_eq!(preview.operation_timeout_ms, 300_000);
        self.guard().await;
        match backend.apply_maintenance(review).await.unwrap() {
            MaintenanceSubmission::NeedsConfirmation(confirmation) => {
                assert_eq!(confirmation.attempt_id(), attempt);
                assert_eq!(confirmation.target(), &target);
                assert_eq!(confirmation.preview(), &preview);
                *confirmation
            }
            _ => panic!("Strict requires exact confirmation before every maintenance dispatch"),
        }
    }
    async fn apply(
        &self,
        backend: &Backend,
        capture: &ObservedMaintenanceTarget,
        intent: MaintenanceIntent,
    ) -> MaintenanceReceipt {
        let confirmation = self.confirm(backend, capture.review(intent).unwrap()).await;
        self.guard().await;
        finished(backend.confirm_maintenance(confirmation).await.unwrap())
    }
    async fn lock(&self, connection: &DedicatedConnection, mode: &str) {
        assert!(matches!(mode, "ROW EXCLUSIVE" | "SHARE"));
        self.guard().await;
        connection
            .client
            .batch_execute(&format!(
                "BEGIN; LOCK TABLE {} IN {mode} MODE",
                self.qualified(SOURCE)
            ))
            .await
            .unwrap();
    }
    async fn unlock(&self, connection: &DedicatedConnection) {
        self.guard().await;
        connection.client.batch_execute("ROLLBACK").await.unwrap();
    }
    async fn wait_blocked(&self, sql: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let count = self.sql(format!("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND usename='dbunk' AND pid<>pg_backend_pid() AND state='active' AND wait_event_type='Lock' AND query={}", crate::quote_literal(sql))).await.unwrap();
                if count == "1" { break; }
                assert_eq!(count, "0", "unique reviewed SQL must identify at most one owned attempt");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.expect("owned operation reached its lock wait");
    }
    async fn cleanup(&self, expected_schema_oid: Option<u32>) -> Result<(), String> {
        let identity = self.sql(format!("SELECT json_build_object('oid',n.oid::bigint,'owner',pg_get_userbyid(n.nspowner),'comment',obj_description(n.oid,'pg_namespace'),'objects',(SELECT coalesce(json_agg(json_build_object('oid',c.oid::bigint,'name',c.relname,'kind',c.relkind,'owner',pg_get_userbyid(c.relowner),'comment',obj_description(c.oid,'pg_class'))),'[]'::json) FROM pg_class c WHERE c.relnamespace=n.oid)) FROM pg_namespace n WHERE n.nspname={}", crate::quote_literal(&self.schema))).await?;
        if identity.is_empty() {
            return Ok(());
        }
        let value: serde_json::Value =
            serde_json::from_str(&identity).map_err(|e| e.to_string())?;
        let oid = value["oid"].as_u64().ok_or("missing schema OID")?;
        if expected_schema_oid.is_some_and(|expected| u64::from(expected) != oid)
            || value["owner"].as_str() != Some("dbunk")
            || value["comment"].as_str() != Some(self.comment.as_str())
        {
            return Err("schema ownership changed; preserving objects".into());
        }
        let objects = value["objects"].as_array().ok_or("missing owned objects")?;
        let mut guards = format!("IF NOT EXISTS (SELECT 1 FROM pg_namespace n WHERE n.oid={oid} AND n.nspname={} AND pg_get_userbyid(n.nspowner)='dbunk' AND obj_description(n.oid,'pg_namespace')={}) THEN RAISE EXCEPTION 'schema ownership changed'; END IF;", crate::quote_literal(&self.schema), crate::quote_literal(&self.comment));
        for object in objects {
            let name = object["name"].as_str().ok_or("missing object name")?;
            let kind = object["kind"].as_str().ok_or("missing object kind")?;
            let object_oid = object["oid"].as_u64().ok_or("missing object OID")?;
            let expected_kind = match name {
                SOURCE | REPLACE => "r",
                VIEW | EMPTY => "m",
                "source_pk" | "view_unique" => "i",
                _ => return Err("unexpected schema child; preserving objects".into()),
            };
            if kind != expected_kind
                || object["owner"].as_str() != Some("dbunk")
                || object["comment"].as_str() != Some(self.comment.as_str())
            {
                return Err("object identity/comment changed; preserving objects".into());
            }
            guards.push_str(&format!("IF NOT EXISTS (SELECT 1 FROM pg_class c WHERE c.oid={object_oid} AND c.relnamespace={oid} AND c.relname={} AND c.relkind={} AND pg_get_userbyid(c.relowner)='dbunk' AND obj_description(c.oid,'pg_class')={}) THEN RAISE EXCEPTION 'object ownership changed'; END IF;", crate::quote_literal(name), crate::quote_literal(kind), crate::quote_literal(&self.comment)));
        }
        let mut drops = String::new();
        for (name, command) in [
            (VIEW, "MATERIALIZED VIEW"),
            (EMPTY, "MATERIALIZED VIEW"),
            (REPLACE, "TABLE"),
            (SOURCE, "TABLE"),
        ] {
            if objects.iter().any(|row| row["name"].as_str() == Some(name)) {
                drops.push_str(&format!(
                    "DROP {command} {} RESTRICT;",
                    self.qualified(name)
                ));
            }
        }
        // Indexes are verified above and disappear only as internal children of
        // their owned relation. Unexpected external dependencies abort RESTRICT.
        let cleanup = format!("BEGIN; DO $guard$ BEGIN {guards} END $guard$; {drops} DROP SCHEMA {} RESTRICT; COMMIT;", crate::quote_double(&self.schema));
        self.sql(cleanup).await?;
        assert_eq!(
            self.sql(format!(
                "SELECT count(*) FROM pg_namespace WHERE nspname={}",
                crate::quote_literal(&self.schema)
            ))
            .await?,
            "0"
        );
        println!(
            "owned schema cleanup schema={} oid={oid} verified_objects={} mode=RESTRICT",
            self.schema,
            objects.len()
        );
        Ok(())
    }
}
fn finished(submission: MaintenanceSubmission) -> MaintenanceReceipt {
    match submission {
        MaintenanceSubmission::Finished(receipt) => *receipt,
        _ => panic!("confirmation unexpectedly repeated"),
    }
}
async fn configure(backend: &Backend, timeout: u32) {
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!("PostgreSQL")
    };
    pg.safe_mode = crate::SafeMode::Strict;
    pg.driver_options
        .get_or_insert_with(Default::default)
        .statement_timeout_ms = Some(timeout);
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "exact owned stage03 fixture UUID, DBUNK_NATIVE_FIXTURE_VERIFIED=1 and exclusive serial fixture writer"]
async fn native_maintenance_owned_actions_identity_cancellation_audit_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let run = std::env::var("DBUNK_NATIVE_MAINTENANCE_RUN_ID")
        .map(|id| uuid::Uuid::parse_str(&id).unwrap())
        .unwrap_or_else(|_| uuid::Uuid::new_v4())
        .simple()
        .to_string();
    let probe = Probe {
        schema: format!("native_maint_{run}"),
        comment: format!("owned native maintenance {run}"),
        application: format!("native_maint_probe_{run}"),
        lock_application: format!("native_maint_lock_{run}"),
        helpers: DriverJoins::default(),
    };
    println!("fixture={FIXTURE} endpoint=127.0.0.1:15432/dbunk_demo schema={} helper_client={} lock_client={}", probe.schema, probe.application, probe.lock_application);
    let baseline = probe.guard().await;
    assert_eq!(
        baseline, 0,
        "exclusive probe requires no other fixture clients"
    );
    assert_eq!(
        probe
            .sql(format!(
                "SELECT count(*) FROM pg_namespace WHERE nspname={}",
                crate::quote_literal(&probe.schema)
            ))
            .await
            .unwrap(),
        "0",
        "never adopt an existing namespace"
    );
    let directory = profile::directory();
    println!(
        "owned temporary profile={} activity_baseline={baseline}",
        directory.path().display()
    );
    let drivers = DriverJoins::default();
    let mut backend = None;
    let mut blocker = None;
    let mut running = None;
    let mut schema_oid = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        configure(backend, 5000).await;
        let document = backend
            .open_data_document("maintenance-live", "owned", &backend.fixture().id)
            .await
            .unwrap();
        let comment = crate::quote_literal(&probe.comment);
        let setup = format!("BEGIN; CREATE SCHEMA {schema}; COMMENT ON SCHEMA {schema} IS {comment}; CREATE TABLE {source} (id integer CONSTRAINT source_pk PRIMARY KEY, payload text); COMMENT ON TABLE {source} IS {comment}; COMMENT ON INDEX {pk} IS {comment}; INSERT INTO {source} SELECT i,'row-'||i FROM generate_series(1,10) i; CREATE TABLE {replace} (id integer); COMMENT ON TABLE {replace} IS {comment}; CREATE MATERIALIZED VIEW {view} AS SELECT * FROM {source}; COMMENT ON MATERIALIZED VIEW {view} IS {comment}; CREATE MATERIALIZED VIEW {empty} AS SELECT * FROM {source} WITH NO DATA; COMMENT ON MATERIALIZED VIEW {empty} IS {comment}; COMMIT; SELECT oid FROM pg_namespace WHERE nspname={schema_literal};", schema=crate::quote_double(&probe.schema), schema_literal=crate::quote_literal(&probe.schema), source=probe.qualified(SOURCE), pk=probe.qualified("source_pk"), replace=probe.qualified(REPLACE), view=probe.qualified(VIEW), empty=probe.qualified(EMPTY));
        schema_oid = Some(probe.sql(setup).await.unwrap().parse::<u32>().unwrap());
        println!("created owned schema oid={}", schema_oid.unwrap());
        let source = probe
            .observe(backend, &document, SOURCE, PgObjectKind::Table)
            .await;
        let before_index = probe
            .sql(format!(
                "SELECT relfilenode FROM pg_class WHERE oid={}::regclass",
                crate::quote_literal(&probe.qualified("source_pk"))
            ))
            .await
            .unwrap();
        for intent in [
            MaintenanceIntent::ReindexTable,
            MaintenanceIntent::Vacuum,
            MaintenanceIntent::Analyze,
        ] {
            let receipt = probe.apply(backend, &source, intent).await;
            assert_eq!(receipt.outcome, MaintenanceOutcome::Completed);
            assert!(receipt.retained_bytes() < MAX_MAINTENANCE_RECEIPT_BYTES);
            println!(
                "action={intent:?} outcome={:?} notices={} truncated={}",
                receipt.outcome,
                receipt.notices.len(),
                receipt.notices_truncated
            );
        }
        let after_index = probe
            .sql(format!(
                "SELECT relfilenode FROM pg_class WHERE oid={}::regclass",
                crate::quote_literal(&probe.qualified("source_pk"))
            ))
            .await
            .unwrap();
        assert_ne!(
            before_index, after_index,
            "ordinary REINDEX replaced owned index storage"
        );
        let view = probe
            .observe(backend, &document, VIEW, PgObjectKind::MaterializedView)
            .await;
        let empty = probe
            .observe(backend, &document, EMPTY, PgObjectKind::MaterializedView)
            .await;
        for capture in [&view, &empty] {
            let receipt = probe
                .apply(
                    backend,
                    capture,
                    MaintenanceIntent::RefreshMaterializedView { concurrently: true },
                )
                .await;
            assert!(matches!(
                receipt.outcome,
                MaintenanceOutcome::RolledBack {
                    reason: MaintenanceFailure::Database { .. }
                }
            ));
            println!(
                "concurrent prerequisite refusal target={} outcome={:?}",
                capture.target().name(),
                receipt.outcome
            );
        }
        assert_eq!(
            probe
                .apply(
                    backend,
                    &empty,
                    MaintenanceIntent::RefreshMaterializedView {
                        concurrently: false
                    }
                )
                .await
                .outcome,
            MaintenanceOutcome::Completed
        );
        probe
            .sql(format!(
                "INSERT INTO {} VALUES (11,'normal refresh');",
                probe.qualified(SOURCE)
            ))
            .await
            .unwrap();
        assert_eq!(
            probe
                .apply(
                    backend,
                    &view,
                    MaintenanceIntent::RefreshMaterializedView {
                        concurrently: false
                    }
                )
                .await
                .outcome,
            MaintenanceOutcome::Completed
        );
        assert_eq!(
            probe
                .sql(format!("SELECT count(*) FROM {}", probe.qualified(VIEW)))
                .await
                .unwrap(),
            "11"
        );
        probe.sql(format!("BEGIN; CREATE UNIQUE INDEX view_unique ON {} (id); COMMENT ON INDEX {} IS {comment}; INSERT INTO {} VALUES (12,'concurrent refresh'); COMMIT;", probe.qualified(VIEW), probe.qualified("view_unique"), probe.qualified(SOURCE))).await.unwrap();
        assert_eq!(
            probe
                .apply(
                    backend,
                    &view,
                    MaintenanceIntent::RefreshMaterializedView { concurrently: true }
                )
                .await
                .outcome,
            MaintenanceOutcome::Completed
        );
        assert_eq!(
            probe
                .sql(format!("SELECT count(*) FROM {}", probe.qualified(VIEW)))
                .await
                .unwrap(),
            "12"
        );
        let stale = probe
            .observe(backend, &document, REPLACE, PgObjectKind::Table)
            .await;
        let old_oid = stale.target().relation_oid();
        probe.sql(format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_class WHERE oid={old_oid} AND relnamespace={} AND relname={} AND obj_description(oid,'pg_class')={comment} AND pg_get_userbyid(relowner)='dbunk') THEN RAISE EXCEPTION 'replacement ownership changed'; END IF; END $guard$; DROP TABLE {} RESTRICT; CREATE TABLE {} (id integer); COMMENT ON TABLE {} IS {comment}; COMMIT;", schema_oid.unwrap(), crate::quote_literal(REPLACE), probe.qualified(REPLACE), probe.qualified(REPLACE), probe.qualified(REPLACE))).await.unwrap();
        let replacement = probe
            .observe(backend, &document, REPLACE, PgObjectKind::Table)
            .await;
        assert_ne!(replacement.target().relation_oid(), old_oid);
        assert_eq!(
            probe
                .apply(backend, &stale, MaintenanceIntent::Vacuum)
                .await
                .outcome,
            MaintenanceOutcome::TargetChanged
        );
        println!(
            "stale name/OID replacement refused old_oid={old_oid} new_oid={}",
            replacement.target().relation_oid()
        );
        probe.guard().await;
        let stored = crate::app::find_connection(&backend.0.state, &backend.fixture().id)
            .await
            .unwrap();
        let spec = ResolvedPostgresConnectSpec::from_connection(&stored).unwrap();
        assert_eq!(
            (
                spec.host.as_str(),
                spec.port,
                spec.database.as_str(),
                spec.user.as_str()
            ),
            ("127.0.0.1", 15432, "dbunk_demo", "dbunk")
        );
        blocker = Some(
            dedicated::connect_tracked(&spec, NoticeSink::Ignore, Some(&drivers))
                .await
                .unwrap(),
        );
        let lock = blocker.as_ref().unwrap();
        probe.guard().await;
        lock.client
            .query_one(
                "SELECT set_config('application_name',$1,false)",
                &[&probe.lock_application],
            )
            .await
            .unwrap();
        probe.guard().await;
        let pid: i32 = lock
            .client
            .query_one("SELECT pg_backend_pid()", &[])
            .await
            .unwrap()
            .get(0);
        println!(
            "owned blocker pid={pid} application={}",
            probe.lock_application
        );
        for (intent, mode) in [
            (MaintenanceIntent::ReindexTable, "ROW EXCLUSIVE"),
            (MaintenanceIntent::Vacuum, "SHARE"),
        ] {
            let confirmation = probe.confirm(backend, source.review(intent).unwrap()).await;
            let sql = confirmation.preview().sql.clone();
            probe.lock(lock, mode).await;
            probe.guard().await;
            let owner = backend.clone();
            running = Some(tokio::spawn(async move {
                finished(owner.confirm_maintenance(confirmation).await.unwrap())
            }));
            probe.wait_blocked(&sql).await;
            probe.guard().await;
            backend.cancel_data(&document).await.unwrap();
            let receipt = tokio::time::timeout(Duration::from_secs(5), running.as_mut().unwrap())
                .await
                .unwrap()
                .unwrap();
            running.take();
            let expected = if intent == MaintenanceIntent::ReindexTable {
                MaintenanceOutcome::RolledBack {
                    reason: MaintenanceFailure::Cancelled,
                }
            } else {
                MaintenanceOutcome::OutcomeUnknown {
                    reason: MaintenanceFailure::Cancelled,
                }
            };
            assert_eq!(receipt.outcome, expected);
            println!(
                "explicit cancel action={intent:?} outcome={:?}",
                receipt.outcome
            );
            probe.unlock(lock).await;
        }
        configure(backend, 200).await;
        let short = probe
            .observe(backend, &document, SOURCE, PgObjectKind::Table)
            .await;
        probe.lock(lock, "SHARE").await;
        let receipt = probe
            .apply(backend, &short, MaintenanceIntent::Vacuum)
            .await;
        assert!(
            matches!(&receipt.outcome, MaintenanceOutcome::InterruptedEffectsPossible { reason: MaintenanceFailure::Database { code } } if code.as_deref()==Some("57014"))
        );
        println!(
            "server statement timeout outcome={:?}; partial effects not measured or claimed",
            receipt.outcome
        );
        probe.unlock(lock).await;
        let audits =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        assert_eq!(audits.len(), 6);
        assert_eq!(
            audits
                .iter()
                .filter(|audit| audit.command == "run_pg_maintenance")
                .count(),
            3
        );
        assert_eq!(
            audits
                .iter()
                .filter(|audit| audit.command == "refresh_materialized_view")
                .count(),
            3
        );
        assert_eq!(
            probe
                .sql(format!("SELECT count(*) FROM {}", probe.qualified(SOURCE)))
                .await
                .unwrap(),
            "12"
        );
        println!("Strict exact confirmations; six completed actions audited; stale/prerequisite/cancelled/unknown attempts added no success audit; source rows=12");
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(150), operation))
            .catch_unwind()
            .await;
    // Retire/join backend ownership before any schema cleanup. Keep the blocker
    // held until shutdown so an abandoned waiter cannot finish a late mutation.
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    if let Some(running) = running {
        running.abort();
        let _ = running.await;
    }
    if let Some(blocker) = blocker {
        if tokio::time::timeout(Duration::from_secs(2), blocker.close())
            .await
            .is_err()
        {
            drivers.abort_all();
        }
    }
    drivers.drain().await;
    // A timed-out async waiter cannot orphan an external SQL helper that could
    // still commit after cleanup. Every spawned helper belongs to this probe.
    probe.helpers.drain().await;
    let cleaned = probe.cleanup(schema_oid).await;
    let final_count = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let current = probe.guard().await;
            if current == baseline {
                break current;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    probe.helpers.drain().await;
    shutdown.expect("maintenance backend and drivers joined");
    cleaned.expect("only owned objects removed with RESTRICT");
    println!(
        "joined cleanup activity baseline={baseline} final={}",
        final_count.expect("activity returned to baseline")
    );
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("bounded maintenance live probe deadline");
}
