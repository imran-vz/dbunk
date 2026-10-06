//! Index and enum draft vocabulary. Parsing here only shapes the typed
//! operation and reports mistakes inline; the backend still validates every
//! expression as one fragment and regenerates the SQL.
use dbunk_lib::backend::object_ddl::{MAX_OBJECT_DDL_INDEX_COLUMNS, ObjectDdlIndexColumn};

/// PostgreSQL's built-in index access methods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IndexMethod {
    #[default]
    Btree,
    Hash,
    Gist,
    Gin,
    Brin,
    Spgist,
}
impl IndexMethod {
    pub const ALL: [Self; 6] = [
        Self::Btree,
        Self::Hash,
        Self::Gist,
        Self::Gin,
        Self::Brin,
        Self::Spgist,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Btree => "btree",
            Self::Hash => "hash",
            Self::Gist => "gist",
            Self::Gin => "gin",
            Self::Brin => "brin",
            Self::Spgist => "spgist",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|method| method.as_str() == value)
    }
    pub fn next(self) -> Self {
        let at = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(at + 1) % Self::ALL.len()]
    }
    /// Only btree supports UNIQUE and per-column ordering.
    pub fn ordered(self) -> bool {
        self == Self::Btree
    }
    /// hash and spgist indexes are single-column.
    pub fn multicolumn(self) -> bool {
        !matches!(self, Self::Hash | Self::Spgist)
    }
}

/// Where `ALTER TYPE ... ADD VALUE` places the new label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EnumPlacement {
    #[default]
    End,
    Before,
    After,
}
impl EnumPlacement {
    pub fn next(self) -> Self {
        match self {
            Self::End => Self::Before,
            Self::Before => Self::After,
            Self::After => Self::End,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::End => "at the end",
            Self::Before => "BEFORE a label",
            Self::After => "AFTER a label",
        }
    }
}

const MAX_EXPRESSION_BYTES: usize = 1024;

/// Splits `a, lower(b) DESC` into ordered columns. Commas inside parentheses
/// or quotes do not split; a trailing `ASC`/`DESC` word sets the direction.
pub fn parse_index_columns(text: &str) -> Result<Vec<ObjectDdlIndexColumn>, &'static str> {
    if text.trim().is_empty() {
        return Err("Add at least one index column");
    }
    if text.contains('\0') {
        return Err("NUL is unsupported in index columns");
    }
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut start = 0;
    for (at, character) in text.char_indices() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None => match character {
                '\'' | '"' => quote = Some(character),
                '(' => depth += 1,
                ')' => {
                    depth = depth
                        .checked_sub(1)
                        .ok_or("Unbalanced parentheses in index columns")?
                }
                ',' if depth == 0 => {
                    items.push(&text[start..at]);
                    start = at + 1;
                }
                _ => {}
            },
        }
    }
    if quote.is_some() {
        return Err("Unclosed quote in index columns");
    }
    if depth != 0 {
        return Err("Unbalanced parentheses in index columns");
    }
    items.push(&text[start..]);
    if items.len() > MAX_OBJECT_DDL_INDEX_COLUMNS {
        return Err("An index takes at most 16 columns");
    }
    let mut columns: Vec<ObjectDdlIndexColumn> = Vec::with_capacity(items.len());
    let mut keys: Vec<String> = Vec::with_capacity(items.len());
    for item in items {
        if item.trim().is_empty() {
            return Err("Remove the empty entry between commas");
        }
        let (expression, descending) = split_direction(item);
        if expression.is_empty() {
            return Err("Each index column needs a column or expression before ASC/DESC");
        }
        if expression.len() > MAX_EXPRESSION_BYTES {
            return Err("Index column expressions are limited to 1 KiB");
        }
        let key = column_key(expression);
        if keys.contains(&key) {
            return Err("Each index column may appear only once");
        }
        keys.push(key);
        columns.push(ObjectDdlIndexColumn {
            expression: expression.to_owned(),
            descending,
        });
    }
    Ok(columns)
}

fn split_direction(item: &str) -> (&str, bool) {
    let item = item.trim();
    if item.eq_ignore_ascii_case("desc") || item.eq_ignore_ascii_case("asc") {
        return ("", false);
    }
    if let Some((head, tail)) = item.rsplit_once(char::is_whitespace) {
        if tail.eq_ignore_ascii_case("desc") {
            return (head.trim_end(), true);
        }
        if tail.eq_ignore_ascii_case("asc") {
            return (head.trim_end(), false);
        }
    }
    (item, false)
}

/// Identity for duplicate detection: whitespace collapsed; unquoted text
/// folds to lower case like PostgreSQL; `"name"` equals `name` when the
/// quoted name is already in folded form.
fn column_key(expression: &str) -> String {
    let compact = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(inner) = compact
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .filter(|inner| {
            !inner.is_empty()
                && inner
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
    {
        return inner.to_owned();
    }
    if compact.contains('"') || compact.contains('\'') {
        compact
    } else {
        compact.to_lowercase()
    }
}

/// The editable column list for a restored operation.
pub fn index_columns_text(columns: &[ObjectDdlIndexColumn]) -> String {
    columns
        .iter()
        .map(|column| {
            if column.descending {
                format!("{} DESC", column.expression)
            } else {
                column.expression.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Mirrors the backend's derivation (`<table>_<columns>_idx`, truncated to
/// 63 bytes) so the name the draft claims absent is visible before review.
pub fn derived_index_name(table: &str, columns: &[ObjectDdlIndexColumn]) -> String {
    let mut parts = vec![identifierish(table)];
    parts.extend(columns.iter().map(|c| identifierish(&c.expression)));
    parts.push("idx".to_owned());
    let joined = parts.join("_");
    let mut end = joined.len().min(63);
    while !joined.is_char_boundary(end) {
        end -= 1;
    }
    joined[..end].to_owned()
}

fn identifierish(value: &str) -> String {
    let simplified = value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = simplified.trim_matches('_');
    if trimmed.is_empty() {
        "expr".to_owned()
    } else {
        trimmed.to_owned()
    }
}
