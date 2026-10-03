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

/// Same tokenization, with refusal before a token beyond the caller's cap can
/// grow the output vector. Whitespace and comments consume no token allowance.
pub(crate) fn lex_sql_spanned_bounded(
    sql: &str,
    maximum_tokens: usize,
) -> Result<Vec<SpannedToken>, SqlLexError> {
    let bytes = sql.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let start = index;
        let token = match bytes[index] {
            byte if byte.is_ascii_whitespace() => {
                index += 1;
                continue;
            }
            b'-' if bytes.get(index + 1) == Some(&b'-') => {
                index += 2;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                continue;
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                index = lex_block_comment(bytes, index + 2)?;
                continue;
            }
            b'\'' => {
                index = lex_single_quote(bytes, index + 1, false)?;
                SqlToken::Opaque
            }
            b'"' => {
                let (value, next) = lex_quoted_identifier(sql, index + 1)?;
                index = next;
                SqlToken::Identifier(SqlIdentifier {
                    value,
                    quoted: true,
                })
            }
            b'$' => {
                index = lex_dollar(bytes, index)?;
                SqlToken::Opaque
            }
            byte if is_identifier_start(byte) => {
                index += 1;
                while index < bytes.len() && is_identifier_continue(bytes[index]) {
                    index += 1;
                }
                let value = &sql[start..index];
                if value.eq_ignore_ascii_case("e") && bytes.get(index) == Some(&b'\'') {
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

fn lex_single_quote(bytes: &[u8], index: usize, escapes: bool) -> Result<usize, ()> {
    if escapes {
        return scan_single_quote(bytes, index, true);
    }
    // Plain-string backslash semantics depend on standard_conforming_strings,
    // which the lexer cannot observe. Accept the literal only when both
    // interpretations end at the same byte, so a backslash can never move a
    // statement or fragment boundary whichever setting the server uses.
    let literal_end = scan_single_quote(bytes, index, false)?;
    let escaped_end = scan_single_quote(bytes, index, true)?;
    if literal_end == escaped_end {
        Ok(literal_end)
    } else {
        Err(())
    }
}

fn scan_single_quote(bytes: &[u8], mut index: usize, escapes: bool) -> Result<usize, ()> {
    while index < bytes.len() {
        match bytes[index] {
            b'\'' if bytes.get(index + 1) == Some(&b'\'') => index += 2,
            b'\'' => return Ok(index + 1),
            b'\\' if escapes && index + 1 < bytes.len() => index += 2,
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
}
