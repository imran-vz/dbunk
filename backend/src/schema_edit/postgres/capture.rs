//! Snapshot to baseline: a pure conversion of one table-structure capture
//! into the engine-neutral model. Anything the model cannot represent
//! faithfully makes the baseline `ReadOnly` with a reason; nothing is guessed.
//!
//! Baseline ids are stable across captures of an unchanged table and disjoint
//! across kinds: a column is its `attnum`; every other object is
//! `kind_tag << 48 | oid`.
use crate::postgres::native_catalog::table_structure::{
    StructureConstraint, StructureForeignKey, StructureGeneratedKind, StructureIdentityKind,
    StructureIndex, StructureIndexKey, StructureReferentialAction, StructureRelationKind,
    TableStructureSnapshot,
};
use crate::schema_edit::model::*;

pub(crate) const TAG_PRIMARY_KEY: u64 = 1;
pub(crate) const TAG_UNIQUE: u64 = 2;
pub(crate) const TAG_FOREIGN_KEY: u64 = 3;
pub(crate) const TAG_CHECK: u64 = 4;
pub(crate) const TAG_INDEX: u64 = 5;

pub(crate) fn object_origin(tag: u64, oid: u32) -> Origin {
    Origin::Existing(BaselineId((tag << 48) | u64::from(oid)))
}

fn column_origin(number: i32) -> Option<Origin> {
    u64::try_from(number)
        .ok()
        .filter(|n| *n > 0)
        .map(|n| Origin::Existing(BaselineId(n)))
}

/// `160004` as `16.4`.
pub(crate) fn server_version_text(number: u32) -> String {
    format!("{}.{}", number / 10_000, number % 10_000)
}

pub(crate) fn features() -> ServerFeatures {
    ServerFeatures {
        rename_column: true,
        drop_column: true,
        enforced_check: true,
        transactional_ddl: true,
        ..ServerFeatures::default()
    }
}

/// The drift token: relation OID plus a digest of the catalog row versions
/// of the table and its sub-objects (see `catalog::DRIFT`).
pub(crate) fn drift_token(oid: u32, digest: &str) -> DriftToken {
    DriftToken(format!("pg:{oid}:{digest}"))
}

pub(crate) fn drift_oid(token: &DriftToken) -> Option<u32> {
    let mut parts = token.0.splitn(3, ':');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("pg"), Some(oid), Some(_)) => oid.parse().ok().filter(|oid| *oid != 0),
        _ => None,
    }
}

/// `pg_class.reltuples`; negative means never analyzed.
pub(crate) fn estimated_rows(reltuples: f64) -> Option<u64> {
    (reltuples.is_finite() && reltuples >= 0.0).then_some(reltuples as u64)
}

