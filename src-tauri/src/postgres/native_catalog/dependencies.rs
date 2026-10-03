//! Baseline downstream drop-impact semantics on the document-owned reader.
//! Bounded edges, addresses, depth and wire text; no DDL or user-table reads.
use super::*;
use crate::postgres::objects::{PgDropDependent, PgDropImpact, PgObjectKind, PgObjectRef};
use std::collections::BTreeSet;

pub const MAX_DROP_IMPACT_BYTES: usize = 1024 * 1024;
pub const MAX_DROP_IMPACT_RESULTS: usize = 200;
const MAX_DEPTH: u32 = 8;
const MAX_ADDRESSES_PER_DEPTH: usize = 201;
const MAX_EDGES_PER_DEPTH: usize = 8192;

pub(crate) async fn read(
    spec: &ResolvedPostgresConnectSpec,
    drivers: &DriverJoins,
    cancellation: watch::Receiver<u64>,
    reference: PgObjectRef,
) -> Result<PgDropImpact, CatalogError> {
    description::validate(&reference)?;
    owned_read(
        spec,
        drivers,
        cancellation,
        Duration::from_secs(30),
        move |client, timeout| {
            Box::pin(async move {
                begin_snapshot(client, timeout).await?;
                let impact = impact_in_snapshot(client, &reference).await?;
                client
                    .batch_execute("COMMIT")
                    .await
                    .map_err(|_| CatalogError::Database)?;
                Ok(impact)
            })
        },
    )
    .await
}

/// The bounded walk inside a caller-owned snapshot. Object-DDL observation
/// reuses it so the impact describes exactly the identity captured with it.
pub(crate) async fn impact_in_snapshot(
    client: &Client,
    reference: &PgObjectRef,
) -> Result<PgDropImpact, CatalogError> {
    let root = resolve(client, reference).await?;
    let mut walk = Walk::new(root);
    for depth in 1..=MAX_DEPTH {
        let (edges, limited) = candidates(client, &walk.frontier).await?;
        walk.advance(edges, depth, limited);
        if walk.frontier.is_empty() {
            break;
        }
    }
    if !walk.frontier.is_empty() {
        let (edges, limited) = candidates(client, &walk.frontier).await?;
        walk.probe(&edges, limited);
    }
    identify(client, &walk).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Address {
    class: i64,
    object: i64,
    sub: i64,
}
impl Address {
    fn checked(class: i64, object: i64, sub: i64) -> Result<Self, CatalogError> {
        if !(1..=u32::MAX as i64).contains(&class)
            || !(1..=u32::MAX as i64).contains(&object)
            || !(0..=i32::MAX as i64).contains(&sub)
        {
            return Err(CatalogError::InvalidResponse);
        }
        Ok(Self { class, object, sub })
    }
}
#[derive(Clone, Copy, Debug)]
struct Candidate {
    address: Address,
    reported: bool,
}
impl Candidate {
    fn normalize(
        address: Address,
        dependency: &str,
        return_owner: Option<i64>,
        relation_class: i64,
        sequence: bool,
    ) -> Result<Self, CatalogError> {
        if !matches!(dependency, "n" | "a" | "i") {
            return Err(CatalogError::InvalidResponse);
        }
        // Only a view's _RETURN rule becomes the relation. A custom rule stays
        // a rule; column subaddresses stay exact and do not become whole tables.
        let address = match return_owner {
            Some(owner) => Address::checked(relation_class, owner, 0)?,
            None => address,
        };
        Ok(Self {
            address,
            reported: dependency == "n" || sequence,
        })
    }
}
struct Walk {
    visited: BTreeSet<Address>,
    frontier: Vec<Address>,
    discovered: Vec<(Address, u32)>,
    truncated: bool,
}
impl Walk {
    fn probe(&mut self, edges: &[Candidate], limited: bool) {
        self.truncated |= limited
            || edges
                .iter()
                .any(|edge| !self.visited.contains(&edge.address));
    }
    fn new(root: Address) -> Self {
        Self {
            visited: BTreeSet::from([root]),
            frontier: vec![root],
            discovered: vec![],
            truncated: false,
        }
    }
    fn advance(&mut self, edges: Vec<Candidate>, depth: u32, limited: bool) {
        self.truncated |= limited;
        let mut normalized = BTreeMap::<Address, bool>::new();
        for candidate in edges {
            if !self.visited.contains(&candidate.address) {
                *normalized.entry(candidate.address).or_default() |= candidate.reported;
            }
        }
        self.truncated |= normalized.len() > MAX_ADDRESSES_PER_DEPTH;
        self.frontier.clear();
        for (address, reported) in normalized.into_iter().take(MAX_ADDRESSES_PER_DEPTH) {
            self.visited.insert(address);
            self.frontier.push(address);
            if reported {
                self.discovered.push((address, depth));
            }
        }
    }
}

/// The `$1` kind label understood by [`RESOLVE`].
pub(crate) fn resolve_kind(kind: PgObjectKind) -> &'static str {
    use PgObjectKind::*;
    match kind {
        Schema => "schema",
        Table => "table",
        View => "view",
        MaterializedView => "materialized-view",
        ForeignTable => "foreign-table",
        Sequence => "sequence",
        Function => "function",
        Procedure => "procedure",
        Aggregate => "aggregate",
        Type => "type",
        Domain => "domain",
        Extension => "extension",
    }
}

