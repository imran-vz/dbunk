//! A small SQLite tokenizer that keeps byte offsets, so captured expressions
//! are exact slices of `sqlite_master.sql` (never re-rendered).
//!
//! It follows SQLite's own token rules closely enough for DDL: whitespace and
//! comments are skipped, strings use `''`, identifiers may be quoted with
//! `"…"`, `` `…` `` or `[…]`, and bytes at or above 0x80 are identifier
//! characters. Token boundaries always fall on ASCII bytes or at the end of
//! an identifier run, so every slice is on a char boundary.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A bare identifier or keyword.
    Word,
    /// `"x"`, `` `x` `` or `[x]`.
    Quoted,
    /// `'x'`.
    String,
    /// `x'00ff'`.
    Blob,
    Number,
    /// `?1`, `:a`, `@a`, `$a`.
    Variable,
    /// An operator or punctuation, one to three bytes.
    Punct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
}

impl Token {
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.start..self.end]
    }

    /// True for an unquoted word equal to `keyword`, ignoring ASCII case.
    pub fn is_keyword(&self, source: &str, keyword: &str) -> bool {
        self.kind == Kind::Word && self.text(source).eq_ignore_ascii_case(keyword)
    }

    pub fn is_punct(&self, source: &str, punct: &str) -> bool {
        self.kind == Kind::Punct && self.text(source) == punct
    }

    /// Something that can name a table, column or collation.
    pub fn is_name(&self) -> bool {
        matches!(self.kind, Kind::Word | Kind::Quoted | Kind::String)
    }
}

/// SQLite's `sqlite3Isspace`: ASCII whitespace only.
pub(super) fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0b' | b'\x0c')
}

/// Trims SQLite whitespace from both ends, as `sqlite3DbSpanDup` does.
pub(super) fn trim_space(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_ascii() && is_space(c as u8))
}

fn is_id_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}

fn is_id_part(byte: u8) -> bool {
    is_id_start(byte) || byte.is_ascii_digit() || byte == b'$'
}

/// Tokenizes `source`. Errors are short user-facing descriptions.
pub(super) fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let bytes = source.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        let start = i;
        if is_space(b) {
            i += 1;
            continue;
        }
        if b == b'-' && bytes.get(i + 1) == Some(&b'-') {
            i = bytes[i..]
                .iter()
                .position(|c| *c == b'\n')
                .map_or(bytes.len(), |offset| i + offset + 1);
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            // SQLite accepts an unterminated block comment at the end.
            i = bytes[i + 2..]
                .windows(2)
                .position(|w| w == b"*/")
                .map_or(bytes.len(), |offset| i + 2 + offset + 2);
            continue;
        }
        let kind = match b {
            b'\'' => {
                i = close(bytes, i + 1, b'\'').ok_or("an unterminated string")?;
                Kind::String
            }
            b'"' | b'`' => {
                i = close(bytes, i + 1, b).ok_or("an unterminated quoted name")?;
                Kind::Quoted
            }
            b'[' => {
                i = bytes[i + 1..]
                    .iter()
                    .position(|c| *c == b']')
                    .map(|offset| i + 1 + offset + 1)
                    .ok_or("an unterminated [name]")?;
                Kind::Quoted
            }
            b'x' | b'X' if bytes.get(i + 1) == Some(&b'\'') => {
                i = close(bytes, i + 2, b'\'').ok_or("an unterminated blob literal")?;
                Kind::Blob
            }
            b'0'..=b'9' => {
                i = number(bytes, i);
                Kind::Number
            }
            b'.' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                i = number(bytes, i);
                Kind::Number
            }
            b'?' => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                Kind::Variable
            }
            b':' | b'@' | b'$' | b'#' => {
                i += 1;
                while i < bytes.len() && is_id_part(bytes[i]) {
                    i += 1;
                }
                if i == start + 1 {
                    return Err(format!("an unexpected {:?}", b as char));
                }
                Kind::Variable
            }
            b if is_id_start(b) => {
                while i < bytes.len() && is_id_part(bytes[i]) {
                    i += 1;
                }
                Kind::Word
            }
            _ => {
                let rest = &bytes[i..];
                let width = if rest.starts_with(b"->>") {
                    3
                } else if [
                    b"||", b"<=", b">=", b"==", b"!=", b"<>", b"<<", b">>", b"->",
                ]
                .iter()
                .any(|op| rest.starts_with(*op))
                {
                    2
                } else if b"(),;.+-*/%<>=&|~".contains(&b) {
                    1
                } else {
                    return Err(format!("an unexpected character {:?}", b as char));
                };
                i += width;
                Kind::Punct
            }
        };
        tokens.push(Token {
            kind,
            start,
            end: i,
        });
    }
    Ok(tokens)
}

/// Index just past the closing `quote`; a doubled quote is an escape.
fn close(bytes: &[u8], mut i: usize, quote: u8) -> Option<usize> {
    while i < bytes.len() {
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

fn number(bytes: &[u8], mut i: usize) -> usize {
    if bytes[i] == b'0' && matches!(bytes.get(i + 1), Some(b'x' | b'X')) {
        i += 2;
        while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
            i += 1;
        }
        return i;
    }
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            i = j;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
    }
    i
}

