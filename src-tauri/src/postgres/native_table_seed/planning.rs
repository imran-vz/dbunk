use super::*;
use crate::seed::{self, ColumnSource, GenKind, SeedDialect};
use futures_util::{pin_mut, TryStreamExt};
pub(super) fn specs(intent: &TableSeedIntent) -> Vec<crate::SeedColumnSpec> {
    intent
        .columns
        .iter()
        .map(|s| {
            let (mut skip, mut constant, mut values, mut generator) = (false, None, None, None);
            match &s.source {
                TableSeedSource::Default => skip = true,
                TableSeedSource::Constant { value } => constant = Some(value.clone()),
                TableSeedSource::Values { values: v } => values = Some(v.clone()),
                TableSeedSource::Auto { generator: g } => generator = g.map(|g| g.id().to_owned()),
            }
            crate::SeedColumnSpec {
                column: s.column.clone(),
                skip,
                constant,
                values,
                generator,
                min: None,
                max: None,
                null_rate: s.null_rate,
            }
        })
        .collect()
}
pub(super) fn draft(
    catalog: &catalog::Catalog,
    intent: &TableSeedIntent,
) -> Result<seed::PlanDraft, TableSeedError> {
    for s in &intent.columns {
        let c = &catalog
            .relation
            .columns
            .iter()
            .find(|c| c.public.name == s.column)
            .ok_or(TableSeedError::InvalidRecipe)?
            .public;
        if (c.generated || c.identity)
            && !matches!(
                s.source,
                TableSeedSource::Default | TableSeedSource::Auto { generator: None }
            )
        {
            return Err(TableSeedError::InvalidRecipe);
        }
    }
    let mut specs = specs(intent);
    for c in catalog
        .relation
        .columns
        .iter()
        .filter(|c| c.public.generated || c.public.identity)
    {
        if let Some(spec) = specs.iter_mut().find(|s| s.column == c.public.name) {
            spec.skip = true;
        } else {
            specs.push(crate::SeedColumnSpec {
                column: c.public.name.clone(),
                skip: true,
                constant: None,
                values: None,
                generator: None,
                min: None,
                max: None,
                null_rate: None,
            });
        }
    }
    seed::analyze_plan(SeedDialect::Postgres, &catalog.structure(), &specs)
        .map_err(|_| TableSeedError::UnsupportedColumn)
}
pub(super) fn columns(
    catalog: &catalog::Catalog,
    draft: Option<&seed::PlanDraft>,
) -> Vec<TableSeedColumn> {
    let foreign = catalog
        .keys
        .iter()
        .filter(|k| k.kind == "f")
        .collect::<Vec<_>>();
    catalog
        .relation
        .columns
        .iter()
        .enumerate()
        .map(|(index, c)| {
            let action = match draft
                .and_then(|d| d.columns().get(index))
                .map(|c| &c.source)
            {
                Some(ColumnSource::Skip) => TableSeedColumnAction::Default,
                Some(ColumnSource::Constant(Some(_))) => TableSeedColumnAction::Constant,
                Some(ColumnSource::Constant(None)) => TableSeedColumnAction::UnsupportedNull,
                Some(ColumnSource::ValueList(_)) => TableSeedColumnAction::Values,
                Some(ColumnSource::FkPool { pool, .. }) => TableSeedColumnAction::ForeignKey {
                    schema: foreign[*pool].parent_schema.clone(),
                    table: foreign[*pool].parent_table.clone(),
                },
                _ => TableSeedColumnAction::Auto,
            };
            TableSeedColumn {
                name: c.public.name.clone(),
                data_type: c.public.data_type.clone(),
                nullable: c.public.nullable,
                has_default: c.public.has_default,
                generated: c.public.generated,
                identity: c.public.identity,
                action,
            }
        })
        .collect()
}
pub(super) fn summary(intent: &TableSeedIntent) -> (String, bool) {
    use std::fmt::Write;
    let mut out = String::new();
    let mut truncated = false;
    for s in &intent.columns {
        let source = match &s.source {
            TableSeedSource::Default => "DEFAULT".into(),
            TableSeedSource::Auto { generator } => {
                generator.map(|g| g.id()).unwrap_or("Auto").into()
            }
            TableSeedSource::Constant { value } => format!(
                "constant {:?}{} ({} UTF-8 bytes)",
                value.chars().take(64).collect::<String>(),
                if value.chars().count() > 64 {
                    " [preview clipped]"
                } else {
                    ""
                },
                value.len()
            ),
            TableSeedSource::Values { values } => format!(
                "{} values; first {:?} [full list in live review]",
                values.len(),
                values[0].chars().take(64).collect::<String>()
            ),
        };
        let line = format!("{}: {}; NULL rate {:?}\n", s.column, source, s.null_rate);
        if out.len() + line.len() > 8192 {
            truncated = true;
            break;
        }
        let _ = out.write_str(&line);
    }
    if out.is_empty() {
        out.push_str("All columns Auto; database-supplied columns omitted");
    }
    (out, truncated)
}
pub(super) async fn finalize(
    connection: &DedicatedConnection,
    plan: &Plan,
) -> Result<seed::SeedPlan, Failure> {
    let draft = draft(&plan.catalog, &plan.intent)?;
    let structure = plan.catalog.structure();
    let keys = plan
        .catalog
        .keys
        .iter()
        .filter(|k| k.kind == "f")
        .collect::<Vec<_>>();
    let mut pools = vec![None; keys.len()];
    let mut bytes = 0usize;
    for index in &draft.needed_pools {
        let key = keys[*index];
        let parent = format!(
            "{}.{}",
            crate::quote_double(&key.parent_schema),
            crate::quote_double(&key.parent_table)
        );
        connection
            .client
            .batch_execute(&format!("LOCK TABLE {parent} IN ACCESS SHARE MODE"))
            .await
            .map_err(database_error)?;
        let oid: Option<u32> = connection
            .client
            .query_one("SELECT pg_catalog.to_regclass($1::text)::oid", &[&parent])
            .await
            .map_err(database_error)?
            .get(0);
        if oid != Some(key.parent_oid) {
            return Err(TableSeedError::TargetChanged.into());
        }
        let names = key
            .parent_columns
            .iter()
            .map(|n| crate::quote_double(n))
            .collect::<Vec<_>>();
        let projected = names
            .iter()
            .map(|n| {
                format!("CASE WHEN pg_catalog.octet_length({n}::text)<=8192 THEN {n}::text END")
            })
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT DISTINCT {projected} FROM {parent} WHERE {} LIMIT 1000",
            names
                .iter()
                .map(|n| format!("{n} IS NOT NULL"))
                .collect::<Vec<_>>()
                .join(" AND ")
        );
        let stream = connection
            .client
            .query_raw(
                &sql,
                std::iter::empty::<&(dyn tokio_postgres::types::ToSql + Sync)>(),
            )
            .await
            .map_err(database_error)?;
        pin_mut!(stream);
        let mut rows = Vec::new();
        while let Some(row) = stream.try_next().await.map_err(database_error)? {
            let mut values = Vec::with_capacity(names.len());
            for i in 0..names.len() {
                let value: Option<String> = row.get(i);
                let value = value.ok_or(TableSeedError::Limit)?;
                bytes = bytes.saturating_add(value.capacity() + std::mem::size_of::<String>());
                if bytes > 4 * 1024 * 1024 {
                    return Err(TableSeedError::Limit.into());
                }
                values.push(value);
            }
            rows.push(values);
        }
        if rows.is_empty() {
            return Err(Failure {
                error: TableSeedError::EmptyParent,
                diagnostic: Some(Box::new(TableSeedDiagnostic {
                    sqlstate: None,
                    constraint: Some(key.name.clone()),
                    column: key.columns.first().cloned(),
                    parent_schema: Some(key.parent_schema.clone()),
                    parent_table: Some(key.parent_table.clone()),
                })),
            });
        }
        pools[*index] = Some(rows);
    }
    let mut maxes = Vec::new();
    for name in &draft.needed_maxes {
        let sql = format!(
            "SELECT COALESCE(MAX({}),0)::bigint FROM {}",
            crate::quote_double(name),
            qualified(&plan.intent.endpoint)
        );
        let value: i64 = connection
            .client
            .query_one(&sql, &[])
            .await
            .map_err(database_error)?
            .get(0);
        let c = &plan
            .catalog
            .relation
            .columns
            .iter()
            .find(|c| &c.public.name == name)
            .ok_or(TableSeedError::TargetChanged)?
            .public;
        let end = value
            .checked_add(i64::from(plan.intent.row_count))
            .ok_or(TableSeedError::IntegerRange)?;
        let max = match c.data_type.as_str() {
            "smallint" => i64::from(i16::MAX),
            "integer" => i64::from(i32::MAX),
            _ => i64::MAX,
        };
        if end > max {
            return Err(TableSeedError::IntegerRange.into());
        }
        maxes.push((name.clone(), value));
    }
    seed::finalize_plan(&structure, draft, pools, &maxes, plan.clock)
        .map_err(|_| TableSeedError::InvalidRecipe.into())
}
/// Worst-case admission before generator clones any literal, list member or FK
/// value. Includes a second copy for parameters and conservative row/SQL overhead.
pub(super) fn batch_size(
    plan: &seed::SeedPlan,
    types: &[String],
    prefix_bytes: usize,
) -> Result<u32, TableSeedError> {
    let mut row = 0usize;
    for c in &plan.columns {
        let bytes = match &c.source {
            ColumnSource::Skip => continue,
            ColumnSource::Constant(s) => s.as_ref().map_or(0, String::len),
            ColumnSource::ValueList(v) => v.iter().map(String::len).max().unwrap_or(0),
            ColumnSource::FkPool { pool, member } => plan.fk_pools[*pool]
                .iter()
                .map(|r| r[*member].len())
                .max()
                .unwrap_or(0),
            ColumnSource::Generated { kind, .. } => match kind {
                GenKind::Sentence => 1024,
                _ => 512,
            },
        };
        row = row
            .checked_add(bytes.saturating_mul(4) + 128)
            .ok_or(TableSeedError::Limit)?;
    }
    if types.is_empty() || row > 8 * 1024 * 1024 {
        return Err(TableSeedError::Limit);
    }
    let sql_row = types
        .iter()
        .try_fold(0usize, |n, t| n.checked_add(t.len() + 32))
        .ok_or(TableSeedError::Limit)?;
    let count = 500usize
        .min(60_000 / types.len())
        .min(8 * 1024 * 1024 / row.max(1))
        .min(
            (1024usize * 1024)
                .checked_sub(prefix_bytes)
                .ok_or(TableSeedError::Limit)?
                / sql_row.max(1),
        );
    if count == 0 {
        return Err(TableSeedError::Limit);
    }
    Ok(count as u32)
}