async fn resolve(client: &Client, reference: &PgObjectRef) -> Result<Address, CatalogError> {
    let kind = resolve_kind(reference.kind);
    let rows = client
        .query(
            RESOLVE,
            &[
                &kind,
                &reference.schema,
                &reference.name,
                &reference.identity_args,
            ],
        )
        .await
        .map_err(|_| CatalogError::Database)?;
    match rows.as_slice() {
        [] => Err(CatalogError::ObjectNotFound),
        [row] => Address::checked(
            row.try_get(0).map_err(|_| CatalogError::InvalidResponse)?,
            row.try_get(1).map_err(|_| CatalogError::InvalidResponse)?,
            0,
        ),
        _ => Err(CatalogError::InvalidResponse),
    }
}
async fn candidates(
    client: &Client,
    frontier: &[Address],
) -> Result<(Vec<Candidate>, bool), CatalogError> {
    let classes = frontier.iter().map(|a| a.class).collect::<Vec<_>>();
    let objects = frontier.iter().map(|a| a.object).collect::<Vec<_>>();
    let subs = frontier.iter().map(|a| a.sub).collect::<Vec<_>>();
    let limit = MAX_EDGES_PER_DEPTH as i64 + 1;
    let rows = client
        .query_raw(
            EDGES,
            [&classes as &(dyn ToSql + Sync), &objects, &subs, &limit],
        )
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(rows);
    let mut candidates = Vec::new();
    while let Some(row) = rows.try_next().await.map_err(|_| CatalogError::Database)? {
        if candidates.len() == MAX_EDGES_PER_DEPTH {
            return Ok((candidates, true));
        }
        let get = |name| {
            row.try_get::<_, i64>(name)
                .map_err(|_| CatalogError::InvalidResponse)
        };
        candidates.push(Candidate::normalize(
            Address::checked(get("class_id")?, get("object_id")?, get("sub_id")?)?,
            text(&row, "dependency")?,
            row.try_get("return_owner")
                .map_err(|_| CatalogError::InvalidResponse)?,
            get("relation_class")?,
            row.try_get("sequence")
                .map_err(|_| CatalogError::InvalidResponse)?,
        )?);
    }
    Ok((candidates, false))
}
async fn identify(client: &Client, walk: &Walk) -> Result<PgDropImpact, CatalogError> {
    let mut result = Impact::new(walk.truncated);
    if walk.discovered.is_empty() {
        return Ok(result.finish());
    }
    let classes = walk
        .discovered
        .iter()
        .map(|(a, _)| a.class)
        .collect::<Vec<_>>();
    let objects = walk
        .discovered
        .iter()
        .map(|(a, _)| a.object)
        .collect::<Vec<_>>();
    let subs = walk
        .discovered
        .iter()
        .map(|(a, _)| a.sub)
        .collect::<Vec<_>>();
    let depths = walk
        .discovered
        .iter()
        .map(|(_, depth)| i64::from(*depth))
        .collect::<Vec<_>>();
    let rows = client
        .query_raw(
            IDENTIFY,
            [&classes as &(dyn ToSql + Sync), &objects, &subs, &depths],
        )
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(rows);
    while let Some(row) = rows.try_next().await.map_err(|_| CatalogError::Database)? {
        if row
            .try_get::<_, bool>("oversized")
            .map_err(|_| CatalogError::InvalidResponse)?
        {
            return Err(CatalogError::DropImpactLimit);
        }
        result.push(
            text(&row, "object_type")?,
            text(&row, "identity")?,
            row.try_get("depth")
                .map_err(|_| CatalogError::InvalidResponse)?,
        )?;
    }
    Ok(result.finish())
}
struct Impact {
    value: PgDropImpact,
    bytes: usize,
}
impl Impact {
    fn new(truncated: bool) -> Self {
        let value = PgDropImpact {
            dependents: vec![],
            truncated,
        };
        Self {
            bytes: json_size(&value).expect("fixed impact shape"),
            value,
        }
    }
    fn push(&mut self, object_type: &str, identity: &str, depth: i32) -> Result<(), CatalogError> {
        if object_type.is_empty() || identity.is_empty() || !(1..=MAX_DEPTH as i32).contains(&depth)
        {
            return Err(CatalogError::InvalidResponse);
        }
        if object_type.len() > MAX_TEXT_BYTES || identity.len() > MAX_TEXT_BYTES {
            return Err(CatalogError::DropImpactLimit);
        }
        if self.value.dependents.len() == MAX_DROP_IMPACT_RESULTS {
            self.value.truncated = true;
            return Ok(());
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Borrowed<'a> {
            object_type: &'a str,
            identity: &'a str,
            depth: u32,
        }
        let row = Borrowed {
            object_type,
            identity,
            depth: depth as u32,
        };
        self.bytes = self.bytes.saturating_add(json_size(&row)? + 1);
        if self.bytes > MAX_DROP_IMPACT_BYTES {
            return Err(CatalogError::DropImpactLimit);
        }
        self.value.dependents.push(PgDropDependent {
            object_type: object_type.into(),
            identity: identity.into(),
            depth: depth as u32,
        });
        Ok(())
    }
    fn finish(self) -> PgDropImpact {
        self.value
    }
}

