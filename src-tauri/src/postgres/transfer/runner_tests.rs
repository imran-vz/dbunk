use super::*;

fn import_review() -> Review {
    let mut review = test_review("connection");
    review.payload.direction = Direction::Import;
    review.payload.source_path = Some("/tmp/source.csv".into());
    review.inspection.direction = Direction::Import;
    review.inspection.source_columns = vec![
        SourceColumn {
            index: 0,
            name: "first".into(),
        },
        SourceColumn {
            index: 1,
            name: "second".into(),
        },
    ];
    review
}

#[test]
fn import_sql_quotes_identifiers_and_uses_only_validated_options() {
    let mut review = import_review();
    review.payload.schema = "odd.schema".into();
    review.payload.table = "a\"table".into();
    review.payload.options = CsvOptions {
        delimiter: ";".into(),
        quote: "'".into(),
        escape: "\\".into(),
        null_token: "NULL".into(),
        header: true,
    };
    let statement = sql::import(&review, &[(1, "a\"column".into())]);
    assert_eq!(
        statement,
        "COPY \"odd.schema\".\"a\"\"table\" (\"a\"\"column\") FROM STDIN WITH (FORMAT csv, DELIMITER E';', QUOTE E'''', ESCAPE E'\\\\', NULL E'NULL', HEADER false, ENCODING E'UTF8')"
    );
}

#[test]
fn mapping_allows_source_reuse_but_rejects_duplicate_and_unsupported_targets() {
    let mut review = import_review();
    review.relation.columns.push(CatalogColumn {
        number: 2,
        type_oid: 25,
        type_modifier: -1,
        collation_oid: 100,
        default_fingerprint: None,
        public: TargetColumn {
            name: "copy".into(),
            data_type: "text".into(),
            nullable: true,
            has_default: false,
            generated: false,
            identity: false,
        },
    });
    assert!(validate_mapping(
        &review,
        &[
            ColumnMapping {
                source_index: 0,
                target_column: "value".into(),
            },
            ColumnMapping {
                source_index: 0,
                target_column: "copy".into(),
            },
        ]
    )
    .is_ok());
    assert!(matches!(
        validate_mapping(
            &review,
            &[
                ColumnMapping {
                    source_index: 0,
                    target_column: "value".into(),
                },
                ColumnMapping {
                    source_index: 1,
                    target_column: "value".into(),
                },
            ]
        ),
        Err(TransferError::InvalidRequest { .. })
    ));
    review.relation.columns[0].public.identity = true;
    assert!(matches!(
        validate_mapping(
            &review,
            &[ColumnMapping {
                source_index: 0,
                target_column: "value".into(),
            }]
        ),
        Err(TransferError::UnsupportedTarget { .. })
    ));
}

#[test]
fn export_sql_guards_fields_and_records_before_copy_projection() {
    let review = test_review("connection");
    let statement = sql::export(&review);
    assert!(statement.starts_with("COPY (SELECT \"__dbunk_output\".\"value\" FROM"));
    assert!(statement.contains("pg_catalog.format('%s'"));
    assert!(statement.contains("IS NOT DISTINCT FROM NULL"));
    assert!(statement.contains("OFFSET 0"));
    assert!(statement.contains("pg_catalog.convert_to"));
    assert!(statement.contains(&format!("<= {}", csv::MAX_FIELD_BYTES)));
    assert!(statement.contains(&format!("> {}", csv::MAX_RECORD_BYTES)));
    assert!(statement.contains("pg_catalog.pg_backend_pid()"));
    assert!(statement.contains("ENCODING E'UTF8'"));
}

#[tokio::test]
async fn inspection_reports_actual_width_error_after_retained_sample_is_full() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.csv");
    let mut csv = String::from("a,b\n");
    for _ in 0..60 {
        csv.push_str("one,two\n");
    }
    csv.push_str("wrong,width,count\n");
    tokio::fs::write(&path, csv).await.unwrap();
    let source = files::open_source(&path).await.unwrap();
    assert!(matches!(
        inspect_source(source, &CsvOptions::default()).await,
        Err(TransferError::Csv {
            record: 62,
            column: None,
            ..
        })
    ));
}

#[tokio::test]
async fn inspection_bounds_retained_header_labels() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.csv");
    tokio::fs::write(&path, vec![b'a'; SAMPLE_VALUE_BYTES + 1])
        .await
        .unwrap();
    let source = files::open_source(&path).await.unwrap();
    assert!(matches!(
        inspect_source(source, &CsvOptions::default()).await,
        Err(TransferError::Csv { record: 1, .. })
    ));
}

#[tokio::test]
async fn inspection_refuses_a_first_record_that_exceeds_the_inspection_budget() {
    for header in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.csv");
        tokio::fs::write(&path, vec![b'a'; 300 * 1024])
            .await
            .unwrap();
        let source = files::open_source(&path).await.unwrap();
        let options = CsvOptions {
            header,
            ..CsvOptions::default()
        };
        assert!(matches!(
            inspect_source(source, &options).await,
            Err(TransferError::Csv {
                record: 1,
                column: None,
                ref reason,
            }) if reason.contains("256 KiB inspection limit")
        ));
    }
}

#[tokio::test]
async fn source_fingerprint_changes_after_an_in_place_write() {
    use tokio::io::AsyncWriteExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.csv");
    tokio::fs::write(&path, b"a\nfirst\n").await.unwrap();
    let source = files::open_source(&path).await.unwrap();
    let reviewed = source.fingerprint.clone();
    let mut writer = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .await
        .unwrap();
    writer.write_all(b"second\n").await.unwrap();
    writer.sync_all().await.unwrap();
    assert_ne!(source.current_fingerprint().await.unwrap(), reviewed);
}

#[tokio::test]
async fn partial_publication_never_replaces_a_destination() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("export.csv");
    let mut partial = PartialFile::create(destination.clone(), "job")
        .await
        .unwrap();
    partial.write_all(b"new").await.unwrap();
    partial.finish_writing().await.unwrap();
    tokio::fs::write(&destination, b"existing").await.unwrap();
    assert_eq!(
        partial.publish().await,
        Err(TransferError::DestinationExists)
    );
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), b"existing");
    assert!(!std::fs::read_dir(directory.path())
        .unwrap()
        .any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("dbunk-partial")));
}

#[cfg(unix)]
#[tokio::test]
async fn source_symlinks_are_refused() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.csv");
    let link = directory.path().join("source.csv");
    tokio::fs::write(&target, b"a\n").await.unwrap();
    symlink(target, &link).unwrap();
    assert!(matches!(
        files::open_source(&link).await,
        Err(TransferError::InvalidRequest { .. })
    ));
}
