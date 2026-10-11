//! Rebase a draft onto a fresh baseline after `BaselineChanged`.
//!
//! Three-way, per field for columns and the table, per object for keys,
//! foreign keys, checks and indexes. A value the user did not touch follows
//! the server; a value only the user changed keeps the user's edit; a value
//! both changed differently keeps the user's edit and is reported as a
//! conflict so the review shows it. Objects are matched by origin, so this
//! is only meaningful when the engine's baseline ids are stable across
//! captures (ADR-0016).
use super::change::{DraftIssue, SchemaObject};
use super::model::{
    ColumnModel, IndexTarget, KeyModel, Origin, SchemaBaseline, SchemaDraft, SchemaEngine,
    TableModel,
};

/// One field or object both sides changed, or a user edit whose object the
/// server removed.
pub type RebaseConflict = DraftIssue;

fn merge<T: Clone + PartialEq>(base: &T, server: &T, user: &T, conflict: &mut bool) -> T {
    if user == base {
        server.clone()
    } else {
        if server != base && server != user {
            *conflict = true;
        }
        user.clone()
    }
}

fn merge_column(
    base: &ColumnModel,
    server: &ColumnModel,
    user: &ColumnModel,
) -> (ColumnModel, Vec<&'static str>) {
    let mut conflicts = Vec::new();
    let mut field = |name: &'static str, conflict: bool| {
        if conflict {
            conflicts.push(name);
        }
    };
    let mut merged = server.clone();
    macro_rules! three_way {
        ($($path:ident).+, $label:literal) => {{
            let mut conflict = false;
            merged.$($path).+ = merge(&base.$($path).+, &server.$($path).+, &user.$($path).+, &mut conflict);
            field($label, conflict);
        }};
    }
    three_way!(name, "name");
    three_way!(data_type, "type");
    three_way!(nullable, "nullable");
    three_way!(default, "default");
    three_way!(generated, "generated");
    three_way!(identity, "identity");
    three_way!(collation, "collation");
    three_way!(comment, "comment");
    three_way!(extra.charset, "character set");
    three_way!(extra.on_update, "on update");
    three_way!(extra.invisible, "invisible");
    (merged, conflicts)
}

/// Object-level three-way merge for one kind of keyed object list.
fn merge_objects<T: Clone + PartialEq>(
    object: SchemaObject,
    base: &[T],
    server: &[T],
    user: &[T],
    origin: impl Fn(&T) -> Origin,
    label: impl Fn(&T) -> String,
    conflicts: &mut Vec<RebaseConflict>,
) -> Vec<T> {
    let find =
        |list: &[T], wanted: Origin| list.iter().find(|item| origin(item) == wanted).cloned();
    let mut out = Vec::new();
    // Server order first, so untouched objects keep their place.
    for current in server {
        let id = origin(current);
        match (find(base, id), find(user, id)) {
            (Some(previous), Some(edited)) => {
                if edited == previous {
                    out.push(current.clone());
                } else {
                    if *current != previous && *current != edited {
                        conflicts.push(DraftIssue::at(
                            object,
                            id,
                            format!(
                                "{} changed on the server and in the draft; the draft wins",
                                label(&edited)
                            ),
                        ));
                    }
                    out.push(edited);
                }
            }
            // The user removed it; honour that unless the server also changed it.
            (Some(previous), None) => {
                if *current != previous {
                    conflicts.push(DraftIssue::at(
                        object,
                        id,
                        format!(
                            "{} was removed in the draft but changed on the server",
                            label(current)
                        ),
                    ));
                }
            }
            // New on the server.
            (None, _) => out.push(current.clone()),
        }
    }
    for edited in user {
        let id = origin(edited);
        if find(server, id).is_some() {
            continue;
        }
        match find(base, id) {
            // Draft-created objects carry over.
            None => out.push(edited.clone()),
            // Removed on the server.
            Some(previous) => {
                if *edited != previous {
                    conflicts.push(DraftIssue::at(
                        object,
                        id,
                        format!(
                            "{} was removed on the server; the draft's edit is dropped",
                            label(edited)
                        ),
                    ));
                }
            }
        }
    }
    out
}

