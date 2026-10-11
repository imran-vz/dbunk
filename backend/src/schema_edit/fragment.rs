//! Shape checks for user-edited SQL fragments (defaults, checks, generated
//! expressions, index predicates) and data types.
//!
//! A fragment must be exactly one expression: non-empty, bounded, free of
//! NUL, with balanced quotes, identifiers, comments and parentheses, and no
//! statement terminator outside a literal. This does not parse SQL; the
//! server rejects anything else at apply, inside the planned transaction
//! where the engine has one.
use super::model::SchemaEngine;

/// Bytes in one fragment.
pub const MAX_FRAGMENT_BYTES: usize = 16 * 1024;
/// Bytes in one data type.
pub const MAX_DATA_TYPE_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FragmentError {
    Empty,
    TooLong,
    Nul,
    UnterminatedQuote,
    UnterminatedComment,
    UnbalancedParentheses,
    StatementTerminator,
    /// Data types only: a character the type grammar never uses.
    InvalidCharacter(char),
}

impl std::fmt::Display for FragmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("is empty"),
            Self::TooLong => f.write_str("is too long"),
            Self::Nul => f.write_str("contains a NUL character"),
            Self::UnterminatedQuote => f.write_str("has an unterminated quote"),
            Self::UnterminatedComment => f.write_str("has an unterminated comment"),
            Self::UnbalancedParentheses => f.write_str("has unbalanced parentheses"),
            Self::StatementTerminator => f.write_str("contains a statement terminator"),
            Self::InvalidCharacter(c) => write!(f, "contains {c:?}"),
        }
    }
}

/// Checks one expression for `engine`'s lexical rules.
pub fn check_fragment(text: &str, engine: SchemaEngine) -> Result<(), FragmentError> {
    if text.trim().is_empty() {
        return Err(FragmentError::Empty);
    }
    if text.len() > MAX_FRAGMENT_BYTES {
        return Err(FragmentError::TooLong);
    }
    if text.contains('\0') {
        return Err(FragmentError::Nul);
    }
    scan(text, engine)
}

/// Checks a column data type: a fragment that also may not contain a comment
/// or a top-level keyword that would extend the column definition (the
/// planner appends NULL/DEFAULT/… itself). Only identifier characters,
/// spaces, parentheses, commas, brackets, dots, quotes and signs are allowed.
pub fn check_data_type(text: &str, engine: SchemaEngine) -> Result<(), FragmentError> {
    if text.trim().is_empty() {
        return Err(FragmentError::Empty);
    }
    if text.len() > MAX_DATA_TYPE_BYTES {
        return Err(FragmentError::TooLong);
    }
    if text.contains('\0') {
        return Err(FragmentError::Nul);
    }
    let mut quoted = None::<char>;
    for c in text.chars() {
        if let Some(q) = quoted {
            if c == q {
                quoted = None;
            }
            continue;
        }
        match c {
            '"' | '\'' | '`' => quoted = Some(c),
            c if c.is_alphanumeric() => {}
            ' ' | '_' | '(' | ')' | ',' | '[' | ']' | '.' | '+' | '-' => {}
            other => return Err(FragmentError::InvalidCharacter(other)),
        }
    }
    if text.contains("--") {
        return Err(FragmentError::InvalidCharacter('-'));
    }
    scan(text, engine)
}

fn scan(text: &str, engine: SchemaEngine) -> Result<(), FragmentError> {
    let bytes = text.as_bytes();
    let mut depth = 0i64;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\'' => {
                // MySQL also honours backslash escapes in strings by default.
                i = close_quote(bytes, i + 1, b'\'', engine == SchemaEngine::MySql)?;
            }
            b'"' => i = close_quote(bytes, i + 1, b'"', engine == SchemaEngine::MySql)?,
            b'`' if engine != SchemaEngine::Postgres => i = close_quote(bytes, i + 1, b'`', false)?,
            b'[' if engine == SchemaEngine::Sqlite => {
                i = bytes[i + 1..]
                    .iter()
                    .position(|c| *c == b']')
                    .map(|offset| i + 1 + offset + 1)
                    .ok_or(FragmentError::UnterminatedQuote)?;
            }
            b'$' if engine == SchemaEngine::Postgres => i = dollar_quote(bytes, i)?,
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                // A line comment runs to the end of the line. MySQL requires a
                // space after `--`, but treating `--x` as a comment is the
                // stricter reading.
                i = bytes[i..]
                    .iter()
                    .position(|c| *c == b'\n')
                    .map_or(bytes.len(), |offset| i + offset + 1);
            }
            b'#' if engine == SchemaEngine::MySql => {
                i = bytes[i..]
                    .iter()
                    .position(|c| *c == b'\n')
                    .map_or(bytes.len(), |offset| i + offset + 1);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => i = block_comment(bytes, i, engine)?,
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                if depth < 0 {
                    return Err(FragmentError::UnbalancedParentheses);
                }
                i += 1;
            }
            b';' => return Err(FragmentError::StatementTerminator),
            _ => i += 1,
        }
    }
    if depth != 0 {
        return Err(FragmentError::UnbalancedParentheses);
    }
    Ok(())
}

/// Index just past the closing quote. A doubled quote is an escaped quote.
fn close_quote(
    bytes: &[u8],
    mut i: usize,
    quote: u8,
    backslash: bool,
) -> Result<usize, FragmentError> {
    while i < bytes.len() {
        let b = bytes[i];
        if backslash && b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return Ok(i + 1);
        }
        i += 1;
    }
    Err(FragmentError::UnterminatedQuote)
}

