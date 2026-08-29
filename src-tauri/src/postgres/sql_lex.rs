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

pub(crate) fn lex_sql(sql: &str) -> Result<Vec<SqlToken>, ()> {
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
            byte @ (b'(' | b')' | b'[' | b']' | b',' | b'.' | b';' | b'*') => {
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
    fn brackets_and_dollar_identifiers_are_lexed_structurally() {
        let tokens = lex_sql("ARRAY[1, 2]").expect("array literal");
        assert_eq!(tokens[1], SqlToken::Symbol('['));
        assert_eq!(tokens.last(), Some(&SqlToken::Symbol(']')));
        let tokens = lex_sql("d$x$ , y").expect("dollar identifier");
        assert_eq!(tokens[0], identifier("d$x$"));
        assert_eq!(tokens[1], SqlToken::Symbol(','));
    }
}