fn key_label(key: &KeyModel) -> String {
    key.name.clone().unwrap_or_else(|| "Key".into())
}

/// SQLite and MySQL ids are positional (column position, constraint order),
/// so a server-side drop or insert shifts them onto other objects. For those
/// engines a merge is only sound when every id present in both baselines
/// still names the same object and no object appeared or vanished.
/// PostgreSQL ids (attnum, OIDs) never move.
fn ids_comparable(old: &SchemaBaseline, new: &SchemaBaseline) -> bool {
    if old.engine != new.engine || old.table.schema != new.table.schema {
        return false;
    }
    if old.engine == SchemaEngine::Postgres {
        return true;
    }
    fn names<T>(
        items: &[T],
        key: impl Fn(&T) -> (Origin, Option<String>),
    ) -> Vec<(Origin, Option<String>)> {
        let mut out: Vec<_> = items.iter().map(key).collect();
        out.sort();
        out
    }
    let (a, b) = (&old.table, &new.table);
    names(&a.columns, |c| (c.origin, Some(c.name.clone())))
        == names(&b.columns, |c| (c.origin, Some(c.name.clone())))
        && names(a.primary_key.as_slice(), |k| (k.origin, k.name.clone()))
            == names(b.primary_key.as_slice(), |k| (k.origin, k.name.clone()))
        && names(&a.uniques, |k| (k.origin, k.name.clone()))
            == names(&b.uniques, |k| (k.origin, k.name.clone()))
        && names(&a.foreign_keys, |k| (k.origin, k.name.clone()))
            == names(&b.foreign_keys, |k| (k.origin, k.name.clone()))
        && names(&a.checks, |c| (c.origin, c.name.clone()))
            == names(&b.checks, |c| (c.origin, c.name.clone()))
        && names(&a.indexes, |i| (i.origin, Some(i.name.clone())))
            == names(&b.indexes, |i| (i.origin, Some(i.name.clone())))
}

