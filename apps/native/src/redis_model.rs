//! Plan 031 step 4, Redis: the pure state behind the keyspace tree, the
//! console and the key inspector. No GPUI and no I/O, so every bound and
//! estimate here is unit-tested.
use dbunk_lib::backend::{
    RedisDatabase, RedisKeyInspection, RedisKeyValue, RedisScanPage, RedisValue,
};
use std::collections::{BTreeMap, HashSet, VecDeque};

/// Keys retained per database in the tree; "Load more" stops here.
pub const TREE_KEYS: usize = 5_000;
/// Keys shown per type group; the rest stay counted but unlisted.
pub const GROUP_ROWS: usize = 500;
/// Console transcript entries kept, oldest dropped first.
pub const CONSOLE_ENTRIES: usize = 200;
/// Lines rendered for one reply.
pub const REPLY_LINES: usize = 500;
/// Transcript lines shown; older lines stay in their entries until dropped.
pub const CONSOLE_ROWS: usize = 5_000;
/// Commands kept for Up/Down recall.
pub const HISTORY: usize = 100;
/// Open key inspector tabs per connection.
pub const INSPECTORS: usize = 8;

/// Display order of the type groups; anything else (module types) is "other".
pub const KINDS: [&str; 6] = ["string", "hash", "list", "set", "zset", "stream"];

/// Splits console input like `redis-cli`: whitespace separates arguments;
/// double quotes take `\n \r \t \" \\ \xHH` escapes; single quotes are
/// literal except `\'`.
pub fn tokenize(input: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            return Ok(tokens);
        };
        let mut token = String::new();
        if first == '"' || first == '\'' {
            chars.next();
            loop {
                match chars.next() {
                    None => return Err("Unbalanced quotes".into()),
                    Some(c) if c == first => break,
                    Some('\\') if first == '\'' => match chars.next() {
                        Some('\'') => token.push('\''),
                        Some(other) => {
                            token.push('\\');
                            token.push(other);
                        }
                        None => return Err("Unbalanced quotes".into()),
                    },
                    Some('\\') => match chars.next() {
                        Some('n') => token.push('\n'),
                        Some('r') => token.push('\r'),
                        Some('t') => token.push('\t'),
                        Some('a') => token.push('\u{7}'),
                        Some('b') => token.push('\u{8}'),
                        Some('x') => {
                            let hex: String = chars.by_ref().take(2).collect();
                            match u8::from_str_radix(&hex, 16) {
                                Ok(byte) if hex.len() == 2 && byte.is_ascii() => {
                                    token.push(char::from(byte))
                                }
                                _ => {
                                    return Err(
                                        "\\x escapes must be two hex digits below 80".into()
                                    );
                                }
                            }
                        }
                        Some(other) => token.push(other),
                        None => return Err("Unbalanced quotes".into()),
                    },
                    Some(c) => token.push(c),
                }
            }
            if chars.peek().is_some_and(|c| !c.is_whitespace()) {
                return Err("A closing quote must be followed by a space".into());
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                token.push(c);
            }
        }
        tokens.push(token);
    }
}

