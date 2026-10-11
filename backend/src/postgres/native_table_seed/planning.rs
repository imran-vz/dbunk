use super::*;
use crate::seed;
use futures_util::{pin_mut, TryStreamExt};
// The planner itself is engine-neutral (`engine`); these take the
// PostgreSQL catalog directly for the runner and its tests.
pub(super) fn draft(
    catalog: &catalog::Catalog,
    intent: &TableSeedIntent,
) -> Result<seed::PlanDraft, TableSeedError> {
    engine::draft(&catalog.table(), intent)
}
pub(super) fn columns(
    catalog: &catalog::Catalog,
    draft: Option<&seed::PlanDraft>,
) -> Vec<TableSeedColumn> {
    engine::columns(&catalog.table(), draft)
}
pub(super) use engine::{capacity, options, summary};
pub(super) async fn finalize(
    connection: &DedicatedConnection,
    plan: &Plan,
    catalog: &catalog::Catalog,
) -> Result<seed::SeedPlan, Failure> {
    let draft = draft(catalog, &plan.intent)?;
    let structure = catalog.structure();
    let keys = catalog
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
        let c = &catalog
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
    engine::capacity(&draft, plan.intent.row_count, &plan.intent)?;
    let mut seed_plan = seed::finalize_plan(&structure, draft, pools, &maxes, plan.clock)
        .map_err(|_| TableSeedError::InvalidRecipe)?;
    seed_plan.options = engine::options(&plan.intent);
    Ok(seed_plan)
}