/// Re-bases `draft` (edited against `old`) onto `new`. The result keeps the
/// draft's issued ids (`next_id`), so new objects never collide.
pub fn rebase(
    old: &SchemaBaseline,
    new: &SchemaBaseline,
    draft: &SchemaDraft,
) -> (SchemaDraft, Vec<RebaseConflict>) {
    if !ids_comparable(old, new) {
        return (
            SchemaDraft {
                table: new.table.clone(),
                next_id: draft.next_id,
            },
            vec![DraftIssue::table(
                "The table's columns or constraints changed on the server in a way the draft \
                 cannot follow; the draft was restarted from the current structure",
            )],
        );
    }
    let base = &old.table;
    let server = &new.table;
    let user = &draft.table;
    let mut conflicts = Vec::new();

    // Columns: draft order for columns the draft keeps, then columns the
    // server added.
    let mut columns = Vec::new();
    for edited in &user.columns {
        let id = edited.origin;
        match (base.column(id), server.column(id)) {
            (_, _) if id.is_new() => columns.push(edited.clone()),
            (Some(previous), Some(current)) => {
                let (merged, fields) = merge_column(previous, current, edited);
                for field in fields {
                    conflicts.push(DraftIssue::at(SchemaObject::Column, id, format!("Column {}: {field} changed on the server and in the draft; the draft wins", merged.name)));
                }
                columns.push(merged);
            }
            (Some(previous), None) => {
                if edited != previous {
                    conflicts.push(DraftIssue::at(
                        SchemaObject::Column,
                        id,
                        format!(
                            "Column {} was removed on the server; the draft's edit is dropped",
                            edited.name
                        ),
                    ));
                }
            }
            (None, _) => {}
        }
    }
    for current in &server.columns {
        let in_base = base.column(current.origin).is_some();
        let in_draft = user.column(current.origin).is_some();
        if !in_base {
            columns.push(current.clone());
        } else if !in_draft {
            // Removed in the draft. Keep removing it, but say so if the
            // server changed it meanwhile.
            if base.column(current.origin) != Some(current) {
                conflicts.push(DraftIssue::at(
                    SchemaObject::Column,
                    current.origin,
                    format!(
                        "Column {} was removed in the draft but changed on the server",
                        current.name
                    ),
                ));
            }
        }
    }

    let mut flag = false;
    let name = merge(&base.name, &server.name, &user.name, &mut flag);
    if flag {
        conflicts.push(DraftIssue::table(
            "The table was renamed on the server and in the draft; the draft wins",
        ));
    }
    let mut flag = false;
    let comment = merge(&base.comment, &server.comment, &user.comment, &mut flag);
    if flag {
        conflicts.push(DraftIssue::table(
            "The table comment changed on the server and in the draft; the draft wins",
        ));
    }
    let mut flag = false;
    let options = merge(&base.options, &server.options, &user.options, &mut flag);
    if flag {
        conflicts.push(DraftIssue::table(
            "Table options changed on the server and in the draft; the draft wins",
        ));
    }

    let primary_key = merge_objects(
        SchemaObject::PrimaryKey,
        base.primary_key.as_slice(),
        server.primary_key.as_slice(),
        user.primary_key.as_slice(),
        |key| key.origin,
        key_label,
        &mut conflicts,
    );
    let mut primary_key = primary_key.into_iter();
    let primary = primary_key.next();
    if primary_key.next().is_some() {
        conflicts.push(DraftIssue::table(
            "The server and the draft define different primary keys; review the keys",
        ));
    }

    let table = TableModel {
        schema: server.schema.clone(),
        name,
        comment,
        columns,
        primary_key: primary,
        uniques: merge_objects(
            SchemaObject::Unique,
            &base.uniques,
            &server.uniques,
            &user.uniques,
            |k| k.origin,
            key_label,
            &mut conflicts,
        ),
        foreign_keys: merge_objects(
            SchemaObject::ForeignKey,
            &base.foreign_keys,
            &server.foreign_keys,
            &user.foreign_keys,
            |k| k.origin,
            |k| k.name.clone().unwrap_or_else(|| "Foreign key".into()),
            &mut conflicts,
        ),
        checks: merge_objects(
            SchemaObject::Check,
            &base.checks,
            &server.checks,
            &user.checks,
            |c| c.origin,
            |c| c.name.clone().unwrap_or_else(|| "Check".into()),
            &mut conflicts,
        ),
        indexes: merge_objects(
            SchemaObject::Index,
            &base.indexes,
            &server.indexes,
            &user.indexes,
            |i| i.origin,
            |i| i.name.clone(),
            &mut conflicts,
        ),
        options,
    };
    let mut rebased = SchemaDraft {
        table,
        next_id: draft.next_id,
    };
    // Members that point at columns the merge dropped are removed, as a
    // column drop would.
    let dangling: Vec<Origin> = rebased
        .table
        .primary_key
        .iter()
        .chain(rebased.table.uniques.iter())
        .flat_map(|key| key.columns.iter().copied())
        .chain(
            rebased
                .table
                .foreign_keys
                .iter()
                .flat_map(|key| key.columns.iter().copied()),
        )
        .chain(rebased.table.indexes.iter().flat_map(|index| {
            index
                .include
                .iter()
                .copied()
                .chain(
                    index
                        .members
                        .iter()
                        .filter_map(|member| match member.target {
                            IndexTarget::Column(origin) => Some(origin),
                            IndexTarget::Expression(_) => None,
                        }),
                )
        }))
        .filter(|origin| rebased.table.column(*origin).is_none())
        .collect();
    for origin in dangling {
        rebased.drop_references(origin);
    }
    (rebased, conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_edit::diff::diff;
    use crate::schema_edit::fixtures::{existing, users};
    use crate::schema_edit::model::*;

    #[test]
    fn untouched_draft_follows_the_server() {
        let old = users(SchemaEngine::Postgres);
        let mut new = old.clone();
        new.table.column_mut(existing(3)).unwrap().comment = Some("server".into());
        let (rebased, conflicts) = rebase(&old, &new, &SchemaDraft::from_baseline(&old));
        assert!(conflicts.is_empty());
        assert!(diff(&new, &rebased).is_empty());
    }

    #[test]
    fn user_edits_carry_over_and_server_edits_are_kept() {
        let old = users(SchemaEngine::Postgres);
        let mut draft = SchemaDraft::from_baseline(&old);
        draft.table.column_mut(existing(3)).unwrap().name = "full_name".into();
        let added = draft.add_column("age", "integer");
        let mut new = old.clone();
        new.table.column_mut(existing(3)).unwrap().nullable = false;
        new.table
            .columns
            .push(ColumnModel::new(existing(9), "server_col", "text"));
        let (rebased, conflicts) = rebase(&old, &new, &draft);
        assert!(conflicts.is_empty());
        let column = rebased.table.column(existing(3)).unwrap();
        assert_eq!(column.name, "full_name");
        assert!(!column.nullable);
        assert!(rebased.table.column(added).is_some());
        assert!(rebased.table.column(existing(9)).is_some());
        assert_eq!(rebased.next_id, draft.next_id);
        // Only the user's edits remain as changes.
        assert_eq!(diff(&new, &rebased).object_count(), 2);
    }

    #[test]
    fn both_sides_changing_a_field_is_a_conflict_and_the_draft_wins() {
        let old = users(SchemaEngine::Postgres);
        let mut draft = SchemaDraft::from_baseline(&old);
        draft.table.column_mut(existing(3)).unwrap().data_type = "varchar(50)".into();
        let mut new = old.clone();
        new.table.column_mut(existing(3)).unwrap().data_type = "varchar(80)".into();
        let (rebased, conflicts) = rebase(&old, &new, &draft);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].origin, Some(existing(3)));
        assert_eq!(
            rebased.table.column(existing(3)).unwrap().data_type,
            "varchar(50)"
        );
    }

    #[test]
    fn edits_to_server_removed_objects_are_dropped_and_reported() {
        let old = users(SchemaEngine::Postgres);
        let mut draft = SchemaDraft::from_baseline(&old);
        draft.table.column_mut(existing(4)).unwrap().comment = Some("x".into());
        draft.table.indexes[0].unique = true;
        let mut new = old.clone();
        new.table.columns.retain(|c| c.origin != existing(4));
        new.table.foreign_keys.clear();
        new.table.indexes.clear();
        let (rebased, conflicts) = rebase(&old, &new, &draft);
        assert!(rebased.table.column(existing(4)).is_none());
        assert!(rebased.table.indexes.is_empty());
        assert_eq!(conflicts.len(), 2);
    }

    #[test]
    fn positional_engines_restart_when_ids_shift() {
        let old = users(SchemaEngine::Sqlite);
        let mut draft = SchemaDraft::from_baseline(&old);
        draft.table.column_mut(existing(3)).unwrap().comment = Some("x".into());
        // The server dropped `email` (cid 1): every later cid moves down.
        let mut new = old.clone();
        new.table.columns.remove(1);
        for (cid, column) in new.table.columns.iter_mut().enumerate() {
            column.origin = existing(cid as u64 + 1);
        }
        let (rebased, conflicts) = rebase(&old, &new, &draft);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(rebased.table, new.table);
        assert_eq!(rebased.next_id, draft.next_id);
        // A same-shape change (a type edit) still merges.
        let mut new = old.clone();
        new.table.column_mut(existing(2)).unwrap().data_type = "text".into();
        let (rebased, conflicts) = rebase(&old, &new, &draft);
        assert!(conflicts.is_empty());
        assert_eq!(
            rebased
                .table
                .column(existing(3))
                .unwrap()
                .comment
                .as_deref(),
            Some("x")
        );
    }

    #[test]
    fn draft_removals_stay_removed() {
        let old = users(SchemaEngine::Postgres);
        let mut draft = SchemaDraft::from_baseline(&old);
        draft.remove_column(existing(3));
        let new = old.clone();
        let (rebased, conflicts) = rebase(&old, &new, &draft);
        assert!(conflicts.is_empty());
        assert_eq!(rebased, draft);
    }
}
