//! Typed version of relationship_metadata's key-subset/junction rules. Numeric
//! catalog identity replaces the legacy schema-stripped table-name keys.
use super::reader::Key;
use std::collections::{BTreeMap, BTreeSet};
pub(super) fn unique(columns: &[i16], keys: &[&Key]) -> bool {
    !columns.is_empty()
        && keys
            .iter()
            .any(|key| !key.columns.is_empty() && key.columns.iter().all(|c| columns.contains(c)))
}
#[derive(Default)]
pub(super) struct Junctions {
    pub tables: BTreeSet<u32>,
    pub constraints: BTreeSet<u32>,
}
pub(super) fn junctions(outgoing: &[Key], unique: &[Key]) -> Junctions {
    let mut by_table: BTreeMap<u32, Vec<&Key>> = BTreeMap::new();
    for key in outgoing {
        by_table.entry(key.table_oid).or_default().push(key);
    }
    let mut result = Junctions::default();
    for identity in unique {
        let Some(fks) = by_table.get(&identity.table_oid) else {
            continue;
        };
        if identity.columns.is_empty()
            || fks.len() < 2
            || !identity
                .columns
                .iter()
                .all(|c| fks.iter().any(|fk| fk.columns.contains(c)))
        {
            continue;
        }
        let spanning = fks
            .iter()
            .filter(|fk| fk.columns.iter().any(|c| identity.columns.contains(c)))
            .collect::<Vec<_>>();
        if spanning
            .iter()
            .any(|fk| identity.columns.iter().all(|c| fk.columns.contains(c)))
        {
            continue;
        }
        let constraints = spanning.iter().map(|fk| fk.oid).collect::<BTreeSet<_>>();
        if constraints.len() < 2 {
            continue;
        }
        result.tables.insert(identity.table_oid);
        result.constraints.extend(constraints);
    }
    result
}
