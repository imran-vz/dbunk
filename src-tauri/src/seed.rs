//! Table Seeding value generation (ADR-0020).
//!
//! Pure and engine-agnostic: everything here is deterministic under a
//! seed and never touches a database. Engine modules build a
//! [`SeedPlan`] (which requires introspection and sampling queries) and
//! call [`generate_rows`] to materialise row batches as the same
//! `Vec<Option<String>>` shape the bulk-insert builders consume.

use crate::{ColumnInfo, SeedColumnSpec, TableStructure};

// ---------------------------------------------------------------------------
// Deterministic RNG — splitmix64
// ---------------------------------------------------------------------------

/// Splitmix64. Chosen over `rand` so generated data is stable across
/// dependency upgrades — a Seed Spec re-run with the same seed must
/// produce identical rows in any future app version.
pub(crate) struct SeedRng(u64);

impl SeedRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform float in `[0, 1)`.
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform integer in `[min, max]` (inclusive).
    pub fn range_i64(&mut self, min: i64, max: i64) -> i64 {
        if max <= min {
            return min;
        }
        let span = (max - min) as u64 + 1;
        min + (self.next_u64() % span) as i64
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next_u64() % items.len() as u64) as usize]
    }
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

/// What kind of value to fabricate for one column. Derived from the
/// column's data type plus name-based semantic inference, or forced by
/// the Seed Spec's per-column generator override.
/// Which SQL dialect the generated values will be inserted into.
///
/// Generation stays deterministic across dialects — the RNG draws are
/// identical — but a few kinds have no portable text form (PG writes
/// `true`, MySQL wants `1`; PG arrays are `{}`, ClickHouse arrays are
/// `[]`). The dialect only decides how a drawn value is *rendered*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SeedDialect {
    Postgres,
    MySql,
    Sqlite,
    ClickHouse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenKind {
    Boolean,
    TinyInt,
    SmallInt,
    Integer,
    BigInt,
    Float,
    Decimal,
    Price,
    Uuid,
    Date,
    Time,
    Timestamp,
    Json,
    Bytea,
    Inet,
    Interval,
    EmptyArray,
    Word,
    Sentence,
    Email,
    FirstName,
    LastName,
    FullName,
    UserName,
    Url,
    Phone,
    City,
    Country,
    StreetAddress,
    Company,
}

const FIRST_NAMES: &[&str] = &[
    "Olivia", "Liam", "Emma", "Noah", "Ava", "Elijah", "Sophia", "Lucas", "Isabella", "Mason",
    "Mia", "Ethan", "Amelia", "Logan", "Harper", "James", "Priya", "Mateo", "Yuki", "Omar",
    "Ingrid", "Kofi", "Lena", "Ravi", "Sofia", "Hugo", "Nadia", "Felix", "Zara", "Ivan",
];

const LAST_NAMES: &[&str] = &[
    "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Martinez",
    "Lopez", "Wilson", "Anderson", "Taylor", "Thomas", "Moore", "Nguyen", "Patel", "Kim", "Tanaka",
    "Schmidt", "Rossi", "Silva", "Kowalski", "Ivanov", "Haddad", "Okafor", "Larsen", "Novak",
    "Costa", "Chen",
];

const WORDS: &[&str] = &[
    "amber", "harbor", "willow", "summit", "meadow", "lantern", "ember", "cobalt", "drift",
    "fable", "garnet", "haven", "indigo", "juniper", "kestrel", "ledger", "marble", "north",
    "orchard", "pebble", "quartz", "ripple", "sable", "tundra", "umber", "velvet", "wander",
    "yonder", "zephyr", "atlas", "breeze", "cinder", "dapple", "eddy", "fjord", "glade",
];

const EMAIL_DOMAINS: &[&str] = &[
    "example.com",
    "example.org",
    "example.net",
    "mail.test",
    "inbox.test",
];

const CITIES: &[&str] = &[
    "Portland",
    "Austin",
    "Denver",
    "Seattle",
    "Chicago",
    "Boston",
    "Atlanta",
    "Lisbon",
    "Berlin",
    "Amsterdam",
    "Oslo",
    "Toronto",
    "Melbourne",
    "Osaka",
    "Bengaluru",
    "Nairobi",
    "Bogota",
    "Krakow",
    "Tallinn",
    "Auckland",
];

const COUNTRIES: &[&str] = &[
    "United States",
    "Canada",
    "Germany",
    "France",
    "Japan",
    "Brazil",
    "India",
    "Australia",
    "Netherlands",
    "Norway",
    "Portugal",
    "Kenya",
    "Poland",
    "Estonia",
    "New Zealand",
    "Mexico",
];

const STREET_KINDS: &[&str] = &["St", "Ave", "Blvd", "Lane", "Way", "Drive", "Court"];

const COMPANY_SUFFIXES: &[&str] = &["Labs", "Co", "Systems", "Works", "Group", "Industries"];

/// Map a frontend generator id to its `GenKind`.
pub(crate) fn parse_generator_id(id: &str) -> Result<GenKind, String> {
    Ok(match id {
        "boolean" => GenKind::Boolean,
        "tinyInt" => GenKind::TinyInt,
        "smallInt" => GenKind::SmallInt,
        "integer" => GenKind::Integer,
        "bigInt" => GenKind::BigInt,
        "float" => GenKind::Float,
        "decimal" => GenKind::Decimal,
        "price" => GenKind::Price,
        "uuid" => GenKind::Uuid,
        "date" => GenKind::Date,
        "time" => GenKind::Time,
        "timestamp" => GenKind::Timestamp,
        "json" => GenKind::Json,
        "word" => GenKind::Word,
        "sentence" => GenKind::Sentence,
        "email" => GenKind::Email,
        "firstName" => GenKind::FirstName,
        "lastName" => GenKind::LastName,
        "fullName" => GenKind::FullName,
        "userName" => GenKind::UserName,
        "url" => GenKind::Url,
        "phone" => GenKind::Phone,
        "city" => GenKind::City,
        "country" => GenKind::Country,
        "streetAddress" => GenKind::StreetAddress,
        "company" => GenKind::Company,
        other => return Err(format!("unknown generator id: {other}")),
    })
}

// ---------------------------------------------------------------------------
// Data-type normalisation
// ---------------------------------------------------------------------------

/// Fold an engine's own type spelling into the PostgreSQL-flavoured
/// vocabulary [`classify_column`] and [`max_char_length`] understand.
///
/// Every engine reports its own names for the same handful of shapes
/// (`String` / `longtext` / `TEXT`, `UInt32` / `int unsigned` /
/// `INTEGER`). Normalising once here keeps the classifier a single
/// table instead of one per engine, and keeps parameterised types
/// (`FixedString(16)` → `varchar(16)`) carrying their length bound
/// through to truncation.
pub(crate) fn normalize_data_type(dialect: SeedDialect, raw: &str) -> String {
    match dialect {
        SeedDialect::Postgres => raw.trim().to_ascii_lowercase(),
        SeedDialect::MySql => normalize_mysql_type(raw),
        SeedDialect::Sqlite => normalize_sqlite_type(raw),
        SeedDialect::ClickHouse => normalize_clickhouse_type(raw),
    }
}

/// Split `base(args)` into its base name and argument text.
fn split_parameterized(value: &str) -> (&str, Option<&str>) {
    match (value.find('('), value.ends_with(')')) {
        (Some(open), true) => (&value[..open], Some(&value[open + 1..value.len() - 1])),
        _ => (value, None),
    }
}