// All identity components are bound values. Exact kind and routine signature
// prevent a recreated or overloaded object from silently replacing the target.
pub(crate) const RESOLVE: &str = r#"
SELECT class_id, object_id FROM (
 SELECT 'pg_namespace'::regclass::oid::bigint AS class_id, n.oid::bigint AS object_id
 FROM pg_namespace n WHERE $1='schema' AND n.nspname=$3
 UNION ALL
 SELECT 'pg_class'::regclass::oid::bigint, c.oid::bigint
 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace
 WHERE n.nspname=$2 AND c.relname=$3 AND
 (($1='table' AND c.relkind IN ('r','p')) OR ($1='view' AND c.relkind='v') OR
  ($1='materialized-view' AND c.relkind='m') OR ($1='foreign-table' AND c.relkind='f') OR ($1='sequence' AND c.relkind='S'))
 UNION ALL
 SELECT 'pg_proc'::regclass::oid::bigint, p.oid::bigint
 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
 WHERE n.nspname=$2 AND p.proname=$3 AND pg_get_function_identity_arguments(p.oid)=$4 AND
 (($1='function' AND p.prokind='f') OR ($1='procedure' AND p.prokind='p') OR ($1='aggregate' AND p.prokind='a'))
 UNION ALL
 SELECT 'pg_type'::regclass::oid::bigint, t.oid::bigint
 FROM pg_type t JOIN pg_namespace n ON n.oid=t.typnamespace LEFT JOIN pg_class c ON c.oid=t.typrelid
 WHERE n.nspname=$2 AND t.typname=$3 AND
 (($1='domain' AND t.typtype='d') OR ($1='type' AND t.typtype IN ('c','e','r','m') AND (t.typtype<>'c' OR c.relkind='c')))
 UNION ALL
 SELECT 'pg_extension'::regclass::oid::bigint, e.oid::bigint
 FROM pg_extension e JOIN pg_namespace n ON n.oid=e.extnamespace
 WHERE $1='extension' AND n.nspname=$2 AND e.extname=$3
) addresses LIMIT 2
"#;

// A fixed-width edge sentinel bounds client work before normalization. The
// statement deadline also bounds PostgreSQL's scan/sort work. No schema filter:
// a schema cascade can reach objects in other schemas. Automatic/internal
// objects are walked silently, except owned/identity sequences are reported.
const EDGES: &str = r#"
WITH frontier(class_id, object_id, sub_id) AS MATERIALIZED (
 SELECT * FROM unnest($1::bigint[], $2::bigint[], $3::bigint[])
)
SELECT d.classid::bigint AS class_id, d.objid::bigint AS object_id, d.objsubid::bigint AS sub_id,
 d.deptype::text AS dependency,
 CASE WHEN r.rulename='_RETURN' THEN r.ev_class::bigint END AS return_owner,
 'pg_class'::regclass::oid::bigint AS relation_class,
 (d.classid='pg_class'::regclass AND EXISTS (SELECT 1 FROM pg_class c WHERE c.oid=d.objid AND c.relkind='S')) AS sequence
FROM frontier f JOIN pg_depend d ON d.refclassid=f.class_id::oid AND d.refobjid=f.object_id::oid
 AND (f.sub_id=0 OR d.refobjsubid=f.sub_id) AND d.deptype IN ('n','a','i')
LEFT JOIN pg_rewrite r ON d.classid='pg_rewrite'::regclass AND r.oid=d.objid
ORDER BY d.classid, d.objid, d.objsubid, d.deptype, d.refclassid, d.refobjid, d.refobjsubid LIMIT $4
"#;
const IDENTIFY: &str = r#"
WITH addresses(class_id, object_id, sub_id, depth) AS MATERIALIZED (
 SELECT * FROM unnest($1::bigint[], $2::bigint[], $3::bigint[], $4::bigint[])
), identified_rows AS MATERIALIZED (
 SELECT i.type::text AS object_type, i.identity::text AS identity, a.depth::integer AS depth
 FROM addresses a CROSS JOIN LATERAL pg_identify_object(a.class_id::oid, a.object_id::oid, a.sub_id::integer) i
), identified AS MATERIALIZED (
 SELECT DISTINCT ON (identity COLLATE "C") object_type, identity, depth
 FROM identified_rows ORDER BY identity COLLATE "C", depth, object_type COLLATE "C"
)
SELECT CASE WHEN octet_length(object_type)<=8192 THEN object_type END AS object_type,
 CASE WHEN octet_length(identity)<=8192 THEN identity END AS identity, depth,
 (octet_length(object_type)>8192 OR octet_length(identity)>8192) AS oversized
FROM identified ORDER BY depth, identity COLLATE "C" LIMIT 201
"#;

#[cfg(test)]
mod tests;
