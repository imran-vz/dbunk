//! Conservative recognition, never SQL rewriting or execution. Call only on
//! non-pretty PG16 pg_get_expr output captured with the documented session
//! settings, a pre-snapshot table lock, and complete dependency/column facts.
//! Reject every recorded dependency except the owning locked table/columns
//! before calling this module, including non-pinned objects in pg_catalog.
//! Only supply column names whose catalog types are the scalar types below.

pub const MAX_EXPRESSION_BYTES: usize = 256 * 1024;
const MAX_TOKENS: usize = 4096;
const MAX_DEPTH: usize = 64;

/// These built-in scalar output functions do not render catalog object names.
pub fn supported_scalar(schema: &str, name: &str) -> bool {
    schema == "pg_catalog" && matches!(name, "bool" | "int2" | "int4" | "int8" | "numeric" | "text")
}

/// Positive grammar: scalar literals/columns, scalar casts, arithmetic,
/// comparisons, Boolean operators and null tests. Everything else is excluded.
/// The borrowed text remains byte-for-byte unchanged, including SQL literals.
pub fn comparable(text: &str, safe_columns: &[&str]) -> bool {
    if text.len() > MAX_EXPRESSION_BYTES {
        return false;
    }
    let mut parser = Parser {
        text,
        columns: safe_columns,
        offset: 0,
        tokens: 0,
    };
    parser.expression(0).is_ok() && parser.next() == Ok(Token::End)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Token<'a> {
    Word(&'a str),
    Identifier(&'a str),
    Literal,
    Operator,
    Open,
    Close,
    Cast,
    End,
}

struct Parser<'a> {
    text: &'a str,
    columns: &'a [&'a str],
    offset: usize,
    tokens: usize,
}

