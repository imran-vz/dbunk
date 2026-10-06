#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlIdentifier {
    pub(crate) value: String,
    pub(crate) quoted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SqlToken {
    Identifier(SqlIdentifier),
    Symbol(char),
    Opaque,
}

/// A token with the byte range it was lexed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SpannedToken {
    pub(crate) token: SqlToken,
    pub(crate) start: usize,
    pub(crate) end: usize,
}

pub(crate) fn lex_sql(sql: &str) -> Result<Vec<SqlToken>, ()> {
    Ok(lex_sql_spanned(sql)?
        .into_iter()
        .map(|spanned| spanned.token)
        .collect())
}

/// The one scan behind every caller, so a classifier and a span-reading
/// caller can never disagree about where a string or comment ends.
pub(crate) fn lex_sql_spanned(sql: &str) -> Result<Vec<SpannedToken>, ()> {
    lex_sql_spanned_bounded(sql, usize::MAX).map_err(|_| ())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlLexError {
    InvalidSql,
    TooManyTokens,
}
impl From<()> for SqlLexError {
    fn from(_: ()) -> Self {
        Self::InvalidSql
    }
}

/// The SQL dialect whose lexical rules decide where strings, comments, and
/// statements end. Each engine must be lexed with its own rules: a construct
/// that is a comment or string in one dialect can hide a whole statement from
/// another dialect's scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SqlDialect {
    /// Nested block comments, dollar quoting, `E'...'` escape strings, and
    /// double-quoted identifiers.
    Postgres,
    /// `#` and `-- ` line comments, flat block comments, `/*! ... */`
    /// executable comments, backtick identifiers, and backslash escapes in
    /// both quote styles (unless `NO_BACKSLASH_ESCAPES`).
    MySql,
    /// Flat block comments, backtick and `[...]` identifiers, and no
    /// backslash escapes.
    Sqlite,
}

/// [`lex_sql_spanned`] under a specific dialect's lexical rules.
pub(crate) fn lex_sql_spanned_dialect(
    sql: &str,
    dialect: SqlDialect,
) -> Result<Vec<SpannedToken>, ()> {
    lex_dialect_bounded(sql, usize::MAX, dialect).map_err(|_| ())
}

/// Whether a token is the opening marker of a MySQL executable comment
/// (`/*!...*/` or MariaDB `/*M!...*/`). Its body is lexed as ordinary SQL, but
/// whether the server runs it depends on the server version, so a statement
/// holding one cannot be classified from its visible tokens. No other token
/// starts with `/*`: ordinary comments are never emitted.
pub(crate) fn is_executable_comment(sql: &str, spanned: &SpannedToken) -> bool {
    spanned.token == SqlToken::Opaque
        && sql
            .as_bytes()
            .get(spanned.start..)
            .is_some_and(|rest| rest.starts_with(b"/*"))
}

/// Same tokenization, with refusal before a token beyond the caller's cap can
/// grow the output vector. Whitespace and comments consume no token allowance.
pub(crate) fn lex_sql_spanned_bounded(
    sql: &str,
    maximum_tokens: usize,
) -> Result<Vec<SpannedToken>, SqlLexError> {
    lex_dialect_bounded(sql, maximum_tokens, SqlDialect::Postgres)
}

