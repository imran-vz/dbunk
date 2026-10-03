//! Serial opt-in against the exact owned stage03 fixture. Each external helper
//! verifies the fixture instance before SQL; every effect is checked by that
//! independent helper. Cleanup checks OID/owner/comment and uses RESTRICT.
use super::*;
use crate::{backend::profile, postgres::dedicated::DriverJoins};
use futures_util::FutureExt;
use std::{io::Write, path::Path, process::Stdio, time::Duration};
/// The operator names the exact owned instance; the helper then refuses any
/// other fixture. Fixture instances are disposable, so none is hard-coded.
fn fixture() -> String {
    std::env::var("DBUNK_NATIVE_FIXTURE_INSTANCE")
        .expect("set DBUNK_NATIVE_FIXTURE_INSTANCE to the verified owned stage03 instance")
}
const ORD: &str = "ord\"字";
const STALE: &str = "stale\"字";
const TABLE: &str = "ident\"字";
const IDSEQ: &str = "idseq\"字";
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
    fn reference(&self, name: &str) -> PgObjectRef {
        PgObjectRef {
            kind: PgObjectKind::Sequence,
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
                let mut child = std::process::Command::new("python3")
                    .arg("-c")
                    .arg(FIXTURE_HELPER)
                    .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools/native"))
                    .arg(fixture())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| e.to_string())?;
                let guarded = format!("SET application_name={}; SET statement_timeout='10s'; SET lock_timeout='3s'; {query}", crate::quote_literal(&application));
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(guarded.as_bytes())
                    .map_err(|e| e.to_string())?;
                let output = child.wait_with_output().map_err(|e| e.to_string())?;
                if !output.status.success() {
                    return Err(String::from_utf8_lossy(&output.stderr).into_owned());
                }
                String::from_utf8(output.stdout)
                    .map(|s| s.trim().to_owned())
                    .map_err(|e| e.to_string())
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result);
            let _ = send.send(result);
        });
        self.helpers.track_task(task);
        receive.await.map_err(|error| error.to_string())?
    }
    async fn guard(&self) -> u64 {
        self.sql("SELECT count(*) FROM pg_catalog.pg_stat_activity WHERE datname='dbunk_demo' AND pid<>pg_backend_pid();".into()).await.unwrap().parse().unwrap()
    }
    /// Independent read of (last_value, is_called); never calls nextval.
    async fn value(&self, name: &str) -> String {
        self.sql(format!(
            "SELECT last_value||','||is_called FROM {}",
            self.qualified(name)
        ))
        .await
        .unwrap()
    }
    async fn observe(
        &self,
        backend: &Backend,
        document: &DataDocument,
        name: &str,
    ) -> ObservedSequence {
        let before = self.value(name).await;
        let observed = backend
            .observe_sequence(document, self.reference(name))
            .await
            .unwrap();
        assert_eq!(
            self.value(name).await,
            before,
            "inspection must never change the sequence"
        );
        let SequenceValue::Read {
            last_value,
            is_called,
        } = observed.observation().value
        else {
            panic!("owner can read its sequence")
        };
        assert_eq!(format!("{last_value},{is_called}"), before);
        observed
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
                ORD | STALE | IDSEQ => "S",
                TABLE => "r",
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
        // The identity sequence is an internal child of its verified table.
        for (name, command) in [(TABLE, "TABLE"), (ORD, "SEQUENCE"), (STALE, "SEQUENCE")] {
            if objects.iter().any(|row| row["name"].as_str() == Some(name)) {
                drops.push_str(&format!(
                    "DROP {command} {} RESTRICT;",
                    self.qualified(name)
                ));
            }
        }
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
async fn configure(backend: &Backend, mode: crate::SafeMode, read_only: bool) {
    let mut stored =
        crate::storage::read_connection_by_id(&backend.0.state.pool, &backend.fixture().id)
            .await
            .unwrap()
            .unwrap();
    let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
        panic!("PostgreSQL")
    };
    pg.safe_mode = mode;
    pg.read_only = read_only;
    crate::storage::upsert_connection(&backend.0.state.pool, &stored)
        .await
        .unwrap();
}
/// Submit once; if policy asks, confirm the identical attempt exactly once.
async fn submit(
    backend: &Backend,
    review: SequenceReview,
    expect_confirmation: bool,
) -> SequenceReceipt {
    let attempt = review.attempt_id().to_owned();
    let preview = review.preview().clone();
    let receipt = match backend.apply_sequence(review).await.unwrap() {
        SequenceSubmission::NeedsConfirmation(confirmation) => {
            assert!(expect_confirmation, "unexpected confirmation request");
            assert_eq!(confirmation.review().attempt_id(), attempt);
            assert_eq!(confirmation.review().preview(), &preview);
            match backend.confirm_sequence(*confirmation).await.unwrap() {
                SequenceSubmission::Finished(receipt) => *receipt,
                _ => panic!("confirmation repeated"),
            }
        }
        SequenceSubmission::Finished(receipt) => {
            assert!(!expect_confirmation, "policy confirmation was skipped");
            *receipt
        }
    };
    assert_eq!(receipt.attempt_id, attempt);
    assert!(receipt.retained_bytes() < MAX_SEQUENCE_RECEIPT_BYTES);
    receipt
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "exact owned stage03 fixture UUID, DBUNK_NATIVE_FIXTURE_VERIFIED=1 and exclusive serial fixture writer"]
async fn native_sequence_inspect_writes_policy_stale_identity_and_cleanup() {
    assert_eq!(
        std::env::var("DBUNK_NATIVE_FIXTURE_VERIFIED").as_deref(),
        Ok("1")
    );
    let run = uuid::Uuid::new_v4().simple().to_string();
    let probe = Probe {
        schema: format!("native_seq_{run}\"字"),
        comment: format!("owned native sequence {run}"),
        application: format!("native_seq_probe_{run}"),
        helpers: DriverJoins::default(),
    };
    println!(
        "fixture={} endpoint=127.0.0.1:15432/dbunk_demo schema={} helper_client={}",
        fixture(),
        probe.schema,
        probe.application
    );
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
    let mut backend = None;
    let mut schema_oid = None;
    let operation = async {
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        configure(backend, crate::SafeMode::Protected, false).await;
        let document = backend
            .open_data_document("sequence-live", "owned", &backend.fixture().id)
            .await
            .unwrap();
        let comment = crate::quote_literal(&probe.comment);
        let setup = format!("BEGIN; CREATE SCHEMA {schema}; COMMENT ON SCHEMA {schema} IS {comment}; CREATE SEQUENCE {ord} AS integer INCREMENT BY 5 MINVALUE 1 MAXVALUE 1000 START WITH 10; COMMENT ON SEQUENCE {ord} IS {comment}; CREATE SEQUENCE {stale}; COMMENT ON SEQUENCE {stale} IS {comment}; CREATE TABLE {table} (id bigint GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME {idseq} START WITH 100), note text); COMMENT ON TABLE {table} IS {comment}; COMMENT ON SEQUENCE {idseq} IS {comment}; COMMIT; SELECT oid FROM pg_namespace WHERE nspname={schema_literal};", schema=crate::quote_double(&probe.schema), schema_literal=crate::quote_literal(&probe.schema), ord=probe.qualified(ORD), stale=probe.qualified(STALE), table=probe.qualified(TABLE), idseq=probe.qualified(IDSEQ));
        schema_oid = Some(probe.sql(setup).await.unwrap().parse::<u32>().unwrap());
        println!("created owned schema oid={}", schema_oid.unwrap());

        // Inspect: exact metadata; independent value unchanged before/after.
        let ord = probe.observe(backend, &document, ORD).await;
        let observation = ord.observation();
        assert_eq!(observation.target.schema(), probe.schema);
        assert_eq!(observation.target.name(), ORD);
        assert_eq!(
            observation.definition,
            SequenceDefinition {
                data_type: SequenceDataType::Integer,
                start: 10,
                increment: 5,
                min_value: 1,
                max_value: 1000,
                cache: 1,
                cycle: false,
            }
        );
        assert_eq!(observation.owned_by, None);
        assert_eq!(probe.value(ORD).await, "10,false");
        let ident = probe.observe(backend, &document, IDSEQ).await;
        assert!(ident.observation().identity);
        assert_eq!(
            ident.observation().owned_by.as_deref(),
            Some(
                format!(
                    "{}.{}.id",
                    crate::quote_double(&probe.schema),
                    crate::quote_double(TABLE)
                )
                .as_str()
            )
        );
        println!(
            "inspect ord value=10,f unchanged; identity owned_by={:?}",
            ident.observation().owned_by
        );

        // Out-of-range refusal before any dispatch.
        for intent in [
            SequenceIntent::Set {
                value: 1001,
                is_called: true,
            },
            SequenceIntent::Set {
                value: 0,
                is_called: false,
            },
            SequenceIntent::Restart { with: Some(0) },
            SequenceIntent::Restart {
                with: Some(i64::from(i32::MAX)),
            },
        ] {
            assert!(matches!(ord.review(intent), Err(SequenceError::OutOfRange)));
        }
        assert_eq!(probe.value(ORD).await, "10,false");
        println!("out-of-range Set/Restart refused before review; value unchanged");

        // Protected: Advance applies without confirmation.
        let receipt = submit(backend, ord.review(SequenceIntent::Advance).unwrap(), false).await;
        assert_eq!(
            receipt.outcome,
            SequenceOutcome::Completed { returned: Some(10) }
        );
        assert_eq!(probe.value(ORD).await, "10,true");
        let ord = probe.observe(backend, &document, ORD).await;
        let receipt = submit(backend, ord.review(SequenceIntent::Advance).unwrap(), false).await;
        assert_eq!(
            receipt.outcome,
            SequenceOutcome::Completed { returned: Some(15) }
        );
        assert_eq!(probe.value(ORD).await, "15,true");
        println!("protected advance returned 10 then 15; helper sees 15,t");

        // Protected: Set requires confirmation; is_called true/false.
        for (value, is_called, expected) in [(100, true, "100,true"), (50, false, "50,false")] {
            let ord = probe.observe(backend, &document, ORD).await;
            let receipt = submit(
                backend,
                ord.review(SequenceIntent::Set { value, is_called })
                    .unwrap(),
                true,
            )
            .await;
            assert_eq!(
                receipt.outcome,
                SequenceOutcome::Completed {
                    returned: Some(value)
                }
            );
            assert_eq!(probe.value(ORD).await, expected);
        }
        let ord = probe.observe(backend, &document, ORD).await;
        let receipt = submit(backend, ord.review(SequenceIntent::Advance).unwrap(), false).await;
        assert_eq!(
            receipt.outcome,
            SequenceOutcome::Completed { returned: Some(50) }
        );
        assert_eq!(probe.value(ORD).await, "50,true");
        println!(
            "confirmed setval(100,true)=100,t; setval(50,false)=50,f; next nextval returned 50"
        );

        // Protected: Restart requires confirmation; with and without WITH.
        for (with, expected) in [(Some(500), "500,false"), (None, "10,false")] {
            let ord = probe.observe(backend, &document, ORD).await;
            let receipt = submit(
                backend,
                ord.review(SequenceIntent::Restart { with }).unwrap(),
                true,
            )
            .await;
            assert_eq!(
                receipt.outcome,
                SequenceOutcome::Completed { returned: None }
            );
            assert_eq!(probe.value(ORD).await, expected);
        }
        println!("confirmed RESTART WITH 500 -> 500,f; RESTART -> 10,f");

        // Identity-column sequence: Advance and Restart.
        let receipt = submit(
            backend,
            ident.review(SequenceIntent::Advance).unwrap(),
            false,
        )
        .await;
        assert_eq!(
            receipt.outcome,
            SequenceOutcome::Completed {
                returned: Some(100)
            }
        );
        assert_eq!(probe.value(IDSEQ).await, "100,true");
        let ident = probe.observe(backend, &document, IDSEQ).await;
        let receipt = submit(
            backend,
            ident
                .review(SequenceIntent::Restart { with: Some(200) })
                .unwrap(),
            true,
        )
        .await;
        assert_eq!(
            receipt.outcome,
            SequenceOutcome::Completed { returned: None }
        );
        assert_eq!(
            probe
                .sql(format!(
                    "INSERT INTO {} (note) VALUES ('probe') RETURNING id",
                    probe.qualified(TABLE)
                ))
                .await
                .unwrap()
                .lines()
                .next(),
            Some("200")
        );
        println!("identity sequence advance returned 100; restart 200 used by next insert");

        // Stale identity: drop and recreate the same name; nothing applied.
        let stale = probe.observe(backend, &document, STALE).await;
        let old_oid = stale.observation().target.sequence_oid();
        let advance = stale.review(SequenceIntent::Advance).unwrap();
        let restart = stale
            .review(SequenceIntent::Restart { with: Some(7) })
            .unwrap();
        probe.sql(format!("BEGIN; DO $guard$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_class WHERE oid={old_oid} AND relnamespace={} AND relname={} AND relkind='S' AND obj_description(oid,'pg_class')={comment} AND pg_get_userbyid(relowner)='dbunk') THEN RAISE EXCEPTION 'replacement ownership changed'; END IF; END $guard$; DROP SEQUENCE {stale} RESTRICT; CREATE SEQUENCE {stale}; COMMENT ON SEQUENCE {stale} IS {comment}; COMMIT;", schema_oid.unwrap(), crate::quote_literal(STALE), stale=probe.qualified(STALE))).await.unwrap();
        // Restart needs confirmation only after its guard passes; Protected asks
        // first, so confirm it and expect the guard to refuse.
        assert_eq!(
            submit(backend, advance, false).await.outcome,
            SequenceOutcome::TargetChanged
        );
        assert_eq!(
            submit(backend, restart, true).await.outcome,
            SequenceOutcome::TargetChanged
        );
        assert_eq!(probe.value(STALE).await, "1,false");
        let replacement = probe.observe(backend, &document, STALE).await;
        assert_ne!(replacement.observation().target.sequence_oid(), old_oid);
        println!(
            "stale drop/recreate refused advance+restart old_oid={old_oid} new_oid={} new value 1,f untouched",
            replacement.observation().target.sequence_oid()
        );

        // Stale definition: concurrent ALTER of INCREMENT refuses the review.
        let ord = probe.observe(backend, &document, ORD).await;
        let review = ord.review(SequenceIntent::Advance).unwrap();
        probe
            .sql(format!(
                "ALTER SEQUENCE {} INCREMENT BY 7",
                probe.qualified(ORD)
            ))
            .await
            .unwrap();
        assert_eq!(
            submit(backend, review, false).await.outcome,
            SequenceOutcome::TargetChanged
        );
        assert_eq!(probe.value(ORD).await, "10,false");
        println!("stale definition (increment changed) refused; value 10,f untouched");

        // Strict: Advance also requires confirmation.
        configure(backend, crate::SafeMode::Strict, false).await;
        let ord = probe.observe(backend, &document, ORD).await;
        let receipt = submit(backend, ord.review(SequenceIntent::Advance).unwrap(), true).await;
        assert_eq!(
            receipt.outcome,
            SequenceOutcome::Completed { returned: Some(10) }
        );
        assert_eq!(probe.value(ORD).await, "10,true");
        println!("strict advance confirmed and returned 10");

        // Stored read-only refuses every write; inspection still allowed.
        configure(backend, crate::SafeMode::Disabled, true).await;
        let ord = probe.observe(backend, &document, ORD).await;
        for intent in [
            SequenceIntent::Advance,
            SequenceIntent::Set {
                value: 20,
                is_called: true,
            },
            SequenceIntent::Restart { with: None },
        ] {
            assert!(matches!(
                backend.apply_sequence(ord.review(intent).unwrap()).await,
                Err(SequenceError::PolicyBlocked)
            ));
        }
        assert_eq!(probe.value(ORD).await, "10,true");
        println!("read-only stored policy refused advance/set/restart; value 10,t untouched");

        let audits =
            crate::storage::read_safety_overrides(&backend.0.state.pool, &backend.fixture().id)
                .await
                .unwrap();
        // Two confirmed setval, two ord restarts, one identity restart, one
        // strict advance. The stale confirmed restart did not complete.
        println!(
            "audited confirmed completions={} (set_sequence={}, restart_sequence={}, advance_sequence={})",
            audits.len(),
            audits.iter().filter(|a| a.command == "set_sequence").count(),
            audits.iter().filter(|a| a.command == "restart_sequence").count(),
            audits.iter().filter(|a| a.command == "advance_sequence").count()
        );
        assert_eq!(audits.len(), 6);
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(150), operation))
            .catch_unwind()
            .await;
    let shutdown = match backend {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
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
    shutdown.expect("sequence backend and drivers joined");
    cleaned.expect("only owned objects removed with RESTRICT");
    println!(
        "joined cleanup activity baseline={baseline} final={}",
        final_count.expect("activity returned to baseline")
    );
    result
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        .expect("bounded sequence live probe deadline");
}
