//! Named parameters and execution shapes for Query Session SQL (ADR-0031):
//! the `:name` scan, the `$k` rewrite with its position map, the binding
//! limits, and the pure planner that picks a shape before anything is sent.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::sql_class::{describe_script, StatementClass, StatementFacts};
use super::sql_lex::{lex_sql_spanned, SpannedToken, SqlIdentifier, SqlToken};

pub(crate) const MAX_NAME_BYTES: usize = 63;
pub(crate) const MAX_PARAMETERS: usize = 256;
pub(crate) const MAX_VALUE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_TOTAL_VALUE_BYTES: usize = 4 * 1024 * 1024;
/// The per Result Set retention cap, which a row limit can only lower.
pub(crate) const MAX_ROW_LIMIT: i64 = 10_000;

/// One supplied parameter. `Debug` never prints the value.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ParameterValue {
    pub name: String,
    pub value: Option<String>,
}

impl fmt::Debug for ParameterValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParameterValue")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Values in `$k` order. `Debug` prints only how many there are.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BoundValues(pub Vec<Option<String>>);

impl fmt::Debug for BoundValues {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "BoundValues(<{} redacted>)", self.0.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ParameterRejectionReason {
    Unlexable,
    MultipleStatements,
    PositionalPlaceholder,
    MissingValue,
    DuplicateName,
    NameTooLong,
    TooManyParameters,
    ValueTooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParameterRejection {
    pub reason: ParameterRejectionReason,
    /// The names the reason is about. Names come from the SQL text, never
    /// from a value.
    pub names: Vec<String>,
}

impl ParameterRejection {
    fn new(reason: ParameterRejectionReason, names: Vec<String>) -> Self {
        Self { reason, names }
    }
}

#[derive(Debug)]
struct Occurrence {
    /// Byte range of `:name`, colon included.
    start: usize,
    end: usize,
    /// Zero-based index into `ParameterScan::names`.
    index: usize,
}

#[derive(Debug)]
pub(crate) struct ParameterScan {
    names: Vec<String>,
    occurrences: Vec<Occurrence>,
    positional: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Enclosure {
    Paren,
    /// `ARRAY[...]` and the nested rows directly inside one.
    Constructor,
    Subscript,
}

/// Finds named parameters with the classifier's own lexer regions. Fails when
/// the SQL does not lex or its brackets and parentheses do not balance.
pub(crate) fn scan_parameters(sql: &str) -> Result<ParameterScan, ()> {
    let tokens = lex_sql_spanned(sql)?;
    let mut scan = ParameterScan {
        names: Vec::new(),
        occurrences: Vec::new(),
        positional: false,
    };
    let mut indexes = HashMap::<&str, usize>::new();
    let mut enclosures = Vec::new();
    for (position, spanned) in tokens.iter().enumerate() {
        let previous = position.checked_sub(1).map(|index| &tokens[index]);
        match spanned.token {
            SqlToken::Symbol('(') => enclosures.push(Enclosure::Paren),
            SqlToken::Symbol('[') => {
                let after_array = previous.is_some_and(|token| is_keyword(&token.token, "array"));
                // A nested row of a constructor starts an element; a bracket
                // that follows an expression subscripts it.
                let nested_row = enclosures.last() == Some(&Enclosure::Constructor)
                    && matches!(
                        previous.map(|token| &token.token),
                        Some(SqlToken::Symbol('[' | ','))
                    );
                enclosures.push(if after_array || nested_row {
                    Enclosure::Constructor
                } else {
                    Enclosure::Subscript
                });
            }
            SqlToken::Symbol(')') => {
                if enclosures.pop() != Some(Enclosure::Paren) {
                    return Err(());
                }
            }
            SqlToken::Symbol(']') => {
                if !matches!(
                    enclosures.pop(),
                    Some(Enclosure::Constructor | Enclosure::Subscript)
                ) {
                    return Err(());
                }
            }
            SqlToken::Opaque => {
                let text = &sql.as_bytes()[spanned.start..spanned.end];
                if text.first() == Some(&b'$') && text.get(1).is_some_and(u8::is_ascii_digit) {
                    scan.positional = true;
                }
                if text != b":" || enclosures.last() == Some(&Enclosure::Subscript) {
                    continue;
                }
                if previous.is_some_and(|token| glues_to_colon(sql, token, spanned.start)) {
                    continue;
                }
                let Some(name) = tokens
                    .get(position + 1)
                    .and_then(|next| parameter_name(next, spanned.end))
                else {
                    continue;
                };
                let next_index = indexes.len();
                let index = *indexes.entry(name).or_insert_with(|| {
                    scan.names.push(name.to_owned());
                    next_index
                });
                scan.occurrences.push(Occurrence {
                    start: spanned.start,
                    end: spanned.end + name.len(),
                    index,
                });
            }
            _ => {}
        }
    }
    if !enclosures.is_empty() {
        return Err(());
    }
    Ok(scan)
}

/// A colon directly after an identifier character, a digit, a closing quote,
/// `)`, `]`, or another colon is a cast, a slice bound, or a `key:value`
/// separator, never the start of a parameter.
fn glues_to_colon(sql: &str, previous: &SpannedToken, colon: usize) -> bool {
    previous.end == colon
        && sql.as_bytes()[..colon].last().is_some_and(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'$' | b'\'' | b'"' | b')' | b']' | b':')
        })
}

/// The name is the whole unquoted identifier glued to the colon. An
/// identifier holding `$` is outside the grammar and is left to the server.
fn parameter_name(next: &SpannedToken, colon_end: usize) -> Option<&str> {
    match &next.token {
        SqlToken::Identifier(SqlIdentifier {
            value,
            quoted: false,
        }) if next.start == colon_end && !value.contains('$') => Some(value),
        _ => None,
    }
}

fn is_keyword(token: &SqlToken, keyword: &str) -> bool {
    matches!(
        token,
        SqlToken::Identifier(SqlIdentifier { value, quoted: false })
            if value.eq_ignore_ascii_case(keyword)
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    /// Zero-based character offsets of `:name` and of the `$k` replacing it.
    original: u32,
    original_len: u32,
    rewritten: u32,
    rewritten_len: u32,
}

/// Maps character positions in rewritten SQL back to the user's SQL. Server
/// error positions count characters, not bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PositionMap {
    segments: Vec<Segment>,
}

impl PositionMap {
    /// Translates a one-based position. A position inside a `$k` maps to the
    /// colon of the name it replaced.
    pub(crate) fn to_original(&self, position: u32) -> u32 {
        let target = position.saturating_sub(1);
        let mut shift = 0_i64;
        for segment in &self.segments {
            if target < segment.rewritten {
                break;
            }
            if target < segment.rewritten + segment.rewritten_len {
                return segment.original + 1;
            }
            shift = i64::from(segment.original + segment.original_len)
                - i64::from(segment.rewritten + segment.rewritten_len);
        }
        u32::try_from(i64::from(target) + shift).unwrap_or(0) + 1
    }
}

impl ParameterScan {
    /// Distinct names in order of first appearance.
    pub(crate) fn names(&self) -> &[String] {
        &self.names
    }

