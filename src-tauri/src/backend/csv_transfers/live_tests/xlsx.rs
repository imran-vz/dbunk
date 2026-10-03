//! Opt-in parser -> private canonical artifact -> real COPY coverage. Shares the
//! exact fixture identity and guarded OID cleanup of the existing CSV probe.
use super::*;
use zip::write::SimpleFileOptions;
fn write_workbook(path: &Path) {
    let mut archive = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let headers = ["dup", "dup", "body", "notes", "quoted_null"]
        .into_iter()
        .map(|name| format!("<c t=\"inlineStr\"><is><t>{name}</t></is></c>"))
        .collect::<String>();
    let sheet = format!(
        r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row>{headers}</row><row><c><v>9223372036854775807</v></c><c><v>12345678901234567890.1234567890</v></c><c t="inlineStr"><is><t>雪,&#10;&quot;quoted&quot;</t></is></c><c t="inlineStr"><is><t>NULL</t></is></c><c t="inlineStr"><is><t>\N</t></is></c></row><row><c><v>-9223372036854775808</v></c><c><v>-0.0000000001</v></c><c t="inlineStr"><is><t></t></is></c><c t="inlineStr"><is><t>text</t></is></c><c t="inlineStr"><is><t></t></is></c></row></sheetData></worksheet>"#
    );
    for (name, xml) in [
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="o" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Exact" sheetId="1" r:id="s"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/s.xml"/></Relationships>"#,
        ),
        ("xl/worksheets/s.xml", sheet.as_str()),
    ] {
        archive
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        archive.write_all(xml.as_bytes()).unwrap();
    }
    archive.finish().unwrap().sync_all().unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "owned stage03 only; DBUNK_NATIVE_FIXTURE_VERIFIED=1; serial explicit opt-in"]
