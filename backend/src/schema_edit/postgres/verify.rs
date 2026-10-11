//! In-transaction verification: the re-captured table against the draft,
//! compared by name (origins differ between a draft and a capture).
//!
//! No false failures: user-entered types are compared after the server
//! canonicalized them (`int` → `integer`), and user-entered expressions only
//! for presence, since the server re-renders them. Untouched expressions are
//! compared byte for byte, unless a column rename or type change in the plan
//! makes the server re-render them too.
use crate::schema_edit::{
    plan_steps, ColumnField, DefaultValue, IndexTarget, Origin, PlanStep, SchemaBaseline,
    SchemaDraft, TableModel,
};

pub(crate) struct Expectation<'a> {
    pub(crate) baseline: &'a SchemaBaseline,
    pub(crate) draft: &'a SchemaDraft,
    /// Indexes built `CONCURRENTLY` after the atomic group commits.
    pub(crate) deferred_indexes: &'a [Origin],
}

/// Draft types the server must canonicalize, by column: every new column
/// and every column whose type text differs from the captured one.
pub(crate) fn type_requests(baseline: &SchemaBaseline, draft: &SchemaDraft) -> Vec<(Origin, String)> {
    draft
        .table
        .columns
        .iter()
        .filter(|column| {
            baseline
                .table
                .column(column.origin)
                .filter(|_| !column.origin.is_new())
                .is_none_or(|previous| previous.data_type != column.data_type)
        })
        .map(|column| (column.origin, column.data_type.clone()))
        .collect()
}

fn comment(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|text| !text.is_empty())
}

/// `COLLATE "default"` is the type's default collation, which a capture
/// does not list.
fn collation(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|name| *name != "default")
}

fn names(table: &TableModel, origins: &[Origin]) -> Vec<String> {
    table.column_names(origins)
}