/// The name a token denotes: quotes removed and doubled quotes collapsed.
/// A bare word is returned as written.
pub(super) fn dequote(text: &str) -> String {
    let bytes = text.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        let inner = &text[1..text.len() - 1];
        match (first, last) {
            (b'"', b'"') => return inner.replace("\"\"", "\""),
            (b'`', b'`') => return inner.replace("``", "`"),
            (b'\'', b'\'') => return inner.replace("''", "'"),
            (b'[', b']') => return inner.to_owned(),
            _ => {}
        }
    }
    text.to_owned()
}

/// SQLite words that never name a column inside an expression. Used only to
/// avoid treating operators and keywords as column references.
const EXPRESSION_KEYWORDS: &[&str] = &[
    "and",
    "or",
    "not",
    "null",
    "is",
    "isnull",
    "notnull",
    "in",
    "like",
    "glob",
    "regexp",
    "match",
    "between",
    "case",
    "when",
    "then",
    "else",
    "end",
    "cast",
    "as",
    "collate",
    "escape",
    "exists",
    "select",
    "distinct",
    "from",
    "where",
    "true",
    "false",
    "current_time",
    "current_date",
    "current_timestamp",
];

/// True when `token` can be a column reference in an expression: a quoted
/// name, or a bare word that is not an expression keyword, and in both cases
/// not a function name (followed by `(`) or a table qualifier (followed by
/// `.`).
pub(super) fn is_column_reference(source: &str, tokens: &[Token], index: usize) -> bool {
    let token = tokens[index];
    let candidate = match token.kind {
        Kind::Quoted => true,
        Kind::Word => !EXPRESSION_KEYWORDS
            .iter()
            .any(|keyword| token.text(source).eq_ignore_ascii_case(keyword)),
        _ => false,
    };
    candidate
        && !tokens
            .get(index + 1)
            .is_some_and(|next| next.is_punct(source, "(") || next.is_punct(source, "."))
}

/// Lowercased names of every token in `text` that can be a column reference.
/// Text that does not tokenize yields every word, so a mention is never
/// missed.
pub(super) fn mentioned_names(text: &str) -> Vec<String> {
    match tokenize(text) {
        Ok(tokens) => (0..tokens.len())
            .filter(|i| is_column_reference(text, &tokens, *i))
            .map(|i| dequote(tokens[i].text(text)).to_ascii_lowercase())
            .collect(),
        Err(_) => text
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_lowercase)
            .collect(),
    }
}

/// Every name-like token (bare or quoted), lowercased and dequoted. Broader
/// than [`mentioned_names`]: used for "does this SQL mention X" evidence.
pub(super) fn all_names(text: &str) -> Vec<String> {
    match tokenize(text) {
        Ok(tokens) => tokens
            .iter()
            .filter(|token| matches!(token.kind, Kind::Word | Kind::Quoted))
            .map(|token| dequote(token.text(text)).to_ascii_lowercase())
            .collect(),
        Err(_) => mentioned_names(text),
    }
}

/// A comparable form of an expression: comments and whitespace dropped,
/// names dequoted and lowercased, keywords lowercased, literals as written.
/// `length("Email")` and `length(email)` compare equal; `'A'` and `'a'` do
/// not. `None` when the text does not tokenize.
pub(super) fn normalized(text: &str) -> Option<Vec<String>> {
    let tokens = tokenize(text).ok()?;
    Some(
        tokens
            .iter()
            .map(|token| match token.kind {
                Kind::Word | Kind::Quoted => {
                    format!("n:{}", dequote(token.text(text)).to_ascii_lowercase())
                }
                Kind::Blob => token.text(text).to_ascii_lowercase(),
                _ => token.text(text).to_owned(),
            })
            .collect(),
    )
}

/// True when two expressions are equal under [`normalized`]; text that does
/// not tokenize compares by bytes.
pub(super) fn same_expression(a: &str, b: &str) -> bool {
    match (normalized(a), normalized(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(source: &str) -> Vec<&str> {
        tokenize(source)
            .unwrap()
            .iter()
            .map(|token| token.text(source))
            .collect()
    }

    #[test]
    fn keeps_exact_slices_and_skips_comments() {
        assert_eq!(
            texts("a /* c */ = 'it''s' -- tail\n|| \"q\"\"x\" [b c] x'0F' 1.5e3 ->> ?2"),
            vec![
                "a",
                "=",
                "'it''s'",
                "||",
                "\"q\"\"x\"",
                "[b c]",
                "x'0F'",
                "1.5e3",
                "->>",
                "?2"
            ]
        );
        assert_eq!(texts("caf\u{e9} > 0"), vec!["caf\u{e9}", ">", "0"]);
        assert!(tokenize("'open").is_err());
        assert!(tokenize("\"open").is_err());
    }

    #[test]
    fn dequotes_every_quote_style() {
        assert_eq!(dequote("\"a\"\"b\""), "a\"b");
        assert_eq!(dequote("`a``b`"), "a`b");
        assert_eq!(dequote("[a b]"), "a b");
        assert_eq!(dequote("'it''s'"), "it's");
        assert_eq!(dequote("plain"), "plain");
    }

    #[test]
    fn column_references_skip_functions_keywords_and_qualifiers() {
        assert_eq!(
            mentioned_names("length(Email) > 3 AND t.\"Name\" IS NOT NULL"),
            vec!["email", "name"]
        );
    }

    #[test]
    fn expressions_compare_modulo_quoting_case_and_spacing() {
        assert!(same_expression(
            "length(email)>3",
            "length( \"EMAIL\" ) > 3"
        ));
        assert!(!same_expression("x = 'A'", "x = 'a'"));
        assert!(!same_expression("x > 1", "x >= 1"));
    }
}
