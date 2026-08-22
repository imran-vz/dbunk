use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use futures_util::future::BoxFuture;
use tokio_postgres::types::ToSql;
use tokio_postgres::Client;

use crate::postgres::dedicated::{self, DedicatedConnection, DedicatedError, NoticeSink};
use crate::postgres::identity::{resolve_identity, RelationIdentityKind, UniqueIndexCandidate};

use super::builder::{MutationColumnDescriptor, MutationTableDescriptor};
use super::protocol::*;
use super::VirtualKeyLookup;

const LOCK_TIMEOUT_SQL: &str = "SET LOCAL lock_timeout = '10s'";

pub(crate) struct MutationConnection {
    pub(crate) inner: DedicatedConnection,
}

pub(crate) async fn connect(
    spec: &crate::postgres::connect_spec::ResolvedPostgresConnectSpec,
) -> Result<MutationConnection, ResultMutationError> {
    let inner = dedicated::connect(spec, NoticeSink::Ignore)
        .await
        .map_err(map_dedicated)?;
    Ok(MutationConnection { inner })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CatalogDescriptor {
    oid: u32,
    identity_object_oid: Option<u32>,
    columns: Vec<CatalogColumn>,
    pub(crate) mutation: MutationTableDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogColumn {
    attnum: i16,
    name: String,
    type_oid: u32,
    type_modifier: i32,
    generated: String,
    identity: String,
    has_default: bool,
    default_expression: Option<String>,
}

pub(crate) type DescriptorCache = HashMap<(String, String), CatalogDescriptor>;

#[derive(Debug, Clone)]
pub(crate) struct AnalysisData {
    pub(crate) columns: Vec<AnalyzedColumn>,
    pub(crate) tables: Vec<AnalyzedTable>,
    pub(crate) descriptors: Vec<CatalogDescriptor>,
}

pub(crate) async fn analyze(
    client: &Client,
    connection_id: &str,
    source: &AnalyzeSource,
    refresh_structure: bool,
    cache: &mut DescriptorCache,
    virtual_keys: &VirtualKeyLookup,
) -> Result<AnalysisData, ResultMutationError> {
    if refresh_structure {
        cache.clear();
    }
    let first = analyze_once(client, connection_id, source, cache, virtual_keys).await;
    if first.as_ref().is_err_and(is_undefined_object) {
        cache.clear();
        analyze_once(client, connection_id, source, cache, virtual_keys).await
    } else {
        first
    }
}

async fn analyze_once(
    client: &Client,
    connection_id: &str,
    source: &AnalyzeSource,
    cache: &mut DescriptorCache,
    virtual_keys: &VirtualKeyLookup,
) -> Result<AnalysisData, ResultMutationError> {
    let (projected, statement_relations) = match source {
        AnalyzeSource::Statement { sql } => {
            let description = describe_statement(client, sql).await?;
            (description.columns, Some(description.relations))
        }
        AnalyzeSource::Relation { schema, table } => {
            let descriptor = descriptor_by_name(client, schema, table, cache).await?;
            (
                descriptor
                    .columns
                    .iter()
                    .map(|column| ProjectedColumn {
                        name: column.name.clone(),
                        table_oid: Some(descriptor.oid),
                        attnum: Some(column.attnum),
                        cast_type: descriptor
                            .mutation
                            .column(&column.name)
                            .map(|value| value.cast_type.clone())
                            .unwrap_or_else(|| "text".into()),
                    })
                    .collect(),
                None,
            )
        }
    };
    if projected.is_empty() {
        return Err(not_analyzable(NotAnalyzableReason::NoProjectedColumns));
    }
    let origin_oids = projected
        .iter()
        .filter_map(|column| column.table_oid)
        .collect::<HashSet<_>>();
    if origin_oids.is_empty() {
        return Err(not_analyzable(NotAnalyzableReason::NoTableOrigins));
    }

    let mut descriptors_by_oid = HashMap::new();
    for oid in origin_oids {
        let descriptor = descriptor_by_oid(client, oid, cache).await?;
        descriptors_by_oid.insert(oid, descriptor);
    }

    if let Some(relations) = statement_relations {
        ensure_unambiguous_range_variables(&projected, &relations)?;
    }

    if has_unsafe_inheritance_children(client, descriptors_by_oid.keys().copied()).await? {
        return Err(unsupported_statement());
    }

    if matches!(source, AnalyzeSource::Statement { .. })
        && possible_temp_shadow(client, descriptors_by_oid.values()).await?
    {
        return Err(not_analyzable(NotAnalyzableReason::PossibleTempShadowing));
    }

    let mut analyzed_columns = Vec::with_capacity(projected.len());
    for column in &projected {
        let Some(oid) = column.table_oid else {
            analyzed_columns.push(expression_column(column));
            continue;
        };
        let Some(descriptor) = descriptors_by_oid.get(&oid) else {
            analyzed_columns.push(expression_column(column));
            continue;
        };
        let Some(attnum) = column.attnum else {
            analyzed_columns.push(expression_column(column));
            continue;
        };
        if attnum <= 0 {
            let system_name = system_column_name(attnum).unwrap_or(&column.name);
            analyzed_columns.push(AnalyzedColumn {
                name: column.name.clone(),
                origin: ColumnOrigin::Table {
                    schema: descriptor.mutation.schema.clone(),
                    table: descriptor.mutation.table.clone(),
                    column: system_name.into(),
                    attnum,
                },
                cast_type: column.cast_type.clone(),
                nullable: true,
                writability: ColumnWritability::SystemColumn,
            });
            continue;
        }
        let Some(catalog_column) = descriptor
            .columns
            .iter()
            .find(|candidate| candidate.attnum == attnum)
        else {
            return Err(undefined_column());
        };
        let mutation_column = descriptor
            .mutation
            .column(&catalog_column.name)
            .ok_or_else(undefined_column)?;
        analyzed_columns.push(AnalyzedColumn {
            name: column.name.clone(),
            origin: ColumnOrigin::Table {
                schema: descriptor.mutation.schema.clone(),
                table: descriptor.mutation.table.clone(),
                column: catalog_column.name.clone(),
                attnum,
            },
            cast_type: mutation_column.cast_type.clone(),
            nullable: mutation_column.nullable,
            writability: mutation_column.writability,
        });
    }

    let table_count = descriptors_by_oid.len();
    let relation_source = matches!(source, AnalyzeSource::Relation { .. });
    let mut ordered = descriptors_by_oid.into_values().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        (&left.mutation.schema, &left.mutation.table)
            .cmp(&(&right.mutation.schema, &right.mutation.table))
    });
    let mut tables = Vec::with_capacity(ordered.len());
    let mut snapshot_descriptors = Vec::with_capacity(ordered.len());
    for descriptor in ordered {
        let (table, builder) = classify_table(
            connection_id,
            descriptor,
            &analyzed_columns,
            table_count,
            relation_source,
            virtual_keys,
        )
        .await?;
        tables.push(table);
        snapshot_descriptors.push(builder);
    }

    Ok(AnalysisData {
        columns: analyzed_columns,
        tables,
        descriptors: snapshot_descriptors,
    })
}

