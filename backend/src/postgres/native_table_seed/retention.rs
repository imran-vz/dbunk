use super::*;
use std::mem::size_of;
/// Owned Vec/String capacities of a PostgreSQL catalog, counted into the plan's
/// fixed 4 MiB capture.
pub(super) fn catalog_bytes(catalog: &Catalog) -> Option<usize> {
    let mut n = 0usize;
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
    for (name, labels) in &catalog.enum_labels {
        add!(name.capacity());
        vec![labels, String];
        for label in labels {
            add!(label.capacity());
        }
    }
    Some(n)
}
