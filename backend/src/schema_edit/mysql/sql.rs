//! MySQL text helpers shared by capture, planner and verification: literal
//! quoting, column definitions and type canonicalization.
use crate::schema_edit::{
    normalize_type, ColumnModel, DefaultValue, Identity, PlanError, ReferentialAction,
};

pub(super) fn ident(name: &str) -> String {
    crate::quote_backtick(name)
}

/// `` `database`.`table` ``.
pub(super) fn qualified(database: &str, table: &str) -> String {
    format!("{}.{}", ident(database), ident(table))
}

/// A MySQL string literal. Quotes are doubled and backslashes escaped, so
/// the text is exact whenever `NO_BACKSLASH_ESCAPES` is off; the executor
/// clears that mode on its connection.
pub(super) fn string_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            '\0' => out.push_str("\\0"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{1a}' => out.push_str("\\Z"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Charset, collation, engine and row format names are rendered bare, so
/// they must be plain words.
pub(super) fn plain_word(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn checked_word<'a>(what: &str, value: &'a str) -> Result<&'a str, PlanError> {
    if plain_word(value) {
        Ok(value)
    } else {
        Err(PlanError::Unsupported {
            reason: format!("{what} {value:?} is not a plain name"),
        })
    }
}

/// The base name of a type: `varchar(10)` → `varchar`, `int unsigned` →
/// `int`.
pub(super) fn type_base(data_type: &str) -> String {
    let normalized = normalize_type(data_type);
    normalized
        .split(['(', ' '])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Types that carry a character set and collation.
pub(super) fn is_textual(data_type: &str) -> bool {
    matches!(
        type_base(data_type).as_str(),
        "char"
            | "varchar"
            | "tinytext"
            | "text"
            | "mediumtext"
            | "longtext"
            | "enum"
            | "set"
            | "character"
            | "nchar"
            | "nvarchar"
    )
}

/// True when `text` is one parenthesized group: `(a + b)` but not
/// `(a) + (b)`.
pub(super) fn wrapped_in_parens(text: &str) -> bool {
    let text = text.trim();
    if !text.starts_with('(') || !text.ends_with(')') {
        return false;
    }
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && q != b'`' {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match b {
            b'\'' | b'"' | b'`' => quote = Some(b),
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && i + 1 != bytes.len() {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    depth == 0
}

fn is_number(text: &str) -> bool {
    let text = text.strip_prefix(['-', '+']).unwrap_or(text);
    !text.is_empty()
        && text.chars().any(|c| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '-' | '+'))
        && text.starts_with(|c: char| c.is_ascii_digit() || c == '.')
}

/// Defaults MySQL accepts without parentheses: literals and the
/// `CURRENT_TIMESTAMP` family.
fn bare_default(text: &str) -> bool {
    let lower: String = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if matches!(lower.as_str(), "null" | "true" | "false") || is_number(&lower) {
        return true;
    }
    if (lower.starts_with("b'") || lower.starts_with("x'")) && lower.ends_with('\'') {
        return true;
    }
    if lower.starts_with("0x") || lower.starts_with("0b") {
        return lower[2..].chars().all(|c| c.is_ascii_alphanumeric());
    }
    if lower.starts_with('\'') && lower.ends_with('\'') && lower.len() >= 2 {
        return true;
    }
    for name in ["current_timestamp", "localtimestamp", "localtime", "now"] {
        if let Some(rest) = lower.strip_prefix(name) {
            if rest.is_empty() {
                return name != "now";
            }
            if let Some(inner) = rest.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                return inner.chars().all(|c| c.is_ascii_digit());
            }
        }
    }
    false
}

/// An expression default as MySQL accepts it: wrapped in parentheses unless
/// it already is, or is a literal or `CURRENT_TIMESTAMP[(n)]`.
pub(super) fn default_expression(text: &str) -> String {
    let text = text.trim();
    if wrapped_in_parens(text) || bare_default(text) {
        text.to_owned()
    } else {
        format!("({text})")
    }
}

/// The whole column definition after its name, restated from the model so a
/// `MODIFY`/`CHANGE` never drops an attribute: type, charset, collation,
/// generation, nullability, default, `ON UPDATE`, `AUTO_INCREMENT`,
/// visibility and comment.
pub(super) fn column_definition(column: &ColumnModel) -> Result<String, PlanError> {
    let data_type = column.data_type.trim();
    let mut sql = data_type.to_owned();
    if is_textual(data_type) {
        if let Some(charset) = &column.extra.charset {
            sql.push_str(" CHARACTER SET ");
            sql.push_str(checked_word("Character set", charset)?);
        }
        if let Some(collation) = &column.collation {
            sql.push_str(" COLLATE ");
            sql.push_str(checked_word("Collation", collation)?);
        }
    }
    if let Some(generated) = &column.generated {
        sql.push_str(&format!(
            " GENERATED ALWAYS AS ({}) {}",
            generated.expression.as_str().trim(),
            if generated.stored {
                "STORED"
            } else {
                "VIRTUAL"
            }
        ));
    }
    sql.push_str(if column.nullable {
        " NULL"
    } else {
        " NOT NULL"
    });
    if column.generated.is_none() {
        match &column.default {
            DefaultValue::None => {}
            DefaultValue::Literal(value) => {
                sql.push_str(" DEFAULT ");
                sql.push_str(&string_literal(value));
            }
            DefaultValue::Expression(expression) => {
                sql.push_str(" DEFAULT ");
                sql.push_str(&default_expression(expression.as_str()));
            }
        }
        if let Some(on_update) = &column.extra.on_update {
            sql.push_str(" ON UPDATE ");
            sql.push_str(on_update.as_str().trim());
        }
        match column.identity {
            Identity::None => {}
            Identity::AutoIncrement => sql.push_str(" AUTO_INCREMENT"),
            Identity::Always | Identity::ByDefault => {
                return Err(PlanError::Unsupported {
                    reason: "Identity columns are PostgreSQL only; use AUTO_INCREMENT".into(),
                })
            }
        }
    }
    if column.extra.invisible {
        sql.push_str(" INVISIBLE");
    }
    if let Some(comment) = column.comment.as_deref().filter(|c| !c.is_empty()) {
        sql.push_str(" COMMENT ");
        sql.push_str(&string_literal(comment));
    }
    Ok(sql)
}

pub(super) fn referential_action(text: &str) -> Option<ReferentialAction> {
    match text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase()
        .as_str()
    {
        "NO ACTION" => Some(ReferentialAction::NoAction),
        "RESTRICT" => Some(ReferentialAction::Restrict),
        "CASCADE" => Some(ReferentialAction::Cascade),
        "SET NULL" => Some(ReferentialAction::SetNull),
        "SET DEFAULT" => Some(ReferentialAction::SetDefault),
        _ => None,
    }
}

/// InnoDB treats `NO ACTION` as `RESTRICT`; servers report either.
pub(super) fn same_action(a: ReferentialAction, b: ReferentialAction) -> bool {
    let fold = |action| match action {
        ReferentialAction::Restrict => ReferentialAction::NoAction,
        other => other,
    };
    fold(a) == fold(b)
}

const INTEGER_TYPES: [&str; 5] = ["tinyint", "smallint", "mediumint", "int", "bigint"];

/// Canonical type text for verification: aliases folded (`integer` → `int`,
/// `bool` → `tinyint(1)`, `numeric` → `decimal`), integer display widths
/// dropped (MySQL 8 omits them, MariaDB adds them) except `tinyint(1)`, and
/// the server's implicit arguments made explicit (`decimal` →
/// `decimal(10,0)`, `char` → `char(1)`).
pub(super) fn canonical_type(text: &str) -> String {
    let normalized = normalize_type(text);
    let (head, args, tail) = match normalized.find('(') {
        Some(open) => match normalized[open..].find(')') {
            Some(close) => (
                normalized[..open].trim().to_owned(),
                Some(normalized[open + 1..open + close].to_owned()),
                normalized[open + close + 1..].trim().to_owned(),
            ),
            None => (normalized.clone(), None, String::new()),
        },
        None => {
            // Multi-word bases first, then trailing attributes.
            let mut head = normalized.clone();
            let mut tail = String::new();
            for attribute in [" unsigned zerofill", " unsigned", " signed", " zerofill"] {
                if let Some(base) = normalized.strip_suffix(attribute) {
                    head = base.to_owned();
                    tail = attribute.trim().to_owned();
                    break;
                }
            }
            (head, None, tail)
        }
    };
    let head = match head.as_str() {
        "integer" => "int".to_owned(),
        "int1" => "tinyint".to_owned(),
        "int2" => "smallint".to_owned(),
        "int3" | "middleint" => "mediumint".to_owned(),
        "int4" => "int".to_owned(),
        "int8" => "bigint".to_owned(),
        "bool" | "boolean" => return "tinyint(1)".to_owned(),
        "numeric" | "dec" | "fixed" => "decimal".to_owned(),
        "double precision" | "real" | "float8" => "double".to_owned(),
        "float4" => "float".to_owned(),
        "character varying" | "nvarchar" | "national varchar" => "varchar".to_owned(),
        "character" | "nchar" | "national char" => "char".to_owned(),
        other => other.to_owned(),
    };
    let tail = match tail.as_str() {
        "signed" => String::new(),
        // ZEROFILL implies UNSIGNED.
        "zerofill" => "unsigned zerofill".to_owned(),
        other => other.to_owned(),
    };
    let args = match (head.as_str(), args) {
        ("tinyint", Some(args)) if args == "1" && tail.is_empty() => Some(args),
        (base, Some(_)) if INTEGER_TYPES.contains(&base) && !tail.contains("zerofill") => None,
        ("decimal", None) => Some("10,0".to_owned()),
        ("decimal", Some(args)) if !args.contains(',') => Some(format!("{args},0")),
        ("char" | "binary" | "bit", None) => Some("1".to_owned()),
        ("year", Some(args)) if args == "4" => None,
        (_, args) => args,
    };
    let mut out = head;
    if let Some(args) = args {
        out.push('(');
        out.push_str(&args);
        out.push(')');
    }
    if !tail.is_empty() {
        out.push(' ');
        out.push_str(&tail);
    }
    out
}

/// Expression text compared loosely: whitespace, quotes-escaping
/// backslashes and identifier backticks removed, case folded, charset
/// introducers dropped (the server stores `''` as `_utf8mb4''`), and
/// redundant outer parentheses stripped. Used only where the server's
/// renderings of one expression differ (catalog versus `SHOW CREATE TABLE`,
/// or the text a user typed versus what the server stored).
pub(super) fn loose_expression(text: &str) -> String {
    let folded: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\\' && *c != '`')
        .collect::<String>()
        .to_lowercase();
    let mut out = without_introducers(&folded);
    while wrapped_in_parens(&out) {
        out = out[1..out.len() - 1].to_owned();
    }
    out
}

/// Whether a recaptured default is the one the draft asked for, allowing
/// only for how the server re-renders a value of `data_type`: numbers
/// (`1` is stored as `1.00` in a `decimal(8,2)`), times (`2024-01-02` as
/// `2024-01-02 00:00:00`), `ENUM`/`SET` member case, `CHAR` padding, and
/// spellings of the current time (`now()` as `CURRENT_TIMESTAMP`). The
/// value itself must match.
pub(super) fn same_default(data_type: &str, wanted: &DefaultValue, found: &DefaultValue) -> bool {
    match (wanted, found) {
        (DefaultValue::None, DefaultValue::None) => true,
        (DefaultValue::Literal(a), DefaultValue::Literal(b)) => same_literal(data_type, a, b),
        (DefaultValue::Expression(a), DefaultValue::Expression(b)) => {
            default_expression_key(a.as_str()) == default_expression_key(b.as_str())
        }
        _ => false,
    }
}

fn same_literal(data_type: &str, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match type_base(data_type).as_str() {
        "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "decimal"
        | "numeric" | "dec" | "fixed" | "float" | "double" | "real" => {
            match (decimal_key(a), decimal_key(b)) {
                (Some(a), Some(b)) => a == b,
                _ => matches!(
                    (a.trim().parse::<f64>(), b.trim().parse::<f64>()),
                    (Ok(a), Ok(b)) if a == b
                ),
            }
        }
        "date" | "datetime" | "timestamp" | "time" => time_key(a) == time_key(b),
        "enum" | "set" => a.eq_ignore_ascii_case(b),
        "char" | "character" | "nchar" => a.trim_end_matches(' ') == b.trim_end_matches(' '),
        _ => false,
    }
}

/// A plain decimal (`-007.50`) as sign, integer digits and fraction digits
/// without redundant zeros (`-7.5`); `None` for anything else.
fn decimal_key(text: &str) -> Option<String> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (whole, fraction) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fraction.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let whole = whole.trim_start_matches('0');
    let fraction = fraction.trim_end_matches('0');
    let mut key = String::new();
    if negative && !(whole.is_empty() && fraction.is_empty()) {
        key.push('-');
    }
    key.push_str(if whole.is_empty() { "0" } else { whole });
    if !fraction.is_empty() {
        key.push('.');
        key.push_str(fraction);
    }
    Some(key)
}

/// A temporal literal without zero fractional seconds or a midnight time.
fn time_key(text: &str) -> &str {
    let mut text = text.trim();
    if let Some((head, fraction)) = text.rsplit_once('.') {
        if !fraction.is_empty() && fraction.bytes().all(|b| b == b'0') {
            text = head;
        }
    }
    text.strip_suffix(" 00:00:00").unwrap_or(text)
}

/// [`loose_expression`] with the server's spellings of the current date and
/// time folded to one.
fn default_expression_key(text: &str) -> String {
    let loose = loose_expression(text);
    let (name, precision) = match loose.split_once('(') {
        Some((name, rest)) => match rest.strip_suffix(')') {
            Some(arguments) if arguments.bytes().all(|b| b.is_ascii_digit()) => (name, arguments),
            _ => return loose,
        },
        None => (loose.as_str(), ""),
    };
    let canonical = match name {
        "now" | "current_timestamp" | "localtime" | "localtimestamp" => "current_timestamp",
        "curdate" | "current_date" if precision.is_empty() => "curdate",
        "curtime" | "current_time" => "curtime",
        _ => return loose,
    };
    format!("{canonical}({precision})")
}

/// Drops `_charset` immediately before a string literal. The introducer
/// must start a token: `my_col'` (an identifier) is never an introducer.
fn without_introducers(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < bytes.len() {
        let starts_token = at == 0
            || !(bytes[at - 1].is_ascii_alphanumeric()
                || bytes[at - 1] == b'_'
                || bytes[at - 1] == b'$'
                || bytes[at - 1] == b'\'');
        if bytes[at] == b'_' && starts_token {
            let name = bytes[at + 1..]
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric())
                .count();
            if name > 0 && bytes.get(at + 1 + name) == Some(&b'\'') {
                at += 1 + name;
                continue;
            }
        }
        let next = text[at..].chars().next().map_or(1, char::len_utf8);
        out.push_str(&text[at..at + next]);
        at += next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema_edit::{ColumnExtra, DraftId, Generated, Origin, SqlFragment};

    #[test]
    fn string_literals_escape_quotes_and_backslashes() {
        assert_eq!(string_literal("plain"), "'plain'");
        assert_eq!(string_literal("it's"), "'it''s'");
        assert_eq!(string_literal("a\\b"), "'a\\\\b'");
        assert_eq!(string_literal("x\ny\0"), "'x\\ny\\0'");
    }

    #[test]
    fn defaults_compare_by_value_across_server_renderings() {
        let literal = |text: &str| DefaultValue::Literal(text.into());
        let expression = |text: &str| DefaultValue::Expression(SqlFragment::new(text));
        // Re-rendered, same value.
        assert!(same_default(
            "decimal(8,2)",
            &literal("1"),
            &literal("1.00")
        ));
        assert!(same_default(
            "decimal(8,4)",
            &literal("0.00"),
            &literal("0.0000")
        ));
        assert!(same_default("int", &literal("+007"), &literal("7")));
        assert!(same_default("double", &literal("1e3"), &literal("1000")));
        assert!(same_default(
            "datetime",
            &literal("2024-01-02"),
            &literal("2024-01-02 00:00:00")
        ));
        assert!(same_default(
            "datetime(3)",
            &literal("2024-01-02 10:00:00"),
            &literal("2024-01-02 10:00:00.000")
        ));
        assert!(same_default("enum('a','B')", &literal("b"), &literal("B")));
        assert!(same_default("char(4)", &literal("ab  "), &literal("ab")));
        assert!(same_default(
            "datetime",
            &expression("now()"),
            &expression("CURRENT_TIMESTAMP")
        ));
        assert!(same_default(
            "datetime(3)",
            &expression("now(3)"),
            &expression("current_timestamp(3)")
        ));
        assert!(same_default(
            "char(36)",
            &expression("uuid()"),
            &expression("(uuid())")
        ));
        assert!(same_default(
            "varchar(9)",
            &expression("concat('a','b')"),
            &expression("concat(_utf8mb4'a',_utf8mb4'b')")
        ));
        // A different value of the same kind is a mismatch.
        assert!(!same_default("int", &literal("1"), &literal("2")));
        assert!(!same_default(
            "decimal(8,2)",
            &literal("1.5"),
            &literal("1.50001")
        ));
        assert!(!same_default("varchar(9)", &literal("a"), &literal("A")));
        assert!(!same_default("varchar(9)", &literal("a "), &literal("a")));
        assert!(!same_default(
            "datetime",
            &literal("2024-01-02"),
            &literal("2024-01-03 00:00:00")
        ));
        assert!(!same_default(
            "char(36)",
            &expression("uuid()"),
            &expression("now()")
        ));
        assert!(!same_default(
            "datetime(3)",
            &expression("now(3)"),
            &expression("now(6)")
        ));
        assert!(!same_default("int", &literal("1"), &expression("1")));
        assert!(!same_default("int", &literal("1"), &DefaultValue::None));
    }

    #[test]
    fn expression_defaults_are_parenthesized_unless_bare() {
        assert_eq!(default_expression("CURRENT_TIMESTAMP"), "CURRENT_TIMESTAMP");
        assert_eq!(
            default_expression("CURRENT_TIMESTAMP(3)"),
            "CURRENT_TIMESTAMP(3)"
        );
        assert_eq!(
            default_expression("current_timestamp()"),
            "current_timestamp()"
        );
        assert_eq!(default_expression("(uuid())"), "(uuid())");
        assert_eq!(default_expression("uuid()"), "(uuid())");
        assert_eq!(default_expression("b'1'"), "b'1'");
        assert_eq!(default_expression("-1.5"), "-1.5");
        assert_eq!(default_expression("(a) + (b)"), "((a) + (b))");
        assert_eq!(default_expression("now()"), "now()");
    }

    #[test]
    fn canonical_types_fold_aliases_and_display_widths() {
        assert_eq!(canonical_type("INT(11)"), "int");
        assert_eq!(canonical_type("integer"), "int");
        assert_eq!(canonical_type("int(10) unsigned"), "int unsigned");
        assert_eq!(canonical_type("bigint(20)"), "bigint");
        assert_eq!(canonical_type("bool"), "tinyint(1)");
        assert_eq!(canonical_type("tinyint(1)"), "tinyint(1)");
        assert_eq!(canonical_type("tinyint(4)"), "tinyint");
        assert_eq!(canonical_type("numeric"), "decimal(10,0)");
        assert_eq!(canonical_type("decimal(8)"), "decimal(8,0)");
        assert_eq!(canonical_type("DECIMAL(8, 2)"), "decimal(8,2)");
        assert_eq!(canonical_type("char"), "char(1)");
        assert_eq!(canonical_type("varchar(20)"), "varchar(20)");
        assert_eq!(
            canonical_type("int(5) zerofill"),
            "int(5) unsigned zerofill"
        );
        assert_eq!(canonical_type("ENUM('a', 'b')"), "enum('a','b')");
    }

    #[test]
    fn loose_expressions_ignore_escaping_and_outer_parentheses() {
        assert_eq!(
            loose_expression("concat(`a`,_utf8mb4\\' \\',`b`)"),
            loose_expression("concat(`a`,_utf8mb4' ',`b`)")
        );
        assert_eq!(loose_expression("((`a` > 0))"), loose_expression("`a` > 0"));
        // The server adds charset introducers to literals a user typed bare.
        assert_eq!(
            loose_expression("(`name` <> _utf8mb4\\'\\')"),
            loose_expression("name <> ''")
        );
        assert_eq!(
            loose_expression("_latin1'a' = x"),
            loose_expression("'a' = x")
        );
        // An identifier ending in a word is not an introducer.
        assert_ne!(loose_expression("a_b'c'"), loose_expression("a'c'"));
        assert_ne!(loose_expression("(a) + (b)"), loose_expression("a + b"));
    }

    #[test]
    fn column_definitions_restate_every_attribute() {
        let mut column = ColumnModel::new(Origin::New(DraftId(1)), "note", "varchar(40)");
        column.nullable = false;
        column.default = DefaultValue::Literal("n/a".into());
        column.collation = Some("utf8mb4_bin".into());
        column.comment = Some("Free text".into());
        column.extra = ColumnExtra {
            charset: Some("utf8mb4".into()),
            on_update: None,
            invisible: true,
        };
        assert_eq!(
            column_definition(&column).unwrap(),
            "varchar(40) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin NOT NULL DEFAULT 'n/a' \
             INVISIBLE COMMENT 'Free text'"
        );
        // A non-textual type never carries a charset.
        column.data_type = "int".into();
        column.default = DefaultValue::None;
        column.extra.invisible = false;
        column.comment = None;
        assert_eq!(column_definition(&column).unwrap(), "int NOT NULL");
        // Generated columns take no default.
        column.generated = Some(Generated {
            expression: SqlFragment::new("`a` + 1"),
            stored: true,
        });
        column.default = DefaultValue::Literal("1".into());
        assert_eq!(
            column_definition(&column).unwrap(),
            "int GENERATED ALWAYS AS (`a` + 1) STORED NOT NULL"
        );
        column.generated = None;
        column.collation = Some("bad name".into());
        column.data_type = "text".into();
        assert!(column_definition(&column).is_err());
    }
}
