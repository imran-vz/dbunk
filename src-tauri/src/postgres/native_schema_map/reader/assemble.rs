use super::*;
use crate::postgres::relationship_metadata;
use std::collections::{BTreeMap, BTreeSet};
fn action(code: &str) -> Result<SchemaMapAction, CatalogError> {
    match code {
        "a" => Ok(SchemaMapAction::NoAction),
        "r" => Ok(SchemaMapAction::Restrict),
        "c" => Ok(SchemaMapAction::Cascade),
        "n" => Ok(SchemaMapAction::SetNull),
        "d" => Ok(SchemaMapAction::SetDefault),
        _ => Err(CatalogError::InvalidResponse),
    }
}
fn keys_valid(
    keys: &[Key],
    tables: &[SchemaMapTable],
    indices: &BTreeMap<u32, usize>,
    limit: usize,
) -> Result<(), CatalogError> {
    if keys.iter().map(|key| key.columns.len()).sum::<usize>() > limit {
        return Err(CatalogError::SchemaMapLimit);
    }
    let mut ids = BTreeSet::new();
    for key in keys {
        let table = indices
            .get(&key.table_oid)
            .and_then(|i| tables.get(*i))
            .ok_or(CatalogError::InvalidResponse)?;
        if key.oid == 0
            || !ids.insert(key.oid)
            || key.columns.is_empty()
            || key.columns.len() > 64
            || key
                .columns
                .iter()
                .any(|n| !table.columns.iter().any(|c| c.attnum == *n))
            || key.columns.iter().collect::<BTreeSet<_>>().len() != key.columns.len()
        {
            return Err(CatalogError::InvalidResponse);
        }
    }
    Ok(())
}
pub(super) fn assemble(
    header: Header,
    scope: SchemaMapScope,
    schema_oid: Option<u32>,
    focus_oid: Option<u32>,
    parts: Parts,
) -> Result<SchemaMapSnapshot, CatalogError> {
    let Parts {
        tables,
        columns,
        foreign_keys,
        unique,
        outgoing,
        triggers,
    } = parts;
    let mut tables = tables
        .into_iter()
        .map(|t| SchemaMapTable {
            identity: SchemaMapIdentity {
                database_oid: header.database_oid,
                relation_oid: t.oid,
            },
            schema_oid: t.schema_oid,
            schema: t.schema,
            name: t.name,
            kind: t.kind,
            external: t.external,
            columns: Vec::new(),
            triggers: Vec::new(),
            junction: false,
        })
        .collect::<Vec<_>>();
    let indices = tables
        .iter()
        .enumerate()
        .map(|(i, t)| (t.identity.relation_oid, i))
        .collect::<BTreeMap<_, _>>();
    if indices.len() != tables.len() {
        return Err(CatalogError::InvalidResponse);
    }
    for row in columns {
        let index = indices
            .get(&row.table_oid)
            .ok_or(CatalogError::InvalidResponse)?;
        tables[*index].columns.push(row.column);
    }
    keys_valid(&unique, &tables, &indices, MAX_SCHEMA_MAP_KEY_COLUMNS)?;
    keys_valid(&outgoing, &tables, &indices, MAX_SCHEMA_MAP_PAIRS)?;
    let junctions = classify::junctions(&outgoing, &unique);
    for table in &mut tables {
        table.junction = junctions.tables.contains(&table.identity.relation_oid);
    }
    if foreign_keys
        .iter()
        .map(|fk| fk.source_columns.len())
        .sum::<usize>()
        > MAX_SCHEMA_MAP_PAIRS
    {
        return Err(CatalogError::SchemaMapLimit);
    }
    let mut edges = Vec::with_capacity(foreign_keys.len());
    for fk in foreign_keys {
        let source = &tables[*indices
            .get(&fk.source)
            .ok_or(CatalogError::InvalidResponse)?];
        let target = &tables[*indices
            .get(&fk.target)
            .ok_or(CatalogError::InvalidResponse)?];
        if fk.source_columns.is_empty()
            || fk.source_columns.len() != fk.target_columns.len()
            || fk.source_columns.len() > 64
        {
            return Err(CatalogError::InvalidResponse);
        }
        let mut nullable = false;
        for n in &fk.source_columns {
            nullable |= source
                .columns
                .iter()
                .find(|c| c.attnum == *n)
                .ok_or(CatalogError::InvalidResponse)?
                .nullable;
        }
        if fk
            .target_columns
            .iter()
            .any(|n| !target.columns.iter().any(|c| c.attnum == *n))
        {
            return Err(CatalogError::InvalidResponse);
        }
        let keys = unique
            .iter()
            .filter(|key| key.table_oid == fk.source)
            .collect::<Vec<_>>();
        let columns_unique = classify::unique(&fk.source_columns, &keys);
        let (_, reason) = relationship_metadata::classify_cardinality(columns_unique);
        let match_type = match fk.match_type.as_str() {
            "s" => "SIMPLE",
            "f" => "FULL",
            "p" => "PARTIAL",
            _ => return Err(CatalogError::InvalidResponse),
        }
        .to_owned();
        edges.push(SchemaMapForeignKey {
            database_oid: header.database_oid,
            constraint_oid: fk.oid,
            name: fk.name,
            source: source.identity,
            target: target.identity,
            columns: fk
                .source_columns
                .into_iter()
                .zip(fk.target_columns)
                .map(|(source, target)| SchemaMapColumnPair { source, target })
                .collect(),
            on_update: action(&fk.on_update)?,
            on_delete: action(&fk.on_delete)?,
            match_type,
            validated: fk.validated,
            deferrable: fk.deferrable,
            columns_nullable: nullable,
            columns_unique,
            cardinality: if columns_unique {
                SchemaMapCardinality::OneToOne
            } else {
                SchemaMapCardinality::OneToMany
            },
            cardinality_reason: reason.to_owned(),
            junction_participant: junctions.constraints.contains(&fk.oid),
        });
    }
    for t in triggers {
        if t.trigger_type < 0 || t.trigger_type & !127 != 0 {
            return Err(CatalogError::InvalidResponse);
        }
        let enabled = match t.enabled.as_str() {
            "O" => SchemaMapTriggerEnabled::Origin,
            "R" => SchemaMapTriggerEnabled::Replica,
            "A" => SchemaMapTriggerEnabled::Always,
            "D" => SchemaMapTriggerEnabled::Disabled,
            _ => return Err(CatalogError::InvalidResponse),
        };
        let index = indices
            .get(&t.table_oid)
            .ok_or(CatalogError::InvalidResponse)?;
        tables[*index].triggers.push(SchemaMapTrigger {
            oid: t.oid,
            name: t.name,
            columns: t.columns,
            timing: relationship_metadata::trigger_timing(t.trigger_type).to_owned(),
            events: relationship_metadata::trigger_events(t.trigger_type),
            orientation: relationship_metadata::trigger_orientation(t.trigger_type).to_owned(),
            enabled,
            function_oid: t.function_oid,
            function_schema: t.function_schema,
            function_name: t.function_name,
        });
    }
    Ok(SchemaMapSnapshot {
        database: header.database,
        database_oid: header.database_oid,
        captured_at: header.captured_at,
        server_version: header.server_version,
        scope,
        schema_oid,
        focus: focus_oid.map(|relation_oid| SchemaMapIdentity {
            database_oid: header.database_oid,
            relation_oid,
        }),
        tables,
        foreign_keys: edges,
    })
}