/// Key bytes as one line: UTF-8 as is, other bytes and controls escaped.
pub fn display_key(name: &[u8]) -> String {
    let mut out = String::new();
    for chunk in name.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
                c => out.push(c),
            }
        }
        for byte in chunk.invalid() {
            out.push_str(&format!("\\x{byte:02x}"));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanState {
    NotStarted,
    Loading,
    More(String),
    Done,
}

/// One logical database in the tree.
#[derive(Debug, Clone)]
pub struct DbNode {
    pub index: u8,
    /// Exact total from `INFO keyspace` when the session opened or refreshed.
    pub total: u64,
    pub expanded: bool,
    pub scan: ScanState,
    pub error: Option<String>,
    /// Sampled keys by type, in arrival order.
    groups: BTreeMap<String, Vec<Vec<u8>>>,
    seen: HashSet<Vec<u8>>,
    /// Type groups the user expanded.
    pub open_groups: HashSet<String>,
}

impl DbNode {
    fn new(database: &RedisDatabase) -> Self {
        Self {
            index: database.index,
            total: database.keys,
            expanded: false,
            scan: ScanState::NotStarted,
            error: None,
            groups: BTreeMap::new(),
            seen: HashSet::new(),
            open_groups: HashSet::new(),
        }
    }

    pub fn sampled(&self) -> usize {
        self.seen.len()
    }

    /// Whether another page may be requested.
    pub fn can_load_more(&self) -> bool {
        matches!(self.scan, ScanState::NotStarted | ScanState::More(_))
            && self.sampled() < TREE_KEYS
    }

    /// The cursor for the next page, marking the node as loading.
    pub fn begin_page(&mut self) -> Option<Option<String>> {
        if !self.can_load_more() {
            return None;
        }
        let cursor = match std::mem::replace(&mut self.scan, ScanState::Loading) {
            ScanState::More(cursor) => Some(cursor),
            _ => None,
        };
        self.error = None;
        Some(cursor)
    }

    /// Merges a page. SCAN may repeat names; duplicates are dropped. Keys
    /// past [`TREE_KEYS`] are not retained.
    pub fn apply_page(&mut self, page: RedisScanPage) {
        for key in page.keys {
            if self.seen.len() >= TREE_KEYS {
                break;
            }
            if key.kind == "none" || !self.seen.insert(key.name.clone()) {
                continue;
            }
            self.groups
                .entry(group_of(&key.kind))
                .or_default()
                .push(key.name);
        }
        self.scan = match page.next_cursor {
            Some(cursor) => ScanState::More(cursor),
            None => ScanState::Done,
        };
    }

    /// A failed page keeps what was loaded; the same cursor can be retried
    /// by the user. Nothing retries on its own.
    pub fn fail_page(&mut self, cursor: Option<String>, error: String) {
        self.scan = match cursor {
            Some(cursor) => ScanState::More(cursor),
            None => ScanState::NotStarted,
        };
        self.error = Some(error);
    }

    /// Type groups in display order with their count label.
    pub fn groups(&self) -> Vec<Group<'_>> {
        let order = |kind: &str| KINDS.iter().position(|k| *k == kind).unwrap_or(KINDS.len());
        let mut groups: Vec<Group<'_>> = self
            .groups
            .iter()
            .map(|(kind, keys)| Group {
                kind,
                keys,
                count: estimate(keys.len(), self.sampled(), self.total, self.complete()),
            })
            .collect();
        groups.sort_by_key(|group| (order(group.kind), group.kind.clone()));
        groups
    }

    /// The scan covered the whole database and every key was retained.
    pub fn complete(&self) -> bool {
        self.scan == ScanState::Done && self.sampled() < TREE_KEYS
    }
}

fn group_of(kind: &str) -> String {
    if KINDS.contains(&kind) {
        kind.to_owned()
    } else {
        "other".to_owned()
    }
}

pub struct Group<'a> {
    pub kind: &'a String,
    pub keys: &'a [Vec<u8>],
    pub count: Count,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Count {
    /// Seen in a complete scan.
    Exact(u64),
    /// Scaled from the sample to the database total.
    Estimate(u64),
}

impl Count {
    pub fn label(self) -> String {
        match self {
            Self::Exact(count) => count.to_string(),
            Self::Estimate(count) => format!("~{count}"),
        }
    }
}

/// Per-type count from a sample: exact after a complete scan, otherwise the
/// sample share of the database total, never below what was seen.
pub fn estimate(seen: usize, sampled: usize, total: u64, complete: bool) -> Count {
    if complete {
        return Count::Exact(seen as u64);
    }
    if sampled == 0 {
        return Count::Estimate(0);
    }
    let scaled = (seen as u128 * u128::from(total.max(sampled as u64)) + sampled as u128 / 2)
        / sampled as u128;
    Count::Estimate((scaled as u64).max(seen as u64))
}

/// The keyspace tree for one session.
#[derive(Debug, Clone, Default)]
pub struct Keyspace {
    pub databases: Vec<DbNode>,
}