fn normalize_mysql_type(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    // `int(11) unsigned zerofill` — the modifiers carry no information
    // the generator can use, and they break exact-match classification.
    let cleaned = lowered.replace(" zerofill", "").replace(" unsigned", "");
    let cleaned = cleaned.trim();
    let (base, args) = split_parameterized(cleaned);

    // `enum('a','b')` / `set('a','b')` stay verbatim — the plan builder
    // turns their members into a value list.
    if base == "enum" || base == "set" {
        return cleaned.to_string();
    }
    match base {
        // MySQL has no boolean: `tinyint(1)` is the canonical stand-in,
        // wider tinyints are genuine small integers.
        "tinyint" if args == Some("1") => "boolean".to_string(),
        "bit" if args == Some("1") || args.is_none() => "boolean".to_string(),
        "bool" | "boolean" => "boolean".to_string(),
        "tinyint" => "tinyint".to_string(),
        "smallint" => "smallint".to_string(),
        "mediumint" | "int" | "integer" => "integer".to_string(),
        "bigint" => "bigint".to_string(),
        "tinytext" | "mediumtext" | "longtext" | "text" => "text".to_string(),
        "tinyblob" | "mediumblob" | "longblob" | "blob" | "binary" | "varbinary" => {
            "blob".to_string()
        }
        _ => cleaned.to_string(),
    }
}

fn normalize_sqlite_type(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    // A column can be declared with no type at all; SQLite gives it
    // BLOB affinity, but text is the friendlier thing to fabricate.
    if lowered.is_empty() {
        return "text".to_string();
    }
    let (base, _) = split_parameterized(&lowered);
    // SQLite type names are free-form and matched by affinity rules, so
    // classify by the same substring tests the engine itself uses —
    // but only for names the shared classifier wouldn't already know.
    if base.contains("int") {
        return match base {
            "tinyint" => "tinyint".to_string(),
            "smallint" | "int2" => "smallint".to_string(),
            "bigint" | "int8" | "unsigned big int" => "bigint".to_string(),
            _ => "integer".to_string(),
        };
    }
    if base.contains("clob") {
        return "text".to_string();
    }
    lowered
}

fn normalize_clickhouse_type(raw: &str) -> String {
    let mut inner = raw.trim();
    // `LowCardinality(Nullable(String))` — peel the wrappers off until
    // the concrete type is exposed.
    loop {
        let lowered = inner.to_ascii_lowercase();
        let stripped = ["nullable(", "lowcardinality("]
            .iter()
            .find(|wrapper| lowered.starts_with(*wrapper) && inner.ends_with(')'))
            .map(|wrapper| inner[wrapper.len()..inner.len() - 1].trim());
        match stripped {
            Some(next) => inner = next,
            None => break,
        }
    }

    let lowered = inner.to_ascii_lowercase();
    let (base, args) = split_parameterized(&lowered);
    if base == "enum8" || base == "enum16" {
        // `Enum8('a' = 1, 'b' = 2)` — keep only the member literals so
        // the shared enum parser sees the same shape MySQL emits.
        return format!("enum({})", args.unwrap_or_default());
    }
    match base {
        "string" => "text".to_string(),
        "fixedstring" => match args {
            Some(length) => format!("varchar({length})"),
            None => "text".to_string(),
        },
        "bool" => "boolean".to_string(),
        "int8" | "uint8" => "tinyint".to_string(),
        "int16" | "uint16" => "smallint".to_string(),
        "int32" | "uint32" => "integer".to_string(),
        "int64" | "uint64" | "int128" | "uint128" | "int256" | "uint256" => "bigint".to_string(),
        "float32" => "real".to_string(),
        "float64" => "double precision".to_string(),
        "decimal" | "decimal32" | "decimal64" | "decimal128" | "decimal256" => {
            "decimal".to_string()
        }
        "date" | "date32" => "date".to_string(),
        "datetime" | "datetime64" => "timestamp".to_string(),
        "uuid" => "uuid".to_string(),
        "ipv4" | "ipv6" => "inet".to_string(),
        "array" => "array".to_string(),
        "json" | "object" => "json".to_string(),
        _ => lowered,
    }
}

/// Members of an `enum(...)` / `set(...)` type, unquoted. Returns
/// `None` for every other type.
pub(crate) fn enum_members(data_type: &str) -> Option<Vec<String>> {
    let (base, args) = split_parameterized(data_type.trim());
    if !matches!(base.to_ascii_lowercase().as_str(), "enum" | "set") {
        return None;
    }
    let args = args?;

    let mut members = Vec::new();
    let mut current = String::new();
    let mut chars = args.chars().peekable();
    let mut in_literal = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\'' if in_literal => {
                // `''` inside a literal is an escaped quote, not the end.
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    current.push('\'');
                } else {
                    in_literal = false;
                    members.push(std::mem::take(&mut current));
                }
            }
            '\'' => in_literal = true,
            // Everything outside a literal (commas, `= 1` ordinals) is
            // separator noise.
            _ if in_literal => current.push(ch),
            _ => {}
        }
    }
    if members.is_empty() {
        None
    } else {
        Some(members)
    }
}

fn is_textual(data_type: &str) -> bool {
    let t = data_type.to_ascii_lowercase();
    t.contains("char") || t == "text" || t.starts_with("text") || t.contains("citext")
}

fn is_numericish(data_type: &str) -> bool {
    let t = data_type.to_ascii_lowercase();
    t.contains("int")
        || t.contains("numeric")
        || t.contains("decimal")
        || t.contains("real")
        || t.contains("double")
        || t.contains("money")
}

/// Name-based semantic inference, applied only when the column's type
/// is compatible with the inferred generator (an `email` integer column
/// stays an integer).
fn infer_from_name(name: &str, data_type: &str) -> Option<GenKind> {
    let n = name.to_ascii_lowercase();
    if is_textual(data_type) {
        if n.contains("email") {
            return Some(GenKind::Email);
        }
        if n == "first_name" || n == "firstname" || n == "given_name" {
            return Some(GenKind::FirstName);
        }
        if n == "last_name" || n == "lastname" || n == "surname" || n == "family_name" {
            return Some(GenKind::LastName);
        }
        if n.contains("company") || n.contains("organization") || n.contains("organisation") {
            return Some(GenKind::Company);
        }
        if n.contains("username") || n == "user_name" || n == "handle" || n == "login" {
            return Some(GenKind::UserName);
        }
        if n.ends_with("name") {
            return Some(GenKind::FullName);
        }
        if n.contains("url") || n.contains("website") || n.contains("homepage") {
            return Some(GenKind::Url);
        }
        if n.contains("phone") || n.contains("mobile") || n == "tel" {
            return Some(GenKind::Phone);
        }
        if n == "city" || n.ends_with("_city") {
            return Some(GenKind::City);
        }
        if n == "country" || n.ends_with("_country") {
            return Some(GenKind::Country);
        }
        if n.contains("address") || n.contains("street") {
            return Some(GenKind::StreetAddress);
        }
        if n.contains("description")
            || n.contains("summary")
            || n.contains("bio")
            || n.contains("notes")
            || n.contains("comment")
            || n.contains("title")
        {
            return Some(GenKind::Sentence);
        }
    }
    if is_numericish(data_type)
        && (n.contains("price")
            || n.contains("amount")
            || n.contains("cost")
            || n.contains("total")
            || n.contains("balance")
            || n.contains("salary"))
    {
        return Some(GenKind::Price);
    }
    None
}

