-- PG16 only. Parameters: retained table OIDs, selected schema, field offset,
-- row limit, raw field byte limit, transport batch byte limit.
WITH tables AS MATERIALIZED (
 SELECT c.oid, c.relname, c.relpersistence, d.description
 FROM pg_class c LEFT JOIN pg_description d ON d.classoid='pg_class'::regclass
 AND d.objoid=c.oid AND d.objsubid=0 WHERE c.oid=ANY($1::oid[])
), columns AS MATERIALIZED (
 SELECT a.*, t.typname, tn.nspname AS type_schema, cn.nspname AS collation_schema,
 co.collname, d.description, ad.oid AS default_oid,
 row_number() OVER (PARTITION BY a.attrelid ORDER BY a.attnum)::integer AS visible_position,
 pg_get_expr(ad.adbin, ad.adrelid, false) AS expression,
 EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid='pg_attrdef'::regclass AND d.objid=ad.oid AND NOT (d.refclassid='pg_class'::regclass AND d.refobjid=a.attrelid)) AS external_dependency
 FROM pg_attribute a JOIN pg_type t ON t.oid=a.atttypid
 JOIN pg_namespace tn ON tn.oid=t.typnamespace
 LEFT JOIN pg_collation co ON co.oid=a.attcollation LEFT JOIN pg_namespace cn ON cn.oid=co.collnamespace
 LEFT JOIN pg_description d ON d.classoid='pg_class'::regclass AND d.objoid=a.attrelid AND d.objsubid=a.attnum
 LEFT JOIN pg_attrdef ad ON ad.adrelid=a.attrelid AND ad.adnum=a.attnum
 WHERE a.attrelid=ANY($1::oid[]) AND a.attnum>0 AND NOT a.attisdropped
), constraints AS MATERIALIZED (
 SELECT c.*, rn.nspname AS referenced_schema, r.relname AS referenced_name,
 pg_get_expr(c.conbin,c.conrelid,false) AS expression, EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid='pg_constraint'::regclass AND d.objid=c.oid AND NOT (d.refclassid='pg_class'::regclass AND d.refobjid=c.conrelid)) AS external_dependency
 FROM pg_constraint c LEFT JOIN pg_class r ON r.oid=c.confrelid
 LEFT JOIN pg_namespace rn ON rn.oid=r.relnamespace
 WHERE c.conrelid=ANY($1::oid[]) AND c.contype IN ('p','u','f','c','x')
), indexes AS MATERIALIZED (
 SELECT i.*, c.relname, c.reloptions, am.amname, am.oid AS amoid, own.conname AS owner,
 pg_get_expr(i.indexprs,i.indrelid,false) AS expression,
 pg_get_expr(i.indpred,i.indrelid,false) AS predicate,
 EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid='pg_class'::regclass AND d.objid=i.indexrelid AND NOT (d.refclassid='pg_class'::regclass AND d.refobjid=i.indrelid)) AS external_dependency
 FROM pg_index i JOIN pg_class c ON c.oid=i.indexrelid JOIN pg_am am ON am.oid=c.relam
 LEFT JOIN pg_constraint own ON own.conindid=i.indexrelid AND own.conrelid=i.indrelid AND own.contype IN ('p','u','x')
 WHERE i.indrelid=ANY($1::oid[])
), index_keys AS MATERIALIZED (
 SELECT i.*, k.position, k.num, a.attname, ia.attoptions,
 opc.opcname, opn.nspname AS opclass_schema, co.collname, cn.nspname AS collation_schema,
 i.indoption[k.position-1] AS options
 FROM indexes i CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY k(num,position)
 LEFT JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num AND NOT a.attisdropped
 LEFT JOIN pg_attribute ia ON ia.attrelid=i.indexrelid AND ia.attnum=k.position
 LEFT JOIN pg_opclass opc ON opc.oid=i.indclass[k.position-1]
 LEFT JOIN pg_namespace opn ON opn.oid=opc.opcnamespace
 LEFT JOIN pg_collation co ON co.oid=i.indcollation[k.position-1]
 LEFT JOIN pg_namespace cn ON cn.oid=co.collnamespace
 WHERE k.position<=i.indnkeyatts
), facts AS (
SELECT t.oid AS table_oid, jsonb_build_object('kind','table') || jsonb_build_object('field',f.field) AS path, f.kind, f.value, f.external_dependency
 FROM tables t CROSS JOIN LATERAL (VALUES
  ('persistence', 'text', to_jsonb(CASE t.relpersistence WHEN 'p' THEN 'permanent' WHEN 'u' THEN 'unlogged' WHEN 't' THEN 'temporary' END), false),
  ('comment', 'text', to_jsonb(t.description), false)) f(field,kind,value,external_dependency)
UNION ALL
SELECT c.attrelid AS table_oid, jsonb_build_object('kind','column','name',c.attname) || jsonb_build_object('field',f.field) AS path, f.kind, f.value, f.external_dependency
 FROM columns c CROSS JOIN LATERAL (VALUES
  ('position', 'integer', to_jsonb(c.visible_position), false),
  ('type', 'reference', jsonb_build_object('namespace', CASE WHEN c.type_schema = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',c.type_schema) END, 'name',c.typname), false),
  ('typeModifier', 'integer', to_jsonb(c.atttypmod), false),
  ('arrayDimensions', 'integer', to_jsonb(c.attndims), false),
  ('nullable', 'boolean', to_jsonb(NOT c.attnotnull), false),
  ('default', 'expression', to_jsonb(CASE WHEN c.attgenerated='' AND c.attidentity='' THEN c.expression END), c.external_dependency),
  ('generatedKind', 'text', to_jsonb(CASE c.attgenerated WHEN '' THEN 'none' WHEN 's' THEN 'stored' END), false),
  ('generatedExpression', 'expression', to_jsonb(CASE WHEN c.attgenerated<>'' THEN c.expression END), c.external_dependency),
  ('identity', 'text', to_jsonb(CASE c.attidentity WHEN '' THEN 'none' WHEN 'a' THEN 'always' WHEN 'd' THEN 'byDefault' END), false),
  ('collation', 'reference', CASE WHEN c.attcollation=0 THEN NULL ELSE jsonb_build_object('namespace', CASE WHEN c.collation_schema = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',c.collation_schema) END, 'name',c.collname) END, false),
  ('comment', 'text', to_jsonb(c.description), false)) f(field,kind,value,external_dependency)
UNION ALL
SELECT c.conrelid AS table_oid, jsonb_build_object('kind','constraint','name',c.conname) || jsonb_build_object('field',f.field) AS path, f.kind, f.value, f.external_dependency
 FROM constraints c CROSS JOIN LATERAL (VALUES
  ('kind', 'text', to_jsonb(CASE c.contype WHEN 'p' THEN 'primaryKey' WHEN 'u' THEN 'unique' WHEN 'f' THEN 'foreignKey' WHEN 'c' THEN 'check' WHEN 'x' THEN 'exclusion' END), false),
  ('keys', 'names', COALESCE((SELECT jsonb_agg(a.attname ORDER BY k.ord) FROM unnest(c.conkey) WITH ORDINALITY k(num,ord) LEFT JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.num AND NOT a.attisdropped WHERE k.num <> 0), '[]'::jsonb), false),
  ('referencedTable', 'reference', CASE WHEN c.contype='f' THEN jsonb_build_object('namespace', CASE WHEN c.referenced_schema = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',c.referenced_schema) END, 'name',c.referenced_name) END, false),
  ('referencedKeys', 'names', (SELECT jsonb_agg(a.attname ORDER BY k.ord) FROM unnest(c.confkey) WITH ORDINALITY k(num,ord) LEFT JOIN pg_attribute a ON a.attrelid=c.confrelid AND a.attnum=k.num AND NOT a.attisdropped WHERE k.num <> 0), false),
  ('updateAction', 'text', to_jsonb(CASE WHEN c.contype='f' THEN c.confupdtype::text END), false),
  ('deleteAction', 'text', to_jsonb(CASE WHEN c.contype='f' THEN c.confdeltype::text END), false),
  ('deleteColumns', 'names', (SELECT jsonb_agg(a.attname ORDER BY k.ord) FROM unnest(c.confdelsetcols) WITH ORDINALITY k(num,ord) LEFT JOIN pg_attribute a ON a.attrelid=c.conrelid AND a.attnum=k.num AND NOT a.attisdropped WHERE k.num <> 0), false),
  ('matchMode', 'text', to_jsonb(CASE WHEN c.contype='f' THEN c.confmatchtype::text END), false),
  ('deferrable', 'boolean', to_jsonb(c.condeferrable), false),
  ('initiallyDeferred', 'boolean', to_jsonb(c.condeferred), false),
  ('validated', 'boolean', to_jsonb(c.convalidated), false),
  ('noInherit', 'boolean', to_jsonb(c.connoinherit), false),
  ('expression', 'expression', to_jsonb(c.expression), c.external_dependency),
  ('equalityOperators', 'operators', (SELECT jsonb_agg(jsonb_build_object('operator',jsonb_build_object('namespace', CASE WHEN ns.nspname = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',ns.nspname) END, 'name',o.oprname),
 'leftType',CASE WHEN o.oprleft=0 THEN NULL ELSE jsonb_build_object('namespace', CASE WHEN ln.nspname = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',ln.nspname) END, 'name',lt.typname) END,
 'rightType',CASE WHEN o.oprright=0 THEN NULL ELSE jsonb_build_object('namespace', CASE WHEN rn.nspname = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',rn.nspname) END, 'name',rt.typname) END) ORDER BY k.ord)
 FROM unnest(c.conpfeqop || c.conppeqop || c.conffeqop) WITH ORDINALITY k(oid,ord)
 LEFT JOIN pg_operator o ON o.oid=k.oid LEFT JOIN pg_namespace ns ON ns.oid=o.oprnamespace
 LEFT JOIN pg_type lt ON lt.oid=o.oprleft LEFT JOIN pg_namespace ln ON ln.oid=lt.typnamespace
 LEFT JOIN pg_type rt ON rt.oid=o.oprright LEFT JOIN pg_namespace rn ON rn.oid=rt.typnamespace), false),
  ('exclusionOperators', 'operators', (SELECT jsonb_agg(jsonb_build_object('operator',jsonb_build_object('namespace', CASE WHEN ns.nspname = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',ns.nspname) END, 'name',o.oprname),
 'leftType',CASE WHEN o.oprleft=0 THEN NULL ELSE jsonb_build_object('namespace', CASE WHEN ln.nspname = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',ln.nspname) END, 'name',lt.typname) END,
 'rightType',CASE WHEN o.oprright=0 THEN NULL ELSE jsonb_build_object('namespace', CASE WHEN rn.nspname = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',rn.nspname) END, 'name',rt.typname) END) ORDER BY k.ord)
 FROM unnest(c.conexclop) WITH ORDINALITY k(oid,ord)
 LEFT JOIN pg_operator o ON o.oid=k.oid LEFT JOIN pg_namespace ns ON ns.oid=o.oprnamespace
 LEFT JOIN pg_type lt ON lt.oid=o.oprleft LEFT JOIN pg_namespace ln ON ln.oid=lt.typnamespace
 LEFT JOIN pg_type rt ON rt.oid=o.oprright LEFT JOIN pg_namespace rn ON rn.oid=rt.typnamespace), false)) f(field,kind,value,external_dependency)
UNION ALL
SELECT i.indrelid AS table_oid, jsonb_build_object('kind','index','name',i.relname,'owner',i.owner) || jsonb_build_object('field',f.field) AS path, f.kind, f.value, f.external_dependency
 FROM indexes i CROSS JOIN LATERAL (VALUES
  ('accessMethod', 'text', to_jsonb(i.amname), false),
  ('unique', 'boolean', to_jsonb(i.indisunique), false),
  ('nullsNotDistinct', 'boolean', to_jsonb(i.indnullsnotdistinct), false),
  ('immediate', 'boolean', to_jsonb(i.indimmediate), false),
  ('keyCount', 'integer', to_jsonb(i.indnkeyatts), false),
  ('includedColumns', 'names', COALESCE((SELECT jsonb_agg(a.attname ORDER BY k.ord) FROM unnest(i.indkey) WITH ORDINALITY k(num,ord) LEFT JOIN pg_attribute a ON a.attrelid=i.indrelid AND a.attnum=k.num AND NOT a.attisdropped WHERE k.ord>i.indnkeyatts), '[]'::jsonb), false),
  ('predicate', 'expression', to_jsonb(i.predicate), i.external_dependency),
  ('relationOptions', 'names', COALESCE((SELECT jsonb_agg(o ORDER BY o COLLATE "C") FROM unnest(i.reloptions) o), '[]'::jsonb), false),
  ('valid', 'boolean', to_jsonb(i.indisvalid), false),
  ('ready', 'boolean', to_jsonb(i.indisready), false),
  ('live', 'boolean', to_jsonb(i.indislive), false)) f(field,kind,value,external_dependency)
UNION ALL
SELECT i.indrelid AS table_oid, jsonb_build_object('kind','indexKey','name',i.relname,'owner',i.owner,'position',i.position-1) || jsonb_build_object('field',f.field) AS path, f.kind, f.value, f.external_dependency
 FROM index_keys i CROSS JOIN LATERAL (VALUES
  ('kind', 'text', to_jsonb(CASE WHEN i.num=0 THEN 'expression' ELSE 'column' END), false),
  ('column', 'text', to_jsonb(i.attname), false),
  ('expression', 'expression', to_jsonb(CASE WHEN i.num=0 THEN i.expression END), i.external_dependency),
  ('sortOptions', 'sortOptions', jsonb_build_object('method',i.amname,'builtin',i.amoid<16384,'bits',i.options), false),
  ('opclass', 'reference', jsonb_build_object('namespace', CASE WHEN i.opclass_schema = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',i.opclass_schema) END, 'name',i.opcname), false),
  ('opclassOptions', 'names', COALESCE((SELECT jsonb_agg(o ORDER BY o COLLATE "C") FROM unnest(i.attoptions) o), '[]'::jsonb), false),
  ('collation', 'reference', CASE WHEN i.indcollation[i.position-1]=0 THEN NULL ELSE jsonb_build_object('namespace', CASE WHEN i.collation_schema = $2 THEN jsonb_build_object('kind','selected') ELSE jsonb_build_object('kind','external','schema',i.collation_schema) END, 'name',i.collname) END, false)) f(field,kind,value,external_dependency)
), invalid AS (
 SELECT EXISTS (SELECT 1 FROM columns WHERE (default_oid IS NOT NULL AND expression IS NULL)
   OR attgenerated NOT IN ('','s') OR attidentity NOT IN ('','a','d'))
 OR EXISTS (SELECT 1 FROM constraints WHERE contype='c' AND expression IS NULL)
 OR EXISTS (SELECT 1 FROM indexes WHERE (indexprs IS NOT NULL AND expression IS NULL)
   OR (indpred IS NOT NULL AND predicate IS NULL)) AS unreadable
), page AS MATERIALIZED (
 SELECT * FROM facts ORDER BY table_oid, path::text COLLATE "C" OFFSET $3 LIMIT $4
), measured AS MATERIALIZED (
 SELECT *, CASE WHEN kind IN ('text','expression') THEN octet_length(convert_to(value #>> '{}','UTF8'))
 ELSE octet_length(convert_to(value::text,'UTF8')) END > $5 AS oversized,
 jsonb_build_object('tableOid',table_oid::bigint,'path',path,'fact',
  CASE WHEN value IS NULL OR value='null'::jsonb THEN jsonb_build_object('kind','null')
  WHEN kind='expression' THEN jsonb_build_object('kind',kind,'value',value,'externalDependency',external_dependency)
  ELSE jsonb_build_object('kind',kind,'value',value) END)::text AS data
 FROM page
), guarded AS MATERIALIZED (
 SELECT table_oid, path, COALESCE(oversized,false) AS oversized,
 CASE WHEN NOT COALESCE(oversized,false) THEN data END AS data FROM measured
), sized AS (
 SELECT *, sum(COALESCE(octet_length(convert_to(data,'UTF8')),0)+64) OVER (ORDER BY table_oid,path::text COLLATE "C") AS batch_bytes
 FROM guarded
)
SELECT oversized, data, (SELECT unreadable FROM invalid) FROM sized WHERE batch_bytes <= $6
ORDER BY table_oid,path::text COLLATE "C"