impl Keyspace {
    pub fn new(databases: &[RedisDatabase], default_db: u8) -> Self {
        let mut nodes: Vec<DbNode> = databases.iter().map(DbNode::new).collect();
        if let Some(node) = nodes.iter_mut().find(|node| node.index == default_db) {
            node.expanded = true;
        }
        Self { databases: nodes }
    }

    pub fn get_mut(&mut self, index: u8) -> Option<&mut DbNode> {
        self.databases.iter_mut().find(|node| node.index == index)
    }
}

/// One visible row of the keyspace tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Db {
        index: u8,
        total: u64,
        expanded: bool,
        loading: bool,
    },
    Group {
        db: u8,
        kind: String,
        count: Count,
        open: bool,
    },
    Key {
        db: u8,
        name: Vec<u8>,
    },
    /// Keys of an open group beyond [`GROUP_ROWS`].
    Hidden {
        db: u8,
        count: usize,
    },
    More {
        db: u8,
        sampled: usize,
        total: u64,
    },
    Error {
        db: u8,
        message: String,
    },
}

impl Keyspace {
    /// Rows in display order: databases, then their type groups, then the
    /// keys of open groups (at most [`GROUP_ROWS`] each).
    pub fn rows(&self) -> Vec<TreeRow> {
        let mut rows = Vec::new();
        for db in &self.databases {
            rows.push(TreeRow::Db {
                index: db.index,
                total: db.total,
                expanded: db.expanded,
                loading: db.scan == ScanState::Loading,
            });
            if !db.expanded {
                continue;
            }
            for group in db.groups() {
                let open = db.open_groups.contains(group.kind);
                rows.push(TreeRow::Group {
                    db: db.index,
                    kind: group.kind.clone(),
                    count: group.count,
                    open,
                });
                if open {
                    rows.extend(group.keys.iter().take(GROUP_ROWS).map(|name| TreeRow::Key {
                        db: db.index,
                        name: name.clone(),
                    }));
                    if group.keys.len() > GROUP_ROWS {
                        rows.push(TreeRow::Hidden {
                            db: db.index,
                            count: group.keys.len() - GROUP_ROWS,
                        });
                    }
                }
            }
            if let Some(message) = &db.error {
                rows.push(TreeRow::Error {
                    db: db.index,
                    message: message.clone(),
                });
            }
            if db.can_load_more() && db.scan != ScanState::NotStarted {
                rows.push(TreeRow::More {
                    db: db.index,
                    sampled: db.sampled(),
                    total: db.total,
                });
            }
        }
        rows
    }
}

/// One console transcript entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub db: u8,
    pub command: String,
    pub lines: Vec<String>,
    pub tone: Tone,
    pub elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Reply,
    Error,
    Note,
}

/// Bounded transcript and command history.
#[derive(Debug, Default)]
pub struct Console {
    pub entries: VecDeque<Entry>,
    history: VecDeque<String>,
    recall: Option<usize>,
    /// A command waiting for explicit confirmation.
    pub pending: Option<Pending>,
    pub running: bool,
    pub db: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub input: String,
    pub tokens: Vec<String>,
    pub command: String,
    pub reason: String,
}

