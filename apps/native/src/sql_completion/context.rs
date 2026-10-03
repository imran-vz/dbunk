//! Small lexical context, not a SQL parser. Opaque regions never yield candidates.
use std::ops::Range;

pub(super) const CONTEXT_BYTES: usize = 64 * 1024;
const TOKEN_CAP: usize = 4096;
pub(super) const KEYWORDS: &[&str] = &[
    "select",
    "from",
    "where",
    "join",
    "left join",
    "right join",
    "inner join",
    "outer join",
    "cross join",
    "on",
    "group by",
    "order by",
    "having",
    "limit",
    "offset",
    "insert into",
    "values",
    "update",
    "set",
    "delete from",
    "create table",
    "alter table",
    "drop table",
    "with",
    "as",
    "distinct",
    "and",
    "or",
    "not",
    "is null",
    "is not null",
    "between",
    "like",
    "in",
    "exists",
];

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Context {
    pub range: Range<usize>,
    pub prefix: String,
    pub quoted: bool,
    pub schema: Option<String>,
    pub target: Option<(Option<String>, String)>,
    pub relation_position: bool,
}
#[derive(Debug)]
struct Token {
    value: String,
    quoted: bool,
    range: Range<usize>,
    identifier: bool,
    closed: bool,
}
impl Token {
    fn keyword(&self, word: &str) -> bool {
        self.identifier && !self.quoted && self.value.eq_ignore_ascii_case(word)
    }
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '$')
}

