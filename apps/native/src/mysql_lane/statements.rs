//! MySQL statement boundaries for "run statement at cursor". The shared
//! selector lexes PostgreSQL, so this one follows MySQL's own rules: `#` and
//! `-- ` comments, backtick identifiers, backslash escapes in strings,
//! executable comments (`/*! … */`, `/*+ … */`) as code, and routine bodies
//! (`CREATE PROCEDURE … BEGIN …; … END`) kept whole. The client-side
//! `DELIMITER` command is not supported.
use std::ops::Range;

/// One statement: its code (leading and trailing comments and whitespace
/// excluded) and the end of its terminator, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statement {
    pub range: Range<usize>,
    /// Byte after the terminating `;`, or `range.end` without one.
    pub extent: usize,
}

/// The text to run. A non-empty selection runs as is; otherwise the
/// statement at the caret: the first one ending at or after it (its `;`
/// included), else the last. `script` runs the whole text. `None` when there
/// is nothing to run or the offsets are not character boundaries.
pub fn select(sql: &str, selection: Range<usize>, script: bool) -> Option<Range<usize>> {
    if selection.start > selection.end
        || selection.end > sql.len()
        || !sql.is_char_boundary(selection.start)
        || !sql.is_char_boundary(selection.end)
    {
        return None;
    }
    if script {
        return (!split(sql).is_empty()).then_some(0..sql.len());
    }
    if !selection.is_empty() {
        return (!split(&sql[selection.clone()]).is_empty()).then_some(selection);
    }
    let statements = split(sql);
    let caret = selection.start;
    statements
        .iter()
        .find(|statement| caret <= statement.extent)
        .or(statements.last())
        .map(|statement| statement.range.clone())
}

/// Statements in order; segments with no code are skipped.
pub fn split(sql: &str) -> Vec<Statement> {
    let bytes = sql.as_bytes();
    let mut statements = Vec::new();
    let mut segment = Segment::default();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        match byte {
            b'\'' | b'"' => {
                let end = quoted(bytes, at, byte, true);
                segment.code(at, end);
                at = end;
            }
            b'`' => {
                let end = quoted(bytes, at, byte, false);
                segment.code(at, end);
                at = end;
            }
            b'#' => at = line_end(bytes, at),
            b'-' if bytes.get(at + 1) == Some(&b'-')
                && bytes
                    .get(at + 2)
                    .is_none_or(|next| next.is_ascii_whitespace() || next.is_ascii_control()) =>
            {
                at = line_end(bytes, at)
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                let end = comment_end(bytes, at);
                if matches!(bytes.get(at + 2), Some(b'!' | b'+')) {
                    segment.code(at, end);
                }
                at = end;
            }
            b';' => {
                segment.settle();
                if segment.depth == 0 {
                    segment.finish(at + 1, &mut statements);
                } else {
                    segment.code(at, at + 1);
                }
                at += 1;
            }
            _ if byte.is_ascii_whitespace() => at += 1,
            _ if is_word(byte) => {
                let mut end = at + 1;
                while end < bytes.len() && is_word(bytes[end]) {
                    end += 1;
                }
                segment.word(&sql[at..end]);
                segment.code(at, end);
                at = end;
            }
            _ => {
                segment.code(at, at + 1);
                at += 1;
            }
        }
    }
    segment.settle();
    segment.finish(bytes.len(), &mut statements);
    // Without a terminator the extent is the code end, not the text end.
    if let Some(last) = statements.last_mut()
        && !sql[last.range.end..last.extent].contains(';')
    {
        last.extent = last.range.end;
    }
    statements
}

/// Identifier bytes; non-ASCII bytes too, so characters are never split.
fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte >= 0x80
}

/// End of a quoted run starting at `start` (unterminated runs to the end).
fn quoted(bytes: &[u8], start: usize, quote: u8, escapes: bool) -> usize {
    let mut at = start + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' if escapes => at += 2,
            byte if byte == quote => {
                if bytes.get(at + 1) == Some(&quote) {
                    at += 2;
                } else {
                    return at + 1;
                }
            }
            _ => at += 1,
        }
    }
    bytes.len()
}