impl Console {
    pub fn push(&mut self, entry: Entry) {
        if self.entries.len() == CONSOLE_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn remember(&mut self, input: &str) {
        self.recall = None;
        if self.history.back().is_some_and(|last| last == input) {
            return;
        }
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(input.to_owned());
    }

    /// Up (`older`) / Down through history; `None` leaves the input alone.
    pub fn recall(&mut self, older: bool) -> Option<String> {
        let last = self.history.len().checked_sub(1)?;
        let next = match (self.recall, older) {
            (None, true) => last,
            (None, false) => return None,
            (Some(at), true) => at.saturating_sub(1),
            (Some(at), false) if at >= last => {
                self.recall = None;
                return Some(String::new());
            }
            (Some(at), false) => at + 1,
        };
        self.recall = Some(next);
        self.history.get(next).cloned()
    }
}

/// Formats a reply the way `redis-cli` does, within [`REPLY_LINES`].
pub fn reply_lines(value: &RedisValue) -> Vec<String> {
    let mut lines = Vec::new();
    let mut omitted = false;
    write_value(value, "", &mut lines, &mut omitted);
    if omitted {
        lines.truncate(REPLY_LINES);
        lines.push("… (reply cut)".into());
    }
    lines
}

fn write_value(value: &RedisValue, indent: &str, lines: &mut Vec<String>, omitted: &mut bool) {
    if lines.len() >= REPLY_LINES {
        *omitted = true;
        return;
    }
    let mut line = |text: String| lines.push(format!("{indent}{text}"));
    match value {
        RedisValue::Nil => line("(nil)".into()),
        RedisValue::Int(value) => line(format!("(integer) {value}")),
        RedisValue::Status(value) => line(value.clone()),
        RedisValue::Text(value) => line(format!("\"{}\"", escape(value))),
        RedisValue::Bytes(value) => line(format!("(bytes) {value}")),
        RedisValue::Error(value) => line(format!("(error) {value}")),
        RedisValue::Omitted(count) => line(format!("… {count} more")),
        RedisValue::Array(items) if items.is_empty() => line("(empty array)".into()),
        RedisValue::Array(items) => {
            let width = items.len().to_string().len();
            for (index, item) in items.iter().enumerate() {
                if lines.len() >= REPLY_LINES {
                    *omitted = true;
                    return;
                }
                if let RedisValue::Omitted(_) = item {
                    write_value(item, indent, lines, omitted);
                    continue;
                }
                let prefix = format!("{:>width$}) ", index + 1);
                let start = lines.len();
                let nested = format!("{indent}{}", " ".repeat(prefix.len()));
                write_value(item, &nested, lines, omitted);
                if let Some(first) = lines.get_mut(start) {
                    *first = format!("{indent}{prefix}{}", &first[nested.len()..]);
                }
            }
        }
    }
}

fn escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\r' => vec!['\\', 'r'],
            '\t' => vec!['\\', 't'],
            c => vec![c],
        })
        .collect()
}

/// A one-line cell for the inspector table.
pub fn cell(value: &RedisValue) -> String {
    match value {
        RedisValue::Text(text) => escape(text),
        RedisValue::Bytes(hex) => hex.clone(),
        other => reply_lines(other).join(" "),
    }
}

/// Inspector rows: (label, value) pairs for the value table.
pub fn inspector_rows(inspection: &RedisKeyInspection) -> Vec<(String, String)> {
    match &inspection.value {
        RedisKeyValue::Missing => Vec::new(),
        RedisKeyValue::String { value, .. } => vec![("value".into(), cell(value))],
        RedisKeyValue::Hash(entries) => entries
            .iter()
            .map(|(field, value)| (cell(field), cell(value)))
            .collect(),
        RedisKeyValue::List(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| (index.to_string(), cell(item)))
            .collect(),
        RedisKeyValue::Set(members) => members
            .iter()
            .map(|member| (String::new(), cell(member)))
            .collect(),
        RedisKeyValue::SortedSet(entries) => entries
            .iter()
            .map(|(member, score)| (score.to_string(), cell(member)))
            .collect(),
        RedisKeyValue::Stream(entries) => entries
            .iter()
            .map(|(id, fields)| {
                (
                    id.clone(),
                    fields
                        .iter()
                        .map(|(field, value)| format!("{}={}", cell(field), cell(value)))
                        .collect::<Vec<_>>()
                        .join("  "),
                )
            })
            .collect(),
        RedisKeyValue::Unsupported(reason) => vec![(String::new(), reason.clone())],
    }
}