impl Expectation<'_> {
    /// True when the plan renames or retypes a column, which re-renders
    /// expressions that use it.
    fn rerenders(&self) -> bool {
        plan_steps(self.baseline, self.draft).iter().any(|step| match step {
            PlanStep::RenameColumn { .. } => true,
            PlanStep::AlterColumn { fields, .. } => fields
                .iter()
                .any(|field| matches!(field, ColumnField::DataType | ColumnField::Collation)),
            _ => false,
        })
    }

    /// `canonical` pairs each `type_requests` entry with the server's
    /// `format_type` rendering, in the same order.
    pub(crate) fn compare(
        &self,
        actual: &TableModel,
        canonical: &[(Origin, String)],
    ) -> Result<(), String> {
        let old = &self.baseline.table;
        let new = &self.draft.table;
        let strict = !self.rerenders();

        if actual.name != new.name {
            return Err(format!("the table is named {}, expected {}", actual.name, new.name));
        }
        if comment(&actual.comment) != comment(&new.comment) {
            return Err("the table comment differs".into());
        }

        // Columns, in order.
        if actual.columns.len() != new.columns.len() {
            return Err(format!(
                "the table has {} columns, expected {}",
                actual.columns.len(),
                new.columns.len()
            ));
        }
        let in_key = |origin: Origin| {
            new.primary_key
                .as_ref()
                .is_some_and(|key| key.columns.contains(&origin))
        };
        for (expected, found) in new.columns.iter().zip(&actual.columns) {
            let name = &expected.name;
            if found.name != *name {
                return Err(format!("column {} is where {name} was expected", found.name));
            }
            let previous = old.column(expected.origin).filter(|_| !expected.origin.is_new());
            let data_type = canonical
                .iter()
                .find(|(origin, _)| *origin == expected.origin)
                .map_or(expected.data_type.as_str(), |(_, text)| text.as_str());
            if found.data_type != data_type {
                return Err(format!(
                    "column {name} has type {}, expected {data_type}",
                    found.data_type
                ));
            }
            // Adding a primary key makes its columns NOT NULL.
            let nullable = expected.nullable && !in_key(expected.origin);
            if found.nullable != nullable {
                return Err(format!("column {name} nullability differs"));
            }
            if found.identity != expected.identity {
                return Err(format!("column {name} identity differs"));
            }
            match (&expected.generated, &found.generated) {
                (None, None) => {}
                (Some(e), Some(f)) if e.stored == f.stored => {
                    let untouched = previous.is_some_and(|p| p.generated == expected.generated);
                    if strict && untouched && e.expression != f.expression {
                        return Err(format!("column {name} generation expression changed"));
                    }
                }
                _ => return Err(format!("column {name} generation differs")),
            }
            match (&expected.default, &found.default) {
                (DefaultValue::None, DefaultValue::None) => {}
                (DefaultValue::None, _) | (_, DefaultValue::None) => {
                    return Err(format!("column {name} default differs"))
                }
                (DefaultValue::Expression(e), f) => {
                    let untouched = previous.is_some_and(|p| p.default == expected.default);
                    if strict && untouched && *f != DefaultValue::Expression(e.clone()) {
                        return Err(format!("column {name} default changed"));
                    }
                }
                (DefaultValue::Literal(_), _) => {}
            }
            if collation(&found.collation) != collation(&expected.collation) {
                return Err(format!("column {name} collation differs"));
            }
            if comment(&found.comment) != comment(&expected.comment) {
                return Err(format!("column {name} comment differs"));
            }
        }

        // Primary key.
        match (&new.primary_key, &actual.primary_key) {
            (None, None) => {}
            (Some(e), Some(f)) => {
                if names(new, &e.columns) != names(actual, &f.columns)
                    || e.name.as_ref().is_some_and(|name| f.name.as_ref() != Some(name))
                {
                    return Err("the primary key differs".into());
                }
            }
            _ => return Err("the primary key differs".into()),
        }

        // Unique keys.
        if new.uniques.len() != actual.uniques.len() {
            return Err("the unique keys differ".into());
        }
        let mut used = vec![false; actual.uniques.len()];
        for expected in &new.uniques {
            let columns = names(new, &expected.columns);
            let found = actual.uniques.iter().enumerate().position(|(i, f)| {
                !used[i]
                    && expected.name.as_ref().is_none_or(|name| f.name.as_ref() == Some(name))
                    && names(actual, &f.columns) == columns
            });
            match found {
                Some(i) => used[i] = true,
                None => {
                    return Err(format!(
                        "unique key {} is missing or differs",
                        expected.name.as_deref().unwrap_or("(unnamed)")
                    ))
                }
            }
        }

        // Foreign keys.
        if new.foreign_keys.len() != actual.foreign_keys.len() {
            return Err("the foreign keys differ".into());
        }
        let schema_of = |schema: &Option<String>, table: &TableModel| {
            schema.clone().unwrap_or_else(|| table.schema.clone())
        };
        let mut used = vec![false; actual.foreign_keys.len()];
        for expected in &new.foreign_keys {
            let columns = names(new, &expected.columns);
            let found = actual.foreign_keys.iter().enumerate().position(|(i, f)| {
                !used[i]
                    && expected.name.as_ref().is_none_or(|name| f.name.as_ref() == Some(name))
                    && names(actual, &f.columns) == columns
                    && schema_of(&f.referenced_schema, actual)
                        == schema_of(&expected.referenced_schema, new)
                    && f.referenced_table == expected.referenced_table
                    && f.referenced_columns == expected.referenced_columns
                    && f.on_update == expected.on_update
                    && f.on_delete == expected.on_delete
                    && f.deferrable == expected.deferrable
            });
            match found {
                Some(i) => used[i] = true,
                None => {
                    return Err(format!(
                        "foreign key {} is missing or differs",
                        expected.name.as_deref().unwrap_or("(unnamed)")
                    ))
                }
            }
        }

        // Checks.
        if new.checks.len() != actual.checks.len() {
            return Err("the checks differ".into());
        }
        let mut used = vec![false; actual.checks.len()];
        for expected in &new.checks {
            let untouched = strict && old.check(expected.origin) == Some(expected);
            let found = actual.checks.iter().enumerate().position(|(i, f)| {
                !used[i]
                    && expected.name.as_ref().is_none_or(|name| f.name.as_ref() == Some(name))
                    && (!untouched || f.expression == expected.expression)
            });
            match found {
                Some(i) => used[i] = true,
                None => {
                    return Err(format!(
                        "check {} is missing or differs",
                        expected.name.as_deref().unwrap_or(expected.expression.as_str())
                    ))
                }
            }
        }

        // Indexes (concurrent builds come after this transaction).
        let expected_indexes: Vec<_> = new
            .indexes
            .iter()
            .filter(|index| !self.deferred_indexes.contains(&index.origin))
            .collect();
        if expected_indexes.len() != actual.indexes.len() {
            return Err(format!(
                "the table has {} indexes, expected {}",
                actual.indexes.len(),
                expected_indexes.len()
            ));
        }
        let method = |value: &Option<String>| {
            value
                .as_deref()
                .filter(|method| !method.eq_ignore_ascii_case("btree"))
                .map(str::to_ascii_lowercase)
        };
        for expected in expected_indexes {
            let name = &expected.name;
            let Some(found) = actual.indexes.iter().find(|f| f.name == *name) else {
                return Err(format!("index {name} is missing"));
            };
            let untouched = strict && old.index(expected.origin) == Some(expected);
            if found.unique != expected.unique
                || method(&found.method) != method(&expected.method)
                || found.members.len() != expected.members.len()
                || names(actual, &found.include) != names(new, &expected.include)
                || found.predicate.is_some() != expected.predicate.is_some()
                || (untouched && found.predicate != expected.predicate)
            {
                return Err(format!("index {name} differs"));
            }
            for (e, f) in expected.members.iter().zip(&found.members) {
                let same = e.descending == f.descending
                    && match (&e.target, &f.target) {
                        (IndexTarget::Column(a), IndexTarget::Column(b)) => {
                            new.column_name(*a) == actual.column_name(*b)
                        }
                        (IndexTarget::Expression(a), IndexTarget::Expression(b)) => {
                            !untouched || a == b
                        }
                        _ => false,
                    };
                if !same {
                    return Err(format!("index {name} members differ"));
                }
            }
        }
        Ok(())
    }
}
