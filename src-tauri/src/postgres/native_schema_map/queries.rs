//! Parameter-only selection. Text/array guards run before JSON serialization;
//! outer reader LIMITs each component stream before retaining any decoded rows.
pub(super) const HEADER: &str = r#"SELECT 1::int AS components,oid::bigint AS database_oid,datname::text AS database,
current_setting('server_version_num')::int AS server_version,
to_char(transaction_timestamp() AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS captured_at
FROM pg_catalog.pg_database WHERE datname=current_database()"#;
pub(super) const SCHEMA: &str =
    "SELECT oid::bigint AS oid FROM pg_catalog.pg_namespace WHERE nspname=$1";
pub(super) const RELATION: &str = "SELECT c.oid::bigint AS oid,c.relnamespace::bigint AS schema_oid,c.relkind::text AS kind FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname=$1 AND c.relname=$2";
// Database matches the explorer's user namespace scope. Explicit schema allows
// that exact namespace. Schema edges originate there; targets can be external.
const SELECTED: &str = r#"WITH base AS (
 SELECT c.oid FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace
 WHERE c.relkind IN ('r','p') AND
 CASE WHEN $2::bigint IS NOT NULL THEN c.oid::bigint=$2
 WHEN $1::bigint IS NOT NULL THEN n.oid::bigint=$1
 ELSE n.nspname<>'information_schema' AND left(n.nspname,3)<>'pg_' END
), edges AS (
 SELECT con.oid,con.conrelid,con.confrelid FROM pg_catalog.pg_constraint con
 WHERE con.contype='f' AND con.conparentid=0 AND
 CASE WHEN $2::bigint IS NOT NULL THEN con.conrelid::bigint=$2 OR con.confrelid::bigint=$2
 ELSE con.conrelid IN(SELECT oid FROM base) END
), involved AS (SELECT oid FROM base UNION SELECT conrelid FROM edges UNION SELECT confrelid FROM edges)
"#;
pub(super) fn tables() -> String {
    format!(
        r#"{SELECTED} SELECT 1::int AS components,c.oid::bigint AS oid,n.oid::bigint AS schema_oid,n.nspname::text AS schema,c.relname::text AS name,
CASE c.relkind WHEN 'r' THEN 'table' WHEN 'p' THEN 'partitioned_table' ELSE 'unsupported' END AS kind,
($1::bigint IS NOT NULL AND $2::bigint IS NULL AND n.oid::bigint<>$1) AS external
FROM involved i JOIN pg_catalog.pg_class c ON c.oid=i.oid JOIN pg_catalog.pg_namespace n ON n.oid=c.relnamespace ORDER BY n.nspname COLLATE "C",c.relname COLLATE "C",c.oid"#
    )
}
pub(super) fn foreign_keys() -> String {
    format!(
        r#"{SELECTED} SELECT (1+coalesce(cardinality(c.conkey),0))::int AS components,
(cardinality(c.conkey)>64 OR cardinality(c.conkey)<>cardinality(c.confkey)) AS array_oversized,
c.oid::bigint AS oid,c.conname::text AS name,c.conrelid::bigint AS source,c.confrelid::bigint AS target,
CASE WHEN cardinality(c.conkey)<=64 THEN c.conkey END AS source_columns,
CASE WHEN cardinality(c.confkey)<=64 THEN c.confkey END AS target_columns,
c.confupdtype::text AS on_update,c.confdeltype::text AS on_delete,c.confmatchtype::text AS match_type,c.convalidated AS validated,c.condeferrable AS deferrable
FROM edges e JOIN pg_catalog.pg_constraint c ON c.oid=e.oid ORDER BY c.conrelid,c.oid"#
    )
}
pub(super) const COLUMNS: &str = r#"SELECT 1::int AS components,a.attrelid::bigint AS table_oid,a.attnum::smallint AS attnum,a.attname::text AS name,
CASE WHEN octet_length(convert_to(format_type(a.atttypid,a.atttypmod),'UTF8'))<=8192 THEN format_type(a.atttypid,a.atttypmod) END AS data_type,
NOT a.attnotnull AS nullable,EXISTS(SELECT FROM pg_catalog.pg_constraint k WHERE k.conrelid=a.attrelid AND k.contype='p' AND a.attnum=ANY(k.conkey)) AS primary_key,
CASE WHEN octet_length(convert_to(col_description(a.attrelid,a.attnum),'UTF8'))<=4096 THEN col_description(a.attrelid,a.attnum) END AS comment,
(coalesce(octet_length(convert_to(col_description(a.attrelid,a.attnum),'UTF8'))>4096,false) OR octet_length(convert_to(format_type(a.atttypid,a.atttypmod),'UTF8'))>8192) AS array_oversized
FROM pg_catalog.pg_attribute a WHERE a.attrelid=ANY($1::oid[]) AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attrelid,a.attnum"#;
pub(super) const UNIQUE_KEYS: &str = r#"SELECT (1+i.indnkeyatts)::int AS components,(i.indnkeyatts>64)AS array_oversized,i.indrelid::bigint AS table_oid,i.indexrelid::bigint AS oid,
CASE WHEN i.indnkeyatts<=64 THEN ARRAY(SELECT k.n FROM unnest(i.indkey::smallint[]) WITH ORDINALITY k(n,o) WHERE k.o<=i.indnkeyatts ORDER BY k.o)END AS columns
FROM pg_catalog.pg_index i WHERE i.indrelid=ANY($1::oid[]) AND i.indisunique AND i.indisvalid AND i.indisready AND i.indpred IS NULL AND i.indexprs IS NULL ORDER BY i.indrelid,i.indexrelid"#;
// Full outgoing key membership for selected nodes makes junction classification
// independent of graph edge filtering. These extra edges are never published.
pub(super) const OUTGOING: &str = r#"SELECT (1+coalesce(cardinality(c.conkey),0))::int AS components,(cardinality(c.conkey)>64)AS array_oversized,c.conrelid::bigint AS table_oid,c.oid::bigint AS oid,
CASE WHEN cardinality(c.conkey)<=64 THEN c.conkey END AS columns
FROM pg_catalog.pg_constraint c WHERE c.contype='f' AND c.conparentid=0 AND c.conrelid=ANY($1::oid[]) ORDER BY c.conrelid,c.oid"#;
pub(super) const TRIGGERS: &str = r#"SELECT (1+cardinality(t.tgattr::smallint[]))::int AS components,(cardinality(t.tgattr::smallint[])>1600)AS array_oversized,
t.tgrelid::bigint AS table_oid,t.oid::bigint AS oid,t.tgname::text AS name,t.tgtype::smallint AS trigger_type,t.tgenabled::text AS enabled,
CASE WHEN cardinality(t.tgattr::smallint[])<=1600 THEN t.tgattr::smallint[] END AS columns,
p.oid::bigint AS function_oid,n.nspname::text AS function_schema,p.proname::text AS function_name
FROM pg_catalog.pg_trigger t JOIN pg_catalog.pg_proc p ON p.oid=t.tgfoid JOIN pg_catalog.pg_namespace n ON n.oid=p.pronamespace
WHERE t.tgrelid=ANY($1::oid[]) AND NOT t.tgisinternal AND t.tgparentid=0 ORDER BY t.tgrelid,t.tgname COLLATE "C",t.oid"#;