async fn native_xlsx_owned_exact_import_and_private_source_cleanup() {
    println!("XLSX probe target: owned fixture {FIXTURE}, 127.0.0.1:15432/dbunk_demo");
    check_fixture().await;
    let baseline = activity().await;
    let schema = format!("native_xlsx_{}", uuid::Uuid::new_v4().simple());
    let comment = format!("owned native XLSX probe {}", uuid::Uuid::new_v4());
    let directory = profile::directory();
    let files = tempfile::tempdir().unwrap();
    let mut identity = None;
    let mut backend = None;
    let operation = async {
        identity = Some(create_target(&schema, &comment).await);
        backend = Some(
            Backend::open_fixture(&directory.path().canonicalize().unwrap())
                .await
                .unwrap(),
        );
        let backend = backend.as_ref().unwrap();
        let mut stored =
            crate::storage::read_connection_by_id(&backend.0.state.pool, profile::CONNECTION_ID)
                .await
                .unwrap()
                .unwrap();
        let crate::StoredConnection::PostgreSQL(pg) = &mut stored else {
            panic!("PostgreSQL required")
        };
        pg.safe_mode = crate::SafeMode::Strict;
        crate::storage::upsert_connection(&backend.0.state.pool, &stored)
            .await
            .unwrap();
        *backend.0.csv_transfers.test_inspector.lock().unwrap() =
            Some(Arc::new(|connection, payload, io| {
                async move {
                    check_fixture().await;
                    runner::inspect_in(connection, payload, io).await
                }
                .boxed()
            }));
        *backend.0.csv_transfers.test_runner.lock().unwrap() =
            Some(Arc::new(|context, connection, review, request| {
                async move {
                    check_fixture().await;
                    runner::run(context, connection, review, request).await
                }
                .boxed()
            }));
        let path = files.path().join("exact.xlsx");
        write_workbook(&path);
        let id = CsvInspectionId::new();
        backend
            .begin_csv_inspection(
                id,
                profile::CONNECTION_ID.into(),
                CsvInspectionIntent::xlsx(path.clone(), target(&schema), "NULL".into()).unwrap(),
            )
            .unwrap();
        let workbook = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let row = backend.get_csv_inspection(id).unwrap();
                if row.phase == CsvInspectionPhase::WorkbookReady {
                    break backend.csv_workbook(id).unwrap();
                }
                assert_ne!(row.phase, CsvInspectionPhase::Failed, "{row:?}");
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(workbook.data().sheets[0].name, "Exact");
        // The privately copied workbook remains authoritative after this change.
        std::fs::write(&path, b"replaced original workbook").unwrap();
        backend.select_csv_workbook_sheet(workbook, 0).unwrap();
        let inspection = tokio::time::timeout(Duration::from_secs(45), async {
            loop {
                let row = backend.get_csv_inspection(id).unwrap();
                if row.phase == CsvInspectionPhase::Ready {
                    break backend.csv_inspection(id).unwrap();
                }
                assert_ne!(row.phase, CsvInspectionPhase::Failed, "{row:?}");
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(inspection.data().source_columns[0].name, "dup");
        assert_eq!(inspection.data().source_columns[1].name, "dup");
        assert_eq!(
            inspection.data().sample_rows[0][0].as_deref(),
            Some("9223372036854775807")
        );
        assert_eq!(inspection.data().sample_rows[0][3], None);
        assert_eq!(inspection.data().sample_rows[0][4].as_deref(), Some("\\N"));
        let private_paths = backend
            .0
            .csv_transfers
            .state
            .lock()
            .unwrap()
            .inspections
            .get(&id)
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .paths();
        assert_eq!(private_paths.len(), 2);
        let review = backend.review_csv_import(inspection, mapping()).unwrap();
        let attempt = start(backend, review);
        backend.release_csv_inspection(id).unwrap();
        let outcome = wait(backend, attempt).await;
        assert_eq!(outcome.effect, CsvEffect::Succeeded, "{outcome:?}");
        assert_eq!(outcome.rows_committed, Some(2));
        assert_eq!(outcome.workbook.as_ref().unwrap().sheet_name, "Exact");
        assert!(private_paths.iter().all(|path| !path.exists()));
        assert_eq!(
            rows(&schema).await,
            serde_json::json!([
                [
                    "-9223372036854775808",
                    "-0.0000000001",
                    "",
                    "text",
                    "",
                    "owned default",
                    "2",
                    "0"
                ],
                [
                    "9223372036854775807",
                    "12345678901234567890.1234567890",
                    "雪,\n\"quoted\"",
                    null,
                    "\\N",
                    "owned default",
                    "1",
                    "11"
                ]
            ])
        );
        assert_eq!(audit_count(backend).await, 1);
        println!("owned XLSX result: exact_numeric_text=true null_empty_literal=true indexed_duplicates=true original_replacement_ignored=true private_artifacts_removed=true committed_rows=2");
    };
    let result =
        std::panic::AssertUnwindSafe(tokio::time::timeout(Duration::from_secs(180), operation))
            .catch_unwind()
            .await;
    let joined = match backend.as_ref() {
        Some(backend) => backend.shutdown().await,
        None => Ok(()),
    };
    let helpers = join_sql_helpers().await;
    if joined.is_err() || helpers.is_err() {
        println!(
            "XLSX residue preserved: files={} profile={}",
            files.keep().display(),
            directory.keep().display()
        );
    }
    let cleaned = if joined.is_ok() && helpers.is_ok() {
        cleanup(&schema, &comment, identity.as_ref()).await
    } else {
        Err("XLSX objects preserved: joins incomplete".into())
    };
    let cleanup_helpers = join_sql_helpers().await;
    assert!(joined.is_ok(), "{joined:?}");
    assert!(helpers.is_ok(), "{helpers:?}");
    assert!(cleaned.is_ok(), "{cleaned:?}");
    assert!(cleanup_helpers.is_ok());
    assert_eq!(activity().await, baseline);
    assert!(join_sql_helpers().await.is_ok());
    assert!(
        matches!(result, Ok(Ok(()))),
        "XLSX probe failed after guarded cleanup"
    );
}