impl<'a> Parser<'a> {
    fn next(&mut self) -> Result<Token<'a>, ()> {
        let bytes = self.text.as_bytes();
        while bytes.get(self.offset).is_some_and(u8::is_ascii_whitespace) {
            self.offset += 1;
        }
        if self.offset == bytes.len() {
            return Ok(Token::End);
        }
        self.tokens += 1;
        if self.tokens > MAX_TOKENS {
            return Err(());
        }
        let start = self.offset;
        self.offset += 1;
        match bytes[start] {
            b'(' => Ok(Token::Open),
            b')' => Ok(Token::Close),
            b':' if bytes.get(self.offset) == Some(&b':') => {
                self.offset += 1;
                Ok(Token::Cast)
            }
            // Comments, custom multi-character operators and casts with
            // qualifiers cannot be smuggled through an operator token.
            b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' => {
                if matches!(bytes[start], b'<' | b'>')
                    && matches!(bytes.get(self.offset), Some(b'=') | Some(b'>'))
                {
                    self.offset += 1;
                }
                if bytes
                    .get(self.offset)
                    .is_some_and(|b| b"+-*/%=<>".contains(b))
                {
                    return Err(());
                }
                Ok(Token::Operator)
            }
            b'\'' | b'"' => {
                let quote = bytes[start];
                loop {
                    let byte = *bytes.get(self.offset).ok_or(())?;
                    self.offset += 1;
                    // Backslash forms are excluded, not interpreted. Doubled
                    // quotes and UTF-8 literal/identifier contents are retained.
                    if byte == b'\\' {
                        return Err(());
                    }
                    if byte == quote {
                        if bytes.get(self.offset) == Some(&quote) {
                            self.offset += 1;
                        } else {
                            return Ok(if quote == b'\'' {
                                Token::Literal
                            } else {
                                Token::Identifier(&self.text[start + 1..self.offset - 1])
                            });
                        }
                    }
                }
            }
            byte if byte.is_ascii_digit() => {
                while bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                    self.offset += 1;
                }
                if bytes.get(self.offset) == Some(&b'.') {
                    self.offset += 1;
                    while bytes.get(self.offset).is_some_and(u8::is_ascii_digit) {
                        self.offset += 1;
                    }
                }
                Ok(Token::Literal)
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                while bytes
                    .get(self.offset)
                    .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
                {
                    self.offset += 1;
                }
                Ok(Token::Word(&self.text[start..self.offset]))
            }
            _ => Err(()),
        }
    }

    fn peek(&mut self) -> Result<Token<'a>, ()> {
        let (offset, tokens) = (self.offset, self.tokens);
        let token = self.next();
        self.offset = offset;
        self.tokens = tokens;
        token
    }

    fn expression(&mut self, depth: usize) -> Result<(), ()> {
        if depth >= MAX_DEPTH {
            return Err(());
        }
        self.atom(depth)?;
        loop {
            match self.peek()? {
                Token::Operator | Token::Word("AND" | "OR") => {
                    self.next()?;
                    self.atom(depth)?;
                }
                Token::Word("IS") => {
                    self.next()?;
                    if self.peek()? == Token::Word("NOT") {
                        self.next()?;
                    }
                    match self.next()? {
                        Token::Word("NULL" | "TRUE" | "FALSE") => (),
                        _ => return Err(()),
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn atom(&mut self, depth: usize) -> Result<(), ()> {
        if depth >= MAX_DEPTH {
            return Err(());
        }
        match self.next()? {
            Token::Operator | Token::Word("NOT") => self.atom(depth + 1)?,
            Token::Literal | Token::Word("NULL" | "true" | "false") => (),
            Token::Word(name) if self.columns.contains(&name) => (),
            Token::Identifier(name)
                if self
                    .columns
                    .iter()
                    .any(|column| quoted_equals(name, column)) => {}
            Token::Open => {
                self.expression(depth + 1)?;
                if self.next()? != Token::Close {
                    return Err(());
                }
            }
            _ => return Err(()),
        }
        while self.peek()? == Token::Cast {
            self.next()?;
            match self.next()? {
                Token::Word("smallint" | "integer" | "bigint" | "numeric" | "text" | "boolean") => {
                }
                _ => return Err(()),
            }
        }
        Ok(())
    }
}

fn quoted_equals(quoted: &str, name: &str) -> bool {
    let mut encoded = quoted.chars();
    for ch in name.chars() {
        if encoded.next() != Some(ch) || (ch == '"' && encoded.next() != Some('"')) {
            return false;
        }
    }
    encoded.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_scalar_projection_without_rewriting_literals() {
        let columns = ["quantity", "Mixed Case", "a\"b"];
        for text in [
            "7",
            "(-42)",
            "'source.literal ''quoted'''::text",
            "(quantity > 0)",
            "(quantity + 1)",
            "((quantity > 0) AND (quantity IS NOT NULL))",
            "(\"Mixed Case\" = 7)",
            "(\"a\"\"b\" <> 8)",
            "true",
            "NULL::integer",
        ] {
            assert!(comparable(text, &columns), "{text}");
        }
    }

    #[test]
    fn excludes_hidden_dependencies_and_unproven_syntax() {
        for text in [
            "('{external.serial}'::regclass[])::text",
            "nextval('s.q'::regclass)",
            "external.f(7)",
            "'calm'::external.mood",
            "'7'::pg_catalog.int4",
            "ARRAY[1]",
            "(quantity OPERATOR(external.+) 1)",
            "E'a\\nb'::text",
            "(quantity COLLATE external.c)",
            "(quantity).field",
            "unknown > 0",
            "7; SELECT 1",
            "7 /* comment */",
            "7 -- comment",
            "(quantity + 1), (quantity + 2)",
        ] {
            assert!(!comparable(text, &["quantity"]), "{text}");
        }
    }

    #[test]
    fn bounds_work_and_rejects_non_scalar_types() {
        assert!(!comparable(&"(".repeat(MAX_DEPTH), &[]));
        assert!(!comparable(&"1 + ".repeat(MAX_TOKENS), &[]));
        assert!(!comparable(&"x".repeat(MAX_EXPRESSION_BYTES + 1), &[]));
        assert!(supported_scalar("pg_catalog", "int4"));
        assert!(!supported_scalar("pg_catalog", "_regclass"));
        assert!(!supported_scalar("external", "text"));
    }
}