fn line_end(bytes: &[u8], start: usize) -> usize {
    bytes[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |offset| start + offset)
}

/// MySQL block comments do not nest.
fn comment_end(bytes: &[u8], start: usize) -> usize {
    bytes[start + 2..]
        .windows(2)
        .position(|pair| pair == b"*/")
        .map_or(bytes.len(), |offset| start + 2 + offset + 2)
}

/// Where the statement header is while looking for a routine kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Header {
    /// No word yet.
    #[default]
    Start,
    /// After `CREATE`/`ALTER` and its options; `usize` is the number of
    /// definer words still allowed (`DEFINER = user@host`).
    Creating(usize),
    /// Not a routine definition, or its kind is known.
    Done,
}

#[derive(Debug, Default)]
struct Segment {
    code: Option<Range<usize>>,
    header: Header,
    /// The statement defines a stored routine, trigger or event whose body
    /// may hold `;` inside `BEGIN … END`.
    routine: bool,
    depth: usize,
    /// An `END` whose meaning depends on the next word (`END IF` etc.).
    pending_end: bool,
}

impl Segment {
    fn code(&mut self, start: usize, end: usize) {
        match &mut self.code {
            Some(range) => range.end = end,
            None => self.code = Some(start..end),
        }
    }

    fn word(&mut self, word: &str) {
        let is = |keyword: &str| word.eq_ignore_ascii_case(keyword);
        if self.pending_end {
            self.pending_end = false;
            if is("IF") || is("LOOP") || is("WHILE") || is("REPEAT") {
                return;
            }
            self.depth = self.depth.saturating_sub(1);
            if is("CASE") {
                return;
            }
        }
        self.header = match self.header {
            Header::Start if is("CREATE") || is("ALTER") => Header::Creating(0),
            Header::Creating(_)
                if is("PROCEDURE") || is("FUNCTION") || is("TRIGGER") || is("EVENT") =>
            {
                self.routine = true;
                Header::Done
            }
            Header::Creating(_) if is("DEFINER") => Header::Creating(2),
            Header::Creating(definer)
                if is("OR") || is("REPLACE") || is("AGGREGATE") || is("CURRENT_USER") =>
            {
                Header::Creating(definer)
            }
            Header::Creating(definer) if definer > 0 => Header::Creating(definer - 1),
            Header::Start | Header::Creating(_) | Header::Done => Header::Done,
        };
        if !self.routine {
            return;
        }
        if is("BEGIN") {
            self.depth += 1;
        } else if self.depth > 0 && is("CASE") {
            self.depth += 1;
        } else if self.depth > 0 && is("END") {
            self.pending_end = true;
        }
    }

    /// Resolves an `END` that ends the statement or precedes a `;`.
    fn settle(&mut self) {
        if std::mem::take(&mut self.pending_end) {
            self.depth = self.depth.saturating_sub(1);
        }
    }

