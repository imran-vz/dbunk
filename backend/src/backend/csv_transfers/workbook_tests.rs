use super::*;
use crate::backend::profile;
use std::io::Write;
use zip::write::SimpleFileOptions;
fn workbook_file() -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    let mut zip = zip::ZipWriter::new(file.as_file_mut());
    for (name, xml) in [
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="o" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
        ),
        (
            "xl/workbook.xml",
            r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Data" sheetId="1" r:id="s" state="veryHidden"/></sheets></workbook>"#,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="s" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/s.xml"/></Relationships>"#,
        ),
        (
            "xl/worksheets/s.xml",
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row><c t="inlineStr"><is><t>value</t></is></c></row><row><c><v>9223372036854775807</v></c></row></sheetData></worksheet>"#,
        ),
    ] {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    file
}
async fn wait(backend: &Backend, id: CsvInspectionId, phase: CsvInspectionPhase) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let row = backend.get_csv_inspection(id).unwrap();
            if row.phase == phase && row.cleanup == CsvCleanup::Complete {
                return;
            }
            assert!(!matches!(row.phase, CsvInspectionPhase::Failed), "{row:?}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
async fn ready(
    backend: &Backend,
    file: &tempfile::NamedTempFile,
) -> (CsvInspectionId, CsvWorkbook) {
    let id = CsvInspectionId::new();
    backend
        .begin_csv_inspection(
            id,
            profile::CONNECTION_ID.into(),
            CsvInspectionIntent::xlsx(
                file.path().to_owned(),
                super::super::tests::target(),
                "NULL".into(),
            )
            .unwrap(),
        )
        .unwrap();
    wait(backend, id, CsvInspectionPhase::WorkbookReady).await;
    (id, backend.csv_workbook(id).unwrap())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selection_uses_private_snapshot_and_revokes_previous_mapping_authority() {
    let (_directory, backend) = super::super::tests::backend().await;
    let file = workbook_file();
    let (id, book) = ready(&backend, &file).await;
    assert_eq!(
        book.data().sheets[0].visibility,
        CsvWorkbookVisibility::VeryHidden
    );
    std::fs::write(file.path(), b"original path replaced after snapshot").unwrap();
    backend.select_csv_workbook_sheet(book.clone(), 0).unwrap();
    wait(&backend, id, CsvInspectionPhase::Ready).await;
    let inspection = backend.csv_inspection(id).unwrap();
    let provenance = inspection.data().workbook.as_ref().unwrap();
    assert_eq!(provenance.rows, 1);
    assert_eq!(provenance.null_token, "NULL");
    assert!(provenance.header_detected);
    assert!(matches!(
        backend.select_csv_workbook_sheet(book, 0),
        Err(CsvError::StaleReview)
    ));
    let fresh = backend.csv_workbook(id).unwrap();
    backend.select_csv_workbook_sheet(fresh, 0).unwrap();
    assert!(backend
        .review_csv_import(
            inspection,
            vec![CsvMapping {
                source_index: 0,
                target_column: "value".into()
            }]
        )
        .is_err());
    wait(&backend, id, CsvInspectionPhase::Ready).await;
    backend.cancel_csv_inspection(id).unwrap();
    wait(&backend, id, CsvInspectionPhase::Cancelled).await;
    backend.release_csv_inspection(id).unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_confirmation_owns_source_after_setup_release_until_cancel_cleanup() {
    let (_directory, backend) = super::super::tests::backend().await;
    let file = workbook_file();
    let (id, book) = ready(&backend, &file).await;
    backend.select_csv_workbook_sheet(book, 0).unwrap();
    wait(&backend, id, CsvInspectionPhase::Ready).await;
    let source = backend
        .0
        .csv_transfers
        .state
        .lock()
        .unwrap()
        .inspections
        .get(&id)
        .unwrap()
        .source
        .clone()
        .unwrap();
    let paths = source.paths();
    let inspection = backend.csv_inspection(id).unwrap();
    let review = backend
        .review_csv_import(
            inspection,
            vec![CsvMapping {
                source_index: 0,
                target_column: "value".into(),
            }],
        )
        .unwrap();
    let attempt = CsvTransferAttemptId::new();
    let submission = backend.begin_csv_transfer(attempt, review).unwrap();
    assert!(matches!(
        submission,
        CsvTransferSubmission::NeedsConfirmation(_)
    ));
    backend.release_csv_inspection(id).unwrap();
    assert!(paths.iter().all(|p| p.exists()));
    backend.cancel_csv_transfer(attempt).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if backend.get_csv_transfer(attempt).unwrap().cleanup == CsvCleanup::Complete {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(paths.iter().all(|p| !p.exists()));
    backend.release_csv_transfer(attempt).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_inspection_retains_artifacts_until_its_file_reader_has_joined() {
    use futures_util::FutureExt;
    let (_directory, backend) = super::super::tests::backend().await;
    let file = workbook_file();
    let (id, workbook) = ready(&backend, &file).await;
    let (started, observed) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let channels = Arc::new(std::sync::Mutex::new(Some((started, held))));
    *backend.0.csv_transfers.test_inspector.lock().unwrap() = Some(Arc::new(move |_, _, io| {
        let (started, held) = channels.lock().unwrap().take().unwrap();
        async move {
            io.file_work(move || {
                let _ = started.send(());
                held.recv().unwrap();
                Err(crate::postgres::transfer::protocol::TransferError::Cancelled)
            })
            .await
        }
        .boxed()
    }));
    backend.select_csv_workbook_sheet(workbook, 0).unwrap();
    observed.await.unwrap();
    let (source, io) = {
        let state = backend.0.csv_transfers.state.lock().unwrap();
        let entry = state.inspections.get(&id).unwrap();
        (entry.source.clone().unwrap(), entry.io.clone())
    };
    let paths = source.paths();
    backend.cancel_csv_inspection(id).unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if backend.get_csv_inspection(id).unwrap().cleanup == CsvCleanup::Failed {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        paths.iter().all(|path| path.exists()),
        "a timed-out reader is not cleanup proof"
    );
    assert!(matches!(
        backend.release_csv_inspection(id),
        Err(CsvError::Active)
    ));
    release.send(()).unwrap();
    // Explicit test teardown joins the deliberately held owner first. The
    // production failed-cleanup record remains retained rather than retried.
    io.cleanup(tokio::time::Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    source
        .cleanup(tokio::time::Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert!(paths.iter().all(|path| !path.exists()));
    assert_eq!(
        backend.get_csv_inspection(id).unwrap().cleanup,
        CsvCleanup::Failed
    );
}
