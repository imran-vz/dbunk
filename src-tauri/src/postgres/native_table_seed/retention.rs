use super::*;
use std::mem::size_of;
/// Validate actual owned Vec/String capacities before publishing a fixed 4 MiB
/// capture. Stack headers are counted conservatively in nested public DTO checks.
pub(super) fn checked(plan: &Plan) -> Option<usize> {
    let mut n = size_of::<Plan>().checked_add(plan.intent.checked_heap_bytes()?)?;
    if let Some(description) = &plan.description {
        n = n.checked_add(description.checked_heap_bytes()?)?;
    }
    macro_rules! add {
        ($bytes:expr) => {
            n = n.checked_add($bytes)?;
        };
    }
    macro_rules! vec {
        ($v:expr,$t:ty $(,)?) => {
            add!($v.capacity().checked_mul(size_of::<$t>())?);
        };
    }
    vec![plan.columns, TableSeedColumn];
    for c in &plan.columns {
        add!(c.name.capacity());
        add!(c.data_type.capacity());
        if let TableSeedColumnAction::ForeignKey { schema, table } = &c.action {
            add!(schema.capacity());
            add!(table.capacity());
        }
    }
    let catalog = &plan.catalog;
    add!(catalog.relation.kind.capacity());
    vec![
        catalog.relation.columns,
        crate::postgres::transfer::runner::catalog::CatalogColumn,
    ];
    for c in &catalog.relation.columns {
        add!(c.public.name.capacity());
        add!(c.public.data_type.capacity());
        if let Some(s) = &c.default_fingerprint {
            add!(s.capacity());
        }
    }
    vec![catalog.keys, catalog::Key];
    for k in &catalog.keys {
        for s in [
            &k.name,
            &k.kind,
            &k.parent_schema,
            &k.parent_table,
            &k.fingerprint,
        ] {
            add!(s.capacity());
        }
        for v in [&k.columns, &k.parent_columns] {
            vec![v, String];
            for s in v {
                add!(s.capacity());
            }
        }
    }
    vec![catalog.indexes, catalog::Index];
    for i in &catalog.indexes {
        add!(i.name.capacity());
        add!(i.fingerprint.capacity());
        vec![i.columns, String];
        for s in &i.columns {
            add!(s.capacity());
        }
    }
    vec![catalog.serial_columns, String];
    for s in &catalog.serial_columns {
        add!(s.capacity());
    }
    (n <= MAX_TABLE_SEED_REVIEW_BYTES).then_some(n)
}
