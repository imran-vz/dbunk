// Names are parameters; every returned text uses a server-side UTF-8 byte guard.
pub(super) const IDENTITY: &str = "SELECT (SELECT oid::bigint FROM pg_catalog.pg_database WHERE datname=current_database())AS database_oid, CASE WHEN octet_length(convert_to(current_database(),'UTF8'))<=63 THEN current_database() END AS database, pg_backend_pid()AS pid";
pub(super) const DATABASE_COUNTS: &str = r#"
WITH relations AS(SELECT c.relkind,c.reltuples,n.oid AS schema_oid
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname NOT IN('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%')
SELECT count(*)FILTER(WHERE relkind IN('r','p'))::bigint AS table_count,
 count(DISTINCT schema_oid)::bigint AS schema_count,
 count(*)FILTER(WHERE relkind='i')::bigint AS index_count,
 coalesce(sum(reltuples::bigint)FILTER(WHERE relkind IN('r','p')AND reltuples>=0),0)::bigint AS known_rows,
 count(*)FILTER(WHERE relkind IN('r','p')AND reltuples<0)::bigint AS unknown_rows FROM relations"#;
pub(super) const DATABASE_SIZE: &str =
    "SELECT pg_catalog.pg_database_size(current_database())::bigint AS value";
pub(super) const CONNECTIONS: &str = "SELECT count(*)::bigint AS value FROM pg_catalog.pg_stat_activity WHERE datname=current_database()";
pub(super) const DATABASE_RELATION_SIZES: &str = r#"
WITH sizes AS(SELECT pg_catalog.pg_table_size(c.oid)AS table_bytes,pg_catalog.pg_indexes_size(c.oid)AS index_bytes
 FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE c.relkind IN('r','p') AND n.nspname NOT IN('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%')
SELECT CASE WHEN count(*)FILTER(WHERE table_bytes IS NULL)=0 THEN coalesce(sum(table_bytes),0)::bigint END AS table_bytes,
 CASE WHEN count(*)FILTER(WHERE index_bytes IS NULL)=0 THEN coalesce(sum(index_bytes),0)::bigint END AS index_bytes FROM sizes"#;
pub(super) const SCHEMA: &str =
    "SELECT oid::bigint AS oid FROM pg_catalog.pg_namespace WHERE nspname=$1";
pub(super) const RELATION: &str = "SELECT c.oid::bigint AS oid,n.oid::bigint AS schema_oid,c.relkind::text AS kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2";
// Baseline list scope excludes child partitions; explicit relation scope can
// inspect one child. No parent row is presented as recursive descendant size.
const SCOPE: &str = r#"FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE c.relkind IN('r','p','v','m') AND ($2::bigint IS NOT NULL OR NOT c.relispartition)
 AND ($1::bigint IS NOT NULL OR (n.nspname NOT IN('pg_catalog','information_schema') AND n.nspname NOT LIKE 'pg_toast%'))
 AND ($1::bigint IS NULL OR n.oid::bigint=$1) AND ($2::bigint IS NULL OR c.oid::bigint=$2)"#;
pub(super) fn totals() -> String {
    format!(
        r#"SELECT count(*)::bigint AS relation_count,count(*)FILTER(WHERE c.relkind IN('r','p'))::bigint AS table_count,
 count(*)FILTER(WHERE c.relkind='v')::bigint AS view_count,count(*)FILTER(WHERE c.relkind='m')::bigint AS materialized_view_count,
 coalesce(sum(c.reltuples::bigint)FILTER(WHERE c.relkind<>'v' AND c.reltuples>=0),0)::bigint AS known_rows,
 count(*)FILTER(WHERE c.relkind<>'v' AND c.reltuples<0)::bigint AS unknown_rows {SCOPE}"#
    )
}
pub(super) fn scope_size() -> String {
    format!("WITH sizes AS(SELECT CASE WHEN c.relkind='v' THEN 0::bigint ELSE pg_catalog.pg_total_relation_size(c.oid)END AS bytes {SCOPE}) SELECT CASE WHEN count(*)FILTER(WHERE bytes IS NULL)=0 THEN coalesce(sum(bytes),0)::bigint END AS value FROM sizes")
}
pub(super) fn page() -> String {
    format!(
        r#"SELECT c.oid::bigint AS oid,n.oid::bigint AS schema_oid,
 CASE WHEN octet_length(convert_to(n.nspname::text,'UTF8'))<=63 THEN n.nspname::text END AS schema,
 CASE WHEN octet_length(convert_to(c.relname::text,'UTF8'))<=63 THEN c.relname::text END AS name,
 c.relkind::text AS kind,c.relispartition AS is_partition,
 CASE WHEN c.reltuples>=0 THEN c.reltuples::bigint END AS estimate
 {SCOPE} AND ($3::text IS NULL OR (n.nspname::text COLLATE "C",c.relname::text COLLATE "C",c.oid::bigint)>($3::text COLLATE "C",$4::text COLLATE "C",$5::bigint))
 ORDER BY n.nspname::text COLLATE "C",c.relname::text COLLATE "C",c.oid LIMIT 257"#
    )
}
pub(super) const PAGE_SIZES: &str = "SELECT x.oid::bigint AS oid,pg_catalog.pg_total_relation_size(x.oid)AS value FROM unnest($1::oid[])WITH ORDINALITY AS x(oid,position) ORDER BY position";