/// PostgreSQL `$tag$ … $tag$`. A `$` that does not open a tag (a positional
/// parameter such as `$1`) is a plain character.
fn dollar_quote(bytes: &[u8], start: usize) -> Result<usize, FragmentError> {
    let mut end = start + 1;
    while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
        end += 1;
    }
    if end >= bytes.len()
        || bytes[end] != b'$'
        || bytes.get(start + 1).is_some_and(u8::is_ascii_digit)
    {
        return Ok(start + 1);
    }
    let tag = &bytes[start..=end];
    let body = end + 1;
    bytes[body..]
        .windows(tag.len())
        .position(|window| window == tag)
        .map(|offset| body + offset + tag.len())
        .ok_or(FragmentError::UnterminatedQuote)
}

/// PostgreSQL nests block comments; MySQL and SQLite do not.
fn block_comment(bytes: &[u8], start: usize, engine: SchemaEngine) -> Result<usize, FragmentError> {
    let nests = engine == SchemaEngine::Postgres;
    let mut depth = 1usize;
    let mut i = start + 2;
    while i + 1 < bytes.len() {
        if nests && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            depth += 1;
            i += 2;
        } else if bytes[i] == b'*' && bytes[i + 1] == b'/' {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return Ok(i);
            }
        } else {
            i += 1;
        }
    }
    Err(FragmentError::UnterminatedComment)
}

/// Canonical text for comparing data types: keywords case-folded outside
/// quotes, runs of whitespace collapsed, and no space around `(`, `)` or `,`.
/// `VARCHAR ( 10 )` and `varchar(10)` compare equal; `"MyType"` keeps its case.
pub fn normalize_type(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quoted = None::<char>;
    let mut pending_space = false;
    for c in text.trim().chars() {
        if let Some(q) = quoted {
            out.push(c);
            if c == q {
                quoted = None;
            }
            continue;
        }
        if c.is_whitespace() {
            pending_space = true;
            continue;
        }
        let tight = matches!(c, '(' | ')' | ',' | '[' | ']');
        if pending_space && !tight && !out.ends_with(['(', ',', '[']) && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        if matches!(c, '"' | '`') {
            quoted = Some(c);
            out.push(c);
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use SchemaEngine::*;

    #[test]
    fn accepts_single_expressions() {
        for (text, engine) in [
            ("now()", Postgres),
            ("'a;b'", Postgres),
            ("$$x;y$$", Postgres),
            ("$tag$ ; $tag$ || 'x'", Postgres),
            ("$1 + 2", Postgres),
            ("(a > 0) AND (b < 10)", Sqlite),
            ("[weird;name] > 0", Sqlite),
            ("`col;x` > 0", MySql),
            ("'it\\'s'", MySql),
            ("a /* note; */ + 1", Postgres),
            ("a /* outer /* inner */ ; */ + 1", Postgres),
            ("a -- trailing; comment", Sqlite),
        ] {
            assert_eq!(check_fragment(text, engine), Ok(()), "{text}");
        }
    }

    #[test]
    fn refuses_malformed_fragments() {
        assert_eq!(check_fragment("  ", Postgres), Err(FragmentError::Empty));
        assert_eq!(
            check_fragment("1; DROP TABLE t", Postgres),
            Err(FragmentError::StatementTerminator)
        );
        assert_eq!(
            check_fragment("'open", Sqlite),
            Err(FragmentError::UnterminatedQuote)
        );
        assert_eq!(
            check_fragment("(1", MySql),
            Err(FragmentError::UnbalancedParentheses)
        );
        assert_eq!(
            check_fragment("1)", MySql),
            Err(FragmentError::UnbalancedParentheses)
        );
        assert_eq!(
            check_fragment("1 /* x", Postgres),
            Err(FragmentError::UnterminatedComment)
        );
        assert_eq!(
            check_fragment("$a$ unterminated", Postgres),
            Err(FragmentError::UnterminatedQuote)
        );
        // MySQL does not nest comments: the first `*/` closes it.
        assert_eq!(
            check_fragment("1 /* a /* b */ ; */", MySql),
            Err(FragmentError::StatementTerminator)
        );
        assert_eq!(check_fragment("a\0", Sqlite), Err(FragmentError::Nul));
        assert_eq!(
            check_fragment(&"x".repeat(MAX_FRAGMENT_BYTES + 1), Sqlite),
            Err(FragmentError::TooLong)
        );
    }

    #[test]
    fn data_types_cannot_extend_the_column_definition() {
        assert_eq!(check_data_type("numeric(10, 2)", Postgres), Ok(()));
        assert_eq!(check_data_type("int[]", Postgres), Ok(()));
        assert_eq!(check_data_type("enum('a','b;c')", MySql), Ok(()));
        assert_eq!(
            check_data_type("timestamp with time zone", Postgres),
            Ok(())
        );
        assert!(check_data_type("int; drop table t", Postgres).is_err());
        assert!(check_data_type("int -- x", Sqlite).is_err());
        assert!(check_data_type("int /* x */", Sqlite).is_err());
        assert!(check_data_type("text = 1", Sqlite).is_err());
    }

    #[test]
    fn normalizes_types_for_comparison() {
        assert_eq!(normalize_type("VARCHAR ( 10 )"), "varchar(10)");
        assert_eq!(normalize_type("numeric(10 , 2)"), "numeric(10,2)");
        assert_eq!(
            normalize_type("Timestamp   WITH time zone"),
            "timestamp with time zone"
        );
        assert_eq!(normalize_type("\"MyType\""), "\"MyType\"");
        assert_eq!(normalize_type("int [ ]"), "int[]");
        assert_eq!(normalize_type("INT UNSIGNED"), "int unsigned");
    }
}
