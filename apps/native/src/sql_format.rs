//! Bounded layout and keyword formatting, not PostgreSQL grammar validation.
//! Protected bodies remain byte-exact. Only ASCII-whitespace gaps and known
//! keyword case can change. Unsupported/ambiguous input is refused atomically.
use std::{fmt, ops::Range};

#[path = "sql_format/keywords.rs"]
mod keywords;
#[path = "sql_format/scanner.rs"]
mod scanner;
use scanner::{Kind, Token};

pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_TOKENS: usize = 4096;
pub const MAX_DEPTH: usize = 64;
pub const MAX_OUTPUT_BYTES: usize = MAX_INPUT_BYTES;
pub const MAX_EDITS: usize = MAX_TOKENS * 2 + 1;
/// Conservative allowance for input ownership, token/gap vectors, edit text,
/// and the optional simultaneous materialized output. Excludes editor history.
pub const WORKING_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    InputLimit,
    TokenLimit,
    DepthLimit,
    OutputLimit,
    Incomplete,
    AmbiguousString,
    UnsupportedWhitespace,
    InvalidToken,
    Unbalanced,
}
impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputLimit => "Formatting supports at most 64 KiB of SQL",
            Self::TokenLimit => "Formatting supports at most 4096 tokens",
            Self::DepthLimit => "Formatting nesting exceeds 64 levels",
            Self::OutputLimit => "Formatted SQL would exceed 64 KiB",
            Self::Incomplete => "SQL contains an unfinished string, identifier or comment",
            Self::AmbiguousString => {
                "Backslash string boundaries depend on the PostgreSQL session setting"
            }
            Self::UnsupportedWhitespace => {
                "Non-ASCII whitespace outside quoted text cannot be formatted safely"
            }
            Self::InvalidToken => "SQL contains a token this formatter cannot safely recognize",
            Self::Unbalanced => "SQL has unmatched parentheses, brackets or CASE/END",
        })
    }
}
impl std::error::Error for FormatError {}

#[derive(Debug, PartialEq, Eq)]
pub struct FormatEdit {
    pub range: Range<usize>,
    pub text: String,
}

#[derive(Clone, Copy)]
enum Gap {
    Keep,
    Empty,
    Space,
    Line { indent: usize, lines: usize },
}
impl Gap {
    fn len(self, original: &str) -> usize {
        match self {
            Self::Keep => original.len(),
            Self::Empty => 0,
            Self::Space => 1,
            Self::Line { indent, lines } => indent + lines,
        }
    }
    fn matches(self, original: &str) -> bool {
        match self {
            Self::Keep => true,
            Self::Empty => original.is_empty(),
            Self::Space => original == " ",
            Self::Line { indent, lines } => {
                original.len() == indent + lines
                    && original.bytes().take(lines).all(|b| b == b'\n')
                    && original.bytes().skip(lines).all(|b| b == b' ')
            }
        }
    }
    fn materialize(self, original: &str) -> String {
        let mut text = String::with_capacity(self.len(original));
        match self {
            Self::Keep => text.push_str(original),
            Self::Empty => {}
            Self::Space => text.push(' '),
            Self::Line { indent, lines } => {
                for _ in 0..lines {
                    text.push('\n');
                }
                for _ in 0..indent {
                    text.push(' ');
                }
            }
        }
        text
    }
}
fn line(depth: usize) -> Gap {
    Gap::Line {
        indent: depth * 2,
        lines: 1,
    }
}

