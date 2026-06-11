//! Table Seeding value generation (ADR-0020).
//!
//! Pure and engine-agnostic: everything here is deterministic under a
//! seed and never touches a database. Engine modules build a
//! [`SeedPlan`] (which requires introspection and sampling queries) and
//! call [`generate_rows`] to materialise row batches as the same
//! `Vec<Option<String>>` shape the bulk-insert builders consume.

use crate::SeedColumnSpec;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GenKind {
    Boolean,
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

pub(crate) struct SeedPlan {
    pub columns: Vec<ColumnPlan>,
    /// FK sample pools: `pools[p]` is a list of parent rows, each a
    /// tuple of member values aligned with the FK's column order.
    pub fk_pools: Vec<Vec<Vec<String>>>,
    /// Reference epoch (seconds) for date/timestamp generation, fixed
    /// once per run so generation stays deterministic within the run.
    pub now_epoch_secs: i64,
}

/// Find the per-column spec entry, if the Seed Spec has one.
pub(crate) fn spec_for<'a>(
    specs: &'a [SeedColumnSpec],
    column: &str,
) -> Option<&'a SeedColumnSpec> {
    specs.iter().find(|s| s.column == column)
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
pub(crate) fn generate_rows(
    plan: &SeedPlan,
    row_count: u32,
    rng: &mut SeedRng,
) -> Vec<Vec<Option<String>>> {
    let mut rows = Vec::with_capacity(row_count as usize);
    for row_index in 0..row_count {
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
) -> String {
    let sequence = row_index as i64 + 1;
    let int_in = |rng: &mut SeedRng, lo: i64, hi: i64| {
        let lo = min.map(|m| m as i64).unwrap_or(lo);
        let hi = max.map(|m| m as i64).unwrap_or(hi);
        rng.range_i64(lo, hi)
    };
    match kind {
        GenKind::Boolean => rng.next_u64().is_multiple_of(2).to_string(),
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
            format!("\\x{:016x}", rng.next_u64())
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
        GenKind::EmptyArray => "{}".to_string(),
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