    /// Replaces every `:name` with its `$k`. Repeats reuse the same `$k`.
    pub(crate) fn rewrite(&self, sql: &str) -> (String, PositionMap) {
        let mut rewritten = String::with_capacity(sql.len());
        let mut segments = Vec::with_capacity(self.occurrences.len());
        let mut cursor = 0usize;
        let mut original_chars = 0u32;
        let mut rewritten_chars = 0u32;
        for occurrence in &self.occurrences {
            let before = &sql[cursor..occurrence.start];
            let before_chars = char_count(before);
            rewritten.push_str(before);
            original_chars += before_chars;
            rewritten_chars += before_chars;
            let placeholder = format!("${}", occurrence.index + 1);
            // A name is ASCII, so its byte length is its character length.
            let original_len = (occurrence.end - occurrence.start) as u32;
            let rewritten_len = placeholder.len() as u32;
            segments.push(Segment {
                original: original_chars,
                original_len,
                rewritten: rewritten_chars,
                rewritten_len,
            });
            rewritten.push_str(&placeholder);
            original_chars += original_len;
            rewritten_chars += rewritten_len;
            cursor = occurrence.end;
        }
        rewritten.push_str(&sql[cursor..]);
        (rewritten, PositionMap { segments })
    }

    /// Orders the supplied values by `$k`. Supplied names the statement does
    /// not use are ignored entirely: the editor keeps a value for every name
    /// in the tab.
    pub(crate) fn bind(
        &self,
        supplied: &[ParameterValue],
    ) -> Result<BoundValues, ParameterRejection> {
        use ParameterRejectionReason::*;
        let too_long = self
            .names
            .iter()
            .filter(|name| name.len() > MAX_NAME_BYTES)
            .cloned()
            .collect::<Vec<_>>();
        if !too_long.is_empty() {
            return Err(ParameterRejection::new(NameTooLong, too_long));
        }
        if self.names.len() > MAX_PARAMETERS {
            return Err(ParameterRejection::new(TooManyParameters, Vec::new()));
        }
        let mut found = HashMap::<&str, (usize, &Option<String>)>::new();
        for parameter in supplied {
            found
                .entry(parameter.name.as_str())
                .and_modify(|(count, _)| *count += 1)
                .or_insert((1, &parameter.value));
        }
        let used = |wanted: fn(usize) -> bool| {
            self.names
                .iter()
                .filter(|name| wanted(found.get(name.as_str()).map_or(0, |(count, _)| *count)))
                .cloned()
                .collect::<Vec<_>>()
        };
        let duplicated = used(|count| count > 1);
        if !duplicated.is_empty() {
            return Err(ParameterRejection::new(DuplicateName, duplicated));
        }
        let missing = used(|count| count == 0);
        if !missing.is_empty() {
            return Err(ParameterRejection::new(MissingValue, missing));
        }
        let values = self
            .names
            .iter()
            .map(|name| found[name.as_str()].1.clone())
            .collect::<Vec<_>>();
        let bytes = |value: &Option<String>| value.as_ref().map_or(0, String::len);
        let too_large = self
            .names
            .iter()
            .zip(&values)
            .filter(|(_, value)| bytes(value) > MAX_VALUE_BYTES)
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        if !too_large.is_empty() {
            return Err(ParameterRejection::new(ValueTooLarge, too_large));
        }
        if values.iter().map(bytes).sum::<usize>() > MAX_TOTAL_VALUE_BYTES {
            return Err(ParameterRejection::new(ValueTooLarge, Vec::new()));
        }
        Ok(BoundValues(values))
    }
}

fn char_count(text: &str) -> u32 {
    text.chars().count() as u32
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExecutionShape {
    /// One simple-protocol request. A row limit caps retention per Result Set
    /// while the driver keeps reading.
    Script { sql: String, row_limit: Option<u32> },
    /// One statement declared as a server-side cursor.
    CursorRead {
        statement: String,
        values: BoundValues,
        row_limit: Option<u32>,
    },
    /// One parameterized statement that must not return rows.
    BoundCommand {
        statement: String,
        values: BoundValues,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionPlan {
    pub shape: ExecutionShape,
    /// The whole script as the policy must see it, with every parameter an
    /// opaque `$k`. `None` when the Script shape already holds that text.
    policy_text: Option<String>,
    positions: PositionMap,
    /// Characters before the statement in the text `positions` maps from.
    statement_offset: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanRefusal {
    Parameters(ParameterRejection),
    InvalidRowLimit,
}

impl ExecutionPlan {
    /// The text the statement policy must classify. A `:name` could pass for
    /// a keyword (`SET a = :where`); an opaque `$k` cannot.
    pub(crate) fn policy_sql(&self) -> &str {
        match (&self.policy_text, &self.shape) {
            (Some(text), _) => text,
            (None, ExecutionShape::Script { sql, .. }) => sql,
            (None, ExecutionShape::CursorRead { statement, .. })
            | (None, ExecutionShape::BoundCommand { statement, .. }) => statement,
        }
    }

    /// Translates a server error position in `prefix + statement` to the
    /// user's SQL. A position inside the prefix has no counterpart.
    pub(crate) fn original_position(&self, position: u32, prefix_chars: u32) -> Option<u32> {
        let in_statement = position.checked_sub(prefix_chars).filter(|at| *at > 0)?;
        Some(
            self.positions
                .to_original(in_statement + self.statement_offset),
        )
    }
}

/// Picks the shape from the SQL text and the payload alone. `parameters` is
/// `Some` in parameter mode, even when empty; only then is the SQL scanned.
pub(crate) fn plan_execution(
    sql: String,
    parameters: Option<&[ParameterValue]>,
    row_limit: Option<i64>,
) -> Result<ExecutionPlan, PlanRefusal> {
    let row_limit = match row_limit {
        None => None,
        Some(limit) if (1..=MAX_ROW_LIMIT).contains(&limit) => Some(limit as u32),
        Some(_) => return Err(PlanRefusal::InvalidRowLimit),
    };
    let reject = |reason, names| PlanRefusal::Parameters(ParameterRejection::new(reason, names));
    if let Some(supplied) = parameters {
        use ParameterRejectionReason::*;
        let scan = scan_parameters(&sql).map_err(|()| reject(Unlexable, Vec::new()))?;
        if !scan.names().is_empty() {
            if scan.positional {
                return Err(reject(PositionalPlaceholder, Vec::new()));
            }
            let (rewritten, positions) = scan.rewrite(&sql);
            let mut statements =
                describe_script(&rewritten).map_err(|()| reject(Unlexable, Vec::new()))?;
            let statement = match (statements.pop(), statements.is_empty()) {
                (Some(statement), true) => statement,
                _ => return Err(reject(MultipleStatements, Vec::new())),
            };
            let values = scan.bind(supplied).map_err(PlanRefusal::Parameters)?;
            let text = rewritten[statement.start..statement.end].to_owned();
            let statement_offset = char_count(&rewritten[..statement.start]);
            return Ok(ExecutionPlan {
                shape: if cursor_eligible(&statement) {
                    ExecutionShape::CursorRead {
                        statement: text,
                        values,
                        row_limit,
                    }
                } else {
                    ExecutionShape::BoundCommand {
                        statement: text,
                        values,
                    }
                },
                policy_text: Some(rewritten),
                positions,
                statement_offset,
            });
        }
    }
    let cursor = row_limit.and_then(|_| match describe_script(&sql).ok()?.as_slice() {
        [statement] if cursor_eligible(statement) && !statement.native_placeholder => {
            Some((statement.start, statement.end))
        }
        _ => None,
    });
    Ok(match cursor {
        Some((start, end)) => ExecutionPlan {
            statement_offset: char_count(&sql[..start]),
            shape: ExecutionShape::CursorRead {
                statement: sql[start..end].to_owned(),
                values: BoundValues(Vec::new()),
                row_limit,
            },
            // The policy still sees the whole script, as the Script shape would.
            policy_text: Some(sql),
            positions: PositionMap::default(),
        },
        None => ExecutionPlan {
            shape: ExecutionShape::Script { sql, row_limit },
            policy_text: None,
            positions: PositionMap::default(),
            statement_offset: 0,
        },
    })
}

/// A cursor cannot wrap a write, and it locks rows as they are fetched, so a
/// row limit would lock a different set than a locking read names.
fn cursor_eligible(statement: &StatementFacts) -> bool {
    statement.class == StatementClass::Read
        && matches!(
            statement.head.as_deref(),
            Some("SELECT" | "VALUES" | "TABLE" | "WITH")
        )
        && !statement.row_locking
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(sql: &str) -> Vec<String> {
        scan_parameters(sql).expect("scan").names().to_vec()
    }

    fn rewrite(sql: &str) -> String {
        scan_parameters(sql).expect("scan").rewrite(sql).0
    }

    fn value(name: &str, value: &str) -> ParameterValue {
        ParameterValue {
            name: name.into(),
            value: Some(value.into()),
        }
    }

    #[test]
    fn names_inside_opaque_regions_are_not_parameters() {
        for sql in [
            "SELECT ':a'",
            "SELECT E':a\\' :b'",
            "SELECT $$ :a $$",
            "SELECT $tag$ :a $tag$",
            "SELECT 1 -- :a",
            "SELECT /* :a /* :b */ */ 1",
            "SELECT \":a\" FROM t",
        ] {
            assert!(names(sql).is_empty(), "{sql}");
        }
        assert_eq!(names("SELECT ':x', :a /* :y */ FROM \":z\" -- :w"), ["a"]);
    }

    #[test]
    fn casts_assignments_and_key_value_colons_are_not_parameters() {
        for sql in [
            "SELECT x::int",
            "SELECT '1'::int",
            "SELECT 1 ::int",
            "SELECT f(a := 1)",
            "SELECT JSON_OBJECT('k':v)",
            "SELECT JSON_OBJECT(k:v)",
            "SELECT \"k\":v",
            "SELECT f(1):v",
            "SELECT 1:v",
            "SELECT : a",
        ] {
            assert!(names(sql).is_empty(), "{sql}");
        }
        assert_eq!(names("SELECT :a::int, f(b := :b)"), ["a", "b"]);
    }

    #[test]
    fn slices_are_not_parameters_but_constructors_and_parenthesized_subscripts_are() {
        for sql in [
            "SELECT a[1:n]",
            "SELECT a[:n]",
            "SELECT a[1 :n]",
            "SELECT a[f(1) :n]",
            // A subscript inside a constructor is still a subscript.
            "SELECT ARRAY[a[:n]]",
            "SELECT (ARRAY[1, 2])[:n]",
        ] {
            assert!(names(sql).is_empty(), "{sql}");
        }
        assert_eq!(names("SELECT a[(:n)]"), ["n"]);
        assert_eq!(names("SELECT ARRAY[:a, :b]"), ["a", "b"]);
        assert_eq!(names("SELECT array [:a]"), ["a"]);
        assert_eq!(names("SELECT ARRAY[[:a, 1], [2, :b]]"), ["a", "b"]);
        assert_eq!(names("SELECT ARRAY[ARRAY[:a], ARRAY[:b]]"), ["a", "b"]);
        assert_eq!(names("SELECT ARRAY[a[(:i)], :b]"), ["i", "b"]);
    }

    #[test]
    fn names_follow_openers_separators_operators_and_whitespace() {
        assert_eq!(
            names("SELECT f(:a,:b) WHERE x=:c AND y = :d\n\t:e"),
            ["a", "b", "c", "d", "e"]
        );
        assert_eq!(names("SELECT :_a1, :A"), ["_a1", "A"]);
        // The grammar has no `$`, so the token is left for the server.
        assert!(names("SELECT :a$b").is_empty());
        // A leading digit is not a name.
        assert!(names("SELECT :1a").is_empty());
    }

    #[test]
    fn repeats_reuse_one_placeholder_in_first_appearance_order() {
        let sql = "SELECT :b, :a, :b, :B WHERE x = :a";
        assert_eq!(names(sql), ["b", "a", "B"]);
        assert_eq!(rewrite(sql), "SELECT $1, $2, $1, $3 WHERE x = $2");
    }

    #[test]
    fn native_placeholders_are_reported_and_left_alone() {
        let scan = scan_parameters("SELECT $1, $23").expect("scan");
        assert!(scan.names().is_empty());
        assert!(scan.positional);
        let scan = scan_parameters("SELECT $1, :a, '$2', $$ $3 $$").expect("scan");
        assert_eq!(scan.names(), ["a"]);
        assert!(scan.positional);
        assert!(!scan_parameters("SELECT '$1', :a").expect("scan").positional);
    }

    #[test]
    fn unlexable_sql_fails_the_scan() {
        for sql in [
            "SELECT é, :a",
            // standard_conforming_strings would move this literal's end.
            r"SELECT 'a\'' , :a --'",
            "SELECT 'open, :a",
            "SELECT (:a",
            "SELECT :a)",
            "SELECT a[:a",
            "SELECT (a[1)]",
            "SELECT /* :a",
        ] {
            assert!(scan_parameters(sql).is_err(), "{sql}");
        }
        // Non-ASCII is fine where the lexer treats it as opaque.
        assert_eq!(names("SELECT 'é', \"ü\", :a -- ß"), ["a"]);
    }

    #[test]
    fn positions_translate_around_placeholders_with_multibyte_text_before() {
        // 'héllo' is 7 characters and 8 bytes.
        let sql = "SELECT 'héllo', :first_name, x FROM t WHERE y = :first_name + :z";
        let scan = scan_parameters(sql).expect("scan");
        let (rewritten, map) = scan.rewrite(sql);
        assert_eq!(rewritten, "SELECT 'héllo', $1, x FROM t WHERE y = $1 + $2");
        let original = |needle: &str, nth: usize| {
            let byte = sql.match_indices(needle).nth(nth).unwrap().0;
            sql[..byte].chars().count() as u32 + 1
        };
        let rewritten_at = |needle: &str, nth: usize| {
            let byte = rewritten.match_indices(needle).nth(nth).unwrap().0;
            rewritten[..byte].chars().count() as u32 + 1
        };
        // Before the first placeholder nothing moves.
        assert_eq!(map.to_original(1), 1);
        assert_eq!(
            map.to_original(rewritten_at("'héllo'", 0)),
            original("'héllo'", 0)
        );
        // Either character of `$1` maps to the colon of the name.
        let first = rewritten_at("$1", 0);
        assert_eq!(map.to_original(first), original(":first_name", 0));
        assert_eq!(map.to_original(first + 1), original(":first_name", 0));
        // After a placeholder the shift is the length difference so far.
        assert_eq!(
            map.to_original(rewritten_at("x FROM", 0)),
            original("x FROM", 0)
        );
        assert_eq!(
            map.to_original(rewritten_at("$1", 1)),
            original(":first_name", 1)
        );
        assert_eq!(map.to_original(rewritten_at("+", 0)), original("+", 0));
        assert_eq!(map.to_original(rewritten_at("$2", 0)), original(":z", 0));
        // Past the end stays one past the original end.
        assert_eq!(
            map.to_original(rewritten.chars().count() as u32 + 1),
            sql.chars().count() as u32 + 1
        );
    }

    #[test]
    fn a_ten_parameter_statement_maps_multi_digit_placeholders() {
        let sql = (0..12)
            .map(|index| format!(":p{index}"))
            .collect::<Vec<_>>()
            .join(", ");
        let scan = scan_parameters(&sql).expect("scan");
        let (rewritten, map) = scan.rewrite(&sql);
        assert!(rewritten.ends_with("$10, $11, $12"));
        let at = rewritten.find("$11").unwrap() as u32 + 1;
        let expected = sql.find(":p10").unwrap() as u32 + 1;
        for offset in 0..3 {
            assert_eq!(map.to_original(at + offset), expected);
        }
    }

    fn rejection(scan: &ParameterScan, supplied: &[ParameterValue]) -> ParameterRejection {
        scan.bind(supplied).expect_err("rejected")
    }

    #[test]
    fn binding_orders_values_and_ignores_unused_names() {
        let scan = scan_parameters("SELECT :b, :a").expect("scan");
        let bound = scan
            .bind(&[
                value("a", "1"),
                ParameterValue {
                    name: "b".into(),
                    value: None,
                },
                // Unused names are ignored even when they repeat or are huge.
                value("unused", "x"),
                value("unused", &"y".repeat(MAX_VALUE_BYTES + 1)),
            ])
            .expect("bound");
        assert_eq!(bound.0, [None, Some("1".to_owned())]);
    }

    #[test]
    fn binding_refuses_missing_and_duplicate_names() {
        let scan = scan_parameters("SELECT :a, :b, :c").expect("scan");
        assert_eq!(
            rejection(&scan, &[value("b", "1")]),
            ParameterRejection::new(
                ParameterRejectionReason::MissingValue,
                vec!["a".into(), "c".into()]
            )
        );
        assert_eq!(
            rejection(
                &scan,
                &[
                    value("a", "1"),
                    value("b", "1"),
                    value("b", "2"),
                    value("c", "1")
                ]
            ),
            ParameterRejection::new(ParameterRejectionReason::DuplicateName, vec!["b".into()])
        );
        // Names are case-sensitive.
        assert_eq!(
            rejection(&scan, &[value("A", "1"), value("b", "1"), value("c", "1")]).names,
            ["a"]
        );
    }

    #[test]
    fn binding_limits_hold_at_and_just_past_each_boundary() {
        let name = |length: usize| "n".repeat(length);
        let sql = format!("SELECT :{}", name(MAX_NAME_BYTES));
        let scan = scan_parameters(&sql).expect("scan");
        assert!(scan.bind(&[value(&name(MAX_NAME_BYTES), "1")]).is_ok());
        let sql = format!("SELECT :{}", name(MAX_NAME_BYTES + 1));
        let scan = scan_parameters(&sql).expect("scan");
        assert_eq!(
            rejection(&scan, &[value(&name(MAX_NAME_BYTES + 1), "1")]),
            ParameterRejection::new(
                ParameterRejectionReason::NameTooLong,
                vec![name(MAX_NAME_BYTES + 1)]
            )
        );

        let many = |count: usize| {
            let sql = (0..count)
                .map(|index| format!(":p{index}"))
                .collect::<Vec<_>>()
                .join(", ");
            let supplied = (0..count)
                .map(|index| value(&format!("p{index}"), "1"))
                .collect::<Vec<_>>();
            scan_parameters(&sql).expect("scan").bind(&supplied)
        };
        assert_eq!(many(MAX_PARAMETERS).expect("at the limit").0.len(), 256);
        assert_eq!(
            many(MAX_PARAMETERS + 1).expect_err("past the limit").reason,
            ParameterRejectionReason::TooManyParameters
        );

        let scan = scan_parameters("SELECT :a").expect("scan");
        assert!(scan
            .bind(&[value("a", &"v".repeat(MAX_VALUE_BYTES))])
            .is_ok());
        assert_eq!(
            rejection(&scan, &[value("a", &"v".repeat(MAX_VALUE_BYTES + 1))]),
            ParameterRejection::new(ParameterRejectionReason::ValueTooLarge, vec!["a".into()])
        );

        let scan = scan_parameters("SELECT :a, :b, :c, :d, :e").expect("scan");
        let supplied = |last: usize| {
            let mut supplied = ["a", "b", "c", "d"]
                .map(|name| value(name, &"v".repeat(MAX_VALUE_BYTES)))
                .to_vec();
            supplied.push(value("e", &"v".repeat(last)));
            supplied
        };
        assert!(scan.bind(&supplied(0)).is_ok());
        assert_eq!(
            rejection(&scan, &supplied(1)),
            ParameterRejection::new(ParameterRejectionReason::ValueTooLarge, Vec::new())
        );
    }

    #[test]
    fn debug_output_never_holds_a_value() {
        let secret = "hunter2-secret";
        let supplied = value("password", secret);
        let plan = plan_execution(
            "SELECT :password".into(),
            Some(std::slice::from_ref(&supplied)),
            None,
        )
        .expect("plan");
        for rendered in [format!("{supplied:?}"), format!("{plan:?}")] {
            assert!(!rendered.contains(secret), "{rendered}");
        }
        assert!(format!("{supplied:?}").contains("password"));
    }
    fn plan(
        sql: &str,
        parameters: Option<&[ParameterValue]>,
        row_limit: Option<i64>,
    ) -> Result<ExecutionShape, PlanRefusal> {
        plan_execution(sql.into(), parameters, row_limit).map(|plan| plan.shape)
    }

    fn script(sql: &str, row_limit: Option<u32>) -> ExecutionShape {
        ExecutionShape::Script {
            sql: sql.into(),
            row_limit,
        }
    }

    fn cursor(statement: &str, values: &[&str], row_limit: Option<u32>) -> ExecutionShape {
        ExecutionShape::CursorRead {
            statement: statement.into(),
            values: BoundValues(values.iter().map(|value| Some((*value).into())).collect()),
            row_limit,
        }
    }

    fn bound(statement: &str, values: &[&str]) -> ExecutionShape {
        ExecutionShape::BoundCommand {
            statement: statement.into(),
            values: BoundValues(values.iter().map(|value| Some((*value).into())).collect()),
        }
    }

    fn refused(reason: ParameterRejectionReason, names: &[&str]) -> PlanRefusal {
        PlanRefusal::Parameters(ParameterRejection::new(
            reason,
            names.iter().map(|name| (*name).into()).collect(),
        ))
    }

    /// SQL without names: the shape depends only on the row limit, whether or
    /// not the payload is in parameter mode.
    #[test]
    fn sql_without_names_is_a_script_unless_a_limit_meets_an_eligible_read() {
        let supplied = [value("a", "1")];
        let modes: [Option<&[ParameterValue]>; 3] = [None, Some(&[]), Some(&supplied)];
        let eligible = [
            ("SELECT 1", "SELECT 1"),
            (
                "WITH c AS (SELECT 1) SELECT * FROM c",
                "WITH c AS (SELECT 1) SELECT * FROM c",
            ),
            ("VALUES (1)", "VALUES (1)"),
            ("TABLE t", "TABLE t"),
            // The cursor text is the statement's own span.
            ("  ; SELECT 1 ; -- tail", "SELECT 1"),
            ("/* head */ select 1;", "select 1"),
        ];
        let ineligible = [
            "WITH c AS (DELETE FROM t RETURNING 1) SELECT * FROM c",
            "SELECT a INTO t2 FROM t",
            "SELECT nextval('s')",
            "SELECT * FROM t FOR UPDATE",
            "SELECT * FROM t FOR NO KEY UPDATE",
            "SELECT * FROM t FOR SHARE",
            "SELECT * FROM t FOR KEY SHARE",
            "SELECT * FROM (SELECT * FROM t FOR UPDATE) s",
            "SHOW search_path",
            "EXPLAIN SELECT 1",
            "UPDATE t SET a = 1",
            "SELECT 1; SELECT 2",
            "(SELECT 1)",
            "\"SELECT\" 1",
            "SELECT $1",
            "",
        ];
        for mode in modes {
            for (sql, statement) in eligible {
                assert_eq!(plan(sql, mode, None), Ok(script(sql, None)), "{sql}");
                assert_eq!(
                    plan(sql, mode, Some(200)),
                    Ok(cursor(statement, &[], Some(200))),
                    "{sql}"
                );
            }
            for sql in ineligible {
                assert_eq!(plan(sql, mode, None), Ok(script(sql, None)), "{sql}");
                assert_eq!(
                    plan(sql, mode, Some(200)),
                    Ok(script(sql, Some(200))),
                    "{sql}"
                );
            }
        }
    }

    #[test]
    fn sql_with_names_is_scanned_only_in_parameter_mode() {
        let supplied = [value("a", "1")];
        // Without the field a stray `:a` reaches the server as it is.
        assert_eq!(plan("SELECT :a", None, None), Ok(script("SELECT :a", None)));
        assert_eq!(
            plan("SELECT :a", None, Some(5)),
            Ok(cursor("SELECT :a", &[], Some(5)))
        );
        assert_eq!(
            plan("SELECT :a", Some(&[]), None),
            Err(refused(ParameterRejectionReason::MissingValue, &["a"]))
        );
        for limit in [None, Some(5_i64)] {
            let row_limit = limit.map(|limit| limit as u32);
            for (sql, statement) in [
                ("SELECT :a", "SELECT $1"),
                (
                    "WITH c AS (SELECT :a) SELECT * FROM c",
                    "WITH c AS (SELECT $1) SELECT * FROM c",
                ),
                ("VALUES (:a)", "VALUES ($1)"),
                (" ; SELECT :a ; ", "SELECT $1"),
            ] {
                assert_eq!(
                    plan(sql, Some(&supplied), limit),
                    Ok(cursor(statement, &["1"], row_limit)),
                    "{sql}"
                );
            }
            // Not cursor-eligible, so the limit does not apply.
            for (sql, statement) in [
                (
                    "WITH c AS (DELETE FROM t WHERE x = :a RETURNING 1) SELECT * FROM c",
                    "WITH c AS (DELETE FROM t WHERE x = $1 RETURNING 1) SELECT * FROM c",
                ),
                ("SELECT :a INTO t2 FROM t", "SELECT $1 INTO t2 FROM t"),
                ("SELECT setval('s', :a)", "SELECT setval('s', $1)"),
                (
                    "SELECT * FROM t WHERE x = :a FOR UPDATE",
                    "SELECT * FROM t WHERE x = $1 FOR UPDATE",
                ),
                (
                    "SELECT * FROM (SELECT * FROM t WHERE x = :a FOR SHARE) s",
                    "SELECT * FROM (SELECT * FROM t WHERE x = $1 FOR SHARE) s",
                ),
                ("EXPLAIN SELECT :a", "EXPLAIN SELECT $1"),
                ("UPDATE t SET x = :a;", "UPDATE t SET x = $1"),
                ("(SELECT :a)", "(SELECT $1)"),
            ] {
                assert_eq!(
                    plan(sql, Some(&supplied), limit),
                    Ok(bound(statement, &["1"])),
                    "{sql}"
                );
            }
            assert_eq!(
                plan("SELECT :a; SELECT 1", Some(&supplied), limit),
                Err(refused(ParameterRejectionReason::MultipleStatements, &[]))
            );
            assert_eq!(
                plan("SELECT :a, $1", Some(&supplied), limit),
                Err(refused(
                    ParameterRejectionReason::PositionalPlaceholder,
                    &[]
                ))
            );
        }
    }

    #[test]
    fn unlexable_sql_is_refused_only_in_parameter_mode() {
        let supplied = [value("a", "1")];
        for limit in [None, Some(5)] {
            assert_eq!(
                plan("SELECT é", None, limit),
                Ok(script("SELECT é", limit.map(|limit| limit as u32)))
            );
            for mode in [Some(&[][..]), Some(&supplied[..])] {
                assert_eq!(
                    plan("SELECT é", mode, limit),
                    Err(refused(ParameterRejectionReason::Unlexable, &[]))
                );
            }
        }
    }

    #[test]
    fn row_limits_outside_the_retention_cap_are_refused_first() {
        for limit in [0, -1, MAX_ROW_LIMIT + 1, i64::MAX] {
            assert_eq!(
                plan("SELECT :a", Some(&[]), Some(limit)),
                Err(PlanRefusal::InvalidRowLimit),
                "{limit}"
            );
        }
        for limit in [1, MAX_ROW_LIMIT] {
            assert_eq!(
                plan("SELECT 1", None, Some(limit)),
                Ok(cursor("SELECT 1", &[], Some(limit as u32)))
            );
        }
    }

    #[test]
    fn server_positions_map_back_through_the_prefix_span_and_placeholders() {
        let sql = "/* é */ ; SELECT 'ü', :name FROM missing WHERE x = :name";
        let supplied = [value("name", "1")];
        let plan = plan_execution(sql.into(), Some(&supplied), None).expect("plan");
        let ExecutionShape::CursorRead { statement, .. } = &plan.shape else {
            panic!("a read with parameters is a cursor read");
        };
        assert_eq!(statement, "SELECT 'ü', $1 FROM missing WHERE x = $1");
        let prefix = "DECLARE c NO SCROLL CURSOR FOR ";
        let prefix_chars = prefix.chars().count() as u32;
        let server = |needle: &str, nth: usize| {
            let byte = statement.match_indices(needle).nth(nth).unwrap().0;
            prefix_chars + statement[..byte].chars().count() as u32 + 1
        };
        let user = |needle: &str, nth: usize| {
            let byte = sql.match_indices(needle).nth(nth).unwrap().0;
            sql[..byte].chars().count() as u32 + 1
        };
        let map = |position| plan.original_position(position, prefix_chars);
        assert_eq!(map(server("SELECT", 0)), Some(user("SELECT", 0)));
        assert_eq!(map(server("$1", 0)), Some(user(":name", 0)));
        assert_eq!(map(server("missing", 0)), Some(user("missing", 0)));
        assert_eq!(map(server("$1", 1)), Some(user(":name", 1)));
        // Inside the prefix there is nothing to point at.
        assert_eq!(map(prefix_chars), None);
        assert_eq!(map(1), None);

        // Without parameters only the statement's offset applies.
        let sql = "  -- é\n SELECT oops";
        let plan = plan_execution(sql.into(), None, Some(1)).expect("plan");
        let oops = sql[..sql.find("oops").unwrap()].chars().count() as u32 + 1;
        assert_eq!(
            plan.original_position(prefix_chars + "SELECT ".len() as u32 + 1, prefix_chars),
            Some(oops)
        );
    }

    #[test]
    fn a_parameter_name_cannot_change_what_the_policy_sees() {
        use crate::postgres::sql_class::classify_script;
        use crate::safety::policy::{
            assert_permitted, requires_confirmation, ResolvedSafetyPolicy, SafetyLevel, WriteIntent,
        };
        let policy = |level, read_only| ResolvedSafetyPolicy {
            environment: crate::Environment::Production,
            level,
            read_only,
        };
        let intent = |sql: &str| WriteIntent::Statement {
            classes: classify_script(sql),
        };
        let planned = |sql: &str, name: &str| {
            plan_execution(sql.into(), Some(&[value(name, "1")]), None).expect("plan")
        };

        // As `:name` text the update looks bounded by a WHERE keyword.
        let sql = "UPDATE t SET a = :where";
        let protected = policy(SafetyLevel::Protected, false);
        assert!(!requires_confirmation(&protected, &intent(sql)));
        let plan = planned(sql, "where");
        assert_eq!(plan.policy_sql(), "UPDATE t SET a = $1");
        assert!(requires_confirmation(
            &protected,
            &intent(plan.policy_sql())
        ));

        // As `:name` text each of these reads looks like a write.
        for (sql, name) in [
            ("SELECT :into FROM t", "into"),
            ("WITH c AS (SELECT :update) SELECT * FROM c", "update"),
            ("SELECT :nextval", "nextval"),
        ] {
            assert_ne!(classify_script(sql), [StatementClass::Read], "{sql}");
            let plan = planned(sql, name);
            assert!(
                matches!(plan.shape, ExecutionShape::CursorRead { .. }),
                "{sql}"
            );
            let intent = intent(plan.policy_sql());
            assert!(!requires_confirmation(
                &policy(SafetyLevel::Strict, false),
                &intent
            ));
            assert!(assert_permitted(&policy(SafetyLevel::Strict, true), &intent, false).is_ok());
        }
    }
}