pub(crate) fn baseline_from_snapshot(
    snapshot: &TableStructureSnapshot,
    drift: DriftToken,
    estimated_rows: Option<u64>,
) -> SchemaBaseline {
    let mut reasons = Vec::new();
    match snapshot.kind {
        StructureRelationKind::Table => {}
        StructureRelationKind::PartitionedTable => {
            reasons.push("Partitioned tables cannot be edited here".to_owned())
        }
        StructureRelationKind::View => reasons.push("Views cannot be edited here".to_owned()),
        StructureRelationKind::MaterializedView => {
            reasons.push("Materialized views cannot be edited here".to_owned())
        }
        StructureRelationKind::ForeignTable => {
            reasons.push("Foreign tables cannot be edited here".to_owned())
        }
    }
    if snapshot.is_partition {
        reasons.push("This table is a partition of another table".to_owned());
    } else if !snapshot.parents.is_empty() {
        reasons.push("This table inherits columns from another table".to_owned());
    }
    if !snapshot.partitions.is_empty() && snapshot.kind == StructureRelationKind::Table {
        reasons.push("Other tables inherit from this table".to_owned());
    }

    let columns = columns(snapshot, &mut reasons);
    let mut table = TableModel {
        schema: snapshot.schema.clone(),
        name: snapshot.table.clone(),
        comment: snapshot.comment.clone(),
        columns,
        primary_key: None,
        uniques: Vec::new(),
        foreign_keys: Vec::new(),
        checks: Vec::new(),
        indexes: Vec::new(),
        options: TableOptions::default(),
    };

    if let Some(key) = &snapshot.primary_key {
        if key.deferrable {
            reasons.push(format!("The primary key {} is deferrable", key.name));
        }
        let members = key
            .columns
            .iter()
            .filter_map(|column| column_origin(column.number))
            .collect();
        check_key_definition(snapshot, key.oid, &key.name, "PRIMARY KEY", &mut reasons);
        table.primary_key = Some(KeyModel {
            origin: object_origin(TAG_PRIMARY_KEY, key.oid),
            name: Some(key.name.clone()),
            columns: members,
        });
    }

    for constraint in &snapshot.constraints {
        match constraint.kind.as_str() {
            "unique" => table
                .uniques
                .push(unique(snapshot, constraint, &mut reasons)),
            "check" => table.checks.push(check(constraint, &mut reasons)),
            "exclusion" => reasons.push(format!(
                "Exclusion constraint {} cannot be edited here",
                constraint.name
            )),
            // Represented elsewhere: the key, the foreign keys and column
            // nullability (PostgreSQL 18 records NOT NULL as a constraint).
            "primary key" | "foreign key" | "not null" => {}
            other => reasons.push(format!(
                "Constraint {} ({other}) cannot be edited here",
                constraint.name
            )),
        }
    }

    for key in &snapshot.outbound {
        table
            .foreign_keys
            .push(foreign_key(snapshot, key, &mut reasons));
    }

    for index in &snapshot.indexes {
        // Indexes that back a key or exclusion constraint are that
        // constraint, not separate indexes.
        if index.constraint_oid.is_some() {
            continue;
        }
        table.indexes.push(plain_index(index, &mut reasons));
    }

    let inbound = snapshot
        .inbound
        .iter()
        .filter(|key| key.source_oid != snapshot.identity.relation_oid)
        .map(|key| InboundReference {
            schema: key.source_schema.clone(),
            table: key.source_table.clone(),
            name: key.name.clone(),
            columns: key.columns.iter().map(|pair| pair.target.clone()).collect(),
        })
        .collect();

    let dependents = snapshot
        .triggers
        .iter()
        .map(|trigger| Dependent {
            kind: "trigger".into(),
            schema: snapshot.schema.clone(),
            name: trigger.name.clone(),
            definition: Some(trigger.definition.clone()),
        })
        .chain(snapshot.rules.iter().map(|rule| Dependent {
            kind: "rule".into(),
            schema: snapshot.schema.clone(),
            name: rule.name.clone(),
            definition: Some(rule.definition.clone()),
        }))
        .chain(snapshot.policies.iter().map(|policy| Dependent {
            kind: "policy".into(),
            schema: snapshot.schema.clone(),
            name: policy.name.clone(),
            definition: None,
        }))
        .collect();

    SchemaBaseline {
        engine: SchemaEngine::Postgres,
        server: ServerFacts {
            flavor: ServerFlavor::Postgres,
            version: server_version_text(snapshot.server_version),
            features: features(),
        },
        table,
        drift,
        editable: if reasons.is_empty() {
            Editability::Editable
        } else {
            Editability::ReadOnly { reasons }
        },
        inbound,
        dependents,
        estimated_rows,
    }
}

fn columns(snapshot: &TableStructureSnapshot, reasons: &mut Vec<String>) -> Vec<ColumnModel> {
    let mut out = Vec::with_capacity(snapshot.columns.len());
    for column in &snapshot.columns {
        let Some(origin) = column_origin(column.number) else {
            reasons.push(format!("Column {} has no position", column.name));
            continue;
        };
        let identity = match column.identity {
            StructureIdentityKind::None => Identity::None,
            StructureIdentityKind::Always => Identity::Always,
            StructureIdentityKind::ByDefault => Identity::ByDefault,
        };
        let stored = match column.generated {
            StructureGeneratedKind::None => None,
            StructureGeneratedKind::Stored => Some(true),
            StructureGeneratedKind::Virtual => Some(false),
        };
        // A generated column keeps its expression in pg_attrdef, so the
        // catalog reports it as the default.
        let (default, generated) = match stored {
            None => (
                column
                    .default_expression
                    .as_ref()
                    .map_or(DefaultValue::None, |text| {
                        DefaultValue::Expression(SqlFragment::new(text.clone()))
                    }),
                None,
            ),
            Some(stored) => match &column.default_expression {
                Some(text) => (
                    DefaultValue::None,
                    Some(Generated {
                        expression: SqlFragment::new(text.clone()),
                        stored,
                    }),
                ),
                None => {
                    reasons.push(format!(
                        "Generated column {} has no expression",
                        column.name
                    ));
                    (DefaultValue::None, None)
                }
            },
        };
        let collation = match (
            column.collation_schema.as_deref(),
            column.collation_name.as_deref(),
        ) {
            (Some("pg_catalog"), Some("default")) | (_, None) => None,
            (Some("pg_catalog"), Some(name)) => Some(name.to_owned()),
            (schema, Some(name)) => {
                reasons.push(format!(
                    "Column {} uses collation {}.{name}, which is outside pg_catalog",
                    column.name,
                    schema.unwrap_or("?")
                ));
                Some(name.to_owned())
            }
        };
        out.push(ColumnModel {
            origin,
            name: column.name.clone(),
            data_type: column.data_type.clone(),
            nullable: column.nullable,
            default,
            generated,
            identity,
            collation,
            comment: column.comment.clone(),
            extra: ColumnExtra::default(),
        });
    }
    out
}