#[derive(Debug)]
struct ProjectedColumn {
    name: String,
    table_oid: Option<u32>,
    attnum: Option<i16>,
    cast_type: String,
}

#[derive(Debug)]
struct StatementDescription {
    columns: Vec<ProjectedColumn>,
    relations: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SqlIdentifier {
    value: String,
    quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SqlToken {
    Identifier(SqlIdentifier),
    Symbol(char),
    Opaque,
}

#[derive(Debug, PartialEq, Eq)]
struct RangeVariable {
    parts: Vec<SqlIdentifier>,
}

impl RangeVariable {
    fn regclass_name(&self) -> String {
        self.parts
            .iter()
            .map(|part| {
                let value = if part.quoted {
                    part.value.clone()
                } else {
                    part.value.to_ascii_lowercase()
                };
                quote_identifier(&value)
            })
            .collect::<Vec<_>>()
            .join(".")
    }
}

async fn describe_statement(
    client: &Client,
    sql: &str,
) -> Result<StatementDescription, ResultMutationError> {
    let statement = match client.prepare(sql).await {
        Ok(statement) => statement,
        Err(error) => {
            let mapped = database_error(error);
            let reason = match mapped {
                ResultMutationError::Database {
                    code: Some(ref code),
                    ref message,
                    ..
                } if code == "42601"
                    && message == "cannot insert multiple commands into a prepared statement" =>
                {
                    NotAnalyzableReason::MultiStatement
                }
                ResultMutationError::Database {
                    code,
                    message,
                    severity,
                    position,
                    ..
                } => NotAnalyzableReason::Database {
                    code,
                    message,
                    severity,
                    position,
                },
                ResultMutationError::ConnectionLost => {
                    return Err(ResultMutationError::ConnectionLost)
                }
                other => return Err(other),
            };
            return Err(not_analyzable(reason));
        }
    };
    let type_oids = statement
        .columns()
        .iter()
        .map(|column| column.type_().oid())
        .collect::<Vec<_>>();
    let type_modifiers = statement
        .columns()
        .iter()
        .map(|column| column.type_modifier())
        .collect::<Vec<_>>();
    let formatted = client
        .query(
            r#"
            SELECT pg_catalog.format_type(types.type_oid, types.type_modifier)
            FROM unnest($1::oid[], $2::int4[])
                 AS types(type_oid, type_modifier)
            "#,
            &[&type_oids, &type_modifiers],
        )
        .await
        .map_err(database_error)?;
    if formatted.len() != statement.columns().len() {
        return Err(ResultMutationError::ConnectionLost);
    }
    let columns: Vec<ProjectedColumn> = statement
        .columns()
        .iter()
        .zip(formatted)
        .map(|(column, formatted)| ProjectedColumn {
            name: column.name().into(),
            table_oid: column.table_oid(),
            attnum: column.column_id(),
            cast_type: formatted.get(0),
        })
        .collect();
    let range_variables = parse_range_variables(sql).map_err(|()| unsupported_statement())?;
    let relations = resolve_range_variables(client, &range_variables).await?;
    Ok(StatementDescription { columns, relations })
}

/// RowDescription intentionally omits range-variable identity. The local parser
/// accepts only top-level SELECT range tables it can enumerate completely, then
/// the catalog resolves those names with the analysis session's search_path.
fn ensure_unambiguous_range_variables(
    columns: &[ProjectedColumn],
    relations: &[u32],
) -> Result<(), ResultMutationError> {
    for oid in columns.iter().filter_map(|column| column.table_oid) {
        if relations
            .iter()
            .filter(|relation| **relation == oid)
            .count()
            != 1
        {
            return Err(unsupported_statement());
        }
    }
    Ok(())
}

async fn resolve_range_variables(
    client: &Client,
    range_variables: &[RangeVariable],
) -> Result<Vec<u32>, ResultMutationError> {
    if range_variables.is_empty() {
        return Ok(Vec::new());
    }
    let names = range_variables
        .iter()
        .map(RangeVariable::regclass_name)
        .collect::<Vec<_>>();
    let rows = client
        .query(
            r#"
            SELECT pg_catalog.to_regclass(names.name)::oid
            FROM unnest($1::text[]) WITH ORDINALITY AS names(name, ord)
            ORDER BY names.ord
            "#,
            &[&names],
        )
        .await
        .map_err(analysis_database_error)?;
    if rows.len() != names.len() {
        return Err(unsupported_statement());
    }
    rows.into_iter()
        .map(|row| {
            row.get::<_, Option<u32>>(0)
                .ok_or_else(unsupported_statement)
        })
        .collect()
}

fn parse_range_variables(sql: &str) -> Result<Vec<RangeVariable>, ()> {
    let mut tokens = lex_sql(sql)?;
    if matches!(tokens.last(), Some(SqlToken::Symbol(';'))) {
        tokens.pop();
    }
    if tokens.is_empty()
        || tokens
            .iter()
            .any(|token| matches!(token, SqlToken::Symbol(';')))
        || !is_keyword(&tokens[0], "select")
    {
        return Err(());
    }

    let mut depth = 0usize;
    let mut from_index = None;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            SqlToken::Symbol('(') => depth += 1,
            SqlToken::Symbol(')') => depth = depth.checked_sub(1).ok_or(())?,
            _ if is_keyword(token, "select") && index != 0 => return Err(()),
            _ if depth == 0
                && ["union", "intersect", "except", "into"]
                    .iter()
                    .any(|keyword| is_keyword(token, keyword)) =>
            {
                return Err(());
            }
            _ if depth == 0 && is_keyword(token, "from") && from_index.is_some() => return Err(()),
            _ if depth == 0 && is_keyword(token, "from") => from_index = Some(index),
            _ => {}
        }
    }
    if depth != 0 {
        return Err(());
    }
    let Some(from_index) = from_index else {
        return Ok(Vec::new());
    };