/// Returns ordered, disjoint edits against the original input. Ranges contain
/// ASCII whitespace, empty gap insertion points, or complete known keywords.
/// Apply together in one editor transaction; protected lexemes are never edited.
pub fn format_edits(sql: &str) -> Result<Vec<FormatEdit>, FormatError> {
    let tokens = scanner::scan(sql)?;
    if tokens.is_empty() {
        return Ok(vec![]);
    }
    let mut gaps = vec![Gap::Space; tokens.len() + 1];
    gaps[0] = Gap::Empty;
    gaps[tokens.len()] = if tokens.last().is_some_and(|t| t.kind == Kind::LineComment) {
        Gap::Keep
    } else {
        Gap::Empty
    };
    for i in 1..tokens.len() {
        gaps[i] = ordinary_gap(&tokens[i - 1], &tokens[i]);
    }
    apply_layout(sql, &tokens, &mut gaps);
    // Whitespace/comment gaps between string literals are semantic in PostgreSQL.
    // Preserve the whole run, including a newline that joins continued literals.
    for (i, token) in tokens.iter().enumerate() {
        if token.kind != Kind::String {
            continue;
        }
        let mut next = i + 1;
        while next < tokens.len() && tokens[next].kind.is_comment() {
            next += 1;
        }
        if next < tokens.len() && tokens[next].kind == Kind::String {
            gaps[i + 1..=next].fill(Gap::Keep);
        }
    }
    for i in 1..tokens.len() {
        if tokens[i - 1].kind == Kind::LineComment && !matches!(gaps[i], Gap::Keep) {
            gaps[i] = line(tokens[i].depth);
        }
    }
    // Premeasure before any edit string or output allocation. Kept tokens are
    // charged verbatim and every added newline/indent counts toward the cap.
    let uppercase = keywords::uppercase_tokens(sql, &tokens);
    let replace_keyword = |i: usize| {
        uppercase[i]
            && sql[tokens[i].start..tokens[i].end]
                .bytes()
                .any(|b| b.is_ascii_lowercase())
    };
    let mut total = tokens.iter().map(|t| t.end - t.start).sum::<usize>();
    let mut count = (0..tokens.len()).filter(|i| replace_keyword(*i)).count();
    for (i, gap) in gaps.iter().enumerate() {
        let range = gap_range(sql, &tokens, i);
        total = total
            .checked_add(gap.len(&sql[range.clone()]))
            .ok_or(FormatError::OutputLimit)?;
        if total > MAX_OUTPUT_BYTES {
            return Err(FormatError::OutputLimit);
        }
        if !gap.matches(&sql[range]) {
            count += 1;
        }
    }
    if count > MAX_EDITS {
        return Err(FormatError::TokenLimit);
    }
    let mut edits = Vec::with_capacity(count);
    for (i, gap) in gaps.into_iter().enumerate() {
        let range = gap_range(sql, &tokens, i);
        let original = &sql[range.clone()];
        if !gap.matches(original) {
            edits.push(FormatEdit {
                range,
                text: gap.materialize(original),
            });
        }
        if i < tokens.len() && replace_keyword(i) {
            let token = &tokens[i];
            edits.push(FormatEdit {
                range: token.start..token.end,
                text: sql[token.start..token.end].to_ascii_uppercase(),
            });
        }
    }
    Ok(edits)
}
fn apply_layout(sql: &str, tokens: &[Token], gaps: &mut [Gap]) {
    let mut list = [false; MAX_DEPTH + 1];
    let mut clauses = [false; MAX_DEPTH + 1];
    for (i, token) in tokens.iter().enumerate() {
        if matches!(
            token.kind,
            Kind::OpenParen | Kind::OpenBracket | Kind::CaseOpen
        ) {
            list[token.depth + 1] = false;
            clauses[token.depth + 1] = false;
        }
        if token.kind == Kind::CloseParen && clauses[token.depth + 1] {
            gaps[i] = line(token.depth);
        }
        if let Some((last, column_list)) = clause(sql, tokens, i) {
            if i > 0 && tokens[i - 1].kind != Kind::Semicolon {
                gaps[i] = line(token.depth);
            }
            let join = word(sql, token, "join")
                || word(sql, &tokens[last], "join")
                || word(sql, token, "on");
            if last + 1 < tokens.len() {
                gaps[last + 1] = if join {
                    Gap::Space
                } else {
                    line(token.depth + 1)
                };
            }
            clauses[token.depth] = true;
            list[token.depth] = column_list;
        }
        if token.kind == Kind::Comma && list[token.depth] && i + 1 < tokens.len() {
            gaps[i + 1] = line(token.depth + 1);
        }
        if token.kind == Kind::Semicolon && i + 1 < tokens.len() {
            gaps[i + 1] = Gap::Line {
                indent: 0,
                lines: 2,
            };
            list.fill(false);
            clauses.fill(false);
        }
    }
}
fn ordinary_gap(previous: &Token, current: &Token) -> Gap {
    if matches!(
        current.kind,
        Kind::Comma | Kind::Semicolon | Kind::CloseParen | Kind::CloseBracket
    ) || matches!(previous.kind, Kind::OpenParen | Kind::OpenBracket)
    {
        return Gap::Empty;
    }
    if matches!(previous.kind, Kind::Dot | Kind::Colon)
        || matches!(current.kind, Kind::Dot | Kind::Colon)
    {
        return if previous.end == current.start {
            Gap::Empty
        } else {
            Gap::Space
        };
    }
    // Distinguish a function's original spelling from a keyword followed by a
    // parenthesized expression; whitespace here is preserved conservatively.
    if current.kind == Kind::OpenParen && previous.end == current.start {
        return Gap::Empty;
    }
    if previous.end == current.start
        && (previous.kind == Kind::Operator || current.kind == Kind::Operator)
    {
        return Gap::Empty;
    }
    Gap::Space
}
fn word(sql: &str, token: &Token, expected: &str) -> bool {
    token.kind == Kind::Word && sql[token.start..token.end].eq_ignore_ascii_case(expected)
}
fn clause(sql: &str, tokens: &[Token], i: usize) -> Option<(usize, bool)> {
    if i > 0 && tokens[i - 1].kind == Kind::Dot {
        return None;
    }
    let at = |offset: usize, expected| {
        tokens
            .get(i + offset)
            .is_some_and(|t| word(sql, t, expected))
    };
    if at(0, "group") || at(0, "order") {
        return at(1, "by").then_some((i + 1, true));
    }
    if ["left", "right", "full", "inner", "cross", "natural"]
        .iter()
        .any(|w| at(0, w))
    {
        if at(1, "join") {
            return Some((i + 1, false));
        }
        if at(1, "outer") && at(2, "join") {
            return Some((i + 2, false));
        }
    }
    if at(0, "join")
        && i > 0
        && [
            "left", "right", "full", "inner", "cross", "natural", "outer",
        ]
        .iter()
        .any(|w| word(sql, &tokens[i - 1], w))
    {
        return None;
    }
    if at(0, "select") || at(0, "returning") {
        return Some((i, true));
    }
    if ["from", "where", "having", "limit", "offset", "join", "on"]
        .iter()
        .any(|w| at(0, w))
    {
        return Some((i, false));
    }
    None
}
fn gap_range(sql: &str, tokens: &[Token], i: usize) -> Range<usize> {
    (if i == 0 { 0 } else { tokens[i - 1].end })..tokens.get(i).map_or(sql.len(), |t| t.start)
}
#[cfg(test)]
pub fn format_sql(sql: &str) -> Result<String, FormatError> {
    let edits = format_edits(sql)?;
    let size = edits.iter().try_fold(sql.len(), |size, edit| {
        size.checked_sub(edit.range.len())
            .and_then(|n| n.checked_add(edit.text.len()))
            .filter(|n| *n <= MAX_OUTPUT_BYTES)
            .ok_or(FormatError::OutputLimit)
    })?;
    let mut result = String::with_capacity(size);
    let mut previous = 0;
    for edit in edits {
        result.push_str(&sql[previous..edit.range.start]);
        result.push_str(&edit.text);
        previous = edit.range.end;
    }
    result.push_str(&sql[previous..]);
    Ok(result)
}
#[cfg(test)]
#[path = "sql_format/tests.rs"]
mod tests;