/// Pick the generator for a column from its data type, with name-based
/// semantic inference layered on top. Returns `None` for types we have
/// no generator for (enums, ranges, geometry, …) — the caller decides
/// whether NULL is acceptable or the Seed Spec must supply values.
pub(crate) fn classify_column(name: &str, data_type: &str) -> Option<GenKind> {
    if let Some(kind) = infer_from_name(name, data_type) {
        return Some(kind);
    }
    let t = data_type.to_ascii_lowercase();
    if t.ends_with("[]") || t.starts_with("array") || t.starts_with('_') {
        return Some(GenKind::EmptyArray);
    }
    Some(match t.as_str() {
        "boolean" | "bool" => GenKind::Boolean,
        "tinyint" => GenKind::TinyInt,
        "smallint" | "int2" => GenKind::SmallInt,
        "integer" | "int" | "int4" | "mediumint" => GenKind::Integer,
        "bigint" | "int8" => GenKind::BigInt,
        "real" | "float4" | "double precision" | "float8" | "float" | "double" => GenKind::Float,
        "uuid" => GenKind::Uuid,
        "date" => GenKind::Date,
        "json" | "jsonb" => GenKind::Json,
        "bytea" | "blob" => GenKind::Bytea,
        "inet" | "cidr" => GenKind::Inet,
        "interval" => GenKind::Interval,
        "money" => GenKind::Price,
        "text" | "citext" => GenKind::Sentence,
        _ => {
            if t.starts_with("numeric") || t.starts_with("decimal") {
                GenKind::Decimal
            } else if t.starts_with("timestamp") || t.starts_with("datetime") {
                GenKind::Timestamp
            } else if t.starts_with("time") {
                GenKind::Time
            } else if t.starts_with("character varying")
                || t.starts_with("varchar")
                || t.starts_with("character")
                || t.starts_with("char")
                || t.starts_with("nvarchar")
            {
                GenKind::Word
            } else {
                return None;
            }
        }
    })
}

/// Max character length parsed from types like `character varying(50)`.
pub(crate) fn max_char_length(data_type: &str) -> Option<usize> {
    let open = data_type.find('(')?;
    let close = data_type[open..].find(')')? + open;
    data_type[open + 1..close].trim().parse().ok()
}

// ---------------------------------------------------------------------------
// Seed plan
// ---------------------------------------------------------------------------

/// Where one column's values come from during generation.
#[derive(Debug, Clone)]
pub(crate) enum ColumnSource {
    /// Per-spec or auto-detected: omit from the INSERT so the database
    /// applies its own DEFAULT (identity/serial columns).
    Skip,
    Constant(Option<String>),
    ValueList(Vec<String>),
    /// Sample from a foreign key's parent rows. Members of one FK pick
    /// the same pool row per generated row so composite FKs stay
    /// consistent.
    FkPool {
        pool: usize,
        member: usize,
    },
    Generated {
        kind: GenKind,
        /// Mix the row sequence into the value to guarantee batch
        /// uniqueness (unique indexes, PKs without defaults).
        unique: bool,
        /// For unique integer kinds: current MAX in the table, so new
        /// values start above existing data.
        unique_base: i64,
        /// User-supplied numeric/date range override.
        min: Option<f64>,
        max: Option<f64>,
        /// Truncation bound from `varchar(n)`.
        max_len: Option<usize>,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct ColumnPlan {
    pub name: String,
    pub source: ColumnSource,
    /// Probability of emitting NULL instead of a value. Always 0.0 for
    /// non-nullable columns; defaults to [`DEFAULT_NULL_RATE`] for
    /// nullable ones unless the Seed Spec overrides it.
    pub null_rate: f64,
}

pub(crate) const DEFAULT_NULL_RATE: f64 = 0.1;

#[derive(Debug)]
pub(crate) struct SeedPlan {
    pub columns: Vec<ColumnPlan>,
    /// FK sample pools: `pools[p]` is a list of parent rows, each a
    /// tuple of member values aligned with the FK's column order.
    pub fk_pools: Vec<Vec<Vec<String>>>,
    /// Reference epoch (seconds) for date/timestamp generation, fixed
    /// once per run so generation stays deterministic within the run.
    pub now_epoch_secs: i64,
    /// Target dialect — decides value *rendering* only.
    pub dialect: SeedDialect,
}

/// Find the per-column spec entry, if the Seed Spec has one.
pub(crate) fn spec_for<'a>(
    specs: &'a [SeedColumnSpec],
    column: &str,
) -> Option<&'a SeedColumnSpec> {
    specs.iter().find(|s| s.column == column)
}

// ---------------------------------------------------------------------------
// Plan building — engine-agnostic
// ---------------------------------------------------------------------------

/// A plan that still needs two database reads before it can generate:
/// the foreign-key parent pools and the current MAX of unique integer
/// columns. Splitting the build in two keeps every decision about
/// *what* to generate pure and testable, leaving engine modules with
/// only the sampling SQL — which is all that actually differs between
/// PostgreSQL, MySQL, SQLite and ClickHouse.
pub(crate) struct PlanDraft {
    columns: Vec<ColumnPlan>,
    dialect: SeedDialect,
    /// Indexes into `structure.foreign_keys` that need a sampled pool.
    /// Only FKs actually referenced by a generated column appear here.
    pub needed_pools: Vec<usize>,
    /// Columns needing a `MAX(col)` probe so unique integer sequences
    /// start above the rows already in the table.
    pub needed_maxes: Vec<String>,
}

