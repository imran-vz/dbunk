use super::super::verify::{type_requests, Expectation};
use super::setup;
use crate::schema_edit::fixtures::existing;
use crate::schema_edit::*;

/// The draft as the server would report it: every draft-issued origin
/// replaced by a catalog-style one, so matching must go by name.
fn as_captured(table: &TableModel) -> TableModel {
    let map = |origin: Origin| match origin {
        Origin::New(DraftId(id)) => Origin::Existing(BaselineId(90_000 + id)),
        other => other,
    };
    let mut table = table.clone();
    for column in &mut table.columns {
        column.origin = map(column.origin);
    }
    for key in table.primary_key.iter_mut().chain(table.uniques.iter_mut()) {
        key.origin = map(key.origin);
        key.columns = key.columns.iter().copied().map(map).collect();
    }
    for key in &mut table.foreign_keys {
        key.origin = map(key.origin);
        key.columns = key.columns.iter().copied().map(map).collect();
    }
    for check in &mut table.checks {
        check.origin = map(check.origin);
    }
    for index in &mut table.indexes {
        index.origin = map(index.origin);
        index.include = index.include.iter().copied().map(map).collect();
        for member in &mut index.members {
            if let IndexTarget::Column(origin) = &mut member.target {
                *origin = map(*origin);
            }
        }
    }
    table
}

fn compare(
    baseline: &SchemaBaseline,
    draft: &SchemaDraft,
    actual: &TableModel,
    deferred: &[Origin],
) -> Result<(), String> {
    let canonical: Vec<(Origin, String)> = type_requests(baseline, draft)
        .into_iter()
        .map(|(origin, text)| {
            let canonical = match text.as_str() {
                "int" => "integer".to_owned(),
                "varchar(20)" => "character varying(20)".to_owned(),
                _ => text,
            };
            (origin, canonical)
        })
        .collect();
    Expectation {
        baseline,
        draft,
        deferred_indexes: deferred,
    }
    .compare(actual, &canonical)
}

#[test]
fn server_canonical_types_are_not_failures() {
    let (baseline, mut draft) = setup();
    let origin = draft.add_column("age", "int");
    draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(20)".into();
    assert_eq!(
        type_requests(&baseline, &draft),
        vec![
            (existing(3), "varchar(20)".to_owned()),
            (origin, "int".to_owned())
        ]
    );
    let mut actual = as_captured(&draft.table);
    actual.columns[2].data_type = "character varying(20)".into();
    actual.columns[4].data_type = "integer".into();
    assert_eq!(compare(&baseline, &draft, &actual, &[]), Ok(()));

    actual.columns[4].data_type = "bigint".into();
    assert!(compare(&baseline, &draft, &actual, &[])
        .unwrap_err()
        .contains("column age has type bigint, expected integer"));
}

#[test]
fn server_named_constraints_match_unnamed_draft_constraints() {
    let (baseline, mut draft) = setup();
    let origin = draft.new_origin();
    draft.table.checks.push(CheckModel {
        origin,
        name: None,
        expression: SqlFragment::new("id > 0"),
        enforced: true,
    });
    let mut actual = as_captured(&draft.table);
    let check = actual.checks.last_mut().unwrap();
    check.name = Some("users_id_check".into());
    check.expression = SqlFragment::new("(id > 0)");
    assert_eq!(compare(&baseline, &draft, &actual, &[]), Ok(()));

    actual.checks.pop();
    assert_eq!(
        compare(&baseline, &draft, &actual, &[]),
        Err("the checks differ".into())
    );
}

#[test]
fn untouched_expressions_compare_byte_for_byte_unless_columns_are_renamed() {
    let (baseline, mut draft) = setup();
    draft.add_column("nick", "text");
    let mut actual = as_captured(&draft.table);
    actual.columns[4].data_type = "text".into();
    actual.checks[0].expression = SqlFragment::new("length((email)::text) > 3");
    assert!(compare(&baseline, &draft, &actual, &[])
        .unwrap_err()
        .contains("users_email_check"));

    // A rename re-renders every expression that uses the column.
    let (baseline, mut draft) = setup();
    draft.table.column_mut(existing(2)).unwrap().name = "mail".into();
    let mut actual = as_captured(&draft.table);
    actual.checks[0].expression = SqlFragment::new("length(mail::text) > 3");
    assert_eq!(compare(&baseline, &draft, &actual, &[]), Ok(()));
}

#[test]
fn adding_a_primary_key_makes_its_columns_not_null() {
    let (baseline, mut draft) = setup();
    draft.table.primary_key.as_mut().unwrap().columns = vec![existing(1), existing(3)];
    let mut actual = as_captured(&draft.table);
    actual.columns[2].nullable = false;
    assert_eq!(compare(&baseline, &draft, &actual, &[]), Ok(()));
}

#[test]
fn concurrent_indexes_are_checked_after_the_transaction() {
    let (baseline, mut draft) = setup();
    let origin = draft.new_origin();
    draft.table.indexes.push(IndexModel {
        origin,
        name: "users_email_idx".into(),
        unique: false,
        method: None,
        members: vec![IndexMember::column(existing(2))],
        include: Vec::new(),
        predicate: None,
    });
    let mut actual = as_captured(&draft.table);
    actual.indexes.pop();
    assert_eq!(compare(&baseline, &draft, &actual, &[origin]), Ok(()));
    assert!(compare(&baseline, &draft, &actual, &[])
        .unwrap_err()
        .contains("indexes"));
}

#[test]
fn structural_differences_fail() {
    let (baseline, mut draft) = setup();
    draft.add_column("nick", "text");
    let good = {
        let mut actual = as_captured(&draft.table);
        actual.columns[4].data_type = "text".into();
        actual
    };
    assert_eq!(compare(&baseline, &draft, &good, &[]), Ok(()));

    let mut missing = good.clone();
    missing.columns.pop();
    assert!(compare(&baseline, &draft, &missing, &[]).is_err());

    let mut swapped = good.clone();
    swapped.columns.swap(0, 1);
    assert!(compare(&baseline, &draft, &swapped, &[]).is_err());

    let mut nullable = good.clone();
    nullable.columns[1].nullable = true;
    assert!(compare(&baseline, &draft, &nullable, &[]).is_err());

    let mut renamed_index = good.clone();
    renamed_index.indexes[0].name = "other".into();
    assert!(compare(&baseline, &draft, &renamed_index, &[]).is_err());

    let mut foreign_key = good.clone();
    foreign_key.foreign_keys[0].on_delete = ReferentialAction::Restrict;
    assert!(compare(&baseline, &draft, &foreign_key, &[]).is_err());

    let mut renamed_table = good.clone();
    renamed_table.name = "members".into();
    assert!(compare(&baseline, &draft, &renamed_table, &[]).is_err());

    // An empty comment is no comment, as PostgreSQL stores it.
    let mut empty = draft.clone();
    empty.table.comment = Some(String::new());
    assert_eq!(compare(&baseline, &empty, &good, &[]), Ok(()));
}
