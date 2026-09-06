use super::*;

pub(super) const BEGIN: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;
SET LOCAL search_path = pg_catalog;
SET LOCAL timezone = 'UTC'; SET LOCAL datestyle = 'ISO, YMD';
SET LOCAL intervalstyle = 'postgres'; SET LOCAL extra_float_digits = 3;
SET LOCAL standard_conforming_strings = on; SET LOCAL quote_all_identifiers = off;";

pub(super) const SCHEMA: &str = "SELECT oid, has_schema_privilege(oid,'USAGE')
FROM pg_catalog.pg_namespace WHERE nspname=$1";

pub(super) const INVENTORY: &str = "SELECT c.oid, c.relname::text AS name, c.relkind::text AS kind,
 c.relispartition AS partition_member,
 EXISTS (SELECT 1 FROM pg_catalog.pg_inherits i WHERE i.inhrelid=c.oid OR i.inhparent=c.oid) AS inherited,
 EXISTS (SELECT 1 FROM pg_catalog.pg_depend d WHERE d.classid='pg_catalog.pg_class'::regclass AND d.objid=c.oid AND d.deptype='e') AS extension_owned,
 EXISTS (SELECT 1 FROM pg_catalog.pg_locks l WHERE l.pid=pg_backend_pid() AND l.locktype='relation'
 AND l.relation=c.oid AND l.database=(SELECT oid FROM pg_catalog.pg_database WHERE datname=current_database())
 AND l.granted AND l.mode='AccessShareLock') AS locked,
 has_table_privilege(c.oid,'SELECT') AS readable
FROM pg_catalog.pg_class c WHERE c.relnamespace=$1 AND c.relkind IN ('r','p','f','v','m','S','c')
ORDER BY c.relname COLLATE \"C\" LIMIT $2";

// Each excluded category is counted through a capped subquery. A saturated
// count is explicitly a lower bound, never a complete inventory claim.
pub(super) async fn excluded_counts(
    client: &Client,
    control: &CaptureControl,
    schema: u32,
    capture: &mut CapturedEndpoint,
) -> Result<(), CompareError> {
    const COUNTS: &str = "SELECT
      (SELECT count(*)::integer FROM (SELECT 1 FROM pg_proc WHERE pronamespace=$1 LIMIT 2001) s),
      (SELECT count(*)::integer FROM (SELECT 1 FROM pg_type WHERE typnamespace=$1 LIMIT 2001) s),
      (SELECT count(*)::integer FROM (SELECT 1 FROM pg_policy p JOIN pg_class c ON c.oid=p.polrelid WHERE c.relnamespace=$1 LIMIT 2001) s),
      (SELECT count(*)::integer FROM (SELECT 1 FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE c.relnamespace=$1 LIMIT 2001) s),
      (SELECT count(*)::integer FROM (SELECT 1 FROM pg_rewrite r JOIN pg_class c ON c.oid=r.ev_class WHERE c.relnamespace=$1 LIMIT 2001) s),
      (SELECT count(*)::integer FROM (SELECT 1 FROM pg_extension WHERE extnamespace=$1 LIMIT 2001) s)";
    let rows = query(client, control, COUNTS, &[&schema]).await?;
    for (i, category) in [
        ExcludedCategory::Routines,
        ExcludedCategory::TypesAndDomains,
        ExcludedCategory::Policies,
        ExcludedCategory::Triggers,
        ExcludedCategory::Rules,
        ExcludedCategory::Extensions,
    ]
    .into_iter()
    .enumerate()
    {
        let count: i32 = rows[0].try_get(i).map_err(|_| CompareError::Unavailable)?;
        capture.excluded_counts.push(ExcludedCount {
            category,
            count: count as u32,
            complete: count <= 2000,
        });
    }
    Ok(())
}
