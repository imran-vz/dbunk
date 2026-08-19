use crate::quote_double;
use crate::MAX_TABLE_PAGE_SIZE;

use super::protocol::*;

pub(crate) const CTID_COLUMN: &str = "ctid";
pub(crate) const CTID_CAST_TYPE: &str = "tid";
pub(crate) const CTID_KEYSET_VERSION: i32 = 140000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelationColumn {
    pub name: String,
    pub cast_type: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UniqueIndexCandidate {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RelationDescriptor {
    pub schema: String,
    pub table: String,
    pub relkind: char,
    pub server_version_num: i32,
    pub columns: Vec<RelationColumn>,
    pub primary_key: Vec<String>,
    pub unique_indexes: Vec<UniqueIndexCandidate>,
}

impl RelationDescriptor {
    pub(crate) fn column(&self, name: &str) -> Option<&RelationColumn> {
        self.columns.iter().find(|column| column.name == name)
    }

    pub(crate) fn identity(&self) -> BrowseIdentity {
        if !self.primary_key.is_empty() {
            return BrowseIdentity {
                kind: BrowseIdentityKind::PrimaryKey,
                columns: self.primary_key.clone(),
            };
        }
        let mut indexes = self.unique_indexes.clone();
        indexes.sort_by(|left, right| {
            left.columns
                .len()
                .cmp(&right.columns.len())
                .then_with(|| left.name.cmp(&right.name))
        });
        if let Some(index) = indexes.first() {
            return BrowseIdentity {
                kind: BrowseIdentityKind::UniqueIndex,
                columns: index.columns.clone(),
            };
        }
        if matches!(self.relkind, 'r' | 'p') {
            return BrowseIdentity {
                kind: BrowseIdentityKind::Virtual,
                columns: vec![CTID_COLUMN.to_string()],
            };
        }
        BrowseIdentity {
            kind: BrowseIdentityKind::None,
            columns: Vec::new(),
        }
    }

    pub(crate) fn visible_columns(&self) -> Vec<BrowseColumn> {
        self.columns
            .iter()
            .map(|column| BrowseColumn {
                name: column.name.clone(),
                cast_type: column.cast_type.clone(),
                nullable: column.nullable,
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BoundParam {
    Text(Option<String>),
    TextArray(Vec<String>),
}

impl BoundParam {
    pub(crate) fn inspection_value(&self) -> Option<String> {
        match self {
            Self::Text(value) => value.clone(),
            Self::TextArray(values) => Some(serde_json::to_string(values).unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuiltBrowseQuery {
    pub sql: String,
    pub params: Vec<BoundParam>,
    pub where_sql: String,
    pub where_params: Vec<BoundParam>,
    pub qualified: String,
    pub visible_columns: Vec<BrowseColumn>,
    pub identity: BrowseIdentity,
    pub projects_ctid: bool,
    pub page_mode: BrowsePageMode,
    pub page: Option<u32>,
    pub page_size: u32,
}

impl BuiltBrowseQuery {
    pub(crate) fn inspection(&self) -> BrowseInspection {
        BrowseInspection {
            sql: self.sql.clone(),
            params: self
                .params
                .iter()
                .map(BoundParam::inspection_value)
                .collect(),
        }
    }

    pub(crate) fn count_sql(&self) -> String {
        if self.where_sql.is_empty() {
            format!("SELECT count(*) FROM {}", self.qualified)
        } else {
            format!(
                "SELECT count(*) FROM {} WHERE {}",
                self.qualified, self.where_sql
            )
        }
    }

    pub(crate) fn explain_sql(&self) -> String {
        if self.where_sql.is_empty() {
            format!("EXPLAIN (FORMAT JSON) SELECT 1 FROM {}", self.qualified)
        } else {
            format!(
                "EXPLAIN (FORMAT JSON) SELECT 1 FROM {} WHERE {}",
                self.qualified, self.where_sql
            )
        }
    }
}

pub(crate) fn build_browse_query(
    descriptor: &RelationDescriptor,
    payload: &BrowseTableDataPayload,
) -> Result<BuiltBrowseQuery, TableBrowseError> {
    let page_size = payload.page_size.clamp(1, MAX_TABLE_PAGE_SIZE);
    let identity = descriptor.identity();
    let mut params = Vec::new();
    let mut predicates = Vec::new();
    for filter in &payload.filters {
        if let Some(predicate) = render_filter(descriptor, filter, &mut params)? {
            predicates.push(predicate);
        }
    }
    let where_params = params.clone();
    let where_sql = predicates.join(" AND ");

    let page_mode = effective_page_mode(payload, &identity, descriptor.server_version_num);
    let page = match (&payload.page_request, page_mode) {
        (BrowsePageRequest::Offset { page }, BrowsePageMode::Offset) => Some((*page).max(1)),
        (_, BrowsePageMode::Offset) => Some(1),
        _ => None,
    };

    if page_mode == BrowsePageMode::Keyset {
        if let BrowsePageRequest::Keyset {
            cursor: Some(cursor),
        } = &payload.page_request
        {
            if cursor.values.len() != identity.columns.len() {
                return Err(TableBrowseError::InvalidCursor);
            }
            predicates.push(render_keyset(descriptor, &identity, cursor, &mut params)?);
        }
    }

    let order_sql = render_order(descriptor, payload, &identity)?;
    let mut select_list = descriptor
        .columns
        .iter()
        .map(|column| format!("{}::text", quote_double(&column.name)))
        .collect::<Vec<_>>();
    let projects_ctid = identity.kind == BrowseIdentityKind::Virtual;
    if projects_ctid {
        select_list.push(format!("{}::text", quote_double(CTID_COLUMN)));
    }

    let qualified = qualified_relation(&descriptor.schema, &descriptor.table);
    let mut sql = format!("SELECT {} FROM {}", select_list.join(", "), qualified);
    if !predicates.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&predicates.join(" AND "));
    }
    if !order_sql.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order_sql);
    }
    let fetch_limit = page_size.saturating_add(1);
    sql.push_str(&format!(" LIMIT {fetch_limit}"));
    if page_mode == BrowsePageMode::Offset {
        let page = page.unwrap_or(1);
        let offset = (page as u64 - 1).saturating_mul(page_size as u64);
        sql.push_str(&format!(" OFFSET {offset}"));
    }

    Ok(BuiltBrowseQuery {
        sql,
        params,
        where_sql,
        where_params,
        qualified,
        visible_columns: descriptor.visible_columns(),
        identity,
        projects_ctid,
        page_mode,
        page,
        page_size,
    })
}

pub(crate) fn build_count_query(
    descriptor: &RelationDescriptor,
    filters: &[BrowseFilter],
) -> Result<(String, Vec<BoundParam>), TableBrowseError> {
    let built = build_browse_query(
        descriptor,
        &BrowseTableDataPayload {
            connection_id: String::new(),
            tab_id: String::new(),
            request_id: 0,
            schema: descriptor.schema.clone(),
            table: descriptor.table.clone(),
            filters: filters.to_vec(),
            sort: Vec::new(),
            page_request: BrowsePageRequest::Offset { page: 1 },
            page_size: 1,
            count_policy: BrowseCountPolicy::None,
            refresh_structure: false,
        },
    )?;
    Ok((built.count_sql(), built.where_params))
}

fn effective_page_mode(
    payload: &BrowseTableDataPayload,
    identity: &BrowseIdentity,
    server_version_num: i32,
) -> BrowsePageMode {
    let BrowsePageRequest::Keyset { .. } = payload.page_request else {
        return BrowsePageMode::Offset;
    };
    if !payload.sort.is_empty() || !identity.exists() {
        return BrowsePageMode::Offset;
    }
    if identity.kind == BrowseIdentityKind::Virtual && server_version_num < CTID_KEYSET_VERSION {
        return BrowsePageMode::Offset;
    }
    BrowsePageMode::Keyset
}

fn render_filter(
    descriptor: &RelationDescriptor,
    filter: &BrowseFilter,
    params: &mut Vec<BoundParam>,
) -> Result<Option<String>, TableBrowseError> {
    match filter {
        BrowseFilter::RawSql { text } => {
            let text = text.trim();
            if text.is_empty() {
                return Err(TableBrowseError::InvalidFilter {
                    reason: "emptyRawSql".into(),
                });
            }
            Ok(Some(format!("({text})")))
        }
        BrowseFilter::Column {
            column,
            operator,
            value,
            values,
        } => {
            let relation_column =
                descriptor
                    .column(column)
                    .ok_or_else(|| TableBrowseError::UnknownColumn {
                        column: column.clone(),
                    })?;
            let quoted = quote_double(column);
            let sql = match operator {
                BrowseFilterOperator::IsNull => format!("{quoted} IS NULL"),
                BrowseFilterOperator::IsNotNull => format!("{quoted} IS NOT NULL"),
                BrowseFilterOperator::InList => {
                    let entries =
                        values
                            .as_ref()
                            .ok_or_else(|| TableBrowseError::InvalidFilter {
                                reason: "emptyInList".into(),
                            })?;
                    if entries.is_empty() {
                        return Err(TableBrowseError::InvalidFilter {
                            reason: "emptyInList".into(),
                        });
                    }
                    params.push(BoundParam::TextArray(entries.clone()));
                    format!(
                        "{quoted} = ANY((${}::text[])::{}[])",
                        params.len(),
                        relation_column.cast_type
                    )
                }
                BrowseFilterOperator::Contains
                | BrowseFilterOperator::NotContains
                | BrowseFilterOperator::StartsWith
                | BrowseFilterOperator::EndsWith => {
                    let raw = required_value(value)?;
                    let escaped = escape_like(&raw);
                    let pattern = match operator {
                        BrowseFilterOperator::Contains | BrowseFilterOperator::NotContains => {
                            format!("%{escaped}%")
                        }
                        BrowseFilterOperator::StartsWith => format!("{escaped}%"),
                        BrowseFilterOperator::EndsWith => format!("%{escaped}"),
                        _ => unreachable!(),
                    };
                    params.push(BoundParam::Text(Some(pattern)));
                    let op = if matches!(operator, BrowseFilterOperator::NotContains) {
                        "NOT ILIKE"
                    } else {
                        "ILIKE"
                    };
                    format!("{quoted}::text {op} ${} ESCAPE '\\'", params.len())
                }
                BrowseFilterOperator::Eq
                | BrowseFilterOperator::Neq
                | BrowseFilterOperator::Lt
                | BrowseFilterOperator::Lte
                | BrowseFilterOperator::Gt
                | BrowseFilterOperator::Gte => {
                    let raw = required_value(value)?;
                    params.push(BoundParam::Text(Some(raw)));
                    let op = match operator {
                        BrowseFilterOperator::Eq => "=",
                        BrowseFilterOperator::Neq => "<>",
                        BrowseFilterOperator::Lt => "<",
                        BrowseFilterOperator::Lte => "<=",
                        BrowseFilterOperator::Gt => ">",
                        BrowseFilterOperator::Gte => ">=",
                        _ => unreachable!(),
                    };
                    format!(
                        "{quoted} {op} (${}::text)::{}",
                        params.len(),
                        relation_column.cast_type
                    )
                }
            };
            Ok(Some(sql))
        }
    }
}

fn render_keyset(
    descriptor: &RelationDescriptor,
    identity: &BrowseIdentity,
    cursor: &BrowseCursor,
    params: &mut Vec<BoundParam>,
) -> Result<String, TableBrowseError> {
    let mut left = Vec::new();
    let mut right = Vec::new();
    for (column_name, value) in identity.columns.iter().zip(cursor.values.iter()) {
        let cast_type = if column_name == CTID_COLUMN {
            CTID_CAST_TYPE.to_string()
        } else {
            descriptor
                .column(column_name)
                .ok_or(TableBrowseError::InvalidCursor)?
                .cast_type
                .clone()
        };
        params.push(BoundParam::Text(Some(value.clone())));
        left.push(quote_double(column_name));
        right.push(format!("(${}::text)::{cast_type}", params.len()));
    }
    Ok(format!("({}) > ({})", left.join(", "), right.join(", ")))
}

fn render_order(
    descriptor: &RelationDescriptor,
    payload: &BrowseTableDataPayload,
    identity: &BrowseIdentity,
) -> Result<String, TableBrowseError> {
    let mut terms = Vec::new();
    let mut seen = Vec::new();
    for key in &payload.sort {
        let _ = descriptor
            .column(&key.column)
            .ok_or_else(|| TableBrowseError::InvalidSort {
                column: key.column.clone(),
            })?;
        terms.push(render_sort_term(&key.column, key.direction, key.nulls));
        seen.push(key.column.as_str());
    }
    if identity.exists() {
        for column in &identity.columns {
            if !seen.iter().any(|existing| existing == column) {
                terms.push(render_sort_term(
                    column,
                    BrowseSortDirection::Asc,
                    BrowseNulls::Default,
                ));
            }
        }
    }
    Ok(terms.join(", "))
}

fn render_sort_term(column: &str, direction: BrowseSortDirection, nulls: BrowseNulls) -> String {
    let direction = match direction {
        BrowseSortDirection::Asc => "ASC",
        BrowseSortDirection::Desc => "DESC",
    };
    let nulls = match nulls {
        BrowseNulls::Default => String::new(),
        BrowseNulls::First => " NULLS FIRST".to_string(),
        BrowseNulls::Last => " NULLS LAST".to_string(),
    };
    format!("{} {direction}{nulls}", quote_double(column))
}

fn required_value(value: &Option<String>) -> Result<String, TableBrowseError> {
    value.clone().ok_or(TableBrowseError::InvalidFilter {
        reason: "missingValue".into(),
    })
}

pub(crate) fn escape_like(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' | '%' | '_' => {
                escaped.push('\\');
                escaped.push(ch);
            }
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn qualified_relation(schema: &str, table: &str) -> String {
    if schema.is_empty() {
        quote_double(table)
    } else {
        format!("{}.{}", quote_double(schema), quote_double(table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn users() -> RelationDescriptor {
        RelationDescriptor {
            schema: "public".into(),
            table: "users".into(),
            relkind: 'r',
            server_version_num: 160000,
            columns: vec![
                RelationColumn {
                    name: "id".into(),
                    cast_type: "integer".into(),
                    nullable: false,
                },
                RelationColumn {
                    name: "name".into(),
                    cast_type: "text".into(),
                    nullable: true,
                },
                RelationColumn {
                    name: "email".into(),
                    cast_type: "text".into(),
                    nullable: false,
                },
            ],
            primary_key: vec!["id".into()],
            unique_indexes: Vec::new(),
        }
    }

    fn composite() -> RelationDescriptor {
        let mut descriptor = users();
        descriptor.primary_key = vec!["id".into(), "email".into()];
        descriptor
    }

    fn keyless() -> RelationDescriptor {
        let mut descriptor = users();
        descriptor.primary_key.clear();
        descriptor
    }

    fn payload() -> BrowseTableDataPayload {
        BrowseTableDataPayload {
            connection_id: "c".into(),
            tab_id: "t".into(),
            request_id: 1,
            schema: "public".into(),
            table: "users".into(),
            filters: Vec::new(),
            sort: Vec::new(),
            page_request: BrowsePageRequest::Offset { page: 1 },
            page_size: 100,
            count_policy: BrowseCountPolicy::None,
            refresh_structure: false,
        }
    }

    fn built(
        descriptor: &RelationDescriptor,
        mutate: impl FnOnce(&mut BrowseTableDataPayload),
    ) -> BuiltBrowseQuery {
        let mut payload = payload();
        mutate(&mut payload);
        build_browse_query(descriptor, &payload).expect("build")
    }

    #[test]
    fn every_comparison_operator_renders_text_cast() {
        for (operator, op) in [
            (BrowseFilterOperator::Eq, "="),
            (BrowseFilterOperator::Neq, "<>"),
            (BrowseFilterOperator::Lt, "<"),
            (BrowseFilterOperator::Lte, "<="),
            (BrowseFilterOperator::Gt, ">"),
            (BrowseFilterOperator::Gte, ">="),
        ] {
            let query = built(&users(), |payload| {
                payload.filters = vec![BrowseFilter::Column {
                    column: "id".into(),
                    operator,
                    value: Some("7".into()),
                    values: None,
                }];
            });
            assert!(
                query
                    .sql
                    .contains(&format!(r#""id" {op} ($1::text)::integer"#)),
                "{}",
                query.sql
            );
            assert_eq!(query.params, vec![BoundParam::Text(Some("7".into()))]);
        }
    }

    #[test]
    fn ilike_operators_escape_metacharacters() {
        let query = built(&users(), |payload| {
            payload.filters = vec![
                BrowseFilter::Column {
                    column: "name".into(),
                    operator: BrowseFilterOperator::Contains,
                    value: Some(r"a%_b\c".into()),
                    values: None,
                },
                BrowseFilter::Column {
                    column: "name".into(),
                    operator: BrowseFilterOperator::NotContains,
                    value: Some("x".into()),
                    values: None,
                },
                BrowseFilter::Column {
                    column: "email".into(),
                    operator: BrowseFilterOperator::StartsWith,
                    value: Some("Ada".into()),
                    values: None,
                },
                BrowseFilter::Column {
                    column: "email".into(),
                    operator: BrowseFilterOperator::EndsWith,
                    value: Some("io".into()),
                    values: None,
                },
            ];
        });
        assert!(query.sql.contains(r#""name"::text ILIKE $1 ESCAPE '\'"#));
        assert!(query
            .sql
            .contains(r#""name"::text NOT ILIKE $2 ESCAPE '\'"#));
        assert!(query.sql.contains(r#""email"::text ILIKE $3 ESCAPE '\'"#));
        assert!(query.sql.contains(r#""email"::text ILIKE $4 ESCAPE '\'"#));
        assert_eq!(
            query.params,
            vec![
                BoundParam::Text(Some(r"%a\%\_b\\c%".into())),
                BoundParam::Text(Some("%x%".into())),
                BoundParam::Text(Some("Ada%".into())),
                BoundParam::Text(Some("%io".into())),
            ]
        );
    }

    #[test]
    fn in_list_and_null_operators() {
        let query = built(&users(), |payload| {
            payload.filters = vec![
                BrowseFilter::Column {
                    column: "id".into(),
                    operator: BrowseFilterOperator::InList,
                    value: None,
                    values: Some(vec!["1".into(), "2".into()]),
                },
                BrowseFilter::Column {
                    column: "name".into(),
                    operator: BrowseFilterOperator::IsNull,
                    value: None,
                    values: None,
                },
                BrowseFilter::Column {
                    column: "email".into(),
                    operator: BrowseFilterOperator::IsNotNull,
                    value: None,
                    values: None,
                },
            ];
        });
        assert!(query.sql.contains(r#""id" = ANY(($1::text[])::integer[])"#));
        assert!(query.sql.contains(r#""name" IS NULL"#));
        assert!(query.sql.contains(r#""email" IS NOT NULL"#));
        assert_eq!(
            query.params,
            vec![BoundParam::TextArray(vec!["1".into(), "2".into()])]
        );
    }

    #[test]
    fn raw_and_typed_filters_and_combine() {
        let query = built(&users(), |payload| {
            payload.filters = vec![
                BrowseFilter::Column {
                    column: "id".into(),
                    operator: BrowseFilterOperator::Gt,
                    value: Some("1".into()),
                    values: None,
                },
                BrowseFilter::RawSql {
                    text: "name ILIKE '%a%'".into(),
                },
            ];
        });
        assert!(query
            .sql
            .contains(r#""id" > ($1::text)::integer AND (name ILIKE '%a%')"#));
        assert_eq!(query.where_params.len(), 1);
    }

    #[test]
    fn sort_nulls_and_identity_tiebreakers() {
        let query = built(&composite(), |payload| {
            payload.sort = vec![BrowseSortKey {
                column: "name".into(),
                direction: BrowseSortDirection::Desc,
                nulls: BrowseNulls::Last,
            }];
        });
        assert!(query
            .sql
            .contains(r#"ORDER BY "name" DESC NULLS LAST, "id" ASC, "email" ASC"#));
        assert_eq!(query.page_mode, BrowsePageMode::Offset);
    }

    #[test]
    fn keyset_single_and_multi_column_and_ctid() {
        let single = built(&users(), |payload| {
            payload.page_request = BrowsePageRequest::Keyset {
                cursor: Some(BrowseCursor {
                    values: vec!["9".into()],
                }),
            };
        });
        assert_eq!(single.page_mode, BrowsePageMode::Keyset);
        assert!(single.sql.contains(r#"("id") > (($1::text)::integer)"#));
        assert!(single.sql.contains("LIMIT 101"));
        assert!(!single.sql.contains("OFFSET"));

        let multi = built(&composite(), |payload| {
            payload.page_request = BrowsePageRequest::Keyset {
                cursor: Some(BrowseCursor {
                    values: vec!["9".into(), "a@b".into()],
                }),
            };
        });
        assert!(multi
            .sql
            .contains(r#"("id", "email") > (($1::text)::integer, ($2::text)::text)"#));

        let ctid = built(&keyless(), |payload| {
            payload.page_request = BrowsePageRequest::Keyset {
                cursor: Some(BrowseCursor {
                    values: vec!["(0,1)".into()],
                }),
            };
        });
        assert!(ctid.projects_ctid);
        assert!(ctid.sql.contains(r#""ctid"::text"#));
        assert!(ctid.sql.contains(r#"("ctid") > (($1::text)::tid)"#));
        assert_eq!(ctid.identity.kind, BrowseIdentityKind::Virtual);
    }

    #[test]
    fn offset_fallback_for_user_sort_and_pre14_ctid() {
        let sorted = built(&users(), |payload| {
            payload.sort = vec![BrowseSortKey {
                column: "name".into(),
                direction: BrowseSortDirection::Asc,
                nulls: BrowseNulls::Default,
            }];
            payload.page_request = BrowsePageRequest::Keyset { cursor: None };
        });
        assert_eq!(sorted.page_mode, BrowsePageMode::Offset);
        assert!(sorted.sql.contains("OFFSET 0"));

        let mut old = keyless();
        old.server_version_num = 130000;
        let query = built(&old, |payload| {
            payload.page_request = BrowsePageRequest::Keyset { cursor: None };
        });
        assert_eq!(query.page_mode, BrowsePageMode::Offset);
        assert!(query.sql.contains(r#"ORDER BY "ctid" ASC"#));
    }

    #[test]
    fn page_size_clamps_and_fetches_one_extra() {
        let query = built(&users(), |payload| {
            payload.page_size = 0;
            payload.page_request = BrowsePageRequest::Offset { page: 3 };
        });
        assert_eq!(query.page_size, 1);
        assert!(query.sql.contains("LIMIT 2 OFFSET 2"));

        let large = built(&users(), |payload| payload.page_size = 5000);
        assert_eq!(large.page_size, 1000);
        assert!(large.sql.contains("LIMIT 1001 OFFSET 0"));
    }

    #[test]
    fn validation_errors() {
        assert!(matches!(
            build_browse_query(&users(), &{
                let mut payload = payload();
                payload.filters = vec![BrowseFilter::Column {
                    column: "nope".into(),
                    operator: BrowseFilterOperator::Eq,
                    value: Some("1".into()),
                    values: None,
                }];
                payload
            }),
            Err(TableBrowseError::UnknownColumn { column }) if column == "nope"
        ));
        assert!(matches!(
            build_browse_query(&users(), &{
                let mut payload = payload();
                payload.filters = vec![BrowseFilter::Column {
                    column: "id".into(),
                    operator: BrowseFilterOperator::InList,
                    value: None,
                    values: Some(Vec::new()),
                }];
                payload
            }),
            Err(TableBrowseError::InvalidFilter { reason }) if reason == "emptyInList"
        ));
        assert!(matches!(
            build_browse_query(&users(), &{
                let mut payload = payload();
                payload.filters = vec![BrowseFilter::Column {
                    column: "id".into(),
                    operator: BrowseFilterOperator::Eq,
                    value: None,
                    values: None,
                }];
                payload
            }),
            Err(TableBrowseError::InvalidFilter { reason }) if reason == "missingValue"
        ));
        assert!(matches!(
            build_browse_query(&users(), &{
                let mut payload = payload();
                payload.sort = vec![BrowseSortKey {
                    column: "missing".into(),
                    direction: BrowseSortDirection::Asc,
                    nulls: BrowseNulls::Default,
                }];
                payload
            }),
            Err(TableBrowseError::InvalidSort { column }) if column == "missing"
        ));
        assert!(matches!(
            build_browse_query(&users(), &{
                let mut payload = payload();
                payload.page_request = BrowsePageRequest::Keyset {
                    cursor: Some(BrowseCursor {
                        values: vec!["1".into(), "2".into()],
                    }),
                };
                payload
            }),
            Err(TableBrowseError::InvalidCursor)
        ));
        assert!(matches!(
            build_browse_query(&users(), &{
                let mut payload = payload();
                payload.filters = vec![BrowseFilter::RawSql { text: "  ".into() }];
                payload
            }),
            Err(TableBrowseError::InvalidFilter { reason }) if reason == "emptyRawSql"
        ));
    }

    #[test]
    fn generated_sql_never_contains_user_values() {
        const SENTINEL: &str = "SENTINEL_DROP_TABLE_USERS";
        let query = built(&users(), |payload| {
            payload.filters = vec![
                BrowseFilter::Column {
                    column: "name".into(),
                    operator: BrowseFilterOperator::Eq,
                    value: Some(SENTINEL.into()),
                    values: None,
                },
                BrowseFilter::Column {
                    column: "email".into(),
                    operator: BrowseFilterOperator::Contains,
                    value: Some(SENTINEL.into()),
                    values: None,
                },
                BrowseFilter::Column {
                    column: "id".into(),
                    operator: BrowseFilterOperator::InList,
                    value: None,
                    values: Some(vec![SENTINEL.into()]),
                },
            ];
        });
        assert!(
            !query.sql.contains(SENTINEL),
            "value leaked into SQL: {}",
            query.sql
        );
        assert!(query.params.iter().any(|param| param
            .inspection_value()
            .unwrap_or_default()
            .contains(SENTINEL)));
    }

    #[test]
    fn unique_index_identity_ties_break_on_name() {
        let descriptor = RelationDescriptor {
            schema: "public".into(),
            table: "items".into(),
            relkind: 'r',
            server_version_num: 160000,
            columns: vec![
                RelationColumn {
                    name: "a".into(),
                    cast_type: "text".into(),
                    nullable: false,
                },
                RelationColumn {
                    name: "b".into(),
                    cast_type: "text".into(),
                    nullable: false,
                },
            ],
            primary_key: Vec::new(),
            unique_indexes: vec![
                UniqueIndexCandidate {
                    name: "z_idx".into(),
                    columns: vec!["a".into()],
                },
                UniqueIndexCandidate {
                    name: "a_idx".into(),
                    columns: vec!["b".into()],
                },
            ],
        };
        assert_eq!(
            descriptor.identity(),
            BrowseIdentity {
                kind: BrowseIdentityKind::UniqueIndex,
                columns: vec!["b".into()],
            }
        );
    }

    #[test]
    fn count_and_explain_sql_reuse_where_without_limit() {
        let query = built(&users(), |payload| {
            payload.filters = vec![BrowseFilter::Column {
                column: "id".into(),
                operator: BrowseFilterOperator::Eq,
                value: Some("1".into()),
                values: None,
            }];
        });
        assert_eq!(
            query.count_sql(),
            r#"SELECT count(*) FROM "public"."users" WHERE "id" = ($1::text)::integer"#
        );
        assert_eq!(
            query.explain_sql(),
            r#"EXPLAIN (FORMAT JSON) SELECT 1 FROM "public"."users" WHERE "id" = ($1::text)::integer"#
        );
        assert!(!query.count_sql().contains("LIMIT"));
    }

    #[test]
    fn foreign_table_identity_is_none() {
        let mut descriptor = keyless();
        descriptor.relkind = 'f';
        assert_eq!(descriptor.identity().kind, BrowseIdentityKind::None);
        let query = built(&descriptor, |payload| {
            payload.page_request = BrowsePageRequest::Keyset { cursor: None };
        });
        assert_eq!(query.page_mode, BrowsePageMode::Offset);
        assert!(!query.projects_ctid);
    }
}