/// Decide every column's value source from the table's structure plus
/// the Seed Spec. Pure: no database access, no clock.
pub(crate) fn analyze_plan(
    dialect: SeedDialect,
    structure: &TableStructure,
    specs: &[SeedColumnSpec],
) -> Result<PlanDraft, String> {
    // Columns guaranteed unique by a single-column unique index or PK;
    // for composite uniques, mixing the sequence into the first member
    // is enough to make the tuple unique.
    let mut unique_columns: Vec<String> = Vec::new();
    for index in &structure.indexes {
        if index.is_unique {
            if let Some(first) = index.columns.first() {
                unique_columns.push(first.clone());
            }
        }
    }
    if let Some(primary_key) = &structure.primary_key {
        if let Some(first) = primary_key.first() {
            unique_columns.push(first.clone());
        }
    }
    // ClickHouse's "primary key" is a sorting key, not a uniqueness
    // constraint, and its skip indices are never unique — treating
    // either as unique would fabricate needless sequences.
    if dialect == SeedDialect::ClickHouse {
        unique_columns.clear();
    }

    // FK membership: column name -> (fk index, member position).
    let mut fk_membership: Vec<(String, usize, usize)> = Vec::new();
    for (fk_index, fk) in structure.foreign_keys.iter().enumerate() {
        for (member, column) in fk.columns.iter().enumerate() {
            fk_membership.push((column.clone(), fk_index, member));
        }
    }

    let mut draft = PlanDraft {
        columns: Vec::with_capacity(structure.columns.len()),
        dialect,
        needed_pools: Vec::new(),
        needed_maxes: Vec::new(),
    };

    for column in &structure.columns {
        let spec = spec_for(specs, &column.name);
        let null_rate = effective_null_rate(column, spec);
        let data_type = normalize_data_type(dialect, &column.data_type);

        if spec.map(|s| s.skip).unwrap_or(false) {
            draft
                .columns
                .push(plain(&column.name, ColumnSource::Skip, 0.0));
            continue;
        }
        if let Some(constant) = spec.and_then(|s| s.constant.clone()) {
            draft.columns.push(plain(
                &column.name,
                ColumnSource::Constant(Some(constant)),
                null_rate,
            ));
            continue;
        }
        if let Some(values) = spec.and_then(|s| s.values.clone()) {
            if values.is_empty() {
                return Err(format!(
                    "column \"{}\" has an empty value list",
                    column.name
                ));
            }
            draft.columns.push(plain(
                &column.name,
                ColumnSource::ValueList(values),
                null_rate,
            ));
            continue;
        }

        if let Some((_, fk_index, member)) = fk_membership
            .iter()
            .find(|(name, _, _)| name == &column.name)
        {
            // An always-NULL FK column needs no parent at all, so don't
            // make an empty parent table fail the run.
            if null_rate >= 1.0 {
                draft
                    .columns
                    .push(plain(&column.name, ColumnSource::Constant(None), 0.0));
                continue;
            }
            if !draft.needed_pools.contains(fk_index) {
                draft.needed_pools.push(*fk_index);
            }
            draft.columns.push(plain(
                &column.name,
                ColumnSource::FkPool {
                    pool: *fk_index,
                    member: *member,
                },
                null_rate,
            ));
            continue;
        }

        // Identity / serial / auto-increment / derived columns fall back
        // to whatever the database itself puts there.
        if is_database_supplied(dialect, column, structure, &data_type) {
            draft
                .columns
                .push(plain(&column.name, ColumnSource::Skip, 0.0));
            continue;
        }

        // An enum column's own definition is the best value list there
        // is — no generator can beat the declared members.
        if spec.and_then(|s| s.generator.as_deref()).is_none() {
            if let Some(members) = enum_members(&data_type) {
                draft.columns.push(plain(
                    &column.name,
                    ColumnSource::ValueList(members),
                    null_rate,
                ));
                continue;
            }
        }

        let kind = match spec.and_then(|s| s.generator.as_deref()) {
            Some(id) => parse_generator_id(id)?,
            None => match classify_column(&column.name, &data_type) {
                Some(kind) => kind,
                None if column.nullable => {
                    draft
                        .columns
                        .push(plain(&column.name, ColumnSource::Constant(None), 0.0));
                    continue;
                }
                None => {
                    return Err(format!(
                        "no generator for column \"{}\" of type {} — set a constant or value list",
                        column.name, column.data_type
                    ));
                }
            },
        };

        let unique = unique_columns.contains(&column.name);
        if unique && is_integer_kind(kind) {
            draft.needed_maxes.push(column.name.clone());
        }

        draft.columns.push(ColumnPlan {
            name: column.name.clone(),
            source: ColumnSource::Generated {
                kind,
                unique,
                // Filled in by `finalize_plan` from the MAX probe.
                unique_base: 0,
                min: spec.and_then(|s| s.min),
                max: spec.and_then(|s| s.max),
                max_len: max_char_length(&data_type),
            },
            // Unique columns skip NULL injection: a guaranteed-unique
            // column that suddenly yields NULLs is surprising even
            // where the index would allow it.
            null_rate: if unique { 0.0 } else { null_rate },
        });
    }

    Ok(draft)
}

/// Fold the sampled parent pools and integer maxima into a runnable
/// plan. `pools` is index-aligned with `structure.foreign_keys`; every
/// index in [`PlanDraft::needed_pools`] must carry a non-empty sample.
pub(crate) fn finalize_plan(
    structure: &TableStructure,
    draft: PlanDraft,
    pools: Vec<Option<Vec<Vec<String>>>>,
    maxes: &[(String, i64)],
    now_epoch_secs: i64,
) -> Result<SeedPlan, String> {
    for fk_index in &draft.needed_pools {
        let sampled = pools.get(*fk_index).and_then(|pool| pool.as_ref());
        if sampled.map(|rows| rows.is_empty()).unwrap_or(true) {
            let fk = structure
                .foreign_keys
                .get(*fk_index)
                .ok_or_else(|| "internal error: foreign key index out of range".to_string())?;
            let column = fk.columns.first().cloned().unwrap_or_default();
            let parent = if fk.referenced_schema.is_empty() {
                fk.referenced_table.clone()
            } else {
                format!("\"{}\".\"{}\"", fk.referenced_schema, fk.referenced_table)
            };
            return Err(format!(
                "column \"{}\" references {}, which is empty — seed \"{}\" first",
                column, parent, fk.referenced_table
            ));
        }
    }

    let mut columns = draft.columns;
    for column in &mut columns {
        if let ColumnSource::Generated {
            unique,
            unique_base,
            ..
        } = &mut column.source
        {
            if *unique {
                *unique_base = maxes
                    .iter()
                    .find(|(name, _)| name == &column.name)
                    .map(|(_, value)| *value)
                    .unwrap_or(0);
            }
        }
    }

    Ok(SeedPlan {
        columns,
        fk_pools: pools
            .into_iter()
            .map(|pool| pool.unwrap_or_default())
            .collect(),
        now_epoch_secs,
        dialect: draft.dialect,
    })
}

fn plain(name: &str, source: ColumnSource, null_rate: f64) -> ColumnPlan {
    ColumnPlan {
        name: name.to_string(),
        source,
        null_rate,
    }
}

fn effective_null_rate(column: &ColumnInfo, spec: Option<&SeedColumnSpec>) -> f64 {
    if !column.nullable {
        return 0.0;
    }
    spec.and_then(|s| s.null_rate)
        .unwrap_or(DEFAULT_NULL_RATE)
        .clamp(0.0, 1.0)
}

pub(crate) fn is_integer_kind(kind: GenKind) -> bool {
    matches!(
        kind,
        GenKind::TinyInt | GenKind::SmallInt | GenKind::Integer | GenKind::BigInt
    )
}

/// Does the database fill this column in on its own when the INSERT
/// omits it? Each engine spells "identity" differently, and getting it
/// wrong either fights a sequence or leaves a NOT NULL column empty.
fn is_database_supplied(
    dialect: SeedDialect,
    column: &ColumnInfo,
    structure: &TableStructure,
    data_type: &str,
) -> bool {
    // MATERIALIZED / ALIAS / EPHEMERAL columns (ClickHouse) are derived
    // — naming one in an INSERT is an error, not just redundant.
    if column.derivation_kind.is_some() {
        return true;
    }
    let default = column.default_value.as_deref().unwrap_or("").trim();
    let has_default = !default.is_empty();
    // A defaulted primary key is an identity column in every dialect.
    if column.is_primary_key && has_default {
        return true;
    }
    match dialect {
        SeedDialect::Postgres => default.starts_with("nextval("),
        SeedDialect::MySql => default.to_ascii_lowercase().contains("auto_increment"),
        // `INTEGER PRIMARY KEY` is an alias for the rowid, which SQLite
        // assigns itself — but only when it's the whole key.
        SeedDialect::Sqlite => {
            column.is_primary_key
                && data_type == "integer"
                && structure
                    .primary_key
                    .as_ref()
                    .map(|key| key.len() == 1)
                    .unwrap_or(false)
        }
        // ClickHouse fills any DEFAULT expression; there is no identity.
        SeedDialect::ClickHouse => has_default,
    }
}

// ---------------------------------------------------------------------------
// Row generation
// ---------------------------------------------------------------------------

