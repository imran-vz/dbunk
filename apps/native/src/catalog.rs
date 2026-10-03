//! Retained catalog rows keep full object identity separate from clipped labels.
use crate::results::encoded_size;
use dbunk_lib::backend::objects::{
    PgCatalogEntry, PgCatalogTruncation, PgObjectCatalog, PgObjectKind, PgObjectRef,
};
use serde::Serialize;
use std::{cell::Cell, rc::Rc};

const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Copy, Serialize)]
pub enum Kind {
    Object(PgObjectKind),
    EventTrigger,
    Role,
    Tablespace,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Object(kind) => match kind {
                PgObjectKind::Schema => "Schema",
                PgObjectKind::Table => "Table",
                PgObjectKind::View => "View",
                PgObjectKind::MaterializedView => "Materialized view",
                PgObjectKind::ForeignTable => "Foreign table",
                PgObjectKind::Sequence => "Sequence",
                PgObjectKind::Function => "Function",
                PgObjectKind::Procedure => "Procedure",
                PgObjectKind::Aggregate => "Aggregate",
                PgObjectKind::Type => "Type",
                PgObjectKind::Domain => "Domain",
                PgObjectKind::Extension => "Extension",
            },
            Self::EventTrigger => "Event trigger",
            Self::Role => "Role",
            Self::Tablespace => "Tablespace",
        }
    }
    pub fn relation(self) -> bool {
        matches!(
            self,
            Self::Object(
                PgObjectKind::Table
                    | PgObjectKind::View
                    | PgObjectKind::MaterializedView
                    | PgObjectKind::ForeignTable
            )
        )
    }
}
#[derive(Serialize)]
pub struct Row {
    pub kind: Kind,
    pub schema: Option<String>,
    pub entry: PgCatalogEntry,
}
impl Row {
    pub fn reference(&self) -> Option<PgObjectRef> {
        let Kind::Object(kind) = self.kind else {
            return None;
        };
        Some(PgObjectRef {
            kind,
            schema: self.schema.clone(),
            name: self.entry.name.clone(),
            identity_args: self.entry.identity_args.clone(),
        })
    }
    pub fn qualified(&self) -> String {
        let quote = |text: &str| format!("\"{}\"", text.replace('"', "\"\""));
        let name = match &self.schema {
            Some(schema) => format!("{}.{}", quote(schema), quote(&self.entry.name)),
            None => quote(&self.entry.name),
        };
        match &self.entry.identity_args {
            Some(args) => format!("{name}({args})"),
            None => name,
        }
    }
    pub fn label(&self) -> String {
        format!("{}  {}", self.kind.label(), clipped(&self.qualified(), 512))
    }
}
fn clipped(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}
pub struct Catalog {
    pub rows: Vec<Row>,
    pub truncated: Vec<PgCatalogTruncation>,
    budget: Rc<Cell<usize>>,
    bytes: usize,
}
impl Catalog {
    pub fn new(catalog: PgObjectCatalog, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        let mut rows = Vec::new();
        for schema in catalog.schemas {
            rows.push(Row {
                kind: Kind::Object(PgObjectKind::Schema),
                schema: None,
                entry: PgCatalogEntry {
                    name: schema.name.clone(),
                    identity_args: None,
                    comment: None,
                    type_class: None,
                },
            });
            for (kind, entries) in [
                (PgObjectKind::Table, schema.tables),
                (PgObjectKind::View, schema.views),
                (PgObjectKind::MaterializedView, schema.materialized_views),
                (PgObjectKind::ForeignTable, schema.foreign_tables),
                (PgObjectKind::Sequence, schema.sequences),
                (PgObjectKind::Function, schema.functions),
                (PgObjectKind::Procedure, schema.procedures),
                (PgObjectKind::Aggregate, schema.aggregates),
                (PgObjectKind::Type, schema.types),
                (PgObjectKind::Domain, schema.domains),
                (PgObjectKind::Extension, schema.extensions),
            ] {
                rows.extend(entries.into_iter().map(|entry| Row {
                    kind: Kind::Object(kind),
                    schema: Some(schema.name.clone()),
                    entry,
                }));
            }
        }
        for (kind, entries) in [
            (Kind::EventTrigger, catalog.event_triggers),
            (Kind::Role, catalog.roles),
            (Kind::Tablespace, catalog.tablespaces),
        ] {
            rows.extend(entries.into_iter().map(|entry| Row {
                kind,
                schema: None,
                entry,
            }));
        }
        // Reserve the encoded rows plus one index per possible visible row.
        let bytes = encoded_size(&(&rows, &catalog.truncated))
            .saturating_add(rows.len() * std::mem::size_of::<usize>());
        if bytes > WORKSPACE_BYTES.saturating_sub(budget.get()) {
            return Err("Workspace memory budget is full; clear results or close another tab");
        }
        budget.set(budget.get() + bytes);
        Ok(Self {
            rows,
            truncated: catalog.truncated,
            budget,
            bytes,
        })
    }
    pub fn matching(&self, search: &str) -> Vec<usize> {
        let needle = search.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                (needle.is_empty()
                    || row.entry.name.to_lowercase().contains(&needle)
                    || row
                        .schema
                        .as_ref()
                        .is_some_and(|schema| schema.to_lowercase().contains(&needle))
                    || row
                        .entry
                        .identity_args
                        .as_ref()
                        .is_some_and(|args| args.to_lowercase().contains(&needle))
                    || row.kind.label().to_lowercase().contains(&needle))
                .then_some(i)
            })
            .collect()
    }
}
impl Drop for Catalog {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overload_and_quoted_identity_survive_display_clipping() {
        let row = Row {
            kind: Kind::Object(PgObjectKind::Function),
            schema: Some("odd\"schema".into()),
            entry: PgCatalogEntry {
                name: "name.with.dot".into(),
                identity_args: Some("integer, ".repeat(100)),
                comment: None,
                type_class: None,
            },
        };
        assert!(
            row.qualified()
                .starts_with("\"odd\"\"schema\".\"name.with.dot\"(")
        );
        assert!(row.label().ends_with('…'));
        assert_eq!(
            row.reference().unwrap().identity_args,
            row.entry.identity_args
        );
        assert!(!row.kind.relation());
    }
    #[test]
    fn retention_refusal_and_release_preserve_other_owners() {
        let budget = Rc::new(Cell::new(WORKSPACE_BYTES));
        let catalog = || PgObjectCatalog {
            schemas: vec![],
            event_triggers: vec![],
            roles: vec![PgCatalogEntry {
                name: "Reader".into(),
                identity_args: None,
                comment: None,
                type_class: None,
            }],
            tablespaces: vec![],
            truncated: vec![PgCatalogTruncation {
                schema: None,
                kind: "roles".into(),
            }],
        };
        assert!(Catalog::new(catalog(), budget.clone()).is_err());
        assert_eq!(budget.get(), WORKSPACE_BYTES);
        budget.set(37);
        let retained = Catalog::new(catalog(), budget.clone()).unwrap();
        assert_eq!(retained.matching("READ"), vec![0]);
        assert_eq!(retained.truncated.len(), 1);
        assert!(budget.get() > 37);
        drop(retained);
        assert_eq!(budget.get(), 37);
    }
}
