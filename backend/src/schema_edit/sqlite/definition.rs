//! Parses the `CREATE TABLE` and `CREATE INDEX` text SQLite stores in
//! `sqlite_master.sql`, keeping every expression as an exact slice of the
//! source, and cross-checks the table parse with `sqlparser`.
//!
//! Anything the engine-neutral model cannot hold (ON CONFLICT clauses, a
//! DESC column primary key, collated or descending key members, MATCH on a
//! foreign key) is an error, so the baseline becomes read-only instead of a
//! rebuild silently dropping it.
use super::lexer::{dequote, tokenize, trim_space, Kind, Token};
use crate::schema_edit::{Deferrable, Generated, ReferentialAction, SqlFragment};
use sqlparser::ast::{ColumnOption, GeneratedExpressionMode, Statement, TableConstraint};
use sqlparser::dialect::SQLiteDialect;
use sqlparser::parser::Parser;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ParsedReference {
    pub name: Option<String>,
    pub table: String,
    /// Empty means the referenced table's primary key.
    pub columns: Vec<String>,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
    pub deferrable: Deferrable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ColumnPrimaryKey {
    pub name: Option<String>,
    pub autoincrement: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ParsedColumn {
    pub name: String,
    /// The declared type as written; empty when there is none.
    pub data_type: String,
    pub not_null: bool,
    /// The default as `PRAGMA table_info.dflt_value` shows it: a term as
    /// written, or the inside of `( … )`, whitespace-trimmed.
    pub default: Option<String>,
    pub collation: Option<String>,
    pub generated: Option<Generated>,
    pub primary_key: Option<ColumnPrimaryKey>,
    /// Column `UNIQUE` constraints, by optional name.
    pub uniques: Vec<Option<String>>,
    pub checks: Vec<(Option<String>, String)>,
    pub references: Vec<ParsedReference>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ParsedConstraint {
    PrimaryKey {
        name: Option<String>,
        columns: Vec<String>,
    },
    Unique {
        name: Option<String>,
        columns: Vec<String>,
    },
    Check {
        name: Option<String>,
        expression: String,
    },
    ForeignKey {
        columns: Vec<String>,
        reference: ParsedReference,
    },
}

impl ParsedConstraint {
    fn kind(&self) -> &'static str {
        match self {
            Self::PrimaryKey { .. } => "primary key",
            Self::Unique { .. } => "unique",
            Self::Check { .. } => "check",
            Self::ForeignKey { .. } => "foreign key",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ParsedTable {
    pub name: String,
    pub columns: Vec<ParsedColumn>,
    pub constraints: Vec<ParsedConstraint>,
    pub without_rowid: bool,
    pub strict: bool,
    /// The parenthesized body, both parentheses included.
    pub body: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum MemberTarget {
    /// A single name; the caller resolves it to a column.
    Name(String),
    Expression(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ParsedIndexMember {
    pub target: MemberTarget,
    pub collation: Option<String>,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ParsedIndex {
    pub name: String,
    pub table: String,
    pub unique: bool,
    pub members: Vec<ParsedIndexMember>,
    pub predicate: Option<String>,
}

struct Cursor<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(source: &'a str) -> Result<Self, String> {
        let tokens = tokenize(source).map_err(|error| format!("it has {error}"))?;
        Ok(Self {
            source,
            tokens,
            pos: 0,
        })
    }

    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<Token> {
        self.tokens.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<Token> {
        let token = self.peek();
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn at_keyword(&self, keyword: &str) -> bool {
        self.peek()
            .is_some_and(|token| token.is_keyword(self.source, keyword))
    }

    fn keyword_at(&self, ahead: usize, keyword: &str) -> bool {
        self.peek_at(ahead)
            .is_some_and(|token| token.is_keyword(self.source, keyword))
    }

    fn eat_keyword(&mut self, keyword: &str) -> bool {
        let found = self.at_keyword(keyword);
        if found {
            self.pos += 1;
        }
        found
    }

    /// Consumes `keywords` only when all of them follow in order.
    fn eat_keywords(&mut self, keywords: &[&str]) -> bool {
        let found = keywords
            .iter()
            .enumerate()
            .all(|(ahead, keyword)| self.keyword_at(ahead, keyword));
        if found {
            self.pos += keywords.len();
        }
        found
    }

    fn at_punct(&self, punct: &str) -> bool {
        self.peek()
            .is_some_and(|token| token.is_punct(self.source, punct))
    }

    fn eat_punct(&mut self, punct: &str) -> bool {
        let found = self.at_punct(punct);
        if found {
            self.pos += 1;
        }
        found
    }

    fn here(&self) -> String {
        match self.peek() {
            Some(token) => {
                let text = token.text(self.source);
                let shown: String = text.chars().take(40).collect();
                format!("near {shown:?}")
            }
            None => "at the end".into(),
        }
    }

    fn unexpected(&self) -> String {
        format!("it has unexpected syntax {}", self.here())
    }

    fn expect_keyword(&mut self, keyword: &str) -> Result<(), String> {
        if self.eat_keyword(keyword) {
            Ok(())
        } else {
            Err(format!("expected {keyword} {}", self.here()))
        }
    }

    fn expect_punct(&mut self, punct: &str) -> Result<Token, String> {
        match self.peek() {
            Some(token) if token.is_punct(self.source, punct) => {
                self.pos += 1;
                Ok(token)
            }
            _ => Err(format!("expected {punct:?} {}", self.here())),
        }
    }

    fn name(&mut self) -> Result<String, String> {
        match self.peek() {
            Some(token) if token.is_name() => {
                self.pos += 1;
                Ok(dequote(token.text(self.source)))
            }
            _ => Err(format!("expected a name {}", self.here())),
        }
    }

    /// At `(`: consumes through the matching `)` and returns the trimmed
    /// text between them.
    fn parenthesized(&mut self) -> Result<&'a str, String> {
        let source: &'a str = self.source;
        let open = self.expect_punct("(")?;
        let mut depth = 1usize;
        while let Some(token) = self.bump() {
            if token.is_punct(self.source, "(") {
                depth += 1;
            } else if token.is_punct(self.source, ")") {
                depth -= 1;
                if depth == 0 {
                    let inner = trim_space(&source[open.end..token.start]);
                    if inner.is_empty() {
                        return Err("it has an empty ( )".into());
                    }
                    return Ok(inner);
                }
            }
        }
        Err("it has unbalanced parentheses".into())
    }

    /// Consumes an expression up to the first top-level token `stop`
    /// accepts (or the end) and returns its trimmed text.
    fn expression_until(
        &mut self,
        stop: impl Fn(&Token, &str) -> bool,
    ) -> Result<&'a str, String> {
        let source: &'a str = self.source;
        let first = self.pos;
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            if depth == 0 && stop(&token, self.source) {
                break;
            }
            if token.is_punct(self.source, "(") {
                depth += 1;
            } else if token.is_punct(self.source, ")") {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            self.pos += 1;
        }
        if self.pos == first || depth != 0 {
            return Err(format!("expected an expression {}", self.here()));
        }
        let start = self.tokens[first].start;
        let end = self.tokens[self.pos - 1].end;
        Ok(trim_space(&source[start..end]))
    }

    /// `ON CONFLICT …` has no place in the model; refuse rather than drop it.
    fn refuse_conflict_clause(&self) -> Result<(), String> {
        if self.at_keyword("ON") && self.keyword_at(1, "CONFLICT") {
            return Err("it uses an ON CONFLICT clause, which the editor does not support".into());
        }
        Ok(())
    }
}

const COLUMN_CONSTRAINT_WORDS: &[&str] = &[
    "CONSTRAINT",
    "PRIMARY",
    "NOT",
    "NULL",
    "UNIQUE",
    "CHECK",
    "DEFAULT",
    "COLLATE",
    "REFERENCES",
    "AS",
];

const TABLE_CONSTRAINT_WORDS: &[&str] = &["CONSTRAINT", "PRIMARY", "UNIQUE", "CHECK", "FOREIGN"];

fn at_any(cursor: &Cursor<'_>, words: &[&str]) -> bool {
    words.iter().any(|word| cursor.at_keyword(word))
}

/// Parses one `CREATE TABLE` statement.
pub(super) fn parse_create_table(source: &str) -> Result<ParsedTable, String> {
    let mut c = Cursor::new(source)?;
    c.expect_keyword("CREATE")?;
    let _ = c.eat_keyword("TEMP") || c.eat_keyword("TEMPORARY");
    if c.at_keyword("VIRTUAL") {
        return Err("it is a virtual table".into());
    }
    c.expect_keyword("TABLE")?;
    if c.eat_keyword("IF") {
        c.expect_keyword("NOT")?;
        c.expect_keyword("EXISTS")?;
    }
    let mut name = c.name()?;
    if c.eat_punct(".") {
        name = c.name()?;
    }
    if !c.at_punct("(") {
        return Err(format!("expected a column list {}", c.here()));
    }
    let open = c.expect_punct("(")?;
    let mut columns = Vec::new();
    while !at_any(&c, TABLE_CONSTRAINT_WORDS) {
        columns.push(parse_column(&mut c)?);
        if c.eat_punct(",") {
            continue;
        }
        if c.at_punct(")") {
            break;
        }
        return Err(c.unexpected());
    }
    if columns.is_empty() {
        return Err("it has no columns".into());
    }
    let mut constraints = Vec::new();
    if !c.at_punct(")") {
        loop {
            constraints.push(parse_table_constraint(&mut c)?);
            if c.eat_punct(",") {
                continue;
            }
            if c.at_punct(")") {
                break;
            }
            // SQLite accepts table constraints without a separating comma.
            if at_any(&c, TABLE_CONSTRAINT_WORDS) {
                continue;
            }
            return Err(c.unexpected());
        }
    }
    let close = c.expect_punct(")")?;
    let (mut without_rowid, mut strict) = (false, false);
    while !c.at_end() {
        if c.eat_keyword("WITHOUT") {
            c.expect_keyword("ROWID")?;
            without_rowid = true;
        } else if c.eat_keyword("STRICT") {
            strict = true;
        } else if c.eat_punct(";") && c.at_end() {
            break;
        } else {
            return Err(c.unexpected());
        }
        if c.eat_punct(",") {
            if c.at_end() {
                return Err(c.unexpected());
            }
            continue;
        }
        if c.eat_punct(";") && !c.at_end() {
            return Err(c.unexpected());
        }
        if !c.at_end() {
            return Err(c.unexpected());
        }
    }
    Ok(ParsedTable {
        name,
        columns,
        constraints,
        without_rowid,
        strict,
        body: open.start..close.end,
    })
}

fn parse_column(c: &mut Cursor<'_>) -> Result<ParsedColumn, String> {
    let name = c.name()?;
    // Type words run until a constraint keyword. SQLite's grammar absorbs a
    // trailing `GENERATED ALWAYS` into the type and strips it again.
    let mut type_tokens: Vec<Token> = Vec::new();
    while let Some(token) = c.peek() {
        let is_type_word = matches!(token.kind, Kind::Word | Kind::Quoted)
            && !COLUMN_CONSTRAINT_WORDS
                .iter()
                .any(|word| token.is_keyword(c.source, word));
        if !is_type_word {
            break;
        }
        type_tokens.push(token);
        c.pos += 1;
    }
    let count = type_tokens.len();
    let mut generated_keyword = false;
    if count >= 2
        && type_tokens[count - 1].is_keyword(c.source, "ALWAYS")
        && type_tokens[count - 2].is_keyword(c.source, "GENERATED")
    {
        type_tokens.truncate(count - 2);
        generated_keyword = true;
    }
    let mut data_type = String::new();
    if let (Some(first), Some(last)) = (type_tokens.first(), type_tokens.last()) {
        let mut end = last.end;
        if !generated_keyword && c.at_punct("(") {
            c.parenthesized()?;
            end = c.tokens[c.pos - 1].end;
        }
        data_type = c.source[first.start..end].to_owned();
    }
    let mut column = ParsedColumn {
        name,
        data_type,
        not_null: false,
        default: None,
        collation: None,
        generated: None,
        primary_key: None,
        uniques: Vec::new(),
        checks: Vec::new(),
        references: Vec::new(),
    };
    if generated_keyword && !c.at_keyword("AS") {
        return Err(format!("expected AS {}", c.here()));
    }
    let mut pending_name: Option<String> = None;
    loop {
        if c.at_end() || c.at_punct(",") || c.at_punct(")") {
            if pending_name.is_some() {
                return Err(c.unexpected());
            }
            break;
        }
        if c.eat_keyword("CONSTRAINT") {
            if pending_name.is_some() {
                return Err(c.unexpected());
            }
            pending_name = Some(c.name()?);
            continue;
        }
        let constraint_name = pending_name.take();
        if c.eat_keywords(&["PRIMARY", "KEY"]) {
            if c.eat_keyword("DESC") {
                return Err(format!(
                    "column {} has a DESC primary key, which the editor does not support",
                    column.name
                ));
            }
            c.eat_keyword("ASC");
            c.refuse_conflict_clause()?;
            let autoincrement = c.eat_keyword("AUTOINCREMENT");
            if column.primary_key.is_some() {
                return Err("it declares a primary key twice".into());
            }
            column.primary_key = Some(ColumnPrimaryKey {
                name: constraint_name,
                autoincrement,
            });
        } else if c.eat_keywords(&["NOT", "NULL"]) {
            c.refuse_conflict_clause()?;
            column.not_null = true;
        } else if c.eat_keyword("NULL") {
            c.refuse_conflict_clause()?;
        } else if c.eat_keyword("UNIQUE") {
            c.refuse_conflict_clause()?;
            column.uniques.push(constraint_name);
        } else if c.eat_keyword("CHECK") {
            let expression = c.parenthesized()?.to_owned();
            column.checks.push((constraint_name, expression));
        } else if c.eat_keyword("DEFAULT") {
            if column.default.is_some() {
                return Err("it declares a default twice".into());
            }
            column.default = Some(parse_default(c)?);
        } else if c.eat_keyword("COLLATE") {
            column.collation = Some(c.name()?);
        } else if c.eat_keyword("REFERENCES") {
            column
                .references
                .push(parse_reference(c, constraint_name)?);
        } else if c.at_keyword("GENERATED") || c.at_keyword("AS") {
            if c.eat_keyword("GENERATED") {
                c.expect_keyword("ALWAYS")?;
            }
            c.expect_keyword("AS")?;
            let expression = c.parenthesized()?.to_owned();
            let stored = if c.eat_keyword("STORED") {
                true
            } else {
                c.eat_keyword("VIRTUAL");
                false
            };
            if column.generated.is_some() {
                return Err("it declares a generated expression twice".into());
            }
            column.generated = Some(Generated {
                expression: SqlFragment::new(expression),
                stored,
            });
        } else {
            return Err(c.unexpected());
        }
    }
    Ok(column)
}

fn parse_default(c: &mut Cursor<'_>) -> Result<String, String> {
    let Some(token) = c.peek() else {
        return Err("expected a default value at the end".into());
    };
    if token.is_punct(c.source, "(") {
        return Ok(c.parenthesized()?.to_owned());
    }
    if token.is_punct(c.source, "+") || token.is_punct(c.source, "-") {
        match c.peek_at(1) {
            Some(number) if number.kind == Kind::Number => {
                c.pos += 2;
                return Ok(c.source[token.start..number.end].to_owned());
            }
            _ => return Err(format!("it has an unsupported default {}", c.here())),
        }
    }
    match token.kind {
        Kind::Number | Kind::String | Kind::Blob | Kind::Word | Kind::Quoted => {
            c.pos += 1;
            Ok(token.text(c.source).to_owned())
        }
        _ => Err(format!("it has an unsupported default {}", c.here())),
    }
}

fn parse_action(c: &mut Cursor<'_>) -> Result<ReferentialAction, String> {
    if c.eat_keywords(&["SET", "NULL"]) {
        Ok(ReferentialAction::SetNull)
    } else if c.eat_keywords(&["SET", "DEFAULT"]) {
        Ok(ReferentialAction::SetDefault)
    } else if c.eat_keyword("CASCADE") {
        Ok(ReferentialAction::Cascade)
    } else if c.eat_keyword("RESTRICT") {
        Ok(ReferentialAction::Restrict)
    } else if c.eat_keywords(&["NO", "ACTION"]) {
        Ok(ReferentialAction::NoAction)
    } else {
        Err(format!("expected a foreign key action {}", c.here()))
    }
}

/// `INITIALLY DEFERRED` is `Some(true)`, `INITIALLY IMMEDIATE` `Some(false)`.
fn initially(c: &mut Cursor<'_>) -> Result<Option<bool>, String> {
    if !c.eat_keyword("INITIALLY") {
        return Ok(None);
    }
    if c.eat_keyword("DEFERRED") {
        Ok(Some(true))
    } else if c.eat_keyword("IMMEDIATE") {
        Ok(Some(false))
    } else {
        Err(format!("expected DEFERRED or IMMEDIATE {}", c.here()))
    }
}

/// After `REFERENCES`.
fn parse_reference(c: &mut Cursor<'_>, name: Option<String>) -> Result<ParsedReference, String> {
    let table = c.name()?;
    let columns = if c.at_punct("(") {
        name_list(c)?
    } else {
        Vec::new()
    };
    let mut on_update = ReferentialAction::NoAction;
    let mut on_delete = ReferentialAction::NoAction;
    loop {
        if c.at_keyword("ON") && c.keyword_at(1, "DELETE") {
            c.pos += 2;
            on_delete = parse_action(c)?;
        } else if c.at_keyword("ON") && c.keyword_at(1, "UPDATE") {
            c.pos += 2;
            on_update = parse_action(c)?;
        } else if c.at_keyword("MATCH") {
            return Err(
                "a foreign key uses a MATCH clause, which the editor does not support".into(),
            );
        } else {
            break;
        }
    }
    let deferrable = if c.at_keyword("NOT") && c.keyword_at(1, "DEFERRABLE") {
        c.pos += 2;
        initially(c)?;
        Deferrable::Never
    } else if c.eat_keyword("DEFERRABLE") {
        match initially(c)? {
            Some(true) => Deferrable::InitiallyDeferred,
            _ => Deferrable::InitiallyImmediate,
        }
    } else {
        Deferrable::Never
    };
    Ok(ParsedReference {
        name,
        table,
        columns,
        on_update,
        on_delete,
        deferrable,
    })
}

/// `( name, … )` of plain column names.
fn name_list(c: &mut Cursor<'_>) -> Result<Vec<String>, String> {
    c.expect_punct("(")?;
    let mut names = vec![c.name()?];
    while c.eat_punct(",") {
        names.push(c.name()?);
    }
    c.expect_punct(")")?;
    Ok(names)
}

/// `( name [ASC], … )` of a primary key or unique constraint. A collation
/// or DESC has no place in the model and is refused.
fn key_columns(c: &mut Cursor<'_>) -> Result<Vec<String>, String> {
    c.expect_punct("(")?;
    let mut names = Vec::new();
    loop {
        names.push(c.name()?);
        if c.at_keyword("COLLATE") || c.at_keyword("DESC") {
            return Err(
                "a key column has a collation or DESC, which the editor does not support".into(),
            );
        }
        c.eat_keyword("ASC");
        if c.eat_punct(",") {
            continue;
        }
        c.expect_punct(")")?;
        return Ok(names);
    }
}

fn parse_table_constraint(c: &mut Cursor<'_>) -> Result<ParsedConstraint, String> {
    let name = if c.eat_keyword("CONSTRAINT") {
        Some(c.name()?)
    } else {
        None
    };
    if c.eat_keywords(&["PRIMARY", "KEY"]) {
        let columns = key_columns(c)?;
        c.refuse_conflict_clause()?;
        Ok(ParsedConstraint::PrimaryKey { name, columns })
    } else if c.eat_keyword("UNIQUE") {
        let columns = key_columns(c)?;
        c.refuse_conflict_clause()?;
        Ok(ParsedConstraint::Unique { name, columns })
    } else if c.eat_keyword("CHECK") {
        let expression = c.parenthesized()?.to_owned();
        c.refuse_conflict_clause()?;
        Ok(ParsedConstraint::Check { name, expression })
    } else if c.eat_keywords(&["FOREIGN", "KEY"]) {
        let columns = name_list(c)?;
        c.expect_keyword("REFERENCES")?;
        let reference = parse_reference(c, name)?;
        Ok(ParsedConstraint::ForeignKey { columns, reference })
    } else {
        Err(c.unexpected())
    }
}

/// Parses one `CREATE INDEX` statement.
pub(super) fn parse_create_index(source: &str) -> Result<ParsedIndex, String> {
    let mut c = Cursor::new(source)?;
    c.expect_keyword("CREATE")?;
    let unique = c.eat_keyword("UNIQUE");
    c.expect_keyword("INDEX")?;
    if c.eat_keyword("IF") {
        c.expect_keyword("NOT")?;
        c.expect_keyword("EXISTS")?;
    }
    let mut name = c.name()?;
    if c.eat_punct(".") {
        name = c.name()?;
    }
    c.expect_keyword("ON")?;
    let table = c.name()?;
    c.expect_punct("(")?;
    let mut members = Vec::new();
    loop {
        let expression = c.expression_until(|token, source| {
            token.is_punct(source, ",")
                || token.is_keyword(source, "COLLATE")
                || token.is_keyword(source, "ASC")
                || token.is_keyword(source, "DESC")
        })?;
        let collation = if c.eat_keyword("COLLATE") {
            Some(c.name()?)
        } else {
            None
        };
        let descending = if c.eat_keyword("DESC") {
            true
        } else {
            c.eat_keyword("ASC");
            false
        };
        let single = tokenize(expression).ok().and_then(|tokens| match tokens.as_slice() {
            [token] if matches!(token.kind, Kind::Word | Kind::Quoted) => {
                Some(dequote(token.text(expression)))
            }
            _ => None,
        });
        members.push(ParsedIndexMember {
            target: match single {
                Some(name) => MemberTarget::Name(name),
                None => MemberTarget::Expression(expression.to_owned()),
            },
            collation,
            descending,
        });
        if c.eat_punct(",") {
            continue;
        }
        c.expect_punct(")")?;
        break;
    }
    let predicate = if c.eat_keyword("WHERE") {
        Some(
            c.expression_until(|token, source| token.is_punct(source, ";"))?
                .to_owned(),
        )
    } else {
        None
    };
    c.eat_punct(";");
    if !c.at_end() {
        return Err(c.unexpected());
    }
    Ok(ParsedIndex {
        name,
        table,
        unique,
        members,
        predicate,
    })
}

/// `sql` (a stored `CREATE TRIGGER`) with its name qualified by `schema`, so
/// re-running it creates the trigger in the table's own schema. `None` when
/// the text is not a recognizable `CREATE TRIGGER`.
pub(super) fn qualify_trigger(sql: &str, schema: &str) -> Option<String> {
    let mut c = Cursor::new(sql).ok()?;
    if !c.eat_keyword("CREATE") {
        return None;
    }
    let _ = c.eat_keyword("TEMP") || c.eat_keyword("TEMPORARY");
    if !c.eat_keyword("TRIGGER") {
        return None;
    }
    if c.eat_keyword("IF") && !c.eat_keywords(&["NOT", "EXISTS"]) {
        return None;
    }
    let name = c.peek().filter(Token::is_name)?;
    if c.peek_at(1).is_some_and(|next| next.is_punct(sql, ".")) {
        return Some(sql.to_owned());
    }
    Some(format!(
        "{}{}.{}",
        &sql[..name.start],
        crate::quote_double(schema),
        &sql[name.start..]
    ))
}

/// Parses the table body with `sqlparser` and checks that it sees the same
/// columns, column constraints and table constraints as [`parse_create_table`].
/// The table options were already read; only the body is handed over, so
/// option order does not matter.
pub(super) fn cross_check(parsed: &ParsedTable, source: &str) -> Result<(), String> {
    let sql = format!("CREATE TABLE t {}", &source[parsed.body.clone()]);
    let statements = Parser::parse_sql(&SQLiteDialect {}, &sql).map_err(|error| error.to_string())?;
    let [Statement::CreateTable(table)] = statements.as_slice() else {
        return Err("sqlparser did not read one CREATE TABLE".into());
    };
    if table.columns.len() != parsed.columns.len() {
        return Err(format!(
            "sqlparser read {} columns, the definition has {}",
            table.columns.len(),
            parsed.columns.len()
        ));
    }
    for (column, ours) in table.columns.iter().zip(&parsed.columns) {
        if column.name.value != ours.name {
            return Err(format!(
                "sqlparser read column {:?} where the definition has {:?}",
                column.name.value, ours.name
            ));
        }
        let mut not_null = false;
        let mut default = false;
        let mut collation = false;
        let mut generated = None;
        let mut primary_keys = 0usize;
        let mut uniques = 0usize;
        let mut references = 0usize;
        let mut checks = 0usize;
        let mut autoincrement = false;
        for definition in &column.options {
            match &definition.option {
                ColumnOption::Null => {}
                ColumnOption::NotNull => not_null = true,
                ColumnOption::Default(_) => default = true,
                ColumnOption::Collation(_) => collation = true,
                ColumnOption::Generated {
                    generation_expr: Some(_),
                    generation_expr_mode,
                    ..
                } => {
                    generated = Some(matches!(
                        generation_expr_mode,
                        Some(GeneratedExpressionMode::Stored)
                    ))
                }
                ColumnOption::PrimaryKey(_) => primary_keys += 1,
                ColumnOption::Unique(_) => uniques += 1,
                ColumnOption::ForeignKey(_) => references += 1,
                ColumnOption::Check(_) => checks += 1,
                ColumnOption::DialectSpecific(tokens) => {
                    if tokens
                        .iter()
                        .any(|token| token.to_string().eq_ignore_ascii_case("AUTOINCREMENT"))
                    {
                        autoincrement = true;
                    }
                }
                _ => {
                    return Err(format!(
                        "sqlparser read an unsupported option on column {:?}",
                        ours.name
                    ))
                }
            }
        }
        let expected_pk = usize::from(ours.primary_key.is_some());
        let agrees = not_null == ours.not_null
            && default == ours.default.is_some()
            && collation == ours.collation.is_some()
            && generated == ours.generated.as_ref().map(|g| g.stored)
            && primary_keys == expected_pk
            && uniques == ours.uniques.len()
            && references == ours.references.len()
            && checks == ours.checks.len()
            && autoincrement
                == ours
                    .primary_key
                    .as_ref()
                    .is_some_and(|key| key.autoincrement);
        if !agrees {
            return Err(format!(
                "sqlparser read different constraints on column {:?}",
                ours.name
            ));
        }
    }
    let theirs: Vec<&str> = table
        .constraints
        .iter()
        .map(|constraint| match constraint {
            TableConstraint::PrimaryKey(_) => "primary key",
            TableConstraint::Unique(_) => "unique",
            TableConstraint::Check(_) => "check",
            TableConstraint::ForeignKey(_) => "foreign key",
            _ => "other",
        })
        .collect();
    let ours: Vec<&str> = parsed.constraints.iter().map(ParsedConstraint::kind).collect();
    if theirs != ours {
        return Err(format!(
            "sqlparser read table constraints {theirs:?}, the definition has {ours:?}"
        ));
    }
    for (constraint, ours) in table.constraints.iter().zip(&parsed.constraints) {
        if let (
            TableConstraint::ForeignKey(key),
            ParsedConstraint::ForeignKey { columns, .. },
        ) = (constraint, ours)
        {
            let names: Vec<&str> = key.columns.iter().map(|c| c.value.as_str()).collect();
            if names != columns.iter().map(String::as_str).collect::<Vec<_>>() {
                return Err("sqlparser read different foreign key columns".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "definition_tests.rs"]
mod tests;