/// Names of the columns that will appear in the INSERT, in plan order.
pub(crate) fn insert_columns(plan: &SeedPlan) -> Vec<String> {
    plan.columns
        .iter()
        .filter(|c| !matches!(c.source, ColumnSource::Skip))
        .map(|c| c.name.clone())
        .collect()
}

/// Generate `row_count` rows for the plan. Output rows align with
/// [`insert_columns`].
#[cfg(test)]
pub(crate) fn generate_rows(
    plan: &SeedPlan,
    row_count: u32,
    rng: &mut SeedRng,
) -> Vec<Vec<Option<String>>> {
    generate_rows_from(plan, 0, row_count, rng)
}

/// Generate one bounded chunk while preserving the absolute row index
/// used by unique generators. Reusing the same RNG across consecutive
/// calls produces the same values as one `generate_rows` call.
pub(crate) fn generate_rows_from(
    plan: &SeedPlan,
    start_row: u32,
    row_count: u32,
    rng: &mut SeedRng,
) -> Vec<Vec<Option<String>>> {
    let mut rows = Vec::with_capacity(row_count as usize);
    for row_offset in 0..row_count {
        let row_index = start_row + row_offset;
        // One pool row per FK per generated row keeps composite FK
        // members consistent with each other.
        let pool_picks: Vec<usize> = plan
            .fk_pools
            .iter()
            .map(|pool| {
                if pool.is_empty() {
                    0
                } else {
                    (rng.next_u64() % pool.len() as u64) as usize
                }
            })
            .collect();

        let mut row = Vec::new();
        for column in &plan.columns {
            if matches!(column.source, ColumnSource::Skip) {
                continue;
            }
            if column.null_rate > 0.0 && rng.next_f64() < column.null_rate {
                row.push(None);
                continue;
            }
            row.push(generate_cell(plan, column, row_index, &pool_picks, rng));
        }
        rows.push(row);
    }
    rows
}

fn generate_cell(
    plan: &SeedPlan,
    column: &ColumnPlan,
    row_index: u32,
    pool_picks: &[usize],
    rng: &mut SeedRng,
) -> Option<String> {
    match &column.source {
        ColumnSource::Skip => None,
        ColumnSource::Constant(value) => value.clone(),
        ColumnSource::ValueList(values) => {
            if values.is_empty() {
                None
            } else {
                Some(rng.pick(values).clone())
            }
        }
        ColumnSource::FkPool { pool, member } => {
            let pool_rows = &plan.fk_pools[*pool];
            if pool_rows.is_empty() {
                return None;
            }
            pool_rows[pool_picks[*pool]].get(*member).cloned()
        }
        ColumnSource::Generated {
            kind,
            unique,
            unique_base,
            min,
            max,
            max_len,
        } => {
            let value = generate_value(
                *kind,
                rng,
                row_index,
                *unique,
                *unique_base,
                *min,
                *max,
                plan.now_epoch_secs,
                plan.dialect,
            );
            Some(match max_len {
                Some(limit) => truncate_chars(&value, *limit),
                None => value,
            })
        }
    }
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

fn format_epoch(secs: i64, format: &str) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.format(format).to_string())
        .unwrap_or_default()
}

const SECONDS_PER_YEAR: i64 = 365 * 24 * 3600;

