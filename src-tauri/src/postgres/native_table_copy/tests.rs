use super::*;
use crate::postgres::transfer::{protocol::TargetColumn, runner::catalog::CatalogColumn};
fn column(name: &str) -> CatalogColumn {
    CatalogColumn {
        number: 1,
        type_oid: 25,
        type_modifier: -1,
        collation_oid: 0,
        default_fingerprint: None,
        public: TargetColumn {
            name: name.into(),
            data_type: "text".into(),
            nullable: false,
            has_default: false,
            generated: false,
            identity: false,
        },
    }
}
fn relation(names: &[&str]) -> RelationState {
    RelationState {
        oid: 1,
        kind: "r".into(),
        row_security: false,
        force_row_security: false,
        populated: true,
        columns: names.iter().map(|n| column(n)).collect(),
    }
}
#[test]
fn mapping_is_by_exact_name_and_discloses_identity_generated_and_defaults() {
    let source = relation(&["日本語", "id", "derived", "unused"]);
    let mut destination = relation(&["id", "missing", "derived", "日本語"]);
    destination.columns[0].public.identity = true;
    destination.columns[1].public.has_default = true;
    destination.columns[2].public.generated = true;
    let columns = mapping(&source, &destination).unwrap();
    assert_eq!(
        columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["id", "missing", "derived", "日本語"]
    );
    assert_eq!(
        columns.iter().map(|c| c.action).collect::<Vec<_>>(),
        [
            TableCopyColumnAction::CopyIdentity,
            TableCopyColumnAction::DefaultOrNull,
            TableCopyColumnAction::Generated,
            TableCopyColumnAction::Copy
        ]
    );
    destination.columns[1].public.has_default = false;
    assert_eq!(
        mapping(&source, &destination).err().unwrap().error,
        TableCopyError::MissingRequiredColumn
    );
}
#[test]
fn shared_copy_projection_preserves_null_token_and_quotes_and_server_bounds() {
    let sql = sql::export_columns(
        "quoted\"schema",
        "table",
        &["NULL", "n"],
        &CsvOptions {
            header: false,
            ..Default::default()
        },
    );
    assert!(sql.contains("NULL::text"));
    assert!(sql.contains("NULL E'\\\\N'"));
    assert!(sql.contains("HEADER false"));
    assert!(sql.contains("\"quoted\"\"schema\".\"table\""));
    assert!(sql.contains("1048576"));
    assert!(sql.contains("8388608"));
    let input = sql::import_columns("s", "t", &["n", "NULL"], &CsvOptions::default());
    assert!(input.starts_with("COPY \"s\".\"t\" (\"n\", \"NULL\") FROM STDIN"));
}
#[tokio::test]
async fn cancellation_before_commit_never_polls_commit_but_terminal_ack_wins_late_cancel() {
    let control = Control::default();
    control.cancel();
    let result = commit_result(
        async { panic!("cancelled commit was dispatched") },
        &control,
        7,
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.outcome, TableCopyOutcome::RolledBack);
    let control = Control::default();
    let result = commit_result(
        async {
            control.cancel();
            Ok(())
        },
        &control,
        7,
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.outcome, TableCopyOutcome::Completed { rows: 7 });
}
#[tokio::test]
async fn lost_commit_ack_is_unknown_and_never_retried() {
    let control = Control::default();
    let count = std::sync::atomic::AtomicUsize::new(0);
    let result = commit_result(
        async {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::future::pending().await
        },
        &control,
        5,
        Instant::now() + Duration::from_millis(5),
    )
    .await;
    assert_eq!(result.outcome, TableCopyOutcome::OutcomeUnknown);
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}
pub(crate) fn plan(intent: TableCopyIntent, targets: [TableCopyConnection; 2]) -> Plan {
    let source = relation(&["v"]);
    let destination = relation(&["v"]);
    let columns = mapping(&source, &destination).unwrap();
    Plan {
        description: TableCopyDescription {
            intent,
            source_connection: targets[0].clone(),
            destination_connection: targets[1].clone(),
            source_relation: TableCopyRelation {
                database_oid: 1,
                relation_oid: 1,
                kind: "r".into(),
            },
            destination_relation: TableCopyRelation {
                database_oid: 1,
                relation_oid: 2,
                kind: "r".into(),
            },
            mapping_sha256: "a".repeat(64),
            copied_columns: 1,
            defaulted_columns: 0,
            generated_columns: 0,
            identity_columns: 0,
        },
        columns,
        source,
        destination,
        names: vec!["v".into()],
    }
}
