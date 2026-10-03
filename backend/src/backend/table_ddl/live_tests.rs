//! Opt-in stage03 only. No global event triggers, roles or baseline objects.
use super::*;
use crate::backend::profile;
use crate::postgres::native_table_ddl::test_transport::HookPoint;
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};
const FIXTURE: &str = "2283820d-33ec-4c4c-ae03-7051092bd410";
async fn sql(query: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move|| {
        let mut child=std::process::Command::new("python3").arg("-c").arg("import sys; sys.path.insert(0,sys.argv[1]); import fixture; owned,target=fixture.check(); assert owned['instance']==sys.argv[2], 'foreign fixture'; print(fixture.sql(target,sys.stdin.read()))")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native")).arg(FIXTURE).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|e|e.to_string())?;
        child.stdin.take().unwrap().write_all(query.as_bytes()).map_err(|e|e.to_string())?;
        let output=child.wait_with_output().map_err(|e|e.to_string())?;
        if !output.status.success(){ return Err(String::from_utf8_lossy(&output.stderr).into_owned()); }
        String::from_utf8(output.stdout).map(|s|s.trim().into()).map_err(|e|e.to_string())
    }).await.map_err(|e|e.to_string())?
}
async fn activity() -> u64 {
    sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid()".into()).await.unwrap().parse().unwrap()
}
fn qualified(schema: &str, table: &str) -> String {
    format!(
        "{}.{}",
        crate::quote_double(schema),
        crate::quote_double(table)
    )
}
fn request(schema: &str, table: &str, column: Option<&str>) -> TableDdlRequest {
    TableDdlRequest {
        schema: schema.into(),
        table: table.into(),
        column: column.map(str::to_owned),
        expected: None,
    }
}
async fn apply(backend: &Backend, review: TableDdlReview) -> TableDdlReceipt {
    let attempt = review.attempt_id().clone();
    let result = match backend.apply_table_ddl(review).await.unwrap() {
        TableDdlSubmission::NeedsConfirmation(c) => backend.confirm_table_ddl(*c).await.unwrap(),
        value => value,
    };
    let TableDdlSubmission::Finished(receipt) = result else {
        panic!("confirmation repeated")
    };
    assert_eq!(receipt.attempt_id, attempt);
    *receipt
}
async fn inspect(
    backend: &Backend,
    document: &DataDocument,
    schema: &str,
    table: &str,
    column: Option<&str>,
) -> TableDdlTarget {
    backend
        .observe_table_ddl(document, request(schema, table, column))
        .await
        .unwrap()
}
#[derive(Clone)]
struct Owned {
    schema: String,
    schema_oid: u32,
    table_oid: u32,
    marker: String,
}
async fn cleanup(owned: &Owned) -> Result<(), String> {
    // Record ownership before CREATE. If setup fails after COMMIT but before
    // decoding its OIDs, recover only the absent-before, uniquely marked name.
    let resolved;
    let owned = if owned.schema_oid == 0 || owned.table_oid == 0 {
        let raw = sql(format!("SELECT json_build_object('schema',n.oid::bigint,'table',c.oid::bigint)::text FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid WHERE n.nspname={} AND pg_catalog.pg_get_userbyid(n.nspowner)='dbunk' AND pg_catalog.pg_get_userbyid(c.relowner)='dbunk' AND pg_catalog.obj_description(n.oid,'pg_namespace')={} AND c.relname='quoted table'",crate::quote_literal(&owned.schema),crate::quote_literal(&owned.marker))).await?;
        if raw.is_empty() {
            let absent = sql(format!(
                "SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={})",
                crate::quote_literal(&owned.schema)
            ))
            .await?;
            return if absent == "t" {
                Ok(())
            } else {
                Err("Unresolved setup identity; preserved".into())
            };
        }
        let row: serde_json::Value =
            serde_json::from_str(&raw).map_err(|_| "Invalid cleanup identity")?;
        resolved = Owned {
            schema: owned.schema.clone(),
            marker: owned.marker.clone(),
            schema_oid: row["schema"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Invalid namespace OID")?,
            table_oid: row["table"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Invalid relation OID")?,
        };
        &resolved
    } else {
        owned
    };

    // Use recorded OIDs, ownership and private schema marker. Locate the current
    // names only within that owned identity, then drop only its one known table.
    let query = format!(
        r#"BEGIN; DO $guard$ DECLARE s text; t text; BEGIN
      SELECT nspname INTO s FROM pg_catalog.pg_namespace WHERE oid={} AND nspowner=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname='dbunk') AND pg_catalog.obj_description(oid,'pg_namespace')={};
      IF s IS NULL OR s NOT LIKE 'native_table_ddl_20261003_%' THEN RAISE EXCEPTION 'owned schema identity changed'; END IF;
      SELECT relname INTO t FROM pg_catalog.pg_class WHERE oid={} AND relnamespace={} AND relowner=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname='dbunk');
      IF t IS NULL THEN RAISE EXCEPTION 'owned table identity changed'; END IF;
      EXECUTE pg_catalog.format('DROP TABLE %I.%I RESTRICT',s,t);
      EXECUTE pg_catalog.format('DROP SCHEMA %I RESTRICT',s);
      END $guard$; COMMIT;"#,
        owned.schema_oid,
        crate::quote_literal(&owned.marker),
        owned.table_oid,
        owned.schema_oid
    );
    sql(query).await?;
    assert_eq!(sql(format!("SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE oid={}) AND NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid={})",owned.schema_oid,owned.table_oid)).await?,"t");
    println!(
        "cleanup name={} schema_oid={} table_oid={} owner=dbunk RESTRICT absent=true",
        owned.schema, owned.schema_oid, owned.table_oid
    );
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 table DDL and deterministic schema ABA races; serial explicit opt-in"]
async fn native_table_ddl_live_exact_changes_races_and_joined_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let baseline = activity().await;
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let names = [
        format!("native_table_ddl_20261003_{}a", &suffix[..24]),
        format!("native_table_ddl_20261003_{}b", &suffix[..24]),
    ];
    let spare = format!("native_table_ddl_20261003_{}x", &suffix[..24]);
    println!("target fixture={FIXTURE} endpoint=127.0.0.1:15432/dbunk_demo schemas={names:?} spare={spare} baseline={baseline}");
    let directory = profile::directory();
    println!("isolated profile={}", directory.path().display());
    let mut backend = None;
    let mut owned = Vec::new();
    let operation = async {
        for name in names.iter().chain(std::iter::once(&spare)) {
            assert_eq!(
                sql(format!(
                    "SELECT NOT EXISTS(SELECT 1 FROM pg_catalog.pg_namespace WHERE nspname={})",
                    crate::quote_literal(name)
                ))
                .await
                .unwrap(),
                "t"
            );
        }
        for name in &names {
            let marker = format!("owned table DDL {suffix} {name}");
            owned.push(Owned {
                schema: name.clone(),
                schema_oid: 0,
                table_oid: 0,
                marker: marker.clone(),
            });
            let result=sql(format!("BEGIN; CREATE SCHEMA {}; COMMENT ON SCHEMA {} IS {}; CREATE TABLE {} (\"odd column\" integer); SELECT json_build_object('schema',n.oid::bigint,'table',c.oid::bigint)::text FROM pg_catalog.pg_namespace n JOIN pg_catalog.pg_class c ON c.relnamespace=n.oid WHERE n.nspname={} AND c.relname='quoted table'; COMMIT;",crate::quote_double(name),crate::quote_double(name),crate::quote_literal(&marker),qualified(name,"quoted table"),crate::quote_literal(name))).await.unwrap();
            let value: serde_json::Value = serde_json::from_str(&result).unwrap();
            let item = Owned {
                schema: name.clone(),
                schema_oid: value["schema"].as_u64().unwrap() as u32,
                table_oid: value["table"].as_u64().unwrap() as u32,
                marker,
            };
            println!(
                "created schema={} oid={} table_oid={}",
                item.schema, item.schema_oid, item.table_oid
            );
            *owned.last_mut().unwrap() = item;
        }
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let b = backend.as_ref().unwrap();
        let document = b
            .open_data_document("table-ddl-live", "one", &b.fixture().id)
            .await
            .unwrap();
        super::tests::policy(b, false, None).await;
        for (column, intent) in [
            (
                None,
                TableDdlIntent::SetComment {
                    comment: Some("exact ' \\ 日本語".into()),
                },
            ),
            (
                Some("odd column"),
                TableDdlIntent::SetComment {
                    comment: Some(String::new()),
                },
            ),
            (
                Some("odd column"),
                TableDdlIntent::Rename {
                    new_name: " ".into(),
                },
            ),
            (Some(" "), TableDdlIntent::SetComment { comment: None }),
            (
                None,
                TableDdlIntent::Rename {
                    new_name: "renamed table".into(),
                },
            ),
        ] {
            let target = inspect(b, &document, &names[0], "quoted table", column).await;
            let review = b.review_table_ddl(target, intent.clone()).await.unwrap();
            let receipt = apply(b, review).await;
            assert!(
                matches!(receipt.outcome, TableDdlOutcome::Applied { .. }),
                "{:?}",
                receipt.outcome
            );
            let current_table =
                if matches!(intent, TableDdlIntent::Rename { .. }) && column.is_none() {
                    "renamed table"
                } else {
                    "quoted table"
                };
            let current_column =
                if matches!(intent, TableDdlIntent::Rename { .. }) && column.is_some() {
                    Some(" ")
                } else {
                    column
                };
            let current = inspect(b, &document, &names[0], current_table, current_column).await;
            if let TableDdlIntent::SetComment { comment } = intent {
                assert_eq!(
                    current.description().comment,
                    comment.filter(|text| !text.is_empty())
                );
            }
        }
        // Restore the original table spelling for deterministic swaps of names.
        let target = inspect(b, &document, &names[0], "renamed table", None).await;
        let review = b
            .review_table_ddl(
                target,
                TableDdlIntent::Rename {
                    new_name: "quoted table".into(),
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            apply(b, review).await.outcome,
            TableDdlOutcome::Applied { .. }
        ));
        for aba in [false, true] {
            let target = inspect(b, &document, &names[0], "quoted table", None).await;
            let review = b
                .review_table_ddl(
                    target,
                    TableDdlIntent::SetComment {
                        comment: Some("must rollback".into()),
                    },
                )
                .await
                .unwrap();
            let a = names[0].clone();
            let other = names[1].clone();
            let temp = spare.clone();
            let swap=format!("BEGIN; ALTER SCHEMA {} RENAME TO {}; ALTER SCHEMA {} RENAME TO {}; ALTER SCHEMA {} RENAME TO {}; COMMIT;",crate::quote_double(&a),crate::quote_double(&temp),crate::quote_double(&other),crate::quote_double(&a),crate::quote_double(&temp),crate::quote_double(&other));
            let swap_back = swap.clone();
            let hook: native_table_ddl::test_transport::Hook = Box::new(move |capture| {
                let query = swap.clone();
                Box::pin(async move {
                    if capture == HookPoint::Captured(2) {
                        sql(query.clone())
                            .await
                            .map_err(|_| TableDdlFailure::Connection)?;
                        if aba {
                            sql(query).await.map_err(|_| TableDdlFailure::Connection)?;
                        }
                    }
                    Ok(())
                })
            });
            let result = b
                .submit(
                    review,
                    true,
                    move |spec, drivers, permit, cancellation, target, intent, preview| {
                        native_table_ddl::test_transport::execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            target,
                            intent,
                            preview,
                            hook,
                        )
                    },
                    load,
                )
                .await
                .unwrap();
            let TableDdlSubmission::Finished(receipt) = result else {
                panic!()
            };
            assert!(
                matches!(
                    receipt.outcome,
                    TableDdlOutcome::RolledBack {
                        reason: TableDdlFailure::TargetChanged
                    }
                ),
                "race aba={aba} {:?}",
                receipt.outcome
            );
            if !aba {
                sql(swap_back).await.unwrap();
            }
            for item in &owned {
                let comment = sql(format!(
                    "SELECT COALESCE(pg_catalog.obj_description({},'pg_class'),'NULL')",
                    item.table_oid
                ))
                .await
                .unwrap();
                assert_ne!(comment, "must rollback");
            }
            println!("race external_namespace_swap aba={aba} transactional_rollback=true");
        }
        // A cancelled caller after the DDL reply but before COMMIT must obtain
        // an acknowledged rollback, even though the statement already ran.
        let target = inspect(b, &document, &names[0], "quoted table", None).await;
        let original_comment = target.description().comment.clone();
        let review = b
            .review_table_ddl(
                target,
                TableDdlIntent::SetComment {
                    comment: Some("cancelled change".into()),
                },
            )
            .await
            .unwrap();
        let (ready, reached) = tokio::sync::oneshot::channel();
        let mut ready = Some(ready);
        let hook: native_table_ddl::test_transport::Hook = Box::new(move |capture| {
            let signal = if capture == HookPoint::Captured(3) {
                ready.take()
            } else {
                None
            };
            Box::pin(async move {
                if let Some(signal) = signal {
                    let _ = signal.send(());
                    std::future::pending::<()>().await;
                }
                Ok(())
            })
        });
        let runner = b.clone();
        let job = tokio::spawn(async move {
            runner
                .submit(
                    review,
                    true,
                    move |spec, drivers, permit, cancellation, target, intent, preview| {
                        native_table_ddl::test_transport::execute(
                            spec,
                            drivers,
                            permit,
                            cancellation,
                            target,
                            intent,
                            preview,
                            hook,
                        )
                    },
                    load,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), reached)
            .await
            .unwrap()
            .unwrap();
        b.cancel_data(&document).await.unwrap();
        let TableDdlSubmission::Finished(receipt) = job.await.unwrap().unwrap() else {
            panic!()
        };
        assert!(matches!(
            receipt.outcome,
            TableDdlOutcome::RolledBack {
                reason: TableDdlFailure::Cancelled
            }
        ));
        assert_eq!(
            inspect(b, &document, &names[0], "quoted table", None)
                .await
                .description()
                .comment,
            original_comment
        );
        println!("cancel_after_statement_before_commit=acknowledged_rollback");

        // A real committed change with an intentionally suppressed runner
        // acknowledgement must stay Unknown, never be rolled back or retried.
        let target = inspect(b, &document, &names[0], "quoted table", None).await;
        let intent = TableDdlIntent::SetComment {
            comment: Some("committed with lost acknowledgement".into()),
        };
        let review = b.review_table_ddl(target, intent.clone()).await.unwrap();
        let expected_target = review.target().clone();
        let attempt = review.attempt_id().clone();
        let commits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed_commits = commits.clone();
        let hook: native_table_ddl::test_transport::Hook = Box::new(move |point| {
            let observed_commits = observed_commits.clone();
            Box::pin(async move {
                if point == HookPoint::Committed {
                    observed_commits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    return Err(TableDdlFailure::Connection);
                }
                Ok(())
            })
        });
        let result = b
            .submit(
                review,
                true,
                move |spec, drivers, permit, cancellation, target, intent, preview| {
                    native_table_ddl::test_transport::execute(
                        spec,
                        drivers,
                        permit,
                        cancellation,
                        target,
                        intent,
                        preview,
                        hook,
                    )
                },
                load,
            )
            .await
            .unwrap();
        let TableDdlSubmission::Finished(receipt) = result else {
            panic!()
        };
        assert_eq!(receipt.attempt_id, attempt);
        assert_eq!(receipt.target, expected_target);
        assert_eq!(receipt.intent, intent);
        assert_eq!(receipt.connection_id, b.fixture().id);
        assert_eq!(
            receipt.outcome,
            TableDdlOutcome::OutcomeUnknown {
                reason: TableDdlFailure::Connection,
            }
        );
        assert_eq!(commits.load(std::sync::atomic::Ordering::SeqCst), 1);
        // Independent connection proves the commit happened despite Unknown.
        assert_eq!(
            sql(format!(
                "SELECT pg_catalog.obj_description({},'pg_class')",
                owned[0].table_oid,
            ))
            .await
            .unwrap(),
            "committed with lost acknowledgement"
        );
        // A fresh read remains available after joined cleanup; it is explicit
        // reconciliation, not reconstruction of authority from this receipt.
        assert_eq!(
            inspect(b, &document, &names[0], "quoted table", None)
                .await
                .description()
                .comment
                .as_deref(),
            Some("committed with lost acknowledgement")
        );
        println!("lost_commit_runner_ack=outcome_unknown real_commit_visible=true commit_count=1 exact_receipt=true explicit_reconciliation=true");

        // Replacing a selected table at the same name must never adopt its OID.
        let target = inspect(b, &document, &names[1], "quoted table", None).await;
        let review = b
            .review_table_ddl(
                target,
                TableDdlIntent::SetComment {
                    comment: Some("must not reach replacement".into()),
                },
            )
            .await
            .unwrap();
        let previous_oid = owned[1].table_oid;
        owned[1].table_oid = 0; // marked-name cleanup also owns the replacement on a decode failure.
        let replacement = sql(format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_class WHERE oid={} AND relnamespace={} AND relname='quoted table' AND relowner=(SELECT oid FROM pg_catalog.pg_roles WHERE rolname='dbunk')) THEN RAISE EXCEPTION 'owned table identity changed'; END IF; END $guard$; DROP TABLE {} RESTRICT; CREATE TABLE {} (id integer); SELECT c.oid FROM pg_catalog.pg_class c WHERE c.relnamespace={} AND c.relname='quoted table'; COMMIT;", previous_oid, owned[1].schema_oid, qualified(&names[1], "quoted table"), qualified(&names[1], "quoted table"), owned[1].schema_oid)).await.unwrap();
        owned[1].table_oid = replacement.parse().unwrap();
        assert_ne!(previous_oid, owned[1].table_oid);
        assert!(matches!(
            apply(b, review).await.outcome,
            TableDdlOutcome::NotDispatched {
                reason: TableDdlFailure::TargetChanged
            }
        ));
        assert_eq!(
            inspect(b, &document, &names[1], "quoted table", None)
                .await
                .description()
                .comment,
            None
        );
        println!(
            "stale_oid_refused=true replaced_owned_oid={previous_oid} replacement_oid={}",
            owned[1].table_oid
        );
        let stale = inspect(b, &document, &names[0], "quoted table", None).await;
        b.cancel_data(&document).await.unwrap();
        assert!(matches!(
            b.review_table_ddl(stale, TableDdlIntent::SetComment { comment: None })
                .await,
            Err(TableDdlError::Unavailable)
        ));
        b.close_data_document(&document).await.unwrap();
        let audits = crate::storage::read_safety_overrides(&b.0.state.pool, &b.fixture().id)
            .await
            .unwrap();
        assert_eq!(audits.len(), 6);
        println!(
            "confirmed_success_audits={} rollback_added_audits=0",
            audits.len()
        );
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(150), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(b) => b.shutdown().await,
        None => Ok(()),
    };
    let mut cleanup_results = Vec::new();
    for item in &owned {
        cleanup_results.push(cleanup(item).await);
    }
    let final_activity = activity().await;
    println!(
        "fixture={FIXTURE} baseline={baseline} final={final_activity} joined_before_cleanup=true"
    );
    shutdown.unwrap();
    for cleaned in cleanup_results {
        cleaned.unwrap();
    }
    assert_eq!(final_activity, baseline);
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("bounded live deadline");
}
