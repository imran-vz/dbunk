// SQL guards prevent oversized catalog text crossing the wire. Only static
// catalog SQL and bounded integer LIMIT parameters reach this reader.
pub(super) const IDENTITY: &str = r#"SELECT pg_catalog.pg_backend_pid() AS pid,
 CASE WHEN pg_catalog.octet_length(pg_catalog.current_database()::text)<=256 THEN pg_catalog.current_database()::text END AS database, pg_catalog.octet_length(pg_catalog.current_database()::text)::bigint AS database_bytes,
 CASE WHEN pg_catalog.octet_length(current_user::text)<=8192 THEN current_user::text END AS current_user, pg_catalog.octet_length(current_user::text)::bigint AS current_user_bytes,
 CASE WHEN pg_catalog.octet_length(session_user::text)<=8192 THEN session_user::text END AS session_user, pg_catalog.octet_length(session_user::text)::bigint AS session_user_bytes,
 CASE WHEN pg_catalog.octet_length(pg_catalog.current_setting('search_path'))<=8192 THEN pg_catalog.current_setting('search_path') END AS search_path, pg_catalog.octet_length(pg_catalog.current_setting('search_path'))::bigint AS search_path_bytes"#;
pub(super) const FACTS: &str = r#"SELECT CASE WHEN pg_catalog.octet_length(pg_catalog.version())<=8192 THEN pg_catalog.version() END AS server_version, pg_catalog.octet_length(pg_catalog.version())::bigint AS server_version_bytes,
 CASE WHEN pg_catalog.octet_length(pg_catalog.current_setting('server_encoding'))<=8192 THEN pg_catalog.current_setting('server_encoding') END AS encoding, pg_catalog.octet_length(pg_catalog.current_setting('server_encoding'))::bigint AS encoding_bytes,
 CASE WHEN pg_catalog.octet_length(d.datcollate)<=8192 THEN d.datcollate END AS locale, pg_catalog.octet_length(d.datcollate)::bigint AS locale_bytes,
 CASE WHEN pg_catalog.octet_length(pg_catalog.current_setting('timezone'))<=8192 THEN pg_catalog.current_setting('timezone') END AS timezone, pg_catalog.octet_length(pg_catalog.current_setting('timezone'))::bigint AS timezone_bytes
 FROM pg_catalog.pg_database d WHERE d.datname=pg_catalog.current_database()"#;
pub(super) const SETTINGS: &str = r#"
WITH s AS MATERIALIZED (
 SELECT name,category,source,setting,unit,short_desc,boot_val,reset_val
 FROM pg_catalog.pg_settings ORDER BY category,name LIMIT $1
)
SELECT CASE WHEN pg_catalog.octet_length(s.name)<=256 THEN s.name END AS name, pg_catalog.octet_length(s.name)::bigint AS name_bytes,
 CASE WHEN pg_catalog.octet_length(s.category)<=256 THEN s.category END AS category, pg_catalog.octet_length(s.category)::bigint AS category_bytes,
 CASE WHEN pg_catalog.octet_length(s.source)<=256 THEN s.source END AS source, pg_catalog.octet_length(s.source)::bigint AS source_bytes,
 CASE WHEN pg_catalog.octet_length(s.setting)<=8192 THEN s.setting END AS setting, pg_catalog.octet_length(s.setting)::bigint AS setting_bytes,
 CASE WHEN pg_catalog.octet_length(s.unit)<=8192 THEN s.unit END AS unit, pg_catalog.octet_length(s.unit)::bigint AS unit_bytes,
 CASE WHEN pg_catalog.octet_length(s.short_desc)<=8192 THEN s.short_desc END AS short_desc, pg_catalog.octet_length(s.short_desc)::bigint AS short_desc_bytes,
 CASE WHEN pg_catalog.octet_length(s.boot_val)<=8192 THEN s.boot_val END AS boot_val, pg_catalog.octet_length(s.boot_val)::bigint AS boot_val_bytes,
 CASE WHEN pg_catalog.octet_length(s.reset_val)<=8192 THEN s.reset_val END AS reset_val, pg_catalog.octet_length(s.reset_val)::bigint AS reset_val_bytes FROM s ORDER BY s.category,s.name
"#;
pub(super) const EXTENSIONS: &str = r#"
WITH s AS MATERIALIZED (
 SELECT e.extname::text AS name,n.nspname::text AS schema,e.extversion AS version,
 pg_catalog.obj_description(e.oid,'pg_extension') AS description
 FROM pg_catalog.pg_extension e JOIN pg_catalog.pg_namespace n ON n.oid=e.extnamespace
 ORDER BY e.extname LIMIT $1
)
SELECT CASE WHEN pg_catalog.octet_length(s.name)<=256 THEN s.name END AS name, pg_catalog.octet_length(s.name)::bigint AS name_bytes,
 CASE WHEN pg_catalog.octet_length(s.schema)<=256 THEN s.schema END AS schema, pg_catalog.octet_length(s.schema)::bigint AS schema_bytes,
 CASE WHEN pg_catalog.octet_length(s.version)<=8192 THEN s.version END AS version, pg_catalog.octet_length(s.version)::bigint AS version_bytes,
 CASE WHEN pg_catalog.octet_length(s.description)<=8192 THEN s.description END AS description, pg_catalog.octet_length(s.description)::bigint AS description_bytes FROM s ORDER BY s.name
"#;