    let mut variables = Vec::new();
    let (first, mut index) = parse_range_variable(&tokens, from_index + 1)?;
    variables.push(first);
    let mut depth = 0usize;
    while index < tokens.len() {
        let token = &tokens[index];
        match token {
            SqlToken::Symbol('(') => depth += 1,
            SqlToken::Symbol(')') => depth = depth.checked_sub(1).ok_or(())?,
            SqlToken::Symbol(',') if depth == 0 => {
                return Err(());
            }
            _ if depth == 0 && is_from_clause_end(token) => break,
            _ if depth == 0 && is_keyword(token, "tablesample") => return Err(()),
            _ if depth == 0 && is_keyword(token, "join") => {
                let (variable, next) = parse_range_variable(&tokens, index + 1)?;
                variables.push(variable);
                index = next;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    Ok(variables)
}

fn parse_range_variable(tokens: &[SqlToken], start: usize) -> Result<(RangeVariable, usize), ()> {
    let first = tokens.get(start).and_then(identifier).ok_or(())?;
    if !first.quoted
        && ["only", "lateral", "table"]
            .iter()
            .any(|keyword| first.value.eq_ignore_ascii_case(keyword))
    {
        return Err(());
    }
    let mut parts = vec![first.clone()];
    let mut index = start + 1;
    while matches!(tokens.get(index), Some(SqlToken::Symbol('.'))) {
        if parts.len() == 3 {
            return Err(());
        }
        parts.push(
            tokens
                .get(index + 1)
                .and_then(identifier)
                .ok_or(())?
                .clone(),
        );
        index += 2;
    }
    if matches!(tokens.get(index), Some(SqlToken::Symbol('(' | '*'))) {
        return Err(());
    }
    Ok((RangeVariable { parts }, index))
}

fn identifier(token: &SqlToken) -> Option<&SqlIdentifier> {
    match token {
        SqlToken::Identifier(identifier) => Some(identifier),
        _ => None,
    }
}

fn is_keyword(token: &SqlToken, keyword: &str) -> bool {
    matches!(
        token,
        SqlToken::Identifier(SqlIdentifier { value, quoted: false })
            if value.eq_ignore_ascii_case(keyword)
    )
}

fn is_from_clause_end(token: &SqlToken) -> bool {
    [
        "where",
        "group",
        "having",
        "window",
        "order",
        "limit",
        "offset",
        "fetch",
        "for",
        "union",
        "intersect",
        "except",
    ]
    .iter()
    .any(|keyword| is_keyword(token, keyword))
}

fn lex_sql(sql: &str) -> Result<Vec<SqlToken>, ()> {
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte.is_ascii_whitespace() => index += 1,
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = lex_block_comment(bytes, index + 2)?;
            }
            b'\'' => {
                index = lex_single_quote(bytes, index + 1, false)?;
                tokens.push(SqlToken::Opaque);
            }
            b'"' => {
                let (value, next) = lex_quoted_identifier(sql, index + 1)?;
                tokens.push(SqlToken::Identifier(SqlIdentifier {
                    value,
                    quoted: true,
                }));
                index = next;
            }
            b'$' => {
                index = lex_dollar(bytes, index)?;
                tokens.push(SqlToken::Opaque);
            }
            byte if is_identifier_start(byte) => {
                let start = index;
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                let value = &sql[start..index];
                if value.eq_ignore_ascii_case("e") && bytes.get(index) == Some(&b'\'') {
                    index = lex_single_quote(bytes, index + 1, true)?;
                    tokens.push(SqlToken::Opaque);
                } else {
                    tokens.push(SqlToken::Identifier(SqlIdentifier {
                        value: value.into(),
                        quoted: false,
                    }));
                }
            }
            byte @ (b'(' | b')' | b',' | b'.' | b';' | b'*') => {
                tokens.push(SqlToken::Symbol(char::from(byte)));
                index += 1;
            }
            byte if byte.is_ascii() => {
                tokens.push(SqlToken::Opaque);
                index += 1;
            }
            _ => return Err(()),
        }
    }
    Ok(tokens)
}

fn lex_block_comment(bytes: &[u8], mut index: usize) -> Result<usize, ()> {
    let mut depth = 1usize;
    while index < bytes.len() {
        if bytes.get(index..index + 2) == Some(b"/*") {
            depth += 1;
            index += 2;
        } else if bytes.get(index..index + 2) == Some(b"*/") {
            depth -= 1;
            index += 2;
            if depth == 0 {
                return Ok(index);
            }
        } else {
            index += 1;
        }
    }
    Err(())
}

fn lex_single_quote(bytes: &[u8], mut index: usize, escapes: bool) -> Result<usize, ()> {
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => return Ok(index + 1),
            b'\\' if escapes && index + 1 < bytes.len() => index += 2,
            // Plain-string backslash semantics depend on standard_conforming_strings.
            b'\\' => return Err(()),
            _ => index += 1,
        }
    }
    Err(())
}

fn lex_quoted_identifier(sql: &str, mut index: usize) -> Result<(String, usize), ()> {
    let bytes = sql.as_bytes();
    let mut value = String::new();
    let mut segment = index;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        value.push_str(&sql[segment..index]);
        if bytes.get(index + 1) == Some(&b'"') {
            value.push('"');
            index += 2;
            segment = index;
        } else {
            return Ok((value, index + 1));
        }
    }
    Err(())
}

fn lex_dollar(bytes: &[u8], index: usize) -> Result<usize, ()> {
    if bytes.get(index + 1).is_some_and(u8::is_ascii_digit) {
        let mut end = index + 2;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        return Ok(end);
    }
    let mut tag_end = index + 1;
    if bytes.get(tag_end) != Some(&b'$') {
        if !bytes
            .get(tag_end)
            .is_some_and(|byte| is_identifier_start(*byte))
        {
            return Err(());
        }
        tag_end += 1;
        while bytes
            .get(tag_end)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            tag_end += 1;
        }
    }
    if bytes.get(tag_end) != Some(&b'$') {
        return Err(());
    }
    let delimiter = &bytes[index..=tag_end];
    let body_start = tag_end + 1;
    bytes[body_start..]
        .windows(delimiter.len())
        .position(|window| window == delimiter)
        .map(|offset| body_start + offset + delimiter.len())
        .ok_or(())
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit() || byte == b'$'
}

fn expression_column(column: &ProjectedColumn) -> AnalyzedColumn {
    AnalyzedColumn {
        name: column.name.clone(),
        origin: ColumnOrigin::Expression,
        cast_type: column.cast_type.clone(),
        nullable: true,
        // Expressions are never accepted by the builder because they have no
        // table column. The wire enum has no separate expression verdict.
        writability: ColumnWritability::Writable,
    }
}

