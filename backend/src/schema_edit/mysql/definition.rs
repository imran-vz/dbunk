//! `SHOW CREATE TABLE` as structure. A small MySQL tokenizer keeps the
//! server's own bytes for every expression (defaults, `ON UPDATE`,
//! generated columns, checks, functional key parts), which the model stores
//! verbatim; `sqlparser` (`MySqlDialect`) parses the same text independently
//! and [`cross_check`] compares the two. Anything this parser does not
//! understand is reported in `unsupported`, which makes the baseline
//! read-only rather than guessed.
use sqlparser::ast::{
    ColumnOption, GeneratedExpressionMode, ObjectNamePart, Statement, TableConstraint,
};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DefDefault {
    Null,
    /// A quoted literal, unescaped.
    String(String),
    /// An unquoted number (MariaDB renders numeric defaults bare).
    Number(String),
    /// `b'101'`, `0x1F`, `x'1F'`: kept as written.
    Bits(String),
    /// Anything else, as written (`CURRENT_TIMESTAMP`, `(uuid())`).
    Expression(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DefColumn {
    pub name: String,
    pub data_type: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
    /// `Some(true)` for `NOT NULL`, `Some(false)` for an explicit `NULL`.
    pub not_null: Option<bool>,
    pub default: Option<DefDefault>,
    pub on_update: Option<String>,
    pub auto_increment: bool,
    pub invisible: bool,
    /// Expression inside `AS (…)` and whether it is `STORED`.
    pub generated: Option<(String, bool)>,
    pub comment: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyKind {
    Primary,
    Unique,
    Index,
    Fulltext,
    Spatial,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DefPart {
    pub column: Option<String>,
    /// Functional key part, the text inside its parentheses.
    pub expression: Option<String>,
    pub prefix: Option<u32>,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DefKey {
    pub kind: KeyKind,
    pub name: Option<String>,
    pub parts: Vec<DefPart>,
    /// `USING BTREE|HASH`, upper-cased.
    pub using: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DefForeignKey {
    pub name: Option<String>,
    pub columns: Vec<String>,
    pub referenced_schema: Option<String>,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct DefCheck {
    pub name: Option<String>,
    /// The text inside `CHECK (…)`.
    pub expression: String,
    pub enforced: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Definition {
    pub name: String,
    pub columns: Vec<DefColumn>,
    pub primary_key: Option<DefKey>,
    /// Unique, plain, fulltext and spatial keys, in definition order.
    pub keys: Vec<DefKey>,
    pub foreign_keys: Vec<DefForeignKey>,
    pub checks: Vec<DefCheck>,
    /// Table options, keys upper-cased (`ENGINE`, `CHARSET`, `COLLATE`,
    /// `ROW_FORMAT`, `COMMENT`, …), values unquoted.
    pub options: Vec<(String, String)>,
    /// Features the model cannot represent, as user-facing reasons.
    pub unsupported: Vec<String>,
}

impl Definition {
    pub fn option(&self, key: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    /// A backtick-quoted identifier, unescaped.
    Ident(String),
    /// A quoted string, unescaped.
    Str(String),
    Word(String),
    Num(String),
    Punct(char),
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    start: usize,
    end: usize,
}

/// Versioned comments (`/*!80023 INVISIBLE */`) are read as their content,
/// as the server does; other comments are skipped.
fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut versioned = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            if bytes.get(i + 2) == Some(&b'!') {
                i += 3;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                versioned += 1;
                continue;
            }
            let end = text[i + 2..]
                .find("*/")
                .ok_or("unterminated comment in the table definition")?;
            i += 2 + end + 2;
            continue;
        }
        if b == b'*' && bytes.get(i + 1) == Some(&b'/') && versioned > 0 {
            versioned -= 1;
            i += 2;
            continue;
        }
        if b == b'#'
            || (b == b'-'
                && bytes.get(i + 1) == Some(&b'-')
                && bytes.get(i + 2).is_none_or(|c| c.is_ascii_whitespace()))
        {
            i = text[i..].find('\n').map_or(bytes.len(), |offset| i + offset);
            continue;
        }
        let start = i;
        match b {
            b'`' => {
                let mut value = String::new();
                i += 1;
                loop {
                    let Some(offset) = text[i..].find('`') else {
                        return Err("unterminated identifier in the table definition".into());
                    };
                    value.push_str(&text[i..i + offset]);
                    i += offset + 1;
                    if bytes.get(i) == Some(&b'`') {
                        value.push('`');
                        i += 1;
                    } else {
                        break;
                    }
                }
                tokens.push(Token {
                    tok: Tok::Ident(value),
                    start,
                    end: i,
                });
            }
            b'\'' | b'"' => {
                let (value, next) = read_string(text, i)?;
                i = next;
                tokens.push(Token {
                    tok: Tok::Str(value),
                    start,
                    end: i,
                });
            }
            b if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80 => {
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'_'
                        || bytes[i] == b'$'
                        || bytes[i] >= 0x80)
                {
                    i += 1;
                }
                let word = &text[start..i];
                if word.bytes().all(|c| c.is_ascii_digit()) {
                    // Fraction and exponent.
                    if bytes.get(i) == Some(&b'.')
                        && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)
                    {
                        i += 1;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    if matches!(bytes.get(i), Some(b'e' | b'E'))
                        && bytes
                            .get(i + 1)
                            .is_some_and(|c| c.is_ascii_digit() || *c == b'-' || *c == b'+')
                    {
                        i += 2;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    tokens.push(Token {
                        tok: Tok::Num(text[start..i].to_owned()),
                        start,
                        end: i,
                    });
                } else {
                    tokens.push(Token {
                        tok: Tok::Word(word.to_owned()),
                        start,
                        end: i,
                    });
                }
            }
            _ => {
                // One character; `text` is UTF-8 and non-ASCII bytes are
                // consumed as words above.
                i += 1;
                tokens.push(Token {
                    tok: Tok::Punct(b as char),
                    start,
                    end: i,
                });
            }
        }
    }
    Ok(tokens)
}

/// Reads a MySQL string starting at the quote; returns the unescaped value
/// and the index just past the closing quote. Doubled quotes and backslash
/// escapes are both honoured, as in the server's default mode.
fn read_string(text: &str, start: usize) -> Result<(String, usize), String> {
    let bytes = text.as_bytes();
    let quote = char::from(bytes[start]);
    let body = start + 1;
    let mut value = String::new();
    let mut chars = text[body..].char_indices();
    while let Some((offset, c)) = chars.next() {
        let at = body + offset;
        if c == quote {
            if text[at + 1..].starts_with(quote) {
                value.push(c);
                chars.next();
                continue;
            }
            return Ok((value, at + 1));
        }
        if c == '\\' {
            let Some((_, escaped)) = chars.next() else {
                break;
            };
            match escaped {
                '0' => value.push('\0'),
                'b' => value.push('\u{8}'),
                'n' => value.push('\n'),
                'r' => value.push('\r'),
                't' => value.push('\t'),
                'Z' => value.push('\u{1a}'),
                // Kept escaped, as MySQL does for LIKE patterns.
                '%' | '_' => {
                    value.push('\\');
                    value.push(escaped);
                }
                other => value.push(other),
            }
            continue;
        }
        value.push(c);
    }
    Err("unterminated string in the table definition".into())
}

/// The value of `text` when it is exactly one quoted string (MariaDB
/// `COLUMN_DEFAULT` quotes literals).
pub(super) fn unquote(text: &str) -> Option<String> {
    let text = text.trim();
    if !text.starts_with('\'') {
        return None;
    }
    match read_string(text, 0) {
        Ok((value, end)) if end == text.len() => Some(value),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Source<'a> {
    text: &'a str,
    tokens: Vec<Token>,
}

impl Source<'_> {
    fn word(&self, index: usize) -> Option<String> {
        match self.tokens.get(index).map(|t| &t.tok) {
            Some(Tok::Word(word)) => Some(word.to_ascii_uppercase()),
            _ => None,
        }
    }
    fn is_word(&self, index: usize, keyword: &str) -> bool {
        self.word(index).as_deref() == Some(keyword)
    }
    fn is_punct(&self, index: usize, c: char) -> bool {
        matches!(self.tokens.get(index).map(|t| &t.tok), Some(Tok::Punct(p)) if *p == c)
    }
    fn name(&self, index: usize) -> Option<String> {
        match self.tokens.get(index).map(|t| &t.tok) {
            Some(Tok::Ident(name)) => Some(name.clone()),
            Some(Tok::Word(word)) => Some(word.clone()),
            _ => None,
        }
    }
    fn string(&self, index: usize) -> Option<String> {
        match self.tokens.get(index).map(|t| &t.tok) {
            Some(Tok::Str(value)) => Some(value.clone()),
            _ => None,
        }
    }
    /// Raw text from token `from` through token `to`, inclusive.
    fn raw(&self, from: usize, to: usize) -> String {
        if from > to || to >= self.tokens.len() {
            return String::new();
        }
        self.text[self.tokens[from].start..self.tokens[to].end]
            .trim()
            .to_owned()
    }
    /// Index of the `)` matching the `(` at `open`, within `end`.
    fn close_of(&self, open: usize, end: usize) -> Option<usize> {
        let mut depth = 0usize;
        for index in open..end {
            if self.is_punct(index, '(') {
                depth += 1;
            } else if self.is_punct(index, ')') {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
        }
        None
    }
    /// Splits `[from, to)` at top-level commas.
    fn split_commas(&self, from: usize, to: usize) -> Vec<(usize, usize)> {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut start = from;
        for index in from..to {
            if self.is_punct(index, '(') {
                depth += 1;
            } else if self.is_punct(index, ')') {
                depth = depth.saturating_sub(1);
            } else if depth == 0 && self.is_punct(index, ',') {
                parts.push((start, index));
                start = index + 1;
            }
        }
        if start < to {
            parts.push((start, to));
        }
        parts
    }
}

/// Words that start a column attribute, ending the data type or a value.
const COLUMN_ATTRIBUTES: [&str; 26] = [
    "CHARACTER",
    "CHARSET",
    "COLLATE",
    "GENERATED",
    "AS",
    "NOT",
    "NULL",
    "DEFAULT",
    "ON",
    "AUTO_INCREMENT",
    "INVISIBLE",
    "VISIBLE",
    "COMMENT",
    "SRID",
    "CHECK",
    "PRIMARY",
    "UNIQUE",
    "KEY",
    "REFERENCES",
    "COLUMN_FORMAT",
    "STORAGE",
    "COMPRESSED",
    "WITH",
    "WITHOUT",
    "ENGINE_ATTRIBUTE",
    "SECONDARY_ENGINE_ATTRIBUTE",
];

/// Table options that `ALTER TABLE` leaves alone unless named, so an
/// unchanged value is never lost. Anything else makes the table read-only.
const KEPT_TABLE_OPTIONS: [&str; 20] = [
    "AUTO_INCREMENT",
    "STATS_PERSISTENT",
    "STATS_AUTO_RECALC",
    "STATS_SAMPLE_PAGES",
    "KEY_BLOCK_SIZE",
    "COMPRESSION",
    "ENCRYPTION",
    "PACK_KEYS",
    "CHECKSUM",
    "MAX_ROWS",
    "MIN_ROWS",
    "AVG_ROW_LENGTH",
    "DELAY_KEY_WRITE",
    "PAGE_CHECKSUM",
    "TRANSACTIONAL",
    "PAGE_COMPRESSED",
    "PAGE_COMPRESSION_LEVEL",
    "ENCRYPTED",
    "ENCRYPTION_KEY_ID",
    "INSERT_METHOD",
];

/// Parses one `SHOW CREATE TABLE` result. `Err` means the text is not a
/// `CREATE TABLE` this parser can read at all.
pub(super) fn parse(text: &str) -> Result<Definition, String> {
    let source = Source {
        text,
        tokens: tokenize(text)?,
    };
    let count = source.tokens.len();
    let mut at = 0usize;
    if !(source.is_word(0, "CREATE") && source.is_word(1, "TABLE")) {
        return Err("the definition is not a CREATE TABLE statement".into());
    }
    at += 2;
    if source.is_word(at, "IF") {
        at += 3;
    }
    let mut definition = Definition {
        name: source
            .name(at)
            .ok_or("the definition has no table name")?,
        ..Definition::default()
    };
    at += 1;
    if source.is_punct(at, '.') {
        definition.name = source.name(at + 1).ok_or("the definition has no table name")?;
        at += 2;
    }
    if !source.is_punct(at, '(') {
        return Err("the definition has no column list".into());
    }
    let close = source
        .close_of(at, count)
        .ok_or("the column list is not closed")?;
    for (from, to) in source.split_commas(at + 1, close) {
        entry(&source, from, to, &mut definition)?;
    }
    table_options(&source, close + 1, count, &mut definition);
    Ok(definition)
}

fn entry(source: &Source, from: usize, to: usize, out: &mut Definition) -> Result<(), String> {
    if matches!(source.tokens.get(from).map(|t| &t.tok), Some(Tok::Ident(_))) {
        let column = column(source, from, to, out);
        out.columns.push(column);
        return Ok(());
    }
    let mut at = from;
    let mut constraint_name = None;
    if source.is_word(at, "CONSTRAINT") {
        at += 1;
        if !matches!(
            source.word(at).as_deref(),
            Some("PRIMARY" | "UNIQUE" | "FOREIGN" | "CHECK")
        ) {
            constraint_name = source.name(at);
            at += 1;
        }
    }
    match source.word(at).as_deref() {
        Some("PRIMARY") if source.is_word(at + 1, "KEY") => {
            let key = key(source, KeyKind::Primary, None, at + 2, to, out)?;
            out.primary_key = Some(key);
        }
        Some("UNIQUE") => {
            let mut next = at + 1;
            if matches!(source.word(next).as_deref(), Some("KEY" | "INDEX")) {
                next += 1;
            }
            let key = key(source, KeyKind::Unique, constraint_name, next, to, out)?;
            out.keys.push(key);
        }
        Some("KEY" | "INDEX") => {
            let key = key(source, KeyKind::Index, None, at + 1, to, out)?;
            out.keys.push(key);
        }
        Some(kind @ ("FULLTEXT" | "SPATIAL")) => {
            let kind = if kind == "FULLTEXT" {
                KeyKind::Fulltext
            } else {
                KeyKind::Spatial
            };
            let mut next = at + 1;
            if matches!(source.word(next).as_deref(), Some("KEY" | "INDEX")) {
                next += 1;
            }
            let key = key(source, kind, None, next, to, out)?;
            out.keys.push(key);
        }
        Some("FOREIGN") if source.is_word(at + 1, "KEY") => {
            let key = foreign_key(source, constraint_name, at + 2, to)?;
            out.foreign_keys.push(key);
        }
        Some("CHECK") => {
            let open = at + 1;
            if !source.is_punct(open, '(') {
                return Err("a CHECK constraint has no expression".into());
            }
            let close = source
                .close_of(open, to)
                .ok_or("a CHECK expression is not closed")?;
            let mut enforced = true;
            let mut next = close + 1;
            if source.is_word(next, "NOT") && source.is_word(next + 1, "ENFORCED") {
                enforced = false;
                next += 2;
            } else if source.is_word(next, "ENFORCED") {
                next += 1;
            }
            if next < to {
                out.unsupported.push(format!(
                    "Check constraint options are not supported: {}",
                    source.raw(next, to - 1)
                ));
            }
            out.checks.push(DefCheck {
                name: constraint_name,
                expression: source.raw(open + 1, close - 1),
                enforced,
            });
        }
        _ => out.unsupported.push(format!(
            "Table element is not supported: {}",
            source.raw(from, to - 1)
        )),
    }
    Ok(())
}

fn column(source: &Source, from: usize, to: usize, out: &mut Definition) -> DefColumn {
    let mut column = DefColumn {
        name: source.name(from).unwrap_or_default(),
        ..DefColumn::default()
    };
    let attribute = |index: usize| {
        source
            .word(index)
            .is_some_and(|word| COLUMN_ATTRIBUTES.contains(&word.as_str()))
    };
    // The type runs to the first attribute word outside parentheses.
    let mut at = from + 1;
    let mut depth = 0usize;
    while at < to {
        if source.is_punct(at, '(') {
            depth += 1;
        } else if source.is_punct(at, ')') {
            depth = depth.saturating_sub(1);
        } else if depth == 0 && attribute(at) {
            break;
        }
        at += 1;
    }
    column.data_type = source.raw(from + 1, at.saturating_sub(1).max(from + 1));
    if at == from + 1 {
        column.data_type.clear();
    }
    // A value (default, ON UPDATE) runs to the next attribute word outside
    // parentheses.
    let value_end = |start: usize| {
        let mut index = start;
        let mut depth = 0usize;
        while index < to {
            if source.is_punct(index, '(') {
                depth += 1;
            } else if source.is_punct(index, ')') {
                depth = depth.saturating_sub(1);
            } else if depth == 0 && index > start && attribute(index) {
                break;
            }
            index += 1;
        }
        index
    };
    let unsupported = |out: &mut Definition, what: String| {
        out.unsupported
            .push(format!("Column {} uses {what}, which is not supported", column_name(source, from)));
    };
    while at < to {
        let word = source.word(at).unwrap_or_default();
        match word.as_str() {
            "CHARACTER" if source.is_word(at + 1, "SET") => {
                column.charset = source.name(at + 2);
                at += 3;
            }
            "CHARSET" => {
                column.charset = source.name(at + 1);
                at += 2;
            }
            "COLLATE" => {
                column.collation = source.name(at + 1);
                at += 2;
            }
            "NOT" if source.is_word(at + 1, "NULL") => {
                column.not_null = Some(true);
                at += 2;
            }
            "NULL" => {
                column.not_null = Some(false);
                at += 1;
            }
            "DEFAULT" => {
                let end = value_end(at + 1);
                column.default = Some(default_value(source, at + 1, end));
                at = end;
            }
            "ON" if source.is_word(at + 1, "UPDATE") => {
                let end = value_end(at + 2);
                column.on_update = Some(source.raw(at + 2, end - 1));
                at = end;
            }
            "AUTO_INCREMENT" => {
                column.auto_increment = true;
                at += 1;
            }
            "INVISIBLE" => {
                column.invisible = true;
                at += 1;
            }
            "VISIBLE" => at += 1,
            "COMMENT" => {
                column.comment = source.string(at + 1);
                at += 2;
            }
            "GENERATED" | "AS" => {
                let mut open = at + 1;
                if word == "GENERATED" {
                    if !(source.is_word(at + 1, "ALWAYS") && source.is_word(at + 2, "AS")) {
                        unsupported(out, source.raw(at, to - 1));
                        break;
                    }
                    open = at + 3;
                }
                let Some(close) = source
                    .is_punct(open, '(')
                    .then(|| source.close_of(open, to))
                    .flatten()
                else {
                    unsupported(out, source.raw(at, to - 1));
                    break;
                };
                let mut stored = false;
                at = close + 1;
                match source.word(at).as_deref() {
                    Some("STORED" | "PERSISTENT") => {
                        stored = true;
                        at += 1;
                    }
                    Some("VIRTUAL") => at += 1,
                    _ => {}
                }
                column.generated = Some((source.raw(open + 1, close - 1), stored));
            }
            "CHECK" => {
                unsupported(out, "a column-level CHECK constraint".into());
                break;
            }
            "SRID" => {
                unsupported(out, "SRID".into());
                break;
            }
            _ => {
                unsupported(out, format!("{:?}", source.raw(at, to - 1)));
                break;
            }
        }
    }
    column
}

fn column_name(source: &Source, from: usize) -> String {
    source.name(from).unwrap_or_default()
}

fn default_value(source: &Source, from: usize, end: usize) -> DefDefault {
    let tokens = &source.tokens[from..end.min(source.tokens.len())];
    match tokens {
        [Token {
            tok: Tok::Word(word),
            ..
        }] if word.eq_ignore_ascii_case("NULL") => DefDefault::Null,
        [Token {
            tok: Tok::Str(value),
            ..
        }] => DefDefault::String(value.clone()),
        [Token {
            tok: Tok::Num(number),
            ..
        }] => DefDefault::Number(number.clone()),
        [Token {
            tok: Tok::Punct('-'),
            end: sign_end,
            ..
        }, Token {
            tok: Tok::Num(number),
            start,
            ..
        }] if sign_end == start => DefDefault::Number(format!("-{number}")),
        [Token {
            tok: Tok::Word(word),
            end: word_end,
            ..
        }, Token {
            tok: Tok::Str(_),
            start,
            ..
        }] if word_end == start && matches!(word.as_str(), "b" | "B" | "x" | "X") => {
            DefDefault::Bits(source.raw(from, end - 1))
        }
        [Token {
            tok: Tok::Word(word),
            ..
        }] if word.len() > 2
            && (word.starts_with("0x") || word.starts_with("0b"))
            && word[2..].chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            DefDefault::Bits(word.clone())
        }
        _ => DefDefault::Expression(source.raw(from, end.saturating_sub(1))),
    }
}

/// Key parts and options from `at` (the optional key name) to `to`.
fn key(
    source: &Source,
    kind: KeyKind,
    constraint_name: Option<String>,
    mut at: usize,
    to: usize,
    out: &mut Definition,
) -> Result<DefKey, String> {
    let mut key = DefKey {
        kind,
        name: constraint_name,
        parts: Vec::new(),
        using: None,
    };
    if !source.is_punct(at, '(') && !source.is_word(at, "USING") {
        if let Some(name) = source.name(at) {
            key.name = Some(name);
            at += 1;
        }
    }
    if source.is_word(at, "USING") {
        key.using = source.word(at + 1);
        at += 2;
    }
    if !source.is_punct(at, '(') {
        return Err("a key has no column list".into());
    }
    let close = source
        .close_of(at, to)
        .ok_or("a key column list is not closed")?;
    for (from, end) in source.split_commas(at + 1, close) {
        key.parts.push(key_part(source, from, end, out));
    }
    let label = key.name.clone().unwrap_or_else(|| "PRIMARY".into());
    let mut next = close + 1;
    while next < to {
        match source.word(next).as_deref() {
            Some("USING") => {
                key.using = source.word(next + 1);
                next += 2;
            }
            Some("VISIBLE") => next += 1,
            Some("NOT") if source.is_word(next + 1, "IGNORED") => next += 2,
            Some("COMMENT") => {
                out.unsupported
                    .push(format!("Index {label} has a comment, which is not supported"));
                next += 2;
            }
            Some("INVISIBLE" | "IGNORED") => {
                out.unsupported
                    .push(format!("Index {label} is invisible, which is not supported"));
                next += 1;
            }
            _ => {
                out.unsupported.push(format!(
                    "Index {label} uses {}, which is not supported",
                    source.raw(next, to - 1)
                ));
                break;
            }
        }
    }
    Ok(key)
}

fn key_part(source: &Source, from: usize, to: usize, out: &mut Definition) -> DefPart {
    let mut part = DefPart {
        column: None,
        expression: None,
        prefix: None,
        descending: false,
    };
    let mut at = from;
    if source.is_punct(at, '(') {
        match source.close_of(at, to) {
            Some(close) => {
                part.expression = Some(source.raw(at + 1, close - 1));
                at = close + 1;
            }
            None => {
                out.unsupported
                    .push("A key part could not be read".to_owned());
                return part;
            }
        }
    } else {
        part.column = source.name(at);
        at += 1;
        if source.is_punct(at, '(') {
            match (source.tokens.get(at + 1).map(|t| &t.tok), source.is_punct(at + 2, ')')) {
                (Some(Tok::Num(length)), true) => {
                    part.prefix = length.parse().ok();
                    at += 3;
                }
                _ => {
                    out.unsupported
                        .push("A key part prefix could not be read".to_owned());
                    return part;
                }
            }
        }
    }
    match source.word(at).as_deref() {
        Some("DESC") => {
            part.descending = true;
            at += 1;
        }
        Some("ASC") => at += 1,
        _ => {}
    }
    if at < to {
        out.unsupported.push(format!(
            "A key part uses {}, which is not supported",
            source.raw(at, to - 1)
        ));
    }
    part
}

fn name_list(source: &Source, open: usize, to: usize) -> Result<(Vec<String>, usize), String> {
    if !source.is_punct(open, '(') {
        return Err("a foreign key has no column list".into());
    }
    let close = source
        .close_of(open, to)
        .ok_or("a foreign key column list is not closed")?;
    let mut names = Vec::new();
    for (from, end) in source.split_commas(open + 1, close) {
        if end != from + 1 {
            return Err("a foreign key column could not be read".into());
        }
        names.push(source.name(from).ok_or("a foreign key column has no name")?);
    }
    Ok((names, close + 1))
}

fn referential_action(source: &Source, at: usize) -> Option<(String, usize)> {
    match source.word(at).as_deref()? {
        "RESTRICT" => Some(("RESTRICT".into(), at + 1)),
        "CASCADE" => Some(("CASCADE".into(), at + 1)),
        "SET" => match source.word(at + 1).as_deref()? {
            "NULL" => Some(("SET NULL".into(), at + 2)),
            "DEFAULT" => Some(("SET DEFAULT".into(), at + 2)),
            _ => None,
        },
        "NO" if source.is_word(at + 1, "ACTION") => Some(("NO ACTION".into(), at + 2)),
        _ => None,
    }
}

fn foreign_key(
    source: &Source,
    name: Option<String>,
    mut at: usize,
    to: usize,
) -> Result<DefForeignKey, String> {
    let mut name = name;
    if !source.is_punct(at, '(') {
        // `FOREIGN KEY index_name (…)`; the constraint name wins.
        if name.is_none() {
            name = source.name(at);
        }
        at += 1;
    }
    let (columns, next) = name_list(source, at, to)?;
    at = next;
    if !source.is_word(at, "REFERENCES") {
        return Err("a foreign key has no REFERENCES clause".into());
    }
    at += 1;
    let mut referenced_schema = None;
    let mut referenced_table = source
        .name(at)
        .ok_or("a foreign key has no referenced table")?;
    at += 1;
    if source.is_punct(at, '.') {
        referenced_schema = Some(referenced_table);
        referenced_table = source
            .name(at + 1)
            .ok_or("a foreign key has no referenced table")?;
        at += 2;
    }
    let (referenced_columns, next) = name_list(source, at, to)?;
    at = next;
    let mut key = DefForeignKey {
        name,
        columns,
        referenced_schema,
        referenced_table,
        referenced_columns,
        on_delete: None,
        on_update: None,
    };
    while at < to {
        if source.is_word(at, "ON") && source.is_word(at + 1, "DELETE") {
            let (action, next) =
                referential_action(source, at + 2).ok_or("unknown ON DELETE action")?;
            key.on_delete = Some(action);
            at = next;
        } else if source.is_word(at, "ON") && source.is_word(at + 1, "UPDATE") {
            let (action, next) =
                referential_action(source, at + 2).ok_or("unknown ON UPDATE action")?;
            key.on_update = Some(action);
            at = next;
        } else {
            return Err(format!(
                "foreign key option is not supported: {}",
                source.raw(at, to - 1)
            ));
        }
    }
    Ok(key)
}

fn table_options(source: &Source, mut at: usize, to: usize, out: &mut Definition) {
    while at < to {
        if source.is_punct(at, ',') {
            at += 1;
            continue;
        }
        let Some(word) = source.word(at) else {
            out.unsupported.push(format!(
                "Table option is not supported: {}",
                source.raw(at, to - 1)
            ));
            return;
        };
        let (key, mut next) = match word.as_str() {
            "DEFAULT" if source.is_word(at + 1, "CHARSET") => ("CHARSET".to_owned(), at + 2),
            "DEFAULT" if source.is_word(at + 1, "CHARACTER") && source.is_word(at + 2, "SET") => {
                ("CHARSET".to_owned(), at + 3)
            }
            "DEFAULT" if source.is_word(at + 1, "COLLATE") => ("COLLATE".to_owned(), at + 2),
            "CHARACTER" if source.is_word(at + 1, "SET") => ("CHARSET".to_owned(), at + 2),
            "CHARSET" | "COLLATE" | "ENGINE" | "ROW_FORMAT" | "COMMENT" | "TABLESPACE" => {
                (word.clone(), at + 1)
            }
            other if KEPT_TABLE_OPTIONS.contains(&other) => (word.clone(), at + 1),
            "PARTITION" => {
                out.unsupported
                    .push("Partitioned tables are not supported".to_owned());
                return;
            }
            "WITH" if source.is_word(at + 1, "SYSTEM") => {
                out.unsupported
                    .push("System-versioned tables are not supported".to_owned());
                return;
            }
            _ => {
                out.unsupported.push(format!(
                    "Table option is not supported: {}",
                    source.raw(at, to - 1)
                ));
                return;
            }
        };
        if source.is_punct(next, '=') {
            next += 1;
        }
        let value = match source.tokens.get(next).map(|t| &t.tok) {
            Some(Tok::Word(value) | Tok::Num(value) | Tok::Str(value) | Tok::Ident(value)) => {
                value.clone()
            }
            _ => {
                out.unsupported
                    .push(format!("Table option {key} could not be read"));
                return;
            }
        };
        next += 1;
        if key == "TABLESPACE" && source.is_word(next, "STORAGE") {
            next += 2;
        }
        out.options.push((key, value));
        at = next;
    }
}

// ---------------------------------------------------------------------------
// sqlparser cross-check
// ---------------------------------------------------------------------------

fn last_name(name: &sqlparser::ast::ObjectName) -> String {
    match name.0.last() {
        Some(ObjectNamePart::Identifier(ident)) => ident.value.clone(),
        _ => String::new(),
    }
}

fn idents(values: &[sqlparser::ast::Ident]) -> Vec<String> {
    values.iter().map(|ident| ident.value.clone()).collect()
}

/// Parses `text` with `sqlparser` and compares column names, order,
/// nullability, `AUTO_INCREMENT`, visibility and generation, and the names,
/// kinds and members of keys, foreign keys and checks with `definition`.
/// Returns the disagreements; a parse failure is one disagreement.
pub(super) fn cross_check(text: &str, definition: &Definition) -> Vec<String> {
    let statements = match Parser::parse_sql(&MySqlDialect {}, text) {
        Ok(statements) => statements,
        Err(error) => return vec![format!("The table definition could not be parsed: {error}")],
    };
    let [Statement::CreateTable(create)] = statements.as_slice() else {
        return vec!["The table definition is not a single CREATE TABLE".into()];
    };
    let mut problems = Vec::new();
    let mut differ = |what: String| problems.push(format!("The parsed definition differs: {what}"));
    if last_name(&create.name) != definition.name {
        differ("table name".into());
    }
    let parsed: Vec<String> = create.columns.iter().map(|c| c.name.value.clone()).collect();
    let ours: Vec<String> = definition.columns.iter().map(|c| c.name.clone()).collect();
    if parsed != ours {
        differ("column names or order".into());
    } else {
        for (parsed, ours) in create.columns.iter().zip(&definition.columns) {
            let options: Vec<&ColumnOption> = parsed.options.iter().map(|o| &o.option).collect();
            let not_null = options.iter().any(|o| matches!(o, ColumnOption::NotNull));
            if not_null != (ours.not_null == Some(true)) {
                differ(format!("nullability of {}", ours.name));
            }
            let auto_increment = options.iter().any(|o| {
                matches!(o, ColumnOption::DialectSpecific(tokens)
                    if tokens.iter().any(|t| t.to_string().eq_ignore_ascii_case("AUTO_INCREMENT")))
            });
            if auto_increment != ours.auto_increment {
                differ(format!("AUTO_INCREMENT of {}", ours.name));
            }
            let invisible = options.iter().any(|o| matches!(o, ColumnOption::Invisible));
            if invisible != ours.invisible {
                differ(format!("visibility of {}", ours.name));
            }
            let generated = options.iter().find_map(|o| match o {
                ColumnOption::Generated {
                    generation_expr: Some(_),
                    generation_expr_mode,
                    ..
                } => Some(matches!(
                    generation_expr_mode,
                    Some(GeneratedExpressionMode::Stored)
                )),
                _ => None,
            });
            if generated != ours.generated.as_ref().map(|(_, stored)| *stored) {
                differ(format!("generation of {}", ours.name));
            }
            let has_default = options.iter().any(|o| matches!(o, ColumnOption::Default(_)));
            if has_default != ours.default.is_some() {
                differ(format!("default of {}", ours.name));
            }
            let has_comment = options.iter().any(|o| matches!(o, ColumnOption::Comment(_)));
            if has_comment != ours.comment.is_some() {
                differ(format!("comment of {}", ours.name));
            }
        }
    }
    let mut primary = None;
    let mut keys: Vec<(KeyKind, Option<String>, usize)> = Vec::new();
    let mut foreign: Vec<(Option<String>, Vec<String>, String, Vec<String>)> = Vec::new();
    let mut checks: Vec<(Option<String>, bool)> = Vec::new();
    for constraint in &create.constraints {
        match constraint {
            TableConstraint::PrimaryKey(key) => primary = Some(key.columns.len()),
            TableConstraint::Unique(key) => keys.push((
                KeyKind::Unique,
                key.index_name
                    .as_ref()
                    .or(key.name.as_ref())
                    .map(|i| i.value.clone()),
                key.columns.len(),
            )),
            TableConstraint::Index(key) => keys.push((
                KeyKind::Index,
                key.name.as_ref().map(|i| i.value.clone()),
                key.columns.len(),
            )),
            TableConstraint::FulltextOrSpatial(key) => keys.push((
                if key.fulltext {
                    KeyKind::Fulltext
                } else {
                    KeyKind::Spatial
                },
                key.opt_index_name.as_ref().map(|i| i.value.clone()),
                key.columns.len(),
            )),
            TableConstraint::ForeignKey(key) => foreign.push((
                key.name.as_ref().map(|i| i.value.clone()),
                idents(&key.columns),
                last_name(&key.foreign_table),
                idents(&key.referred_columns),
            )),
            TableConstraint::Check(check) => checks.push((
                check.name.as_ref().map(|i| i.value.clone()),
                check.enforced != Some(false),
            )),
            _ => differ("an unexpected constraint kind".into()),
        }
    }
    if primary != definition.primary_key.as_ref().map(|k| k.parts.len()) {
        differ("primary key".into());
    }
    let ours: Vec<(KeyKind, Option<String>, usize)> = definition
        .keys
        .iter()
        .map(|k| (k.kind, k.name.clone(), k.parts.len()))
        .collect();
    if keys != ours {
        differ("keys or indexes".into());
    }
    let ours: Vec<(Option<String>, Vec<String>, String, Vec<String>)> = definition
        .foreign_keys
        .iter()
        .map(|k| {
            (
                k.name.clone(),
                k.columns.clone(),
                k.referenced_table.clone(),
                k.referenced_columns.clone(),
            )
        })
        .collect();
    if foreign != ours {
        differ("foreign keys".into());
    }
    let ours: Vec<(Option<String>, bool)> = definition
        .checks
        .iter()
        .map(|c| (c.name.clone(), c.enforced))
        .collect();
    if checks != ours {
        differ("check constraints".into());
    }
    problems
}