/// Always quote catalog names. This is valid PostgreSQL, preserves mixed case,
/// and avoids maintaining a partial reserved-word table.
pub(super) fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub(super) fn parse(before: &str, after: &str) -> Option<Context> {
    if before.len() > CONTEXT_BYTES || after.len() > CONTEXT_BYTES {
        return None;
    }
    let mut tokens = Vec::<Token>::new();
    let mut i = 0;
    let bytes = before.as_bytes();
    while i < bytes.len() {
        if tokens.len() == TOKEN_CAP {
            return None;
        }
        let start = i;
        let c = before[i..].chars().next()?;
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if before[i..].starts_with("--") {
            i = before[i..].find('\n').map(|n| i + n + 1)?;
            continue;
        }
        if before[i..].starts_with("/*") {
            i += 2;
            let mut depth = 1usize;
            while i < bytes.len() && depth != 0 {
                if before[i..].starts_with("/*") {
                    depth += 1;
                    i += 2;
                } else if before[i..].starts_with("*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += before[i..].chars().next()?.len_utf8();
                }
            }
            if depth != 0 {
                return None;
            }
            continue;
        }
        if c == '\'' {
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 1;
                    if i < bytes.len() {
                        i += before[i..].chars().next()?.len_utf8();
                    }
                } else if bytes[i] == b'\'' {
                    i += 1;
                    if bytes.get(i) == Some(&b'\'') {
                        i += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    i += before[i..].chars().next()?.len_utf8();
                }
            }
            if !closed {
                return None;
            }
            tokens.push(Token {
                value: String::new(),
                quoted: false,
                range: start..i,
                identifier: false,
                closed: true,
            });
            continue;
        }
        if c == '$' {
            let end = before[i + 1..].find('$').map(|n| i + n + 2);
            if let Some(end) = end
                && before[i + 1..end - 1]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                let delimiter = &before[i..end];
                i = end + before[end..].find(delimiter)? + delimiter.len();
                tokens.push(Token {
                    value: String::new(),
                    quoted: false,
                    range: start..i,
                    identifier: false,
                    closed: true,
                });
                continue;
            }
        }
        if c == '"' {
            i += 1;
            let mut value = String::new();
            let mut closed = false;
            while i < bytes.len() {
                let c = before[i..].chars().next()?;
                i += c.len_utf8();
                if c == '"' {
                    if bytes.get(i) == Some(&b'"') {
                        value.push('"');
                        i += 1;
                    } else {
                        closed = true;
                        break;
                    }
                } else {
                    value.push(c);
                }
            }
            tokens.push(Token {
                value,
                quoted: true,
                range: start..i,
                identifier: true,
                closed,
            });
            continue;
        }
        if word(c) && !c.is_ascii_digit() && c != '$' {
            i += c.len_utf8();
            while i < bytes.len() && before[i..].chars().next().is_some_and(word) {
                i += before[i..].chars().next()?.len_utf8();
            }
            tokens.push(Token {
                value: before[start..i].into(),
                quoted: false,
                range: start..i,
                identifier: true,
                closed: true,
            });
        } else {
            i += c.len_utf8();
            if c == ';' {
                tokens.clear();
            } else {
                tokens.push(Token {
                    value: c.to_string(),
                    quoted: false,
                    range: start..i,
                    identifier: false,
                    closed: true,
                });
            }
        }
    }
    let partial = tokens
        .last()
        .is_some_and(|t| t.identifier && t.range.end == before.len());
    let (start, prefix, quoted, closed) = if partial {
        let token = tokens.pop()?;
        (token.range.start, token.value, token.quoted, token.closed)
    } else {
        (before.len(), String::new(), false, true)
    };
    // A caret between doubled quote bytes has ambiguous partial decoding.
    if quoted && closed && after.starts_with('"') {
        return None;
    }
    let suffix = if quoted && !closed {
        // Closing quote may lie after the caret; consume the complete identifier,
        // including doubled quotes, so acceptance never leaves a stray suffix.
        let mut n = 0;
        while n < after.len() {
            let c = after[n..].chars().next()?;
            n += c.len_utf8();
            if c == '"' {
                if after.as_bytes().get(n) == Some(&b'"') {
                    n += 1;
                } else {
                    break;
                }
            }
        }
        n
    } else if quoted {
        0
    } else {
        after
            .chars()
            .take_while(|c| word(*c))
            .map(char::len_utf8)
            .sum()
    };
    if before.len() + suffix - start > 1024 {
        return None;
    }
    let schema = if tokens.last().is_some_and(|t| t.value == ".") {
        tokens
            .get(tokens.len().checked_sub(2)?)
            .filter(|t| t.identifier)
            .map(ident)
    } else {
        None
    };
    let relation_position = tokens.last().is_some_and(|t| {
        [
            "from", "join", "update", "into", "table", "describe", "desc",
        ]
        .iter()
        .any(|k| t.keyword(k))
    });
    let predicate = tokens.iter().rposition(|t| {
        ["where", "and", "or", "on", "having"]
            .iter()
            .any(|k| t.keyword(k))
    });
    let mut target = None;
    if let Some(predicate) = predicate {
        for index in 0..predicate {
            if ["from", "join", "update", "into"]
                .iter()
                .any(|k| tokens[index].keyword(k))
            {
                let Some(first) = tokens.get(index + 1).filter(|t| t.identifier) else {
                    continue;
                };
                target = if tokens.get(index + 2).is_some_and(|t| t.value == ".") {
                    tokens
                        .get(index + 3)
                        .filter(|t| t.identifier)
                        .map(|last| (Some(ident(first)), ident(last)))
                } else {
                    Some((None, ident(first)))
                };
            }
        }
    }
    Some(Context {
        range: start..before.len() + suffix,
        prefix,
        quoted,
        schema,
        target,
        relation_position,
    })
}
fn ident(token: &Token) -> String {
    if token.quoted {
        token.value.clone()
    } else {
        token.value.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_regions_and_statement_boundary() {
        for sql in [
            "select '-- from public.",
            "select $$ where x",
            "select /* nested /* x */ from ",
            "-- select ",
        ] {
            assert!(parse(sql, "").is_none(), "{sql}");
        }
        assert_eq!(
            parse("select * from old where x; select * from ", "")
                .unwrap()
                .target,
            None
        );
    }
    #[test]
    fn quoted_schema_and_whole_identifier() {
        let c = parse("select * from \"a.b\".\"Mi", "xed\" where true").unwrap();
        assert_eq!(c.schema.as_deref(), Some("a.b"));
        assert_eq!(c.prefix, "Mi");
        assert_eq!(
            &"select * from \"a.b\".\"Mixed\" where true"[c.range],
            "\"Mixed\""
        );
        assert_eq!(quote("A\"B"), "\"A\"\"B\"");
    }
    #[test]
    fn predicate_identity_case_and_suffix() {
        let c = parse("select * from \"a.b\".\"T\" where co", "lumn").unwrap();
        assert_eq!(c.target, Some((Some("a.b".into()), "T".into())));
        assert_eq!(c.range.end, c.range.start + 6);
        assert!(parse(&"a".repeat(CONTEXT_BYTES + 1), "").is_none());
    }
    #[test]
    fn doubled_quote_boundary_preserves_identifier_and_following_comment() {
        let before = "select * from \"A\"\"";
        let after = "B\" -- keep this comment";
        let parsed = parse(before, after).unwrap();
        let full = format!("{before}{after}");
        assert_eq!(&full[parsed.range.clone()], "\"A\"\"B\"");
        assert_eq!(&full[parsed.range.end..], " -- keep this comment");
        assert_eq!(parsed.prefix, "A\"");
        assert!(parse("select * from \"A\"", "\"B\"").is_none());
    }
}