/// `TTL` reply in words.
pub fn ttl_label(ttl: i64) -> String {
    match ttl {
        -2 => "missing".into(),
        -1 => "no expiry".into(),
        seconds => format!("{seconds} s"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::RedisKey;

    fn page(keys: &[(&str, &str)], next: Option<&str>) -> RedisScanPage {
        RedisScanPage {
            keys: keys
                .iter()
                .map(|(name, kind)| RedisKey {
                    name: name.as_bytes().to_vec(),
                    kind: (*kind).into(),
                })
                .collect(),
            next_cursor: next.map(str::to_owned),
        }
    }

    #[test]
    fn tokenizer_matches_redis_cli_quoting() {
        assert_eq!(tokenize("  SET  k v ").unwrap(), ["SET", "k", "v"]);
        assert_eq!(
            tokenize(r#"SET "a b" 'c d' "x\"y\n" 'it\'s' "\x41""#).unwrap(),
            ["SET", "a b", "c d", "x\"y\n", "it's", "A"]
        );
        assert_eq!(tokenize(r#"SET k """#).unwrap(), ["SET", "k", ""]);
        assert!(tokenize("").unwrap().is_empty());
        assert!(tokenize(r#"SET "open"#).is_err());
        assert!(tokenize(r#"SET "a"b"#).is_err());
        assert!(tokenize(r#"SET "\xZZ""#).is_err());
    }

    #[test]
    fn binary_and_control_key_bytes_are_escaped_on_one_line() {
        assert_eq!(display_key(b"user:1"), "user:1");
        assert_eq!(display_key(b"a\nb\x00"), "a\\nb\\x00");
        assert_eq!(display_key(&[0xff, b'k']), "\\xffk");
    }

    #[test]
    fn pages_group_by_type_dedupe_and_label_estimates() {
        let mut keyspace = Keyspace::new(
            &[
                RedisDatabase {
                    index: 0,
                    keys: 100,
                },
                RedisDatabase { index: 1, keys: 0 },
            ],
            0,
        );
        let db = keyspace.get_mut(0).unwrap();
        assert!(db.expanded);
        assert_eq!(db.begin_page(), Some(None));
        // A second request while loading is refused: one page at a time.
        assert_eq!(db.begin_page(), None);
        db.apply_page(page(
            &[
                ("a", "string"),
                ("b", "hash"),
                ("a", "string"),
                ("gone", "none"),
                ("j", "ReJSON-RL"),
            ],
            Some("42"),
        ));
        assert_eq!(db.sampled(), 3);
        let groups = db.groups();
        let kinds: Vec<&str> = groups.iter().map(|group| group.kind.as_str()).collect();
        assert_eq!(kinds, ["string", "hash", "other"]);
        // 1 of 3 sampled keys, scaled to 100.
        assert_eq!(groups[0].count, Count::Estimate(33));
        assert_eq!(groups[0].count.label(), "~33");
        assert_eq!(db.begin_page(), Some(Some("42".into())));
        db.apply_page(page(&[("c", "string")], None));
        assert!(db.complete());
        assert_eq!(db.groups()[0].count, Count::Exact(2));
        assert!(!db.can_load_more());
    }

    #[test]
    fn failed_page_keeps_samples_and_cursor_without_retrying() {
        let mut keyspace = Keyspace::new(&[RedisDatabase { index: 0, keys: 9 }], 0);
        let db = keyspace.get_mut(0).unwrap();
        db.begin_page();
        db.apply_page(page(&[("a", "set")], Some("7")));
        let cursor = db.begin_page().unwrap();
        db.fail_page(cursor, "timeout".into());
        assert_eq!(db.scan, ScanState::More("7".into()));
        assert_eq!(db.error.as_deref(), Some("timeout"));
        assert_eq!(db.sampled(), 1);
        // Only an explicit request continues, from the same cursor.
        assert_eq!(db.begin_page(), Some(Some("7".into())));
        assert_eq!(db.error, None);
    }

    #[test]
    fn retained_keys_are_bounded_and_estimates_never_undercount() {
        let mut keyspace = Keyspace::new(
            &[RedisDatabase {
                index: 0,
                keys: 1_000_000,
            }],
            0,
        );
        let db = keyspace.get_mut(0).unwrap();
        let names: Vec<String> = (0..TREE_KEYS + 10).map(|n| format!("k{n}")).collect();
        let keys: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "string")).collect();
        db.begin_page();
        db.apply_page(page(&keys, None));
        assert_eq!(db.sampled(), TREE_KEYS);
        assert!(!db.complete(), "cut samples are never reported as exact");
        assert!(!db.can_load_more());
        assert_eq!(db.groups()[0].count, Count::Estimate(1_000_000));
        // A stale total below the sample is not used to scale down.
        assert_eq!(estimate(5, 10, 3, false), Count::Estimate(5));
        assert_eq!(estimate(0, 0, 50, false), Count::Estimate(0));
    }

    #[test]
    fn tree_rows_follow_expansion_and_cap_open_groups() {
        let mut keyspace = Keyspace::new(
            &[
                RedisDatabase {
                    index: 0,
                    keys: 10_000,
                },
                RedisDatabase { index: 2, keys: 3 },
            ],
            0,
        );
        let db = keyspace.get_mut(0).unwrap();
        db.begin_page();
        let names: Vec<String> = (0..GROUP_ROWS + 3).map(|n| format!("h{n}")).collect();
        let mut keys: Vec<(&str, &str)> =
            names.iter().map(|name| (name.as_str(), "hash")).collect();
        keys.push(("s", "string"));
        db.apply_page(page(&keys, Some("9")));
        let rows = keyspace.rows();
        assert!(matches!(
            &rows[0],
            TreeRow::Db {
                index: 0,
                expanded: true,
                ..
            }
        ));
        assert!(matches!(&rows[1], TreeRow::Group { kind, open: false, .. } if kind == "string"));
        assert!(matches!(&rows[2], TreeRow::Group { kind, open: false, .. } if kind == "hash"));
        assert!(
            matches!(&rows[3], TreeRow::More { db: 0, sampled, .. } if *sampled == GROUP_ROWS + 4)
        );
        assert!(matches!(
            &rows[4],
            TreeRow::Db {
                index: 2,
                expanded: false,
                ..
            }
        ));
        assert_eq!(rows.len(), 5);

        keyspace
            .get_mut(0)
            .unwrap()
            .open_groups
            .insert("hash".into());
        let rows = keyspace.rows();
        let keys = rows
            .iter()
            .filter(|row| matches!(row, TreeRow::Key { .. }))
            .count();
        assert_eq!(keys, GROUP_ROWS);
        assert!(rows.contains(&TreeRow::Hidden { db: 0, count: 3 }));
    }

    #[test]
    fn console_transcript_and_history_are_bounded() {
        let mut console = Console::default();
        for n in 0..CONSOLE_ENTRIES + 5 {
            console.push(Entry {
                db: 0,
                command: format!("GET {n}"),
                lines: vec![],
                tone: Tone::Reply,
                elapsed_ms: None,
            });
        }
        assert_eq!(console.entries.len(), CONSOLE_ENTRIES);
        assert_eq!(console.entries.front().unwrap().command, "GET 5");
        for n in 0..HISTORY + 3 {
            console.remember(&format!("PING {n}"));
        }
        console.remember(&format!("PING {}", HISTORY + 2));
        assert_eq!(console.recall(false), None);
        assert_eq!(console.recall(true).as_deref(), Some("PING 102"));
        assert_eq!(console.recall(true).as_deref(), Some("PING 101"));
        assert_eq!(console.recall(false).as_deref(), Some("PING 102"));
        assert_eq!(console.recall(false).as_deref(), Some(""));
        for _ in 0..HISTORY + 5 {
            console.recall(true);
        }
        assert_eq!(console.recall(true).as_deref(), Some("PING 3"));
    }

    #[test]
    fn replies_format_like_redis_cli_and_are_cut() {
        let reply = RedisValue::Array(vec![
            RedisValue::Text("a\"b".into()),
            RedisValue::Int(2),
            RedisValue::Array(vec![RedisValue::Nil, RedisValue::Status("OK".into())]),
            RedisValue::Omitted(7),
        ]);
        assert_eq!(
            reply_lines(&reply),
            [
                "1) \"a\\\"b\"",
                "2) (integer) 2",
                "3) 1) (nil)",
                "   2) OK",
                "… 7 more"
            ]
        );
        assert_eq!(reply_lines(&RedisValue::Array(vec![])), ["(empty array)"]);
        let long = RedisValue::Array((0..REPLY_LINES as i64 + 9).map(RedisValue::Int).collect());
        let lines = reply_lines(&long);
        assert_eq!(lines.len(), REPLY_LINES + 1);
        assert_eq!(lines.last().unwrap(), "… (reply cut)");
    }
}