fn constraint_by_oid(snapshot: &TableStructureSnapshot, oid: u32) -> Option<&StructureConstraint> {
    snapshot.constraints.iter().find(|c| c.oid == oid)
}

fn backing_index(snapshot: &TableStructureSnapshot, oid: u32) -> Option<&StructureIndex> {
    snapshot
        .indexes
        .iter()
        .find(|index| index.constraint_oid == Some(oid))
}

fn key_columns(index: &StructureIndex) -> Vec<&StructureIndexKey> {
    let mut keys: Vec<&StructureIndexKey> = index.keys.iter().filter(|k| !k.included).collect();
    keys.sort_by_key(|key| key.position);
    keys
}

/// A key constraint is representable only when the server's own definition
/// is exactly `PRIMARY KEY (…)` / `UNIQUE (…)`: no INCLUDE, NULLS NOT
/// DISTINCT, storage parameters, tablespace or deferral.
fn check_key_definition(
    snapshot: &TableStructureSnapshot,
    oid: u32,
    name: &str,
    keyword: &str,
    reasons: &mut Vec<String>,
) {
    let (Some(constraint), Some(index)) = (
        constraint_by_oid(snapshot, oid),
        backing_index(snapshot, oid),
    ) else {
        reasons.push(format!("The index behind key {name} was not found"));
        return;
    };
    let expected = format!(
        "{keyword} ({})",
        key_columns(index)
            .iter()
            .map(|key| key.definition.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let has_include = index.keys.iter().any(|key| key.included);
    if constraint.definition != expected || has_include {
        reasons.push(format!(
            "Key {name} has options the editor cannot represent"
        ));
    }
}

fn unique(
    snapshot: &TableStructureSnapshot,
    constraint: &StructureConstraint,
    reasons: &mut Vec<String>,
) -> KeyModel {
    if constraint.deferrable {
        reasons.push(format!(
            "Unique constraint {} is deferrable",
            constraint.name
        ));
    }
    check_key_definition(snapshot, constraint.oid, &constraint.name, "UNIQUE", reasons);
    let mut columns = Vec::new();
    if let Some(index) = backing_index(snapshot, constraint.oid) {
        for key in key_columns(index) {
            match key.column_number.and_then(column_origin) {
                Some(origin) => columns.push(origin),
                None => reasons.push(format!(
                    "Unique constraint {} uses an expression",
                    constraint.name
                )),
            }
        }
    }
    KeyModel {
        origin: object_origin(TAG_UNIQUE, constraint.oid),
        name: Some(constraint.name.clone()),
        columns,
    }
}

/// `CHECK (expr)` exactly, with the outer parentheses matching each other.
pub(crate) fn check_expression(definition: &str) -> Option<&str> {
    let open = "CHECK ".len();
    if !definition.starts_with("CHECK (") {
        return None;
    }
    let close = matching_paren(definition, open)?;
    (close + 1 == definition.len()).then(|| &definition[open + 1..close])
}

fn check(constraint: &StructureConstraint, reasons: &mut Vec<String>) -> CheckModel {
    if !constraint.validated {
        reasons.push(format!(
            "Check {} is NOT VALID; validate or drop it first",
            constraint.name
        ));
    }
    let expression = match check_expression(&constraint.definition) {
        Some(expression) => expression.to_owned(),
        None => {
            reasons.push(format!(
                "Check {} has options the editor cannot represent",
                constraint.name
            ));
            constraint.definition.clone()
        }
    };
    CheckModel {
        origin: object_origin(TAG_CHECK, constraint.oid),
        name: Some(constraint.name.clone()),
        expression: SqlFragment::new(expression),
        enforced: true,
    }
}

fn action(value: StructureReferentialAction) -> ReferentialAction {
    match value {
        StructureReferentialAction::NoAction => ReferentialAction::NoAction,
        StructureReferentialAction::Restrict => ReferentialAction::Restrict,
        StructureReferentialAction::Cascade => ReferentialAction::Cascade,
        StructureReferentialAction::SetNull => ReferentialAction::SetNull,
        StructureReferentialAction::SetDefault => ReferentialAction::SetDefault,
    }
}

fn foreign_key(
    snapshot: &TableStructureSnapshot,
    key: &StructureForeignKey,
    reasons: &mut Vec<String>,
) -> ForeignKeyModel {
    if !key.validated {
        reasons.push(format!(
            "Foreign key {} is NOT VALID; validate or drop it first",
            key.name
        ));
    }
    if key.match_type != "SIMPLE" {
        reasons.push(format!(
            "Foreign key {} uses MATCH {}",
            key.name, key.match_type
        ));
    }
    if let Some(definition) = constraint_by_oid(snapshot, key.oid).map(|c| c.definition.as_str())
    {
        // PostgreSQL 15 column lists for SET NULL / SET DEFAULT, and 18 NOT
        // ENFORCED, have no place in the model.
        if definition.contains("SET NULL (")
            || definition.contains("SET DEFAULT (")
            || definition.ends_with("NOT ENFORCED")
        {
            reasons.push(format!(
                "Foreign key {} has options the editor cannot represent",
                key.name
            ));
        }
    }
    let deferrable = match (key.deferrable, key.initially_deferred) {
        (false, _) => Deferrable::Never,
        (true, false) => Deferrable::InitiallyImmediate,
        (true, true) => Deferrable::InitiallyDeferred,
    };
    let mut columns = Vec::with_capacity(key.columns.len());
    for pair in &key.columns {
        match column_origin(pair.source_number) {
            Some(origin) => columns.push(origin),
            None => reasons.push(format!("Foreign key {} has an unknown column", key.name)),
        }
    }
    ForeignKeyModel {
        origin: object_origin(TAG_FOREIGN_KEY, key.oid),
        name: Some(key.name.clone()),
        columns,
        referenced_schema: (key.target_schema != snapshot.schema).then(|| key.target_schema.clone()),
        referenced_table: key.target_table.clone(),
        referenced_columns: key.columns.iter().map(|pair| pair.target.clone()).collect(),
        on_update: action(key.on_update),
        on_delete: action(key.on_delete),
        deferrable,
    }
}

fn plain_index(index: &StructureIndex, reasons: &mut Vec<String>) -> IndexModel {
    if !index.valid || !index.ready {
        reasons.push(format!(
            "Index {} is invalid (an interrupted concurrent build); drop or rebuild it first",
            index.name
        ));
    }
    let (members, include) = match parse_index_definition(index) {
        Some(parsed) => parsed,
        None => {
            reasons.push(format!(
                "Index {} uses options the editor cannot represent",
                index.name
            ));
            fallback_members(index)
        }
    };
    IndexModel {
        origin: object_origin(TAG_INDEX, index.oid),
        name: index.name.clone(),
        unique: index.unique,
        method: (index.method != "btree").then(|| index.method.clone()),
        members,
        include,
        predicate: index.predicate.clone().map(SqlFragment::new),
    }
}

fn key_target(key: &StructureIndexKey) -> IndexTarget {
    match key.column_number.and_then(column_origin) {
        Some(origin) => IndexTarget::Column(origin),
        None => IndexTarget::Expression(SqlFragment::new(strip_outer_parens(&key.definition))),
    }
}

fn fallback_members(index: &StructureIndex) -> (Vec<IndexMember>, Vec<Origin>) {
    let members = key_columns(index)
        .into_iter()
        .map(|key| IndexMember {
            target: key_target(key),
            descending: false,
            prefix_length: None,
            collation: None,
        })
        .collect();
    let include = index
        .keys
        .iter()
        .filter(|key| key.included)
        .filter_map(|key| key.column_number.and_then(column_origin))
        .collect();
    (members, include)
}

/// Reads members, sort order and INCLUDE columns from the server's full
/// `pg_get_indexdef`, cross-checked against the per-key definitions. Any
/// other per-key option (operator class, collation, NULLS order) or index
/// option (NULLS NOT DISTINCT, storage parameters) refuses.
pub(crate) fn parse_index_definition(
    index: &StructureIndex,
) -> Option<(Vec<IndexMember>, Vec<Origin>)> {
    let definition = index.definition.as_str();
    let marker = format!(" USING {} (", index.method);
    let open = definition.find(&marker)? + marker.len() - 1;
    let close = matching_paren(definition, open)?;
    let elements = split_top_level(&definition[open + 1..close])?;
    let keys = key_columns(index);
    if elements.len() != keys.len() {
        return None;
    }
    let mut members = Vec::with_capacity(keys.len());
    for (element, key) in elements.iter().zip(keys) {
        let (body, descending) = match element.strip_suffix(" DESC") {
            Some(body) => (body, true),
            None => (*element, false),
        };
        let target = key_target(key);
        let matches = match &target {
            IndexTarget::Column(_) => body == key.definition,
            IndexTarget::Expression(_) => single_expression(body),
        };
        if !matches {
            return None;
        }
        members.push(IndexMember {
            target,
            descending,
            prefix_length: None,
            collation: None,
        });
    }

    let mut rest = &definition[close + 1..];
    let included: Vec<&StructureIndexKey> = {
        let mut keys: Vec<&StructureIndexKey> =
            index.keys.iter().filter(|key| key.included).collect();
        keys.sort_by_key(|key| key.position);
        keys
    };
    let mut include = Vec::with_capacity(included.len());
    if let Some(after) = rest.strip_prefix(" INCLUDE (") {
        let start = definition.len() - after.len() - 1;
        let end = matching_paren(definition, start)?;
        let elements = split_top_level(&definition[start + 1..end])?;
        if elements.len() != included.len() {
            return None;
        }
        for (element, key) in elements.iter().zip(&included) {
            let origin = key.column_number.and_then(column_origin)?;
            if *element != key.definition {
                return None;
            }
            include.push(origin);
        }
        rest = &definition[end + 1..];
    } else if !included.is_empty() {
        return None;
    }
    let tail_ok = rest.is_empty() || (index.predicate.is_some() && rest.starts_with(" WHERE "));
    tail_ok.then_some((members, include))
}

/// Byte index of the `)` matching the `(` at `open`, skipping quoted text.
pub(crate) fn matching_paren(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return None,
                        Some(b) if *b == quote => {
                            if bytes.get(i + 1) == Some(&quote) {
                                i += 2;
                                continue;
                            }
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Top-level comma-separated elements, trimmed. `None` when unbalanced.
fn split_top_level(text: &str) -> Option<Vec<&str>> {
    let bytes = text.as_bytes();
    let mut elements = Vec::new();
    let mut depth = 0i64;
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return None,
                        Some(b) if *b == quote => {
                            if bytes.get(i + 1) == Some(&quote) {
                                i += 2;
                                continue;
                            }
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
            }
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b',' if depth == 0 => {
                elements.push(text[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return None;
    }
    elements.push(text[start..].trim());
    Some(elements)
}

/// One expression element with nothing after it: `(expr)` or
/// `name(args)` / `schema.name(args)`, where the final `)` closes the
/// first `(`. Rejects a trailing operator class, collation or NULLS order.
fn single_expression(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && bytes[i] != b'(' {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'"' {
                        if bytes.get(i + 1) == Some(&b'"') {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                if i >= bytes.len() {
                    return false;
                }
                i += 1;
            }
            b if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b == b'.' || b >= 0x80 => {
                i += 1
            }
            _ => return false,
        }
    }
    i < bytes.len() && matching_paren(text, i) == Some(bytes.len() - 1)
}

/// `(a + b)` as `a + b`; a function call or bare name is unchanged.
pub(crate) fn strip_outer_parens(text: &str) -> String {
    if text.starts_with('(') && matching_paren(text, 0) == Some(text.len() - 1) {
        text[1..text.len() - 1].to_owned()
    } else {
        text.to_owned()
    }
}