async fn classify_table(
    connection_id: &str,
    descriptor: CatalogDescriptor,
    analyzed_columns: &[AnalyzedColumn],
    table_count: usize,
    relation_source: bool,
    virtual_keys: &VirtualKeyLookup,
) -> Result<(AnalyzedTable, CatalogDescriptor), ResultMutationError> {
    let schema = descriptor.mutation.schema.clone();
    let table = descriptor.mutation.table.clone();
    let mut builder = descriptor.mutation.clone();
    let mut projection_indexes: HashMap<String, usize> = HashMap::new();
    for (index, column) in analyzed_columns.iter().enumerate() {
        if let ColumnOrigin::Table {
            schema: origin_schema,
            table: origin_table,
            column: origin_column,
            ..
        } = &column.origin
        {
            if origin_schema == &schema && origin_table == &table {
                projection_indexes
                    .entry(origin_column.clone())
                    .or_insert(index);
            }
        }
    }
    for column in &mut builder.columns {
        column.projected = projection_indexes.contains_key(&column.name);
    }
    add_projected_system_columns(&mut builder, analyzed_columns);

    // Table Browse returns ctid/tableoid through its separate rowIdentity
    // channel. A relation-source analysis therefore treats these identity
    // columns as available even though they have no visible projection index.
    if relation_source && builder.identity.kind == MutationIdentityKind::CtidFallback {
        for name in builder.identity.columns.clone() {
            if builder.column(&name).is_none() {
                builder.columns.push(MutationColumnDescriptor {
                    cast_type: if name == "ctid" { "tid" } else { "oid" }.into(),
                    name,
                    nullable: false,
                    writability: ColumnWritability::SystemColumn,
                    has_default: false,
                    has_identity: false,
                    projected: true,
                });
            }
        }
    }

    let mut identity_projected = (relation_source
        && builder.identity.kind == MutationIdentityKind::CtidFallback)
        || identity_is_projected(&builder.identity, &projection_indexes);
    let needs_virtual_key =
        builder.identity.kind == MutationIdentityKind::None || !identity_projected;
    let mut invalid_virtual_key = false;
    if needs_virtual_key {
        if let Some(key) = virtual_keys(connection_id.into(), schema.clone(), table.clone()).await?
        {
            let valid = key.version == 1
                && !key.columns.is_empty()
                && key.columns.iter().all(|name| {
                    builder.column(name).is_some() && projection_indexes.contains_key(name)
                });
            if valid {
                builder.identity = MutationIdentity {
                    kind: MutationIdentityKind::VirtualKey,
                    columns: key.columns,
                };
                identity_projected = true;
            } else {
                invalid_virtual_key = true;
            }
        }
    }
    let identity_projection_indexes = builder
        .identity
        .columns
        .iter()
        .filter_map(|column| projection_indexes.get(column).copied())
        .collect::<Vec<_>>();
    let writable_columns = builder
        .columns
        .iter()
        .any(|column| column.projected && column.writability == ColumnWritability::Writable);
    let identity_reason = if invalid_virtual_key {
        Some(CapabilityReason::InvalidVirtualKey)
    } else if builder.identity.kind == MutationIdentityKind::None {
        Some(CapabilityReason::NoIdentity)
    } else if !identity_projected {
        Some(CapabilityReason::IdentityNotProjected)
    } else {
        None
    };
    let updatable_reason = identity_reason
        .or_else(|| (!writable_columns).then_some(CapabilityReason::NoWritableColumns));
    let deletable_reason = identity_reason
        .or_else(|| (table_count != 1).then_some(CapabilityReason::MultipleOriginTables));
    let insertable_reason = if table_count != 1 {
        Some(CapabilityReason::MultipleOriginTables)
    } else if !writable_columns {
        Some(CapabilityReason::NoWritableColumns)
    } else {
        None
    };

    let table_analysis = AnalyzedTable {
        schema,
        table,
        identity: builder.identity.clone(),
        identity_projected,
        identity_projection_indexes,
        updatable: verdict(updatable_reason),
        deletable: verdict(deletable_reason),
        insertable: verdict(insertable_reason),
    };
    let mut descriptor = descriptor;
    descriptor.mutation = builder;
    Ok((table_analysis, descriptor))
}

fn verdict(reason: Option<CapabilityReason>) -> CapabilityVerdict {
    CapabilityVerdict {
        allowed: reason.is_none(),
        reason,
    }
}

fn identity_is_projected(identity: &MutationIdentity, indexes: &HashMap<String, usize>) -> bool {
    identity.kind != MutationIdentityKind::None
        && !identity.columns.is_empty()
        && identity
            .columns
            .iter()
            .all(|name| indexes.contains_key(name))
}

fn add_projected_system_columns(
    descriptor: &mut MutationTableDescriptor,
    columns: &[AnalyzedColumn],
) {
    for column in columns {
        let ColumnOrigin::Table {
            schema,
            table,
            column: name,
            attnum,
        } = &column.origin
        else {
            continue;
        };
        if schema != &descriptor.schema
            || table != &descriptor.table
            || *attnum > 0
            || descriptor.column(name).is_some()
        {
            continue;
        }
        descriptor.columns.push(MutationColumnDescriptor {
            name: name.clone(),
            cast_type: column.cast_type.clone(),
            nullable: true,
            writability: ColumnWritability::SystemColumn,
            has_default: false,
            has_identity: false,
            projected: true,
        });
    }
}

pub(crate) async fn refresh_for_apply(
    client: &Client,
    snapshot: &[CatalogDescriptor],
    cache: &mut DescriptorCache,
) -> Result<Vec<MutationTableDescriptor>, ResultMutationError> {
    cache.clear();
    let mut refreshed = Vec::with_capacity(snapshot.len());
    for previous in snapshot {
        let mut current = descriptor_by_name(
            client,
            &previous.mutation.schema,
            &previous.mutation.table,
            cache,
        )
        .await?;
        let projected = previous
            .mutation
            .columns
            .iter()
            .filter(|column| column.projected)
            .map(|column| column.name.as_str())
            .collect::<HashSet<_>>();
        for column in &mut current.mutation.columns {
            column.projected = projected.contains(column.name.as_str());
        }
        for column in &previous.mutation.columns {
            if column.writability == ColumnWritability::SystemColumn && column.projected {
                current.mutation.columns.push(column.clone());
            }
        }
        if previous.mutation.identity.kind == MutationIdentityKind::VirtualKey {
            let valid = previous.mutation.identity.columns.iter().all(|name| {
                current.mutation.column(name).is_some() && projected.contains(name.as_str())
            });
            if valid {
                current.mutation.identity = previous.mutation.identity.clone();
            }
        }
        ensure_descriptor_unchanged(previous, &current)?;
        refreshed.push(current.mutation);
    }
    Ok(refreshed)
}