#[allow(clippy::too_many_arguments)]
fn generate_value(
    kind: GenKind,
    rng: &mut SeedRng,
    row_index: u32,
    unique: bool,
    unique_base: i64,
    min: Option<f64>,
    max: Option<f64>,
    now_epoch_secs: i64,
    dialect: SeedDialect,
) -> String {
    let sequence = row_index as i64 + 1;
    let int_in = |rng: &mut SeedRng, lo: i64, hi: i64| {
        let lo = min.map(|m| m as i64).unwrap_or(lo);
        let hi = max.map(|m| m as i64).unwrap_or(hi);
        rng.range_i64(lo, hi)
    };
    match kind {
        GenKind::Boolean => {
            let value = rng.next_u64().is_multiple_of(2);
            match dialect {
                // Only PostgreSQL accepts the literal words; MySQL's
                // tinyint(1), SQLite's dynamic typing and ClickHouse's
                // UInt8 all want the digit.
                SeedDialect::Postgres => value.to_string(),
                SeedDialect::MySql | SeedDialect::Sqlite | SeedDialect::ClickHouse => {
                    u8::from(value).to_string()
                }
            }
        }
        GenKind::TinyInt => {
            if unique {
                (unique_base + sequence).to_string()
            } else {
                // Narrow on purpose: the smallest integer any engine
                // calls "tiny" holds -128..127.
                int_in(rng, 0, 100).to_string()
            }
        }
        GenKind::SmallInt => {
            if unique {
                (unique_base + sequence).to_string()
            } else {
                int_in(rng, 1, 32_000).to_string()
            }
        }
        GenKind::Integer | GenKind::BigInt => {
            if unique {
                (unique_base + sequence).to_string()
            } else {
                int_in(rng, 1, 100_000).to_string()
            }
        }
        GenKind::Float | GenKind::Decimal => {
            let lo = min.unwrap_or(0.0);
            let hi = max.unwrap_or(10_000.0);
            let value = lo + rng.next_f64() * (hi - lo);
            let value = if unique {
                value + unique_base as f64 + sequence as f64
            } else {
                value
            };
            format!("{value:.4}")
        }
        GenKind::Price => {
            let lo = min.unwrap_or(1.0);
            let hi = max.unwrap_or(1_000.0);
            let value = lo + rng.next_f64() * (hi - lo);
            let value = if unique {
                value + unique_base as f64 + sequence as f64
            } else {
                value
            };
            format!("{value:.2}")
        }
        GenKind::Uuid => {
            // Random v4-shaped UUID from the seeded RNG, so runs stay
            // reproducible. Collision odds are negligible.
            let a = rng.next_u64();
            let b = rng.next_u64();
            format!(
                "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
                a >> 32,
                (a >> 16) & 0xFFFF,
                a & 0xFFF,
                0x8000 | ((b >> 48) & 0x3FFF),
                b & 0xFFFF_FFFF_FFFF
            )
        }
        GenKind::Date => {
            let offset = if unique {
                sequence * 86_400
            } else {
                rng.range_i64(0, SECONDS_PER_YEAR)
            };
            format_epoch(now_epoch_secs - offset, "%Y-%m-%d")
        }
        GenKind::Time => {
            let secs = rng.range_i64(0, 86_399);
            format_epoch(secs, "%H:%M:%S")
        }
        GenKind::Timestamp => {
            let offset = if unique {
                sequence
            } else {
                rng.range_i64(0, SECONDS_PER_YEAR)
            };
            format_epoch(now_epoch_secs - offset, "%Y-%m-%d %H:%M:%S")
        }
        GenKind::Json => {
            let word = rng.pick(WORDS);
            let score = rng.range_i64(1, 100);
            format!("{{\"tag\": \"{word}\", \"score\": {score}}}")
        }
        GenKind::Bytea => {
            let bytes = rng.next_u64();
            match dialect {
                // PG's bytea input format; everywhere else the column
                // takes raw bytes, so plain hex characters are the
                // portable payload.
                SeedDialect::Postgres => format!("\\x{bytes:016x}"),
                SeedDialect::MySql | SeedDialect::Sqlite | SeedDialect::ClickHouse => {
                    format!("{bytes:016x}")
                }
            }
        }
        GenKind::Inet => {
            format!(
                "10.{}.{}.{}",
                rng.range_i64(0, 255),
                rng.range_i64(0, 255),
                rng.range_i64(1, 254)
            )
        }
        GenKind::Interval => {
            format!("{} days", rng.range_i64(1, 90))
        }
        GenKind::EmptyArray => match dialect {
            SeedDialect::ClickHouse => "[]".to_string(),
            SeedDialect::Postgres | SeedDialect::MySql | SeedDialect::Sqlite => "{}".to_string(),
        },
        GenKind::Word => {
            let word = rng.pick(WORDS);
            if unique {
                format!("{word}-{sequence}")
            } else {
                (*word).to_string()
            }
        }
        GenKind::Sentence => {
            let count = rng.range_i64(6, 12) as usize;
            let mut words: Vec<&str> = (0..count).map(|_| *rng.pick(WORDS)).collect();
            let first = words[0];
            let capitalized = format!(
                "{}{}",
                first.chars().next().unwrap_or('A').to_ascii_uppercase(),
                &first[1..]
            );
            let capitalized_ref: &str = &capitalized;
            words[0] = capitalized_ref;
            let sentence = format!("{}.", words.join(" "));
            if unique {
                format!("{sentence} ({sequence})")
            } else {
                sentence
            }
        }
        GenKind::Email => {
            let first = rng.pick(FIRST_NAMES).to_ascii_lowercase();
            let last = rng.pick(LAST_NAMES).to_ascii_lowercase();
            let domain = rng.pick(EMAIL_DOMAINS);
            if unique {
                format!("{first}.{last}{sequence}@{domain}")
            } else {
                format!("{first}.{last}@{domain}")
            }
        }
        GenKind::FirstName => {
            let name = rng.pick(FIRST_NAMES);
            if unique {
                format!("{name}-{sequence}")
            } else {
                (*name).to_string()
            }
        }
        GenKind::LastName => {
            let name = rng.pick(LAST_NAMES);
            if unique {
                format!("{name}-{sequence}")
            } else {
                (*name).to_string()
            }
        }
        GenKind::FullName => {
            let first = rng.pick(FIRST_NAMES);
            let last = rng.pick(LAST_NAMES);
            if unique {
                format!("{first} {last} {sequence}")
            } else {
                format!("{first} {last}")
            }
        }
        GenKind::UserName => {
            let first = rng.pick(FIRST_NAMES).to_ascii_lowercase();
            let number = rng.range_i64(1, 999);
            if unique {
                format!("{first}{sequence}")
            } else {
                format!("{first}{number}")
            }
        }
        GenKind::Url => {
            let word = rng.pick(WORDS);
            if unique {
                format!("https://{word}-{sequence}.example.com")
            } else {
                format!("https://{word}.example.com")
            }
        }
        GenKind::Phone => {
            if unique {
                format!("+1555{:07}", sequence)
            } else {
                format!(
                    "+1{}{:07}",
                    rng.range_i64(200, 989),
                    rng.range_i64(0, 9_999_999)
                )
            }
        }
        GenKind::City => {
            let city = rng.pick(CITIES);
            if unique {
                format!("{city}-{sequence}")
            } else {
                (*city).to_string()
            }
        }
        GenKind::Country => (*rng.pick(COUNTRIES)).to_string(),
        GenKind::StreetAddress => {
            format!(
                "{} {} {}",
                rng.range_i64(1, 9999),
                capitalize(rng.pick::<&str>(WORDS)),
                rng.pick(STREET_KINDS)
            )
        }
        GenKind::Company => {
            let word = capitalize(rng.pick::<&str>(WORDS));
            let suffix = rng.pick(COMPANY_SUFFIXES);
            if unique {
                format!("{word} {suffix} {sequence}")
            } else {
                format!("{word} {suffix}")
            }
        }
    }
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan_with(columns: Vec<ColumnPlan>) -> SeedPlan {
        SeedPlan {
            columns,
            fk_pools: Vec::new(),
            now_epoch_secs: 1_750_000_000,
            dialect: SeedDialect::Postgres,
        }
    }

    fn generated(name: &str, kind: GenKind) -> ColumnPlan {
        ColumnPlan {
            name: name.to_string(),
            source: ColumnSource::Generated {
                kind,
                unique: false,
                unique_base: 0,
                min: None,
                max: None,
                max_len: None,
            },
            null_rate: 0.0,
        }
    }

    #[test]
    fn same_seed_generates_identical_rows() {
        let plan = plan_with(vec![
            generated("name", GenKind::FullName),
            generated("email", GenKind::Email),
            generated("age", GenKind::Integer),
            generated("created_at", GenKind::Timestamp),
        ]);
        let a = generate_rows(&plan, 50, &mut SeedRng::new(42));
        let b = generate_rows(&plan, 50, &mut SeedRng::new(42));
        assert_eq!(a, b);
        let c = generate_rows(&plan, 50, &mut SeedRng::new(43));
        assert_ne!(a, c);
    }

    #[test]
    fn chunked_generation_matches_one_shot_generation() {
        let plan = plan_with(vec![
            generated("name", GenKind::FullName),
            generated("email", GenKind::Email),
            generated("sequence", GenKind::Integer),
        ]);
        let expected = generate_rows(&plan, 53, &mut SeedRng::new(42));
        let mut rng = SeedRng::new(42);
        let mut chunked = generate_rows_from(&plan, 0, 20, &mut rng);
        chunked.extend(generate_rows_from(&plan, 20, 20, &mut rng));
        chunked.extend(generate_rows_from(&plan, 40, 13, &mut rng));
        assert_eq!(expected, chunked);
    }

    #[test]
    fn unique_integer_sequences_above_base() {
        let plan = plan_with(vec![ColumnPlan {
            name: "id".to_string(),
            source: ColumnSource::Generated {
                kind: GenKind::Integer,
                unique: true,
                unique_base: 100,
                min: None,
                max: None,
                max_len: None,
            },
            null_rate: 0.0,
        }]);
        let rows = generate_rows(&plan, 3, &mut SeedRng::new(1));
        let values: Vec<String> = rows.into_iter().map(|r| r[0].clone().unwrap()).collect();
        assert_eq!(values, vec!["101", "102", "103"]);
    }

    #[test]
    fn unique_text_kinds_never_collide() {
        for kind in [
            GenKind::Word,
            GenKind::Email,
            GenKind::UserName,
            GenKind::Uuid,
            GenKind::FullName,
        ] {
            let plan = plan_with(vec![ColumnPlan {
                name: "u".to_string(),
                source: ColumnSource::Generated {
                    kind,
                    unique: true,
                    unique_base: 0,
                    min: None,
                    max: None,
                    max_len: None,
                },
                null_rate: 0.0,
            }]);
            let rows = generate_rows(&plan, 500, &mut SeedRng::new(7));
            let mut seen = std::collections::HashSet::new();
            for row in &rows {
                assert!(
                    seen.insert(row[0].clone().unwrap()),
                    "duplicate value for {kind:?}"
                );
            }
        }
    }

    #[test]
    fn null_rate_zero_and_one_are_exact() {
        let mut never = generated("a", GenKind::Word);
        never.null_rate = 0.0;
        let mut always = generated("b", GenKind::Word);
        always.null_rate = 1.0;
        let plan = plan_with(vec![never, always]);
        let rows = generate_rows(&plan, 100, &mut SeedRng::new(9));
        for row in &rows {
            assert!(row[0].is_some());
            assert!(row[1].is_none());
        }
    }

    #[test]
    fn composite_fk_members_stay_aligned() {
        let plan = SeedPlan {
            columns: vec![
                ColumnPlan {
                    name: "ref_a".to_string(),
                    source: ColumnSource::FkPool { pool: 0, member: 0 },
                    null_rate: 0.0,
                },
                ColumnPlan {
                    name: "ref_b".to_string(),
                    source: ColumnSource::FkPool { pool: 0, member: 1 },
                    null_rate: 0.0,
                },
            ],
            fk_pools: vec![vec![
                vec!["1".to_string(), "x".to_string()],
                vec!["2".to_string(), "y".to_string()],
                vec!["3".to_string(), "z".to_string()],
            ]],
            now_epoch_secs: 1_750_000_000,
            dialect: SeedDialect::Postgres,
        };
        let rows = generate_rows(&plan, 200, &mut SeedRng::new(11));
        for row in &rows {
            let pair = (row[0].clone().unwrap(), row[1].clone().unwrap());
            assert!(matches!(
                pair,
                (a, b) if (a == "1" && b == "x") || (a == "2" && b == "y") || (a == "3" && b == "z")
            ));
        }
    }

    #[test]
    fn skip_columns_are_omitted_from_output() {
        let plan = plan_with(vec![
            ColumnPlan {
                name: "id".to_string(),
                source: ColumnSource::Skip,
                null_rate: 0.0,
            },
            generated("name", GenKind::Word),
        ]);
        assert_eq!(insert_columns(&plan), vec!["name".to_string()]);
        let rows = generate_rows(&plan, 5, &mut SeedRng::new(3));
        assert!(rows.iter().all(|r| r.len() == 1));
    }

    #[test]
    fn classify_prefers_name_inference_when_type_compatible() {
        assert_eq!(
            classify_column("email", "character varying(255)"),
            Some(GenKind::Email)
        );
        assert_eq!(
            classify_column("email_count", "integer"),
            Some(GenKind::Integer)
        );
        assert_eq!(
            classify_column("unit_price", "numeric(10,2)"),
            Some(GenKind::Price)
        );
        assert_eq!(
            classify_column("created_at", "timestamp with time zone"),
            Some(GenKind::Timestamp)
        );
        assert_eq!(classify_column("tags", "text[]"), Some(GenKind::EmptyArray));
        assert_eq!(classify_column("mood", "mood_enum"), None);
    }

    #[test]
    fn varchar_length_is_respected() {
        assert_eq!(max_char_length("character varying(12)"), Some(12));
        assert_eq!(max_char_length("text"), None);
        let plan = plan_with(vec![ColumnPlan {
            name: "code".to_string(),
            source: ColumnSource::Generated {
                kind: GenKind::Sentence,
                unique: false,
                unique_base: 0,
                min: None,
                max: None,
                max_len: Some(12),
            },
            null_rate: 0.0,
        }]);
        let rows = generate_rows(&plan, 20, &mut SeedRng::new(5));
        for row in &rows {
            assert!(row[0].as_ref().unwrap().chars().count() <= 12);
        }
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use crate::{ForeignKeyInfo, IndexInfo, StructureCapabilities};

    fn column(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            nullable: false,
            default_value: None,
            is_primary_key: false,
            ordinal_position: 1,
            derivation_kind: None,
        }
    }

    fn structure(columns: Vec<ColumnInfo>) -> TableStructure {
        TableStructure {
            columns,
            primary_key: None,
            foreign_keys: Vec::new(),
            indexes: Vec::new(),
            constraints: Vec::new(),
            capabilities: StructureCapabilities {
                columns: true,
                primary_key: true,
                foreign_keys: true,
                indexes: true,
                constraints: true,
                can_insert_rows: true,
                can_update_rows: true,
                can_delete_rows: true,
                can_alter_schema: true,
                uniqueness_guarantee: "exact".to_string(),
                triggers: false,
                policies: false,
                privileges: false,
            },
            triggers: Vec::new(),
            policies: Vec::new(),
            privileges: Vec::new(),
            row_security: None,
            table_engine: None,
            partition_by: None,
            sample_by: None,
        }
    }

    fn source_of<'a>(draft: &'a PlanDraft, name: &str) -> &'a ColumnSource {
        &draft
            .columns
            .iter()
            .find(|column| column.name == name)
            .expect("column in plan")
            .source
    }

    fn run(dialect: SeedDialect, structure: &TableStructure) -> PlanDraft {
        analyze_plan(dialect, structure, &[]).expect("plan")
    }

    // -- type normalisation ------------------------------------------------

    #[test]
    fn mysql_types_drop_modifiers_and_fold_synonyms() {
        let cases = [
            ("int(11) unsigned", "integer"),
            ("BIGINT(20) UNSIGNED ZEROFILL", "bigint"),
            ("tinyint(1)", "boolean"),
            ("tinyint(4)", "tinyint"),
            ("longtext", "text"),
            ("mediumblob", "blob"),
            ("varchar(120)", "varchar(120)"),
            ("decimal(10,2)", "decimal(10,2)"),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                normalize_data_type(SeedDialect::MySql, raw),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn clickhouse_types_unwrap_nullable_and_lowcardinality() {
        let cases = [
            ("LowCardinality(Nullable(String))", "text"),
            ("Nullable(UInt8)", "tinyint"),
            ("UInt32", "integer"),
            ("Int64", "bigint"),
            ("Float64", "double precision"),
            ("FixedString(16)", "varchar(16)"),
            ("DateTime64(3)", "timestamp"),
            ("Date32", "date"),
            ("Array(String)", "array"),
            ("IPv4", "inet"),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                normalize_data_type(SeedDialect::ClickHouse, raw),
                expected,
                "{raw}"
            );
        }
    }

    #[test]
    fn sqlite_untyped_column_falls_back_to_text() {
        assert_eq!(normalize_data_type(SeedDialect::Sqlite, ""), "text");
        assert_eq!(
            normalize_data_type(SeedDialect::Sqlite, "UNSIGNED BIG INT"),
            "bigint"
        );
        assert_eq!(
            normalize_data_type(SeedDialect::Sqlite, "VARCHAR(40)"),
            "varchar(40)"
        );
    }

    #[test]
    fn enum_members_parse_from_both_engine_spellings() {
        assert_eq!(
            enum_members("enum('small','large')"),
            Some(vec!["small".to_string(), "large".to_string()])
        );
        assert_eq!(
            enum_members(&normalize_data_type(
                SeedDialect::ClickHouse,
                "Enum8('a' = 1, 'b' = 2)"
            )),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            enum_members("enum('it''s')"),
            Some(vec!["it's".to_string()])
        );
        assert_eq!(enum_members("varchar(20)"), None);
    }

    // -- database-supplied columns ----------------------------------------

    #[test]
    fn mysql_auto_increment_column_is_skipped() {
        let mut id = column("id", "int");
        id.is_primary_key = true;
        id.default_value = Some("AUTO_INCREMENT".to_string());
        let mut table = structure(vec![id, column("name", "varchar(30)")]);
        table.primary_key = Some(vec!["id".to_string()]);

        let draft = run(SeedDialect::MySql, &table);
        assert!(matches!(source_of(&draft, "id"), ColumnSource::Skip));
        assert!(matches!(
            source_of(&draft, "name"),
            ColumnSource::Generated { .. }
        ));
    }

    #[test]
    fn sqlite_integer_rowid_key_is_skipped_but_composite_key_is_not() {
        let mut id = column("id", "INTEGER");
        id.is_primary_key = true;
        let mut table = structure(vec![id]);
        table.primary_key = Some(vec!["id".to_string()]);
        let draft = run(SeedDialect::Sqlite, &table);
        assert!(matches!(source_of(&draft, "id"), ColumnSource::Skip));

        // Two-column keys are not rowid aliases — SQLite assigns nothing.
        let mut left = column("left_id", "INTEGER");
        left.is_primary_key = true;
        let mut right = column("right_id", "INTEGER");
        right.is_primary_key = true;
        let mut composite = structure(vec![left, right]);
        composite.primary_key = Some(vec!["left_id".to_string(), "right_id".to_string()]);
        let draft = run(SeedDialect::Sqlite, &composite);
        assert!(matches!(
            source_of(&draft, "left_id"),
            ColumnSource::Generated { .. }
        ));
    }

    #[test]
    fn clickhouse_derived_columns_are_never_inserted() {
        let mut materialized = column("day", "Date");
        materialized.derivation_kind = Some("MATERIALIZED".to_string());
        materialized.default_value = Some("MATERIALIZED toDate(ts)".to_string());
        let mut defaulted = column("source", "String");
        defaulted.default_value = Some("DEFAULT 'web'".to_string());
        let table = structure(vec![
            column("ts", "DateTime"),
            materialized,
            defaulted,
            column("label", "LowCardinality(String)"),
        ]);

        let draft = run(SeedDialect::ClickHouse, &table);
        assert!(matches!(source_of(&draft, "day"), ColumnSource::Skip));
        assert!(matches!(source_of(&draft, "source"), ColumnSource::Skip));
        assert!(matches!(
            source_of(&draft, "label"),
            ColumnSource::Generated { .. }
        ));
    }

    #[test]
    fn clickhouse_sorting_key_is_not_treated_as_unique() {
        let mut sorted = column("user_id", "UInt32");
        sorted.is_primary_key = true;
        let mut table = structure(vec![sorted]);
        table.primary_key = Some(vec!["user_id".to_string()]);

        let draft = run(SeedDialect::ClickHouse, &table);
        assert!(draft.needed_maxes.is_empty());
        assert!(matches!(
            source_of(&draft, "user_id"),
            ColumnSource::Generated { unique: false, .. }
        ));
    }

    // -- enums, FKs, unique sampling ---------------------------------------

    #[test]
    fn enum_columns_become_a_value_list_of_their_declared_members() {
        let table = structure(vec![column("size", "enum('small','large')")]);
        let draft = run(SeedDialect::MySql, &table);
        match source_of(&draft, "size") {
            ColumnSource::ValueList(values) => {
                assert_eq!(values, &vec!["small".to_string(), "large".to_string()]);
            }
            other => panic!("expected a value list, got {other:?}"),
        }
    }

    #[test]
    fn only_referenced_foreign_keys_need_a_parent_pool() {
        let mut table = structure(vec![column("customer_id", "int"), column("note", "text")]);
        table.foreign_keys = vec![
            ForeignKeyInfo {
                name: "fk_customer".to_string(),
                columns: vec!["customer_id".to_string()],
                referenced_schema: "shop".to_string(),
                referenced_table: "customers".to_string(),
                referenced_columns: vec!["id".to_string()],
                on_update: None,
                on_delete: None,
            },
            // Declared on a column this table doesn't generate.
            ForeignKeyInfo {
                name: "fk_unused".to_string(),
                columns: vec!["absent".to_string()],
                referenced_schema: "shop".to_string(),
                referenced_table: "regions".to_string(),
                referenced_columns: vec!["id".to_string()],
                on_update: None,
                on_delete: None,
            },
        ];

        let draft = run(SeedDialect::MySql, &table);
        assert_eq!(draft.needed_pools, vec![0]);
    }

    #[test]
    fn an_empty_parent_table_fails_before_any_write() {
        let mut table = structure(vec![column("customer_id", "int")]);
        table.foreign_keys = vec![ForeignKeyInfo {
            name: "fk_customer".to_string(),
            columns: vec!["customer_id".to_string()],
            referenced_schema: String::new(),
            referenced_table: "customers".to_string(),
            referenced_columns: vec!["id".to_string()],
            on_update: None,
            on_delete: None,
        }];

        let draft = run(SeedDialect::Sqlite, &table);
        let error = finalize_plan(&table, draft, vec![Some(Vec::new())], &[], 0)
            .expect_err("empty parent must fail");
        assert!(error.contains("customers"), "unexpected error: {error}");
    }

    #[test]
    fn unique_integer_columns_request_a_max_probe_and_start_above_it() {
        let mut table = structure(vec![column("code", "int")]);
        table.indexes = vec![IndexInfo {
            name: "uq_code".to_string(),
            columns: vec!["code".to_string()],
            is_unique: true,
            is_primary: false,
            method: None,
        }];

        let draft = run(SeedDialect::MySql, &table);
        assert_eq!(draft.needed_maxes, vec!["code".to_string()]);

        let plan = finalize_plan(&table, draft, Vec::new(), &[("code".to_string(), 500)], 0)
            .expect("plan");
        let rows = generate_rows(&plan, 3, &mut SeedRng::new(1));
        let values: Vec<i64> = rows
            .iter()
            .map(|row| row[0].as_ref().unwrap().parse().unwrap())
            .collect();
        assert_eq!(values, vec![501, 502, 503]);
    }

    // -- dialect-specific rendering ---------------------------------------

    #[test]
    fn booleans_render_as_digits_outside_postgres() {
        for (dialect, expected) in [
            (SeedDialect::Postgres, ["true", "false"]),
            (SeedDialect::MySql, ["1", "0"]),
            (SeedDialect::Sqlite, ["1", "0"]),
            (SeedDialect::ClickHouse, ["1", "0"]),
        ] {
            let table = structure(vec![column("flag", "boolean")]);
            let draft = analyze_plan(dialect, &table, &[]).expect("plan");
            let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
            let rows = generate_rows(&plan, 40, &mut SeedRng::new(7));
            for row in &rows {
                let value = row[0].as_ref().unwrap();
                assert!(
                    expected.contains(&value.as_str()),
                    "{dialect:?} produced {value}"
                );
            }
        }
    }

    #[test]
    fn clickhouse_arrays_render_with_bracket_syntax() {
        let table = structure(vec![column("tags", "Array(String)")]);
        let draft = run(SeedDialect::ClickHouse, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        let rows = generate_rows(&plan, 2, &mut SeedRng::new(1));
        assert_eq!(rows[0][0].as_deref(), Some("[]"));
    }

    #[test]
    fn fixed_string_length_is_carried_through_to_truncation() {
        let table = structure(vec![column("blurb", "FixedString(8)")]);
        let draft = run(SeedDialect::ClickHouse, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        let rows = generate_rows(&plan, 20, &mut SeedRng::new(5));
        for row in &rows {
            assert!(row[0].as_ref().unwrap().chars().count() <= 8);
        }
    }

    #[test]
    fn tiny_integers_stay_inside_a_single_byte() {
        let table = structure(vec![column("score", "UInt8")]);
        let draft = run(SeedDialect::ClickHouse, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        let rows = generate_rows(&plan, 200, &mut SeedRng::new(9));
        for row in &rows {
            let value: i64 = row[0].as_ref().unwrap().parse().unwrap();
            assert!((0..=127).contains(&value), "out of tinyint range: {value}");
        }
    }
}
