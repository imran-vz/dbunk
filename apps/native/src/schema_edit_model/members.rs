//! Comma-separated member lists typed in the inspector: key columns, index
//! members and referenced columns. Names resolve to draft origins, so a
//! renamed column keeps its place in every key that uses it.
use dbunk_lib::backend::schema_edit::{
    IndexMember, IndexTarget, Origin, SchemaEngine, SqlFragment, TableModel, same_name,
};

/// Splits on top-level commas, ignoring commas inside parentheses and quotes.
/// Empty items are dropped and each item is trimmed.
pub(super) fn split_list(text: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(open) => {
                if c == open {
                    quote = None;
                }
                current.push(c);
            }
            None => match c {
                '\'' | '"' | '`' => {
                    quote = Some(c);
                    current.push(c);
                }
                '(' => {
                    depth += 1;
                    current.push(c);
                }
                ')' => {
                    depth = depth.saturating_sub(1);
                    current.push(c);
                }
                ',' if depth == 0 => items.push(std::mem::take(&mut current)),
                _ => current.push(c),
            },
        }
    }
    items.push(current);
    items
        .into_iter()
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Strips one pair of identifier quotes (`"name"` or `` `name` ``).
pub(super) fn unquote(name: &str) -> &str {
    let name = name.trim();
    for quote in ['"', '`'] {
        if let Some(inner) = name
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner;
        }
    }
    name
}

fn find(table: &TableModel, engine: SchemaEngine, name: &str) -> Result<Origin, String> {
    let name = unquote(name);
    table
        .columns
        .iter()
        .find(|column| same_name(engine, &column.name, name))
        .map(|column| column.origin)
        .ok_or_else(|| format!("No column named {name}"))
}

/// Column names to draft origins, in order.
pub(super) fn names_to_origins(
    text: &str,
    table: &TableModel,
    engine: SchemaEngine,
) -> Result<Vec<Origin>, String> {
    split_list(text)
        .iter()
        .map(|name| find(table, engine, name))
        .collect()
}

pub(super) fn origins_text(origins: &[Origin], table: &TableModel) -> String {
    table.column_names(origins).join(", ")
}

/// Strips a trailing ` DESC` or ` ASC` (any case).
fn direction(item: &str) -> (&str, bool) {
    let lower = item.to_ascii_lowercase();
    for (suffix, descending) in [(" desc", true), (" asc", false)] {
        if lower.ends_with(suffix) {
            return (item[..item.len() - suffix.len()].trim_end(), descending);
        }
    }
    (item, false)
}

/// True when `text` is one parenthesized group, `(…)`.
fn wrapped(text: &str) -> bool {
    if !(text.starts_with('(') && text.ends_with(')')) {
        return false;
    }
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    for (index, c) in text.char_indices() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 && index + 1 != text.len() {
                        return false;
                    }
                }
                _ => {}
            },
        }
    }
    depth == 0
}

/// Index members: `name`, `name DESC`, MySQL prefix `name(10)` and
/// expressions `(lower(email))`. A member equal to an existing one keeps its
/// collation, which the text form does not show.
pub(super) fn parse_members(
    text: &str,
    table: &TableModel,
    engine: SchemaEngine,
    existing: &[IndexMember],
) -> Result<Vec<IndexMember>, String> {
    let mut members = Vec::new();
    for item in split_list(text) {
        let (body, descending) = direction(&item);
        let (target, prefix_length) = if wrapped(body) {
            let inner = body[1..body.len() - 1].trim();
            if inner.is_empty() {
                return Err("An index expression is empty".into());
            }
            (IndexTarget::Expression(SqlFragment::new(inner)), None)
        } else if let Some(open) = body.find('(').filter(|_| body.ends_with(')')) {
            let length = body[open + 1..body.len() - 1]
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|length| *length > 0)
                .ok_or_else(|| format!("{body} is not a column with a prefix length"))?;
            (
                IndexTarget::Column(find(table, engine, &body[..open])?),
                Some(length),
            )
        } else {
            (IndexTarget::Column(find(table, engine, body)?), None)
        };
        let collation = existing
            .iter()
            .find(|member| {
                member.target == target
                    && member.descending == descending
                    && member.prefix_length == prefix_length
            })
            .and_then(|member| member.collation.clone());
        members.push(IndexMember {
            target,
            descending,
            prefix_length,
            collation,
        });
    }
    Ok(members)
}

pub(super) fn members_text(members: &[IndexMember], table: &TableModel) -> String {
    members
        .iter()
        .map(|member| {
            let mut text = match &member.target {
                IndexTarget::Column(origin) => table.column_name(*origin).unwrap_or("?").to_owned(),
                IndexTarget::Expression(sql) => format!("({sql})"),
            };
            if let Some(length) = member.prefix_length {
                text.push_str(&format!("({length})"));
            }
            if member.descending {
                text.push_str(" DESC");
            }
            text
        })
        .collect::<Vec<_>>()
        .join(", ")
}
