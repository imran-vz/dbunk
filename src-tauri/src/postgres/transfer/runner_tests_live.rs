//! Explicitly ignored: generated data on disposable PostgreSQL ports only.
use super::protocol::*;
use crate::commands::pg_transfer::{inspect, start_export, start_import};
use crate::{AppState, PgStoredConnection, SafeMode, StoredConnection};
use futures_util::FutureExt;
use sqlx::{Connection, Executor};
use std::path::Path;
use std::time::Duration;

fn stored(database: &str, port: u16) -> PgStoredConnection {
    PgStoredConnection {
        organization: Default::default(),
        id: database.into(),
        name: "Disposable CSV fixture".into(),
        host: "127.0.0.1".into(),
        port,
        database: database.into(),
        user: "dbunk".into(),
        password: "dbunk".into(),
        role: "read/write".into(),
        environment: crate::Environment::Test,
        safe_mode: SafeMode::Strict,
        read_only: false,
        last_activity_at: None,
        ssl: false,
        tls_options: None,
        driver_options: None,
        ssh_tunnel: Default::default(),
    }
}
async fn connect(database: &str, port: u16) -> sqlx::PgConnection {
    sqlx::PgConnection::connect_with(
        &sqlx::postgres::PgConnectOptions::new()
            .host("127.0.0.1")
            .port(port)
            .username("dbunk")
            .password("dbunk")
            .database(database)
            .ssl_mode(sqlx::postgres::PgSslMode::Disable),
    )
    .await
    .expect("disposable PostgreSQL fixture")
}
async fn terminal(state: &AppState, job: Snapshot) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let snapshot = state.pg_transfers.get(&job.job_id).unwrap();
            if snapshot.phase.terminal() {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("CSV transfer must terminate")
}
async fn inspection(
    state: &AppState,
    id: &str,
    table: &str,
    source: Option<&Path>,
    options: CsvOptions,
) -> Result<Inspection, TransferError> {
    inspect(
        state,
        InspectPayload {
            connection_id: id.into(),
            schema: "public".into(),
            table: table.into(),
            direction: if source.is_some() {
                Direction::Import
            } else {
                Direction::Export
            },
            source_path: source.map(|p| p.to_str().unwrap().into()),
            options,
        },
    )
    .await
}
async fn import(state: &AppState, review: Inspection, targets: &[&str]) -> Snapshot {
    let job = start_import(
        state,
        StartImportPayload {
            inspection_token: review.inspection_token,
            mapping: targets
                .iter()
                .enumerate()
                .map(|(source_index, target)| ColumnMapping {
                    source_index,
                    target_column: (*target).into(),
                })
                .collect(),
            confirmed: true,
        },
    )
    .await
    .unwrap();
    terminal(state, job).await
}
async fn export(
    state: &AppState,
    id: &str,
    table: &str,
    destination: &Path,
    options: CsvOptions,
) -> Snapshot {
    let review = inspection(state, id, table, None, options).await.unwrap();
    let job = start_export(
        state,
        StartExportPayload {
            inspection_token: review.inspection_token,
            destination_path: destination.to_str().unwrap().into(),
        },
    )
    .await
    .unwrap();
    terminal(state, job).await
}
async fn setup(port: u16) -> (tempfile::TempDir, AppState, sqlx::PgConnection, String) {
    let (profile, state) = crate::test_app_state().await;
    let id = format!("dbunk_csv_{}", uuid::Uuid::new_v4().simple());
    let mut admin = connect("dbunk_demo", port).await;
    admin
        .execute(format!("CREATE DATABASE {}", crate::quote_double(&id)).as_str())
        .await
        .unwrap();
    crate::commands::connections::save_connection_inner(
        &state,
        StoredConnection::PostgreSQL(stored(&id, port)),
    )
    .await
    .unwrap();
    (profile, state, admin, id)
}
async fn cleanup(state: &AppState, admin: &mut sqlx::PgConnection, id: &str) {
    crate::socket_lifecycle::with_connection_fence(state, id, async {
        crate::socket_lifecycle::invalidate_connection_caches(
            id,
            Some(crate::DatabaseEngine::PostgreSQL),
        );
    })
    .await;
    admin
        .execute(format!("DROP DATABASE {} WITH (FORCE)", crate::quote_double(id)).as_str())
        .await
        .unwrap();
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable PostgreSQL on 15432; creates and drops a generated database"]
async fn live_csv_exports_materialized_views_and_foreign_tables() {
    let (_profile, state, mut admin, id) = setup(15432).await;
    let result = std::panic::AssertUnwindSafe(async {
        let files = tempfile::tempdir().unwrap();
        let mut db = connect(&id, 15432).await;
        db.execute(
            r#"
            CREATE TABLE export_source (value text);
            INSERT INTO export_source VALUES ('retained-value');
            CREATE MATERIALIZED VIEW export_materialized AS SELECT * FROM export_source;
            CREATE VIEW export_view AS SELECT * FROM export_source;
            CREATE EXTENSION postgres_fdw;
            "#,
        )
        .await
        .unwrap();
        // The foreign server loops back inside the disposable fixture container.
        db.execute(
            format!(
                "CREATE SERVER csv_loopback FOREIGN DATA WRAPPER postgres_fdw \
                 OPTIONS (host '127.0.0.1', port '5432', dbname '{id}'); \
                 CREATE USER MAPPING FOR CURRENT_USER SERVER csv_loopback \
                 OPTIONS (user 'dbunk', password 'dbunk'); \
                 CREATE FOREIGN TABLE export_foreign (value text) SERVER csv_loopback \
                 OPTIONS (schema_name 'public', table_name 'export_source')"
            )
            .as_str(),
        )
        .await
        .unwrap();
        for table in [
            "export_source",
            "export_view",
            "export_materialized",
            "export_foreign",
        ] {
            let destination = files.path().join(format!("{table}.csv"));
            let job = export(&state, &id, table, &destination, CsvOptions::default()).await;
            assert_eq!(job.phase, Phase::Completed, "{table}: {job:?}");
            let csv = std::fs::read_to_string(destination).unwrap();
            assert_eq!(csv.lines().count(), 2, "{table}: {csv}");
            assert!(csv.contains("retained-value"), "{table}: {csv}");
        }
    })
    .catch_unwind()
    .await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable PostgreSQL on 15432; creates and drops a generated database"]
async fn live_csv_import_commits_rows_routed_or_suppressed_by_triggers() {
    let (_profile, state, mut admin, id) = setup(15432).await;
    let result = std::panic::AssertUnwindSafe(async {
        let files = tempfile::tempdir().unwrap();
        let source = files.path().join("trigger.csv");
        let mut db = connect(&id, 15432).await;
        db.execute(
            r#"
            CREATE TABLE trigger_target (value text);
            CREATE TABLE routed_rows (value text);
            CREATE FUNCTION route_csv_row() RETURNS trigger LANGUAGE plpgsql AS $$
            BEGIN
                IF NEW.value = 'route' THEN
                    INSERT INTO routed_rows VALUES (NEW.value);
                    RETURN NULL;
                ELSIF NEW.value = 'skip' THEN
                    RETURN NULL;
                END IF;
                RETURN NEW;
            END $$;
            CREATE TRIGGER route_csv BEFORE INSERT ON trigger_target
                FOR EACH ROW EXECUTE FUNCTION route_csv_row();
            "#,
        )
        .await
        .unwrap();
        for (csv, processed, committed) in [
            ("value\nkeep\nroute\nskip\n", 3, 1),
            ("value\nroute\nskip\n", 2, 0),
        ] {
            std::fs::write(&source, csv).unwrap();
            let review = inspection(
                &state,
                &id,
                "trigger_target",
                Some(&source),
                CsvOptions::default(),
            )
            .await
            .unwrap();
            let job = import(&state, review, &["value"]).await;
            assert_eq!(job.phase, Phase::Completed, "{job:?}");
            assert_eq!(job.rows_processed, Some(processed));
            assert_eq!(job.rows_committed, Some(committed));
        }
        let kept: Vec<String> = sqlx::query_scalar("SELECT value FROM trigger_target")
            .fetch_all(&mut db)
            .await
            .unwrap();
        assert_eq!(kept, ["keep"]);
        let routed: Vec<String> = sqlx::query_scalar("SELECT value FROM routed_rows")
            .fetch_all(&mut db)
            .await
            .unwrap();
        assert_eq!(routed, ["route", "route"]);
    })
    .catch_unwind()
    .await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable PostgreSQL on 15432; creates and drops a generated database"]
async fn live_csv_import_rejects_default_expression_changed_after_inspection() {
    let (_profile, state, mut admin, id) = setup(15432).await;
    let result = std::panic::AssertUnwindSafe(async {
        let files = tempfile::tempdir().unwrap();
        let source = files.path().join("default-change.csv");
        std::fs::write(&source, "value\nreviewed\n").unwrap();

        let mut db = connect(&id, 15432).await;
        db.execute("CREATE TABLE default_target (value text, omitted integer NOT NULL DEFAULT 1)")
            .await
            .unwrap();
        let review = inspection(
            &state,
            &id,
            "default_target",
            Some(&source),
            CsvOptions::default(),
        )
        .await
        .unwrap();

        db.execute("ALTER TABLE default_target ALTER COLUMN omitted SET DEFAULT 2")
            .await
            .unwrap();
        let rejected = import(&state, review, &["value"]).await;

        assert_eq!(rejected.phase, Phase::Failed, "{rejected:?}");
        assert_eq!(rejected.failure, Some(TransferError::TargetChanged));
        assert_eq!(rejected.rows_committed, None);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM default_target")
                .fetch_one(&mut db)
                .await
                .unwrap(),
            0
        );
    })
    .catch_unwind()
    .await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable PostgreSQL on 15432; creates and drops a generated database"]
async fn live_csv_roundtrip_atomicity_catalog_files_and_fences() {
    let (_profile, state, mut admin, id) = setup(15432).await;
    let result=std::panic::AssertUnwindSafe(async {
        let files=tempfile::tempdir().unwrap();
        let mut db=connect(&id,15432).await;
        db.execute("CREATE TABLE source (a text,b text,c text); CREATE TABLE target (a text,b text,c text,omitted text NOT NULL DEFAULT 'default'); CREATE TABLE effects(value text); CREATE FUNCTION record_effect() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN INSERT INTO effects VALUES (NEW.a); RETURN NEW; END $$; CREATE TRIGGER csv_effect AFTER INSERT ON target FOR EACH ROW EXECUTE FUNCTION record_effect();").await.unwrap();
        let rows=[(Some("comma,value"),Some("quote\"value"),Some("line\none")),(None,Some(""),Some("\\N")),(Some("é🙂"),Some("CR\rLF\r\n"),Some("null"))];
        for (a,b,c) in rows { sqlx::query("INSERT INTO source VALUES ($1,$2,$3)").bind(a).bind(b).bind(c).execute(&mut db).await.unwrap(); }
        let options=CsvOptions::default();
        let destination=files.path().join("roundtrip.csv");
        let out=export(&state,&id,"source",&destination,options.clone()).await;
        assert_eq!(out.phase,Phase::Completed,"{out:?}");
        let review=inspection(&state,&id,"target",Some(&destination),options.clone()).await.unwrap();
        assert_eq!(review.source_columns.len(),3);
        let input=import(&state,review,&["a","b","c"]).await;
        assert_eq!(input.phase,Phase::Completed,"{input:?}"); assert_eq!(input.rows_committed,Some(3));
        type RoundTripRow = (Option<String>,Option<String>,Option<String>,String);
        let actual:Vec<RoundTripRow>=sqlx::query_as("SELECT a,b,c,omitted FROM target ORDER BY c NULLS FIRST").fetch_all(&mut db).await.unwrap();
        let expected:Vec<RoundTripRow>=sqlx::query_as("SELECT a,b,c,'default'::text FROM source ORDER BY c NULLS FIRST").fetch_all(&mut db).await.unwrap();
        assert_eq!(actual,expected);
        db.execute("CREATE TABLE custom_target (a text,b text,c text)").await.unwrap();
        let custom_options=CsvOptions {delimiter:";".into(),quote:"'".into(),escape:"\\".into(),null_token:"NULL".into(),header:false};
        let custom=files.path().join("custom.csv");
        assert_eq!(export(&state,&id,"source",&custom,custom_options.clone()).await.phase,Phase::Completed);
        let r=inspection(&state,&id,"custom_target",Some(&custom),custom_options).await.unwrap();
        assert_eq!(import(&state,r,&["a","b","c"]).await.phase,Phase::Completed);
        assert!(sqlx::query_scalar::<_,bool>("SELECT NOT EXISTS ((SELECT * FROM source EXCEPT ALL SELECT * FROM custom_target) UNION ALL (SELECT * FROM custom_target EXCEPT ALL SELECT * FROM source))").fetch_one(&mut db).await.unwrap());
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM effects").fetch_one(&mut db).await.unwrap(),3);
        let collision=export(&state,&id,"source",&destination,options.clone()).await;
        assert_eq!(collision.failure,Some(TransferError::DestinationExists));
        assert_eq!(std::fs::read_dir(files.path()).unwrap().count(),2,"no partial after collision");

        db.execute("CREATE TABLE checked (a integer CHECK(a>0)); CREATE TABLE empty_table(a text); CREATE TABLE partitioned (a integer) PARTITION BY RANGE(a); CREATE TABLE partition_child PARTITION OF partitioned FOR VALUES FROM(0) TO(100); INSERT INTO partitioned VALUES(1),(2); CREATE TABLE rls_table(a text); ALTER TABLE rls_table ENABLE ROW LEVEL SECURITY; CREATE TABLE identity_table(a integer GENERATED ALWAYS AS IDENTITY,b text);").await.unwrap();
        let late=files.path().join("late.csv");
        // A valid inspection prefix followed by an invalid final record must roll back.
        let mut data=String::from("a\n"); for _ in 0..100 {data.push_str("1\n");} data.push_str("-1\n"); std::fs::write(&late,data).unwrap();
        let r=inspection(&state,&id,"checked",Some(&late),options.clone()).await.unwrap();
        let failed=import(&state,r,&["a"]).await; assert_eq!(failed.phase,Phase::Failed,"{failed:?}");
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM checked").fetch_one(&mut db).await.unwrap(),0);
        let mut malformed=String::from("a\n");for _ in 0..150_000 {malformed.push_str("1\n");} malformed.push_str("\"unfinished");std::fs::write(&late,malformed).unwrap();
        let r=inspection(&state,&id,"checked",Some(&late),options.clone()).await.unwrap();
        assert_eq!(import(&state,r,&["a"]).await.phase,Phase::Failed);
        assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM checked").fetch_one(&mut db).await.unwrap(),0);
        assert!(matches!(inspection(&state,&id,"rls_table",Some(&destination),options.clone()).await,Err(TransferError::UnsupportedTarget {..})));
        let empty=files.path().join("empty.csv");assert_eq!(export(&state,&id,"empty_table",&empty,options.clone()).await.phase,Phase::Completed);assert_eq!(std::fs::read_to_string(&empty).unwrap().trim().trim_matches('"'),"a");
        db.execute("CREATE TABLE oversized(a text); INSERT INTO oversized VALUES(repeat('x',1048577))").await.unwrap();
        let oversized=files.path().join("oversized-export.csv");
        assert_eq!(export(&state,&id,"oversized",&oversized,options.clone()).await.phase,Phase::Failed);
        assert!(!oversized.exists(),"an oversized database field never publishes a partial export");
        let partitions=files.path().join("partitions.csv");assert_eq!(export(&state,&id,"partitioned",&partitions,options.clone()).await.phase,Phase::Completed);let text=std::fs::read_to_string(partitions).unwrap();assert!(text.contains('1')&&text.contains('2'));
        let identity_source=files.path().join("identity.csv");std::fs::write(&identity_source,"b\nvalue\n").unwrap();
        let r=inspection(&state,&id,"identity_table",Some(&identity_source),options.clone()).await.unwrap();
        assert_eq!(import(&state,r,&["b"]).await.phase,Phase::Completed);
        assert_eq!(sqlx::query_scalar::<_,i32>("SELECT a FROM identity_table").fetch_one(&mut db).await.unwrap(),1);
        let r=inspection(&state,&id,"identity_table",Some(&identity_source),options.clone()).await.unwrap();
        assert!(matches!(import(&state,r,&["a"]).await.failure,Some(TransferError::UnsupportedTarget {..})));

        std::fs::write(&late,"a\n2\n").unwrap();let changed=inspection(&state,&id,"checked",Some(&late),options.clone()).await.unwrap();std::fs::write(&late,"a\n333\n").unwrap();assert_eq!(import(&state,changed,&["a"]).await.failure,Some(TransferError::SourceChanged));
        let changed=inspection(&state,&id,"checked",Some(&late),options.clone()).await.unwrap();db.execute("ALTER TABLE checked ADD COLUMN other text").await.unwrap();assert_eq!(import(&state,changed,&["a"]).await.failure,Some(TransferError::TargetChanged));
        let token=inspection(&state,&id,"checked",Some(&late),options.clone()).await.unwrap().inspection_token;
        crate::socket_lifecycle::with_connection_fence(&state,&id,async {}).await;
        assert!(matches!(start_import(&state,StartImportPayload {inspection_token:token,mapping:vec![ColumnMapping {source_index:0,target_column:"a".into()}],confirmed:true}).await,Err(TransferError::InspectionExpired)));

        let cancel_path=files.path().join("cancel.csv");let reviewed=inspection(&state,&id,"source",None,options).await.unwrap();
        db.execute("BEGIN; LOCK TABLE source IN ACCESS EXCLUSIVE MODE").await.unwrap();
        let job=start_export(&state,StartExportPayload {inspection_token:reviewed.inspection_token,destination_path:cancel_path.to_str().unwrap().into()}).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        crate::socket_lifecycle::with_connection_fence(&state,&id,async {}).await;
        assert_eq!(terminal(&state,job).await.phase,Phase::Cancelled);assert!(!cancel_path.exists());
        db.execute("ROLLBACK").await.unwrap();
        assert!(!std::fs::read_dir(files.path()).unwrap().any(|p|p.unwrap().file_name().to_string_lossy().contains("dbunk-partial")));
    }).catch_unwind().await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable PostgreSQL on 15432; creates and drops a generated database"]
async fn live_csv_export_uses_canonical_output_and_distinct_limit_errors() {
    let (_profile, state, mut admin, id) = setup(15432).await;
    let result = std::panic::AssertUnwindSafe(async {
        let files = tempfile::tempdir().unwrap();
        let mut db = connect(&id, 15432).await;
        db.execute(
            r#"
            CREATE TYPE guarded_payload AS (payload text);
            CREATE FUNCTION guarded_payload_as_text(guarded_payload)
            RETURNS text LANGUAGE sql IMMUTABLE AS $$ SELECT 'short' $$;
            CREATE CAST (guarded_payload AS text)
            WITH FUNCTION guarded_payload_as_text(guarded_payload) AS ASSIGNMENT;
            CREATE TABLE canonical_output(value guarded_payload);
            INSERT INTO canonical_output
            VALUES (ROW('normal')::guarded_payload), (ROW(NULL)::guarded_payload);
            CREATE TABLE oversized_custom_output(value guarded_payload);
            INSERT INTO oversized_custom_output
            VALUES (ROW(repeat('x', 1048577))::guarded_payload);
            CREATE TABLE oversized_record(a text, b text, c text, d text,
                                          e text, f text, g text, h text);
            INSERT INTO oversized_record
            SELECT value, value, value, value, value, value, value, value
            FROM (SELECT repeat('x', 600000) AS value) AS generated;
            CREATE VIEW division_by_zero AS SELECT 1 / 0 AS value;
            "#,
        )
        .await
        .unwrap();

        let canonical = files.path().join("canonical.csv");
        let output = export(
            &state,
            &id,
            "canonical_output",
            &canonical,
            CsvOptions::default(),
        )
        .await;
        assert_eq!(output.phase, Phase::Completed, "{output:?}");
        let exported = std::fs::read_to_string(canonical).unwrap();
        assert!(exported.contains("(normal)"), "{exported:?}");
        assert!(exported.contains("()"), "{exported:?}");
        assert!(!exported.contains("short"), "{exported:?}");

        let oversized = files.path().join("oversized-custom.csv");
        let output = export(
            &state,
            &id,
            "oversized_custom_output",
            &oversized,
            CsvOptions::default(),
        )
        .await;
        assert_eq!(output.phase, Phase::Failed, "{output:?}");
        assert_eq!(
            output.failure,
            Some(TransferError::ExportLimitExceeded {
                limit: ExportLimit::Field,
            })
        );
        assert!(!oversized.exists());

        let oversized = files.path().join("oversized-record.csv");
        let output = export(
            &state,
            &id,
            "oversized_record",
            &oversized,
            CsvOptions::default(),
        )
        .await;
        assert_eq!(output.phase, Phase::Failed, "{output:?}");
        assert_eq!(
            output.failure,
            Some(TransferError::ExportLimitExceeded {
                limit: ExportLimit::Record,
            })
        );
        assert!(!oversized.exists());

        let divided = files.path().join("division.csv");
        let output = export(
            &state,
            &id,
            "division_by_zero",
            &divided,
            CsvOptions::default(),
        )
        .await;
        assert!(matches!(
            output.failure,
            Some(TransferError::Database {
                code: Some(ref code),
                ..
            }) if code == "22012"
        ));
        assert!(!divided.exists());
    })
    .catch_unwind()
    .await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "requires disposable TLS PostgreSQL on 15433 and DBUNK_CSV_TLS_ROOT"]
async fn live_csv_verified_tls_and_driver_read_only() {
    let root = std::env::var("DBUNK_CSV_TLS_ROOT").expect("path to disposable CA");
    let (_profile, state, mut admin, id) = setup(15433).await;
    let result = std::panic::AssertUnwindSafe(async {
        let mut db = connect(&id, 15433).await;
        db.execute("CREATE TABLE tls_probe(a text); INSERT INTO tls_probe VALUES('verified')")
            .await
            .unwrap();
        let mut connection = stored(&id, 15433);
        connection.read_only = true;
        connection.tls_options = Some(crate::PgTlsOptions {
            mode: crate::PgTlsMode::VerifyFull,
            root_cert_path: Some(root),
            client_cert_path: None,
            client_key_path: None,
            server_name: None,
        });
        connection.driver_options = Some(crate::PgDriverOptions {
            statement_timeout_ms: Some(3000),
            ..Default::default()
        });
        crate::commands::connections::save_connection_inner(
            &state,
            StoredConnection::PostgreSQL(connection.clone()),
        )
        .await
        .unwrap();
        let files = tempfile::tempdir().unwrap();
        let path = files.path().join("tls.csv");
        let out = export(&state, &id, "tls_probe", &path, CsvOptions::default()).await;
        assert_eq!(out.phase, Phase::Completed, "{out:?}");
        assert!(std::fs::read_to_string(path).unwrap().contains("verified"));
        connection.tls_options.as_mut().unwrap().server_name = Some("wrong.invalid".into());
        // A mismatched hostname is rejected before a transfer is admitted.
        let payload = InspectPayload {
            connection_id: id.clone(),
            schema: "public".into(),
            table: "tls_probe".into(),
            direction: Direction::Export,
            source_path: None,
            options: CsvOptions::default(),
        };
        assert!(
            super::runner::inspect(StoredConnection::PostgreSQL(connection), payload)
                .await
                .is_err()
        );
    })
    .catch_unwind()
    .await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[tokio::test]
#[serial_test::serial]
#[ignore = "resource measurement; disposable PostgreSQL 15432; DBUNK_CSV_MEMORY_MIB=32 or 512"]
async fn live_csv_memory_plateau() {
    use std::io::Write;
    let mib = std::env::var("DBUNK_CSV_MEMORY_MIB")
        .expect("explicit fixture size")
        .parse::<usize>()
        .unwrap();
    assert!([32, 512].contains(&mib));
    let (_profile, state, mut admin, id) = setup(15432).await;
    let result = std::panic::AssertUnwindSafe(async {
        let files = tempfile::tempdir().unwrap();
        let path = files.path().join("large.csv");
        let rows = mib * 1024;
        {
            let mut file =
                std::io::BufWriter::with_capacity(65536, std::fs::File::create(&path).unwrap());
            file.write_all(b"a\n").unwrap();
            let mut row = vec![b'x'; 1023];
            row.push(b'\n');
            for _ in 0..rows {
                file.write_all(&row).unwrap();
            }
            file.flush().unwrap();
        }
        let mut db = connect(&id, 15432).await;
        db.execute("CREATE TABLE large(a text)").await.unwrap();
        let reviewed = inspection(&state, &id, "large", Some(&path), CsvOptions::default())
            .await
            .unwrap();
        assert!(reviewed.sample_rows.len() <= 50);
        assert!(serde_json::to_vec(&reviewed).unwrap().len() < 70000);
        let input = import(&state, reviewed, &["a"]).await;
        assert_eq!(input.phase, Phase::Completed, "{input:?}");
        assert_eq!(input.rows_committed, Some(rows as u64));
        let out = export(
            &state,
            &id,
            "large",
            &files.path().join("out.csv"),
            CsvOptions::default(),
        )
        .await;
        assert_eq!(out.phase, Phase::Completed, "{out:?}");
        println!(
            "CSV_MEMORY_MIB={mib}; import_bytes={}; export_bytes={}",
            input.bytes_processed, out.bytes_processed
        );
        let oversized = files.path().join("oversized.csv");
        {
            let mut file = std::fs::File::create(&oversized).unwrap();
            file.write_all(b"a\n").unwrap();
            for _ in 0..17 {
                file.write_all(&[b'x'; 65536]).unwrap();
            }
        }
        let reviewed = inspection(
            &state,
            &id,
            "large",
            Some(&oversized),
            CsvOptions::default(),
        )
        .await
        .unwrap();
        let rejected = import(&state, reviewed, &["a"]).await;
        assert_eq!(rejected.phase, Phase::Failed);
        assert!(matches!(rejected.failure, Some(TransferError::Csv { .. })));
    })
    .catch_unwind()
    .await;
    cleanup(&state, &mut admin, &id).await;
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