fn ensure_descriptor_unchanged(
    previous: &CatalogDescriptor,
    current: &CatalogDescriptor,
) -> Result<(), ResultMutationError> {
    if current == previous {
        Ok(())
    } else {
        Err(ResultMutationError::AnalysisExpired)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyCheckpoint {
    BeforeTransaction,
    BeforeOperation,
    AdmitCommit,
}

pub(crate) type ApplyCheck = Arc<
    dyn Fn(ApplyCheckpoint) -> BoxFuture<'static, Result<(), ResultMutationError>> + Send + Sync,
>;

pub(crate) type ApplyCommit = Arc<dyn Fn() -> BoxFuture<'static, CommitOutcome> + Send + Sync>;

pub(crate) enum CommitOutcome {
    Committed,
    Failed(ResultMutationError),
}

pub(crate) struct ApplyExecution {
    pub(crate) result: Result<ApplyResult, ResultMutationError>,
    pub(crate) healthy: bool,
}

pub(crate) async fn execute_apply(
    client: &Client,
    snapshot: &[CatalogDescriptor],
    plan: &MutationPlan,
    check: &ApplyCheck,
    commit: &ApplyCommit,
) -> ApplyExecution {
    let started = Instant::now();
    let snapshot_descriptors = snapshot
        .iter()
        .map(|descriptor| descriptor.mutation.clone())
        .collect::<Vec<_>>();
    if let Err(error) = super::builder::build_mutation_plan(&snapshot_descriptors, plan) {
        return ApplyExecution {
            result: Err(error),
            healthy: true,
        };
    }
    if let Err(error) = check(ApplyCheckpoint::BeforeTransaction).await {
        return ApplyExecution {
            result: Err(error),
            healthy: true,
        };
    }
    if let Err(error) = client.batch_execute("BEGIN").await {
        return ApplyExecution {
            result: Err(database_error(error)),
            healthy: !client.is_closed(),
        };
    }
    if let Err(error) = check(ApplyCheckpoint::BeforeOperation).await {
        return failed_apply(client, error).await;
    }
    if let Err(error) = client.batch_execute(LOCK_TIMEOUT_SQL).await {
        return failed_apply(client, database_error(error)).await;
    }
    let descriptors = match lock_and_refresh_for_apply(client, snapshot, plan).await {
        Ok(descriptors) => descriptors,
        Err(error) => return failed_apply(client, error).await,
    };
    let statements = match super::builder::build_mutation_plan(&descriptors, plan) {
        Ok(statements) => statements,
        Err(error) => return failed_apply(client, error).await,
    };
    let mut operations = Vec::with_capacity(statements.len());
    for statement in &statements {
        if let Err(error) = check(ApplyCheckpoint::BeforeOperation).await {
            return failed_apply(client, error).await;
        }
        let values = statement
            .params
            .iter()
            .map(|param| match param {
                DmlParam::Text { value } => value.clone(),
            })
            .collect::<Vec<Option<String>>>();
        let refs = values
            .iter()
            .map(|value| value as &(dyn ToSql + Sync))
            .collect::<Vec<_>>();
        let affected = match client.execute(&statement.sql, &refs).await {
            Ok(affected) => affected,
            Err(error) => {
                return failed_apply(client, map_apply_error(error, statement.op_index)).await;
            }
        };
        if affected == 0 {
            return failed_apply(
                client,
                ResultMutationError::Conflict {
                    op_index: statement.op_index,
                },
            )
            .await;
        }
        if affected > 1 {
            return failed_apply(
                client,
                ResultMutationError::IdentityNotUnique {
                    op_index: statement.op_index,
                },
            )
            .await;
        }
        operations.push(AppliedOperation {
            op_index: statement.op_index,
            rows_affected: affected,
        });
    }
    if let Err(error) = check(ApplyCheckpoint::AdmitCommit).await {
        return failed_apply(client, error).await;
    }
    match commit().await {
        CommitOutcome::Committed => {}
        CommitOutcome::Failed(error) => {
            // A transport failure during COMMIT has an unknown outcome. Discard
            // the socket and never represent it as a successful rollback.
            return ApplyExecution {
                result: Err(error),
                healthy: false,
            };
        }
    }
    ApplyExecution {
        result: Ok(ApplyResult {
            operations,
            runtime_ms: started.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
        }),
        healthy: true,
    }
}

async fn lock_and_refresh_for_apply(
    client: &Client,
    snapshot: &[CatalogDescriptor],
    plan: &MutationPlan,
) -> Result<Vec<MutationTableDescriptor>, ResultMutationError> {
    let targets = operation_targets(plan);
    let mut targeted_snapshots = Vec::with_capacity(targets.len());
    for (target, op_index) in &targets {
        let previous = snapshot
            .iter()
            .find(|descriptor| {
                descriptor.mutation.schema == target.schema
                    && descriptor.mutation.table == target.table
            })
            .ok_or(ResultMutationError::InvalidPlan {
                reason: InvalidPlanReason::TableMismatch,
            })?;
        let relation = client
            .query_opt(
                r#"
                SELECT n.nspname::text, c.relname::text
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE c.oid = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
                "#,
                &[&previous.oid],
            )
            .await
            .map_err(database_error)?
            .ok_or(ResultMutationError::AnalysisExpired)?;
        let schema: String = relation.get(0);
        let table: String = relation.get(1);
        if schema != previous.mutation.schema || table != previous.mutation.table {
            return Err(ResultMutationError::AnalysisExpired);
        }
        let lock = format!(
            "LOCK TABLE {}.{} IN ROW EXCLUSIVE MODE",
            quote_identifier(&schema),
            quote_identifier(&table)
        );
        client
            .batch_execute(&lock)
            .await
            .map_err(|error| map_apply_error(error, *op_index))?;
        targeted_snapshots.push(previous.clone());
    }

    if has_unsafe_inheritance_children(client, targeted_snapshots.iter().map(|value| value.oid))
        .await?
    {
        return Err(ResultMutationError::AnalysisExpired);
    }

    let mut cache = DescriptorCache::new();
    let refreshed = refresh_for_apply(client, &targeted_snapshots, &mut cache)
        .await
        .map_err(|error| {
            if is_undefined_object(&error) {
                ResultMutationError::AnalysisExpired
            } else {
                error
            }
        })?;
    let mut refreshed_by_table = targets
        .into_iter()
        .map(|(table, _)| (table.schema, table.table))
        .zip(refreshed)
        .collect::<HashMap<_, _>>();
    Ok(snapshot
        .iter()
        .map(|descriptor| {
            refreshed_by_table
                .remove(&(
                    descriptor.mutation.schema.clone(),
                    descriptor.mutation.table.clone(),
                ))
                .unwrap_or_else(|| descriptor.mutation.clone())
        })
        .collect())
}

fn operation_targets(plan: &MutationPlan) -> Vec<(MutationTable, usize)> {
    let mut seen = HashSet::new();
    plan.operations
        .iter()
        .enumerate()
        .filter_map(|(op_index, operation)| {
            let table = match operation {
                MutationOp::Update { table, .. }
                | MutationOp::Delete { table, .. }
                | MutationOp::Insert { table, .. } => table,
            };
            seen.insert((table.schema.clone(), table.table.clone()))
                .then(|| (table.clone(), op_index))
        })
        .collect()
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

async fn failed_apply(client: &Client, error: ResultMutationError) -> ApplyExecution {
    ApplyExecution {
        result: Err(error),
        healthy: rollback_and_verify(client).await,
    }
}

async fn rollback_and_verify(client: &Client) -> bool {
    client.batch_execute("ROLLBACK").await.is_ok()
        && client.query_one("SELECT 1", &[]).await.is_ok()
}

async fn descriptor_by_oid(
    client: &Client,
    oid: u32,
    cache: &mut DescriptorCache,
) -> Result<CatalogDescriptor, ResultMutationError> {
    if let Some(descriptor) = cache.values().find(|value| value.oid == oid) {
        return Ok(descriptor.clone());
    }
    let relation = client
        .query_opt(
            r#"
            SELECT n.nspname::text, c.relname::text
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.oid = $1 AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
            "#,
            &[&oid],
        )
        .await
        .map_err(database_error)?
        .ok_or_else(undefined_table)?;
    let schema: String = relation.get(0);
    let table: String = relation.get(1);
    let key = (schema.clone(), table.clone());
    if let Some(descriptor) = cache.get(&key) {
        if descriptor.oid == oid {
            return Ok(descriptor.clone());
        }
        cache.remove(&key);
    }
    let descriptor = load_descriptor(client, &schema, &table).await?;
    cache.insert(key, descriptor.clone());
    Ok(descriptor)
}

async fn descriptor_by_name(
    client: &Client,
    schema: &str,
    table: &str,
    cache: &mut DescriptorCache,
) -> Result<CatalogDescriptor, ResultMutationError> {
    let key = (schema.to_string(), table.to_string());
    if let Some(descriptor) = cache.get(&key) {
        return Ok(descriptor.clone());
    }
    let descriptor = load_descriptor(client, schema, table).await?;
    cache.insert(key, descriptor.clone());
    Ok(descriptor)
}

async fn load_descriptor(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<CatalogDescriptor, ResultMutationError> {
    let header = client
        .query_opt(
            r#"
            SELECT c.oid, c.relkind::text
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = $1 AND c.relname = $2
              AND c.relkind IN ('r', 'p', 'f', 'v', 'm')
            "#,
            &[&schema, &table],
        )
        .await
        .map_err(database_error)?
        .ok_or_else(undefined_table)?;
    let oid: u32 = header.get(0);
    let relkind = header.get::<_, String>(1).chars().next().unwrap_or('r');
    let rows = client
        .query(
            r#"
            SELECT a.attnum, a.attname::text,
                   pg_catalog.format_type(a.atttypid, a.atttypmod),
                   NOT a.attnotnull, a.attgenerated::text,
                   a.attidentity::text, a.atthasdef, a.atttypid,
                   a.atttypmod, pg_catalog.pg_get_expr(d.adbin, d.adrelid)
            FROM pg_attribute a
            LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
            WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped
            ORDER BY a.attnum
            "#,
            &[&oid],
        )
        .await
        .map_err(database_error)?;
    let mut catalog_columns = Vec::with_capacity(rows.len());
    let mut mutation_columns = Vec::with_capacity(rows.len());
    for row in rows {
        let attnum: i16 = row.get(0);
        let name: String = row.get(1);
        let generated: String = row.get(4);
        let identity: String = row.get(5);
        let writability = if !generated.is_empty() {
            ColumnWritability::Generated
        } else if identity == "a" {
            ColumnWritability::IdentityAlways
        } else {
            ColumnWritability::Writable
        };
        catalog_columns.push(CatalogColumn {
            attnum,
            name: name.clone(),
            type_oid: row.get(7),
            type_modifier: row.get(8),
            generated: generated.clone(),
            identity: identity.clone(),
            has_default: row.get(6),
            default_expression: row.get(9),
        });
        mutation_columns.push(MutationColumnDescriptor {
            name,
            cast_type: row.get(2),
            nullable: row.get(3),
            writability,
            has_default: row.get(6),
            has_identity: !identity.is_empty(),
            projected: false,
        });
    }
    let primary_key = client
        .query(
            r#"
            SELECT ix.indexrelid, a.attname::text
            FROM pg_index ix
            JOIN unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord) ON true
            JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = u.attnum
            WHERE ix.indrelid = $1 AND ix.indisprimary AND u.ord <= ix.indnkeyatts
            ORDER BY u.ord
            "#,
            &[&oid],
        )
        .await
        .map_err(database_error)?
        .into_iter()
        .map(|row| (row.get::<_, u32>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    let primary_identity_oid = primary_key.first().map(|(oid, _)| *oid);
    let primary_key = primary_key
        .into_iter()
        .map(|(_, column)| column)
        .collect::<Vec<_>>();
    let unique_rows = client
        .query(
            r#"
            SELECT i.oid, i.relname::text,
                   array_agg(a.attname::text ORDER BY u.ord)
            FROM pg_index ix
            JOIN pg_class i ON i.oid = ix.indexrelid
            JOIN unnest(ix.indkey) WITH ORDINALITY AS u(attnum, ord) ON true
            JOIN pg_attribute a ON a.attrelid = ix.indrelid AND a.attnum = u.attnum
            WHERE ix.indrelid = $1
              AND ix.indisvalid AND ix.indisunique AND NOT ix.indisprimary
              AND ix.indimmediate AND ix.indpred IS NULL AND ix.indexprs IS NULL
              AND u.ord <= ix.indnkeyatts
            GROUP BY i.oid, i.relname
            "#,
            &[&oid],
        )
        .await
        .map_err(database_error)?;
    let non_nullable = mutation_columns
        .iter()
        .filter(|column| !column.nullable)
        .map(|column| column.name.as_str())
        .collect::<HashSet<_>>();
    let unique_indexes = unique_rows
        .into_iter()
        .filter_map(|row| {
            let oid: u32 = row.get(0);
            let name: String = row.get(1);
            let columns: Vec<String> = row.get(2);
            (!columns.is_empty()
                && columns
                    .iter()
                    .all(|column| non_nullable.contains(column.as_str())))
            .then_some((oid, UniqueIndexCandidate { name, columns }))
        })
        .collect::<Vec<_>>();
    let unique_identity_oid = unique_indexes
        .iter()
        .min_by(|(_, left), (_, right)| {
            left.columns
                .len()
                .cmp(&right.columns.len())
                .then_with(|| left.name.cmp(&right.name))
        })
        .map(|(oid, _)| *oid);
    let unique_index_candidates = unique_indexes
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect::<Vec<_>>();
    let resolved = resolve_identity(relkind, &primary_key, &unique_index_candidates);
    let identity_object_oid = match resolved.kind {
        RelationIdentityKind::PrimaryKey => primary_identity_oid,
        RelationIdentityKind::UniqueIndex => unique_identity_oid,
        RelationIdentityKind::CtidFallback | RelationIdentityKind::None => None,
    };
    let identity = MutationIdentity {
        kind: match resolved.kind {
            RelationIdentityKind::PrimaryKey => MutationIdentityKind::PrimaryKey,
            RelationIdentityKind::UniqueIndex => MutationIdentityKind::UniqueIndex,
            RelationIdentityKind::CtidFallback => MutationIdentityKind::CtidFallback,
            RelationIdentityKind::None => MutationIdentityKind::None,
        },
        columns: resolved.columns,
    };
    Ok(CatalogDescriptor {
        oid,
        identity_object_oid,
        columns: catalog_columns,
        mutation: MutationTableDescriptor {
            schema: schema.into(),
            table: table.into(),
            columns: mutation_columns,
            identity,
        },
    })
}

async fn possible_temp_shadow<'a>(
    client: &Client,
    descriptors: impl Iterator<Item = &'a CatalogDescriptor>,
) -> Result<bool, ResultMutationError> {
    let names = descriptors
        .map(|descriptor| descriptor.mutation.table.clone())
        .collect::<Vec<_>>();
    let row = client
        .query_one(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                WHERE n.nspname ~ '^pg_temp_[0-9]+$' AND c.relname = ANY($1)
            )
            "#,
            &[&names],
        )
        .await
        .map_err(database_error)?;
    Ok(row.get(0))
}

async fn has_unsafe_inheritance_children(
    client: &Client,
    oids: impl Iterator<Item = u32>,
) -> Result<bool, ResultMutationError> {
    let oids = oids.collect::<Vec<_>>();
    if oids.is_empty() {
        return Ok(false);
    }
    let row = client
        .query_one(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM pg_class parent
                JOIN pg_inherits inheritance ON inheritance.inhparent = parent.oid
                WHERE parent.oid = ANY($1) AND parent.relkind = 'r'
            )
            "#,
            &[&oids],
        )
        .await
        .map_err(database_error)?;
    Ok(row.get(0))
}

fn system_column_name(attnum: i16) -> Option<&'static str> {
    match attnum {
        -1 => Some("ctid"),
        -2 => Some("xmin"),
        -3 => Some("cmin"),
        -4 => Some("xmax"),
        -5 => Some("cmax"),
        -6 => Some("tableoid"),
        _ => None,
    }
}

pub(crate) fn is_undefined_object(error: &ResultMutationError) -> bool {
    matches!(error_code(error), Some("42703" | "42P01"))
}

pub(crate) fn is_dead_socket(error: &ResultMutationError) -> bool {
    matches!(error, ResultMutationError::ConnectionLost)
}

fn error_code(error: &ResultMutationError) -> Option<&str> {
    match error {
        ResultMutationError::Database { code, .. } => code.as_deref(),
        ResultMutationError::NotAnalyzable {
            reason: NotAnalyzableReason::Database { code, .. },
        } => code.as_deref(),
        _ => None,
    }
}

fn map_apply_error(error: tokio_postgres::Error, op_index: usize) -> ResultMutationError {
    let mapped = database_error(error);
    match mapped {
        ResultMutationError::Database {
            code: Some(ref code),
            ..
        } if code == "55P03" => ResultMutationError::LockTimeout { op_index },
        ResultMutationError::Database {
            code,
            message,
            severity,
            position,
            ..
        } => ResultMutationError::Database {
            code,
            message,
            severity,
            position,
            op_index: Some(op_index),
        },
        other => other,
    }
}

pub(crate) fn database_error(error: tokio_postgres::Error) -> ResultMutationError {
    map_dedicated(dedicated::database_error(error))
}

fn map_dedicated(error: DedicatedError) -> ResultMutationError {
    match error {
        DedicatedError::ConnectionLost => ResultMutationError::ConnectionLost,
        DedicatedError::Timeout { operation } => ResultMutationError::Timeout { operation },
        DedicatedError::Database {
            code,
            message,
            severity,
            position,
        } => ResultMutationError::Database {
            code,
            message,
            severity,
            position,
            op_index: None,
        },
    }
}

fn undefined_table() -> ResultMutationError {
    ResultMutationError::Database {
        code: Some("42P01".into()),
        message: "undefined table".into(),
        severity: Some("ERROR".into()),
        position: None,
        op_index: None,
    }
}

fn undefined_column() -> ResultMutationError {
    ResultMutationError::Database {
        code: Some("42703".into()),
        message: "undefined column".into(),
        severity: Some("ERROR".into()),
        position: None,
        op_index: None,
    }
}

fn not_analyzable(reason: NotAnalyzableReason) -> ResultMutationError {
    ResultMutationError::NotAnalyzable { reason }
}

fn unsupported_statement() -> ResultMutationError {
    not_analyzable(NotAnalyzableReason::Database {
        code: None,
        message: "PostgreSQL could not unambiguously resolve relation range variables".into(),
        severity: None,
        position: None,
    })
}

fn analysis_database_error(error: tokio_postgres::Error) -> ResultMutationError {
    match database_error(error) {
        ResultMutationError::Database {
            code,
            message,
            severity,
            position,
            ..
        } => not_analyzable(NotAnalyzableReason::Database {
            code,
            message,
            severity,
            position,
        }),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog_descriptor() -> CatalogDescriptor {
        CatalogDescriptor {
            oid: 42,
            identity_object_oid: Some(43),
            columns: vec![CatalogColumn {
                attnum: 1,
                name: "id".into(),
                type_oid: 23,
                type_modifier: -1,
                generated: String::new(),
                identity: String::new(),
                has_default: false,
                default_expression: None,
            }],
            mutation: MutationTableDescriptor {
                schema: "public".into(),
                table: "rows".into(),
                columns: vec![MutationColumnDescriptor {
                    name: "id".into(),
                    cast_type: "integer".into(),
                    nullable: false,
                    writability: ColumnWritability::Writable,
                    has_default: false,
                    has_identity: false,
                    projected: true,
                }],
                identity: MutationIdentity {
                    kind: MutationIdentityKind::PrimaryKey,
                    columns: vec!["id".into()],
                },
            },
        }
    }

    #[test]
    fn generated_classification_is_non_empty_not_version_specific() {
        for generated in ["s", "v", "future"] {
            let writability = if !generated.is_empty() {
                ColumnWritability::Generated
            } else {
                ColumnWritability::Writable
            };
            assert_eq!(writability, ColumnWritability::Generated);
        }
    }

    #[test]
    fn every_postgres_system_attnum_is_recognized() {
        assert_eq!(system_column_name(-1), Some("ctid"));
        assert_eq!(system_column_name(-2), Some("xmin"));
        assert_eq!(system_column_name(-3), Some("cmin"));
        assert_eq!(system_column_name(-4), Some("xmax"));
        assert_eq!(system_column_name(-5), Some("cmax"));
        assert_eq!(system_column_name(-6), Some("tableoid"));
        assert_eq!(system_column_name(0), None);
    }

    #[test]
    fn range_variable_check_allows_duplicate_projection_but_rejects_self_join() {
        let duplicate_projection = vec![
            ProjectedColumn {
                name: "id".into(),
                table_oid: Some(42),
                attnum: Some(1),
                cast_type: "integer".into(),
            },
            ProjectedColumn {
                name: "id_again".into(),
                table_oid: Some(42),
                attnum: Some(1),
                cast_type: "integer".into(),
            },
        ];
        assert!(ensure_unambiguous_range_variables(&duplicate_projection, &[42]).is_ok());
        assert!(matches!(
            ensure_unambiguous_range_variables(&duplicate_projection, &[42, 42]),
            Err(ResultMutationError::NotAnalyzable {
                reason: NotAnalyzableReason::Database { code: None, .. }
            })
        ));
    }

    fn range_names(sql: &str) -> Result<Vec<String>, ()> {
        parse_range_variables(sql)
            .map(|variables| variables.iter().map(RangeVariable::regclass_name).collect())
    }

    #[test]
    fn range_parser_ignores_comments_strings_and_dollar_quotes() {
        assert_eq!(
            range_names(
                "SELECT 'from fake join fake', $$select * from fake$$, rows.id \
                 FROM /* join ignored */ public.rows rows -- from ignored\n\
                 WHERE rows.body = $1"
            ),
            Ok(vec!["\"public\".\"rows\"".into()])
        );
    }

    #[test]
    fn range_parser_preserves_quoted_names_and_expression_nesting() {
        assert_eq!(
            range_names(
                "SELECT substring(r.body FROM 1), count(*) OVER (PARTITION BY r.id) \
                 FROM \"Odd Schema\".\"Odd Rows\" AS r"
            ),
            Ok(vec!["\"Odd Schema\".\"Odd Rows\"".into()])
        );
    }

    #[test]
    fn range_parser_enumerates_joins_and_repeated_spelling() {
        assert_eq!(
            range_names(
                "SELECT l.id, r.body FROM public.rows l \
                 JOIN rows r ON (l.id = r.id AND r.body = $1)"
            ),
            Ok(vec!["\"public\".\"rows\"".into(), "\"rows\"".into()])
        );
    }

    #[test]
    fn range_parser_fails_closed_for_comma_nested_cte_and_function_ranges() {
        for sql in [
            "SELECT a.id FROM rows a, rows b",
            "SELECT nested.id FROM (SELECT id FROM rows) nested",
            "WITH nested AS (SELECT id FROM rows) SELECT id FROM nested",
            "SELECT value FROM unnest(ARRAY[1]) value",
        ] {
            assert_eq!(range_names(sql), Err(()), "{sql}");
        }
    }

    #[test]
    fn catalog_work_is_scoped_and_attributed_to_operation_targets() {
        let plan = MutationPlan {
            operations: vec![
                MutationOp::Insert {
                    table: MutationTable {
                        schema: "public".into(),
                        table: "other".into(),
                    },
                    values: Vec::new(),
                },
                MutationOp::Delete {
                    table: MutationTable {
                        schema: "public".into(),
                        table: "rows".into(),
                    },
                    identity: Vec::new(),
                    guards: Vec::new(),
                },
                MutationOp::Update {
                    table: MutationTable {
                        schema: "public".into(),
                        table: "rows".into(),
                    },
                    identity: Vec::new(),
                    guards: Vec::new(),
                    set: Vec::new(),
                },
            ],
        };
        assert_eq!(
            operation_targets(&plan),
            vec![
                (
                    MutationTable {
                        schema: "public".into(),
                        table: "other".into(),
                    },
                    0,
                ),
                (
                    MutationTable {
                        schema: "public".into(),
                        table: "rows".into(),
                    },
                    1,
                ),
            ]
        );
    }

    #[test]
    fn maps_structural_sqlstates_without_message_matching() {
        let undefined = undefined_column();
        assert!(is_undefined_object(&undefined));
        let cancelled = ResultMutationError::Database {
            code: Some("57014".into()),
            message: "redacted".into(),
            severity: None,
            position: None,
            op_index: None,
        };
        assert_eq!(error_code(&cancelled), Some("57014"));
    }

    #[test]
    fn apply_fingerprint_expires_on_oid_and_mutation_relevant_catalog_drift() {
        let previous = catalog_descriptor();
        assert!(ensure_descriptor_unchanged(&previous, &previous).is_ok());

        let mut changed = previous.clone();
        changed.oid += 1;
        assert_eq!(
            ensure_descriptor_unchanged(&previous, &changed),
            Err(ResultMutationError::AnalysisExpired)
        );

        let mutations: [fn(&mut CatalogDescriptor); 8] = [
            |value| value.mutation.columns[0].cast_type = "bigint".into(),
            |value| value.mutation.columns[0].writability = ColumnWritability::Generated,
            |value| value.mutation.columns[0].has_default = true,
            |value| value.mutation.columns[0].has_identity = true,
            |value| value.mutation.identity.kind = MutationIdentityKind::UniqueIndex,
            |value| value.columns[0].type_oid = 20,
            |value| value.columns[0].default_expression = Some("42".into()),
            |value| value.identity_object_oid = Some(44),
        ];
        for mutate in mutations {
            let mut changed = previous.clone();
            mutate(&mut changed);
            assert_eq!(
                ensure_descriptor_unchanged(&previous, &changed),
                Err(ResultMutationError::AnalysisExpired)
            );
        }
    }
}