fn lex_dialect_bounded(
    sql: &str,
    maximum_tokens: usize,
    dialect: SqlDialect,
) -> Result<Vec<SpannedToken>, SqlLexError> {
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    // MySQL only: inside `/*! ... */`, whose body the server lexes as
    // ordinary SQL until a token-level `*/`.
    let mut in_executable_comment = false;
    while index < bytes.len() {
        let start = index;
        let token = match bytes[index] {
            byte if byte.is_ascii_whitespace() => {
                index += 1;
                continue;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-')
                && (dialect != SqlDialect::MySql
                    || mysql_dash_comment_follows(bytes.get(index + 2))) =>
            {
                index = line_comment_end(bytes, index + 2);
                continue;
            }
            b'#' if dialect == SqlDialect::MySql => {
                index = line_comment_end(bytes, index + 1);
                continue;
            }
            b'*' if in_executable_comment && bytes.get(index + 1) == Some(&b'/') => {
                index += 2;
                in_executable_comment = false;
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => match dialect {
                SqlDialect::Postgres => {
                    index = lex_block_comment(bytes, index + 2)?;
                    continue;
                }
                // SQLite comments do not nest, and an unterminated one runs
                // to the end of input.
                SqlDialect::Sqlite => {
                    index = flat_block_comment_end(bytes, index + 2).unwrap_or(bytes.len());
                    continue;
                }
                SqlDialect::MySql => {
                    if let Some(body) = executable_comment_body(bytes, index) {
                        if in_executable_comment {
                            return Err(SqlLexError::InvalidSql);
                        }
                        in_executable_comment = true;
                        index = body;
                        SqlToken::Opaque
                    } else {
                        // MySQL comments do not nest. Optimizer hints
                        // (`/*+ ... */`) also end at the first `*/` and
                        // cannot hold a statement.
                        index = flat_block_comment_end(bytes, index + 2)
                            .ok_or(SqlLexError::InvalidSql)?;
                        continue;
                    }
                }
            },
            b'\'' => {
                index = match dialect {
                    SqlDialect::Postgres => lex_single_quote(bytes, index + 1, false)?,
                    SqlDialect::MySql => lex_ambiguous_quote(bytes, index + 1, b'\'')?,
                    SqlDialect::Sqlite => scan_quoted(bytes, index + 1, b'\'', false)?,
                };
                SqlToken::Opaque
            }
            // A MySQL double-quoted string, or an identifier under
            // ANSI_QUOTES. Either way its end must not depend on the mode.
            b'"' if dialect == SqlDialect::MySql => {
                index = lex_ambiguous_quote(bytes, index + 1, b'"')?;
                SqlToken::Opaque
            }
            b'"' => {
                let (value, next) = lex_quoted_identifier(sql, index + 1, b'"')?;
                index = next;
                SqlToken::Identifier(SqlIdentifier {
                    value,
                    quoted: true,
                })
            }
            b'`' if dialect != SqlDialect::Postgres => {
                let (value, next) = lex_quoted_identifier(sql, index + 1, b'`')?;
                index = next;
                SqlToken::Identifier(SqlIdentifier {
                    value,
                    quoted: true,
                })
            }
            // SQLite `[name]` identifiers have no escape: the first `]` ends.
            b'[' if dialect == SqlDialect::Sqlite => {
                let close = bytes[index + 1..]
                    .iter()
                    .position(|byte| *byte == b']')
                    .map(|offset| index + 1 + offset)
                    .ok_or(SqlLexError::InvalidSql)?;
                let value = sql[index + 1..close].to_owned();
                index = close + 1;
                SqlToken::Identifier(SqlIdentifier {
                    value,
                    quoted: true,
                })
            }
            b'$' if dialect == SqlDialect::Postgres => {
                index = lex_dollar(bytes, index)?;
                SqlToken::Opaque
            }
            byte if is_identifier_start(byte) => {
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                let value = &sql[start..index];
                if dialect == SqlDialect::Postgres
                    && value.eq_ignore_ascii_case("e")
                    && bytes.get(index) == Some(&b'\'')
                {
                    index = lex_single_quote(bytes, index + 1, true)?;
                    SqlToken::Opaque
                } else {
                    SqlToken::Identifier(SqlIdentifier {
                        value: value.into(),
                        quoted: false,
                    })
                }
            }
            byte @ (b'(' | b')' | b'[' | b']' | b',' | b'.' | b';' | b'*') => {
                index += 1;
                SqlToken::Symbol(char::from(byte))
            }
            byte if byte.is_ascii() => {
                index += 1;
                SqlToken::Opaque
            }
            _ => return Err(SqlLexError::InvalidSql),
        };
        if tokens.len() == maximum_tokens {
            return Err(SqlLexError::TooManyTokens);
        }
        tokens.push(SpannedToken {
            token,
            start,
            end: index,
        });
    }
    if in_executable_comment {
        return Err(SqlLexError::InvalidSql);
    }
    Ok(tokens)
}

/// MySQL starts a `--` comment only when whitespace or a control character
/// (or the end of input) follows, so `1--1` is arithmetic.
fn mysql_dash_comment_follows(next: Option<&u8>) -> bool {
    next.is_none_or(|byte| *byte <= b' ' || *byte == 0x7f)
}

fn line_comment_end(bytes: &[u8], mut index: usize) -> usize {
    while index < bytes.len() && bytes[index] != b'\n' {
        index += 1;
    }
    index
}

/// The end of a non-nesting block comment whose body starts at `index`.
fn flat_block_comment_end(bytes: &[u8], index: usize) -> Option<usize> {
    bytes
        .get(index..)?
        .windows(2)
        .position(|window| window == b"*/")
        .map(|offset| index + offset + 2)
}

/// Where the body of a MySQL `/*!NNNNN` or MariaDB `/*M!NNNNN` executable
/// comment starts, after the optional version digits.
fn executable_comment_body(bytes: &[u8], index: usize) -> Option<usize> {
    let rest = &bytes[index..];
    let mut body = if rest.starts_with(b"/*!") {
        index + 3
    } else if rest.starts_with(b"/*M!") {
        index + 4
    } else {
        return None;
    };
    while bytes.get(body).is_some_and(u8::is_ascii_digit) {
        body += 1;
    }
    Some(body)
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

fn lex_single_quote(bytes: &[u8], index: usize, escapes: bool) -> Result<usize, ()> {
    if escapes {
        return scan_quoted(bytes, index, b'\'', true);
    }
    // Plain-string backslash semantics depend on standard_conforming_strings,
    // which the lexer cannot observe.
    lex_ambiguous_quote(bytes, index, b'\'')
}

/// Accepts a quoted literal only when both backslash interpretations end at
/// the same byte, so a backslash can never move a statement or fragment
/// boundary whichever setting the server uses (PostgreSQL
/// standard_conforming_strings, MySQL NO_BACKSLASH_ESCAPES and ANSI_QUOTES).
fn lex_ambiguous_quote(bytes: &[u8], index: usize, quote: u8) -> Result<usize, ()> {
    let literal_end = scan_quoted(bytes, index, quote, false)?;
    let escaped_end = scan_quoted(bytes, index, quote, true)?;
    if literal_end == escaped_end {
        Ok(literal_end)
    } else {
        Err(())
    }
}

fn scan_quoted(bytes: &[u8], mut index: usize, quote: u8, escapes: bool) -> Result<usize, ()> {
    while index < bytes.len() {
        match bytes[index] {
            byte if byte == quote && bytes.get(index + 1) == Some(&quote) => index += 2,
            byte if byte == quote => return Ok(index + 1),
            b'\\' if escapes && index + 1 < bytes.len() => index += 2,
            _ => index += 1,
        }
    }
    Err(())
}

fn lex_quoted_identifier(sql: &str, mut index: usize, quote: u8) -> Result<(String, usize), ()> {
    let bytes = sql.as_bytes();
    let mut value = String::new();
    let mut segment = index;
    while index < bytes.len() {
        if bytes[index] != quote {
            index += 1;
            continue;
        }
        value.push_str(&sql[segment..index]);
        if bytes.get(index + 1) == Some(&quote) {
            value.push(char::from(quote));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_lexer_matches_shared_tokens_and_refuses_at_exact_cap() {
        let sql = "/* ignored */ SELECT :name, ':hidden', $$:also_hidden$$ -- ignored\n";
        let expected = lex_sql_spanned(sql).unwrap();
        assert_eq!(
            lex_sql_spanned_bounded(sql, expected.len()).unwrap(),
            expected
        );
        assert_eq!(
            lex_sql_spanned_bounded(sql, expected.len() - 1),
            Err(SqlLexError::TooManyTokens)
        );
        assert_eq!(lex_sql_spanned_bounded(" -- comment", 0).unwrap(), vec![]);
        assert_eq!(
            lex_sql_spanned_bounded("SELECT", 0),
            Err(SqlLexError::TooManyTokens)
        );
        assert_eq!(
            lex_sql_spanned_bounded("'unterminated", 8),
            Err(SqlLexError::InvalidSql)
        );
    }

    fn identifier(value: &str) -> SqlToken {
        SqlToken::Identifier(SqlIdentifier {
            value: value.into(),
            quoted: false,
        })
    }

    #[test]
    fn plain_string_backslash_is_accepted_when_both_semantics_agree() {
        let tokens = lex_sql(r"email ~ '^[^@]+@[^@]+\.[a-z]{2,}$' AND x").expect("regex literal");
        assert_eq!(
            tokens,
            vec![
                identifier("email"),
                SqlToken::Opaque,
                SqlToken::Opaque,
                identifier("AND"),
                identifier("x"),
            ]
        );
        assert!(lex_sql(r"SELECT 'C:\temp'").is_ok());
        assert!(lex_sql(r"SELECT '\\'").is_ok());
    }

    #[test]
    fn plain_string_backslash_is_rejected_when_the_boundary_moves() {
        // standard_conforming_strings=off would end this literal after `a'`.
        assert!(lex_sql(r"'a\'' ; DROP TABLE t; --'").is_err());
        assert!(lex_sql(r"'abc\'").is_err());
        // E strings always escape.
        assert!(lex_sql(r"E'a\'b'").is_ok());
    }

    #[test]
    fn spans_cover_each_token_and_skip_comments_and_whitespace() {
        let sql = "SELECT 'é' /* c */ , \"q\"\"x\" -- tail\n $1 $t$ ; $t$ e'\\''";
        let spans = lex_sql_spanned(sql).expect("spanned scan");
        let texts = spans
            .iter()
            .map(|spanned| &sql[spanned.start..spanned.end])
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            [
                "SELECT",
                "'é'",
                ",",
                "\"q\"\"x\"",
                "$1",
                "$t$ ; $t$",
                "e'\\''"
            ]
        );
        assert_eq!(
            spans
                .iter()
                .map(|spanned| spanned.token.clone())
                .collect::<Vec<_>>(),
            lex_sql(sql).expect("projection")
        );
        assert!(lex_sql_spanned("SELECT é").is_err());
    }

    #[test]
    fn brackets_and_dollar_identifiers_are_lexed_structurally() {
        let tokens = lex_sql("ARRAY[1, 2]").expect("array literal");
        assert_eq!(tokens[1], SqlToken::Symbol('['));
        assert_eq!(tokens.last(), Some(&SqlToken::Symbol(']')));
        let tokens = lex_sql("d$x$ , y").expect("dollar identifier");
        assert_eq!(tokens[0], identifier("d$x$"));
        assert_eq!(tokens[1], SqlToken::Symbol(','));
    }

    fn texts(sql: &str, dialect: SqlDialect) -> Vec<&str> {
        lex_sql_spanned_dialect(sql, dialect)
            .expect("dialect scan")
            .into_iter()
            .map(|spanned| &sql[spanned.start..spanned.end])
            .collect()
    }

    fn quoted(value: &str) -> SqlToken {
        SqlToken::Identifier(SqlIdentifier {
            value: value.into(),
            quoted: true,
        })
    }

    #[test]
    fn mysql_dash_comment_needs_trailing_whitespace_or_control() {
        assert_eq!(
            texts("SELECT 1--1;", SqlDialect::MySql),
            ["SELECT", "1", "-", "-", "1", ";"]
        );
        assert_eq!(
            texts("SELECT 1-- c ;\nX --\tc\n Y --", SqlDialect::MySql),
            ["SELECT", "1", "X", "Y"]
        );
        // PostgreSQL keeps treating any `--` as a comment.
        assert_eq!(texts("SELECT 1--1;", SqlDialect::Postgres), ["SELECT", "1"]);
    }

    #[test]
    fn mysql_hash_comments_and_flat_block_comments() {
        assert_eq!(
            texts("SELECT 1 # '\n; X", SqlDialect::MySql),
            ["SELECT", "1", ";", "X"]
        );
        assert_eq!(
            texts("/* a /* b */ X; /* */", SqlDialect::MySql),
            ["X", ";"]
        );
        assert_eq!(texts("/*+ hint */ X", SqlDialect::MySql), ["X"]);
        assert!(lex_sql_spanned_dialect("/* open", SqlDialect::MySql).is_err());
        // PostgreSQL nests, so the same text is one comment and then garbage.
        assert_eq!(
            texts("/* a /* b */ X; /* */ */ Y", SqlDialect::Postgres),
            ["Y"]
        );
    }

    #[test]
    fn mysql_executable_comment_body_is_lexed_behind_a_marker() {
        let sql = "SELECT 1 /*!50000 INTO OUTFILE '/tmp/x' */; Y";
        let spans = lex_sql_spanned_dialect(sql, SqlDialect::MySql).expect("scan");
        let spanned_texts = spans
            .iter()
            .map(|spanned| &sql[spanned.start..spanned.end])
            .collect::<Vec<_>>();
        assert_eq!(
            spanned_texts,
            ["SELECT", "1", "/*!50000", "INTO", "OUTFILE", "'/tmp/x'", ";", "Y"]
        );
        assert!(is_executable_comment(sql, &spans[2]));
        assert!(!spans
            .iter()
            .enumerate()
            .any(|(index, spanned)| index != 2 && is_executable_comment(sql, spanned)));
        assert_eq!(
            texts("/*M!100100 X */", SqlDialect::MySql),
            ["/*M!100100", "X"]
        );
        // A string inside the body can hold `*/` without closing it.
        assert_eq!(
            texts("/*! X '*/' */ Y", SqlDialect::MySql),
            ["/*!", "X", "'*/'", "Y"]
        );
        assert!(lex_sql_spanned_dialect("/*! X", SqlDialect::MySql).is_err());
        assert!(lex_sql_spanned_dialect("/*! /*! X */ */", SqlDialect::MySql).is_err());
    }

    #[test]
    fn mysql_quotes_escape_with_backslashes_and_backticks_quote_identifiers() {
        let tokens = lex_sql_spanned_dialect("`a``b` `'` $$ x $$", SqlDialect::MySql)
            .expect("scan")
            .into_iter()
            .map(|spanned| spanned.token)
            .collect::<Vec<_>>();
        assert_eq!(tokens[0], quoted("a`b"));
        assert_eq!(tokens[1], quoted("'"));
        // No dollar quoting in MySQL.
        assert_eq!(tokens[2], SqlToken::Opaque);
        assert!(tokens.contains(&identifier("x")));
        assert_eq!(
            texts(r#"SELECT 'C:\temp', "a""b", 'it''s'"#, SqlDialect::MySql),
            ["SELECT", r"'C:\temp'", ",", r#""a""b""#, ",", "'it''s'"]
        );
        // Ends differ with and without NO_BACKSLASH_ESCAPES / ANSI_QUOTES.
        assert!(lex_sql_spanned_dialect(r"SELECT 'a\'; X; '", SqlDialect::MySql).is_err());
        assert!(lex_sql_spanned_dialect(r#"SELECT "a\" , '" ; X ; '"#, SqlDialect::MySql).is_err());
    }

    #[test]
    fn sqlite_identifiers_comments_and_literal_strings() {
        let tokens = lex_sql_spanned_dialect("`a``b` [x ` y] \"q\"", SqlDialect::Sqlite)
            .expect("scan")
            .into_iter()
            .map(|spanned| spanned.token)
            .collect::<Vec<_>>();
        assert_eq!(tokens, [quoted("a`b"), quoted("x ` y"), quoted("q")]);
        assert_eq!(
            texts(r"SELECT 'a\'; X", SqlDialect::Sqlite),
            ["SELECT", r"'a\'", ";", "X"]
        );
        assert_eq!(
            texts("/* a /* b */ X; /* open", SqlDialect::Sqlite),
            ["X", ";"]
        );
        assert_eq!(texts("SELECT $a, E'x'", SqlDialect::Sqlite).len(), 6);
        assert!(lex_sql_spanned_dialect("[open", SqlDialect::Sqlite).is_err());
    }
}
