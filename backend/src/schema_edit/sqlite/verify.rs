//! Compares the re-captured table with the expected one inside the apply
//! transaction. Origins are ignored (a rebuild renumbers them); names,
//! order, normalized types, nullability, defaults, expressions, keys,
//! foreign keys, checks and indexes must all match.
use super::lexer::{normalized, same_expression};
use super::planner::render_default;
use super::quote;
use crate::schema_edit::{types_equal, ForeignKeyModel, IndexTarget, TableModel};

fn expression_key(text: &str) -> Vec<String> {
    normalized(text).unwrap_or_else(|| vec![text.to_owned()])
}

fn names(table: &TableModel, origins: &[crate::schema_edit::Origin]) -> Vec<String> {
    table
        .column_names(origins)
        .into_iter()
        .map(|name| name.to_ascii_lowercase())
        .collect()
}

fn foreign_key(table: &TableModel, key: &ForeignKeyModel) -> String {
    format!(
        "{:?}|{:?}|{}|{:?}|{}|{}|{:?}",
        key.name,
        names(table, &key.columns),
        key.referenced_table.to_ascii_lowercase(),
        key.referenced_columns
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect::<Vec<_>>(),
        key.on_update.sql(),
        key.on_delete.sql(),
        key.deferrable
    )
}

fn indexes(table: &TableModel) -> Vec<String> {
    let mut out: Vec<String> = table
        .indexes
        .iter()
        .map(|index| {
            let members: Vec<(Vec<String>, Option<String>, bool)> = index
                .members
                .iter()
                .map(|member| {
                    let target = match &member.target {
                        IndexTarget::Column(origin) => {
                            expression_key(&quote(table.column_name(*origin).unwrap_or("?")))
                        }
                        IndexTarget::Expression(sql) => expression_key(sql.as_str()),
                    };
                    (
                        target,
                        member.collation.as_ref().map(|c| c.to_ascii_lowercase()),
                        member.descending,
                    )
                })
                .collect();
            format!(
                "{}|{}|{:?}|{:?}",
                index.name.to_ascii_lowercase(),
                index.unique,
                members,
                index.predicate.as_ref().map(|p| expression_key(p.as_str()))
            )
        })
        .collect();
    out.sort();
    out
}

/// `Ok` when `actual` (re-captured) has the structure `expected` describes.
/// The error names the first difference.
pub(super) fn compare_tables(expected: &TableModel, actual: &TableModel) -> Result<(), String> {
    if expected.name != actual.name {
        return Err(format!(
            "the table is named {} instead of {}",
            actual.name, expected.name
        ));
    }
    if expected.options.strict != actual.options.strict
        || expected.options.without_rowid != actual.options.without_rowid
    {
        return Err("STRICT or WITHOUT ROWID differs".into());
    }
    if expected.columns.len() != actual.columns.len() {
        return Err(format!(
            "the table has {} columns instead of {}",
            actual.columns.len(),
            expected.columns.len()
        ));
    }
    for (want, have) in expected.columns.iter().zip(&actual.columns) {
        let label = &want.name;
        if want.name != have.name {
            return Err(format!("column {} is {} instead", want.name, have.name));
        }
        if !types_equal(&want.data_type, &have.data_type) {
            return Err(format!("the type of {label} differs"));
        }
        if want.nullable != have.nullable {
            return Err(format!("NOT NULL on {label} differs"));
        }
        if want.identity != have.identity {
            return Err(format!("AUTOINCREMENT on {label} differs"));
        }
        let defaults_match = match (render_default(&want.default), render_default(&have.default)) {
            (None, None) => true,
            (Some(a), Some(b)) => same_expression(&a, &b),
            _ => false,
        };
        if !defaults_match {
            return Err(format!("the default of {label} differs"));
        }
        let generated_match = match (&want.generated, &have.generated) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.stored == b.stored && same_expression(a.expression.as_str(), b.expression.as_str())
            }
            _ => false,
        };
        if !generated_match {
            return Err(format!("the generated expression of {label} differs"));
        }
        let collation = |c: &Option<String>| c.as_ref().map(|c| c.to_ascii_lowercase());
        if collation(&want.collation) != collation(&have.collation) {
            return Err(format!("the collation of {label} differs"));
        }
    }
    let key = |table: &TableModel, key: &crate::schema_edit::KeyModel| {
        (key.name.clone(), names(table, &key.columns))
    };
    let want_pk = expected.primary_key.as_ref().map(|k| key(expected, k));
    let have_pk = actual.primary_key.as_ref().map(|k| key(actual, k));
    if want_pk != have_pk {
        return Err("the primary key differs".into());
    }
    let mut want_uniques: Vec<_> = expected.uniques.iter().map(|k| key(expected, k)).collect();
    let mut have_uniques: Vec<_> = actual.uniques.iter().map(|k| key(actual, k)).collect();
    want_uniques.sort();
    have_uniques.sort();
    if want_uniques != have_uniques {
        return Err("the unique constraints differ".into());
    }
    let mut want_fks: Vec<_> = expected
        .foreign_keys
        .iter()
        .map(|k| foreign_key(expected, k))
        .collect();
    let mut have_fks: Vec<_> = actual
        .foreign_keys
        .iter()
        .map(|k| foreign_key(actual, k))
        .collect();
    want_fks.sort();
    have_fks.sort();
    if want_fks != have_fks {
        return Err("the foreign keys differ".into());
    }
    let checks = |table: &TableModel| {
        let mut out: Vec<(Option<String>, Vec<String>)> = table
            .checks
            .iter()
            .map(|check| (check.name.clone(), expression_key(check.expression.as_str())))
            .collect();
        out.sort();
        out
    };
    if checks(expected) != checks(actual) {
        return Err("the checks differ".into());
    }
    if indexes(expected) != indexes(actual) {
        return Err("the indexes differ".into());
    }
    Ok(())
}