    fn finish(&mut self, extent: usize, statements: &mut Vec<Statement>) {
        if let Some(range) = std::mem::take(self).code {
            statements.push(Statement { range, extent });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<&str> {
        split(sql)
            .into_iter()
            .map(|statement| &sql[statement.range])
            .collect()
    }

    fn at(sql: &str, caret: usize) -> Option<&str> {
        select(sql, caret..caret, false).map(|range| &sql[range])
    }

    #[test]
    fn splits_on_semicolons_outside_mysql_strings_identifiers_and_comments() {
        let sql = "SELECT 'a;\\';b', \"c;\"\"d\" FROM `t;``x`; # one; two\n\
                   SELECT 2 -- x; y\n; /* a; b */ SELECT 3 --x;\nSELECT 4";
        assert_eq!(
            texts(sql),
            [
                "SELECT 'a;\\';b', \"c;\"\"d\" FROM `t;``x`",
                "SELECT 2",
                // `--` without a following space is two minus signs.
                "SELECT 3 --x",
                "SELECT 4",
            ]
        );
        // Comment-only and empty segments are not statements.
        assert!(split(" ; # only\n -- comment\n /* c */ ;").is_empty());
        // Executable comments and hints are code.
        assert_eq!(
            texts("/*!40101 SET NAMES utf8 */; SELECT /*+ NO_ICP(t) */ 1"),
            ["/*!40101 SET NAMES utf8 */", "SELECT /*+ NO_ICP(t) */ 1"]
        );
        // Unterminated quotes run to the end instead of splitting.
        assert_eq!(texts("SELECT 'a; SELECT 2"), ["SELECT 'a; SELECT 2"]);
    }

    #[test]
    fn routine_bodies_stay_whole() {
        let body = "CREATE DEFINER = root@localhost PROCEDURE p()\n\
                    BEGIN\n  DECLARE x INT DEFAULT 0;\n  IF x THEN SELECT 1; END IF;\n\
                    \x20 WHILE x < 2 DO SET x = x + 1; END WHILE;\n\
                    \x20 SET x = CASE WHEN x THEN 1 ELSE 2 END;\n\
                    \x20 CASE x WHEN 1 THEN SELECT 1; ELSE BEGIN SELECT 2; END; END CASE;\n\
                    END";
        let sql = format!("{body};\nSELECT 9;");
        assert_eq!(texts(&sql), [body, "SELECT 9"]);
        let trigger = "CREATE TRIGGER t BEFORE INSERT ON o FOR EACH ROW BEGIN SET NEW.a = 1; END";
        assert_eq!(
            texts(&format!("{trigger}; SELECT 1")),
            [trigger, "SELECT 1"]
        );
        // Without a body a routine ends at its first `;`.
        assert_eq!(
            texts("CREATE FUNCTION f() RETURNS INT RETURN 1; SELECT f()"),
            ["CREATE FUNCTION f() RETURNS INT RETURN 1", "SELECT f()"]
        );
    }

    #[test]
    fn keywords_outside_routine_headers_never_open_blocks() {
        // Column names `event` and `begin`, and a transaction `BEGIN`.
        assert_eq!(
            texts("CREATE TABLE log (event INT, begin INT); BEGIN; SELECT end FROM log"),
            [
                "CREATE TABLE log (event INT, begin INT)",
                "BEGIN",
                "SELECT end FROM log"
            ]
        );
        assert_eq!(
            texts("CREATE VIEW v AS SELECT event, begin FROM log; SELECT 1"),
            ["CREATE VIEW v AS SELECT event, begin FROM log", "SELECT 1"]
        );
    }

    #[test]
    fn the_caret_picks_its_statement_and_a_selection_runs_as_is() {
        let sql = "SELECT 1;\nSELECT 'é';  \n\nSELECT 3";
        assert_eq!(at(sql, 0), Some("SELECT 1"));
        // Right after a `;` still belongs to that statement.
        assert_eq!(at(sql, sql.find(';').unwrap() + 1), Some("SELECT 1"));
        // The next line belongs to the next statement.
        assert_eq!(at(sql, sql.find('\n').unwrap() + 1), Some("SELECT 'é'"));
        // Blank lines before a statement choose it; the end chooses the last.
        assert_eq!(at(sql, sql.rfind('\n').unwrap()), Some("SELECT 3"));
        assert_eq!(at(sql, sql.len()), Some("SELECT 3"));

        let second = sql.find("SELECT 'é'").unwrap();
        let range = second..second + "SELECT 'é'".len();
        assert_eq!(select(sql, range.clone(), false), Some(range));
        assert_eq!(select(sql, 0..3, true), Some(0..sql.len()));
        // Nothing to run, or offsets inside a character.
        assert_eq!(select("  # c\n", 0..0, false), None);
        assert_eq!(select(sql, 0..9, false).map(|r| &sql[r]), Some("SELECT 1;"));
        assert_eq!(select(sql, 8..9, false), None);
        let inside = sql.find('é').unwrap() + 1;
        assert_eq!(select(sql, inside..inside, false), None);
    }
}
