//! Table Seeding value generation (ADR-0014).
//!
//! Pure and engine-agnostic: everything here is deterministic under a
//! seed and never touches a database. Engine modules build a
//! [`SeedPlan`] (which requires introspection and sampling queries) and
//! call [`generate_rows`] to materialise row batches as the same
//! `Vec<Option<String>>` shape the bulk-insert builders consume.

use crate::{ColumnInfo, SeedColumnSpec, TableStructure};

pub(crate) mod data;
pub(crate) mod phone;
pub(crate) mod place;
pub(crate) mod text;

pub use data::Locale;
pub use text::Realism;

/// Recorded with each seed run: the same seed reproduces the same rows
/// only under the same version. Bump it whenever the data files
/// ([`data::DATA_VERSION`]), a generator's algorithm or the order of RNG
/// draws changes.
pub(crate) const SEED_GENERATOR_VERSION: u32 = 2;
use place::Place;
pub use text::GenOptions;
use text::Persona;
pub(crate) use text::{LengthPlan, MAX_GENERATED_CHARS};

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
    MiddleName,
    Paragraph,
    AddressLine2,
    Region,
    PostalCode,
    CountryCode,
    FullAddress,
}

/// What one generated row has to draw before its cells.
#[derive(Debug, Default, Clone, Copy)]
struct Needs {
    persona: bool,
    place: bool,
    locale: bool,
}

impl Needs {
    fn of(plan: &SeedPlan) -> Self {
        let mut needs = Needs::default();
        for column in &plan.columns {
            if let ColumnSource::Generated { kind, .. } = column.source {
                needs.persona |= kind.uses_persona();
                needs.place |= kind.uses_place();
                needs.locale |= kind.uses_locale();
            }
        }
        needs
    }
}

/// The made-up person and place one row's columns share, so a row's name,
/// email, address and phone agree with each other.
struct Row {
    persona: Option<Persona>,
    place: Option<Place>,
    locale: Option<Locale>,
}

impl Row {
    fn draw(rng: &mut SeedRng, options: &GenOptions, needs: Needs) -> Self {
        let persona = needs.persona.then(|| Persona::draw(rng, options));
        let locale = persona
            .as_ref()
            .map(|p| p.locale)
            .or_else(|| (needs.place || needs.locale).then(|| *rng.pick(&options.locales)));
        let place = match (needs.place, locale) {
            (true, Some(locale)) => Some(Place::draw(rng, locale)),
            _ => None,
        };
        Self {
            persona,
            place,
            locale,
        }
    }
}

impl GenKind {
    /// Character-length limits of a text generator: the shortest value it
    /// can build and the longest it will. `None` for kinds that are not
    /// length-planned.
    pub(crate) fn text_limits(self) -> Option<(usize, usize)> {
        Some(match self {
            GenKind::FirstName | GenKind::MiddleName | GenKind::LastName => (1, 255),
            GenKind::FullName => (3, 255),
            GenKind::Email => (text::MIN_EMAIL_CHARS, text::MAX_EMAIL_CHARS),
            GenKind::UserName => (3, 255),
            GenKind::Word | GenKind::Sentence | GenKind::Paragraph => (1, MAX_GENERATED_CHARS),
            GenKind::Phone => (phone::MIN_PHONE_CHARS, phone::MAX_PHONE_CHARS),
            GenKind::StreetAddress => (place::MIN_STREET_CHARS, 255),
            GenKind::AddressLine2 | GenKind::City | GenKind::Region => (1, 255),
            GenKind::PostalCode => (place::MIN_POSTCODE_CHARS, place::MAX_POSTCODE_CHARS),
            GenKind::Country => (2, 60),
            GenKind::CountryCode => (2, 3),
            GenKind::FullAddress => (place::MIN_FULL_ADDRESS_CHARS, 1000),
            GenKind::Company => (3, 255),
            GenKind::Url => (place::MIN_URL_CHARS, 2048),
            _ => return None,
        })
    }

    /// Values of these kinds are ASCII; others may need four UTF-8 bytes
    /// per character.
    fn ascii(self) -> bool {
        matches!(
            self,
            GenKind::Word
                | GenKind::Sentence
                | GenKind::Paragraph
                | GenKind::Email
                | GenKind::UserName
                | GenKind::Phone
                | GenKind::PostalCode
                | GenKind::CountryCode
                | GenKind::Url
        )
    }

    /// Values of these kinds come from the row's place.
    fn uses_place(self) -> bool {
        matches!(
            self,
            GenKind::StreetAddress
                | GenKind::AddressLine2
                | GenKind::City
                | GenKind::Region
                | GenKind::PostalCode
                | GenKind::FullAddress
        )
    }

    /// Values of these kinds follow the row's locale without a person or
    /// place of their own.
    fn uses_locale(self) -> bool {
        matches!(
            self,
            GenKind::Phone
                | GenKind::Company
                | GenKind::Url
                | GenKind::Country
                | GenKind::CountryCode
        )
    }

    /// Longest value this kind can produce under `length`, in characters,
    /// including a unique tag. Natural values stay far below a wide window
    /// (one sentence, not 100,000 characters), which keeps batches large.
    pub(crate) fn worst_case_chars(self, length: LengthPlan) -> usize {
        let natural = match self {
            GenKind::Word => 64,
            GenKind::Sentence => 256,
            GenKind::Paragraph => 6 * 241,
            GenKind::Phone => phone::MAX_PHONE_CHARS,
            GenKind::PostalCode => place::MAX_POSTCODE_CHARS,
            GenKind::CountryCode => 3,
            GenKind::Country => 60,
            GenKind::Url => 160,
            GenKind::FullAddress => 400,
            _ => 255,
        };
        let produced = if length.uniform {
            length.hi
        } else {
            length.hi.min(natural)
        };
        produced + 16
    }

    /// Values of these kinds come from the row's persona.
    fn uses_persona(self) -> bool {
        matches!(
            self,
            GenKind::FirstName
                | GenKind::MiddleName
                | GenKind::LastName
                | GenKind::FullName
                | GenKind::Email
                | GenKind::UserName
        )
    }

    /// Characters a unique tag adds beyond the row number's digits.
    fn unique_tag_overhead(self) -> usize {
        match self {
            GenKind::Sentence | GenKind::Paragraph => 3,
            GenKind::StreetAddress
            | GenKind::AddressLine2
            | GenKind::FullAddress
            | GenKind::Company => 2,
            GenKind::Url => 4,
            _ => 1,
        }
    }

    /// The tag a unique column appends: a separator the row number never
    /// contains, so values with distinct row numbers never collide.
    fn unique_tag(self, sequence: u64) -> String {
        match self {
            GenKind::Email => format!(".{sequence}"),
            GenKind::UserName => format!("_{sequence}"),
            GenKind::Sentence | GenKind::Paragraph => format!(" ({sequence})"),
            GenKind::FullName => format!(" {sequence}"),
            GenKind::StreetAddress
            | GenKind::AddressLine2
            | GenKind::FullAddress
            | GenKind::Company => {
                format!(" #{sequence}")
            }
            GenKind::Url => format!("?id={sequence}"),
            _ => format!("-{sequence}"),
        }
    }
}

const WORDS: &[&str] = &[
    "amber", "harbor", "willow", "summit", "meadow", "lantern", "ember", "cobalt", "drift",
    "fable", "garnet", "haven", "indigo", "juniper", "kestrel", "ledger", "marble", "north",
    "orchard", "pebble", "quartz", "ripple", "sable", "tundra", "umber", "velvet", "wander",
    "yonder", "zephyr", "atlas", "breeze", "cinder", "dapple", "eddy", "fjord", "glade",
];

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
        "middleName" => GenKind::MiddleName,
        "paragraph" => GenKind::Paragraph,
        "addressLine2" => GenKind::AddressLine2,
        "region" => GenKind::Region,
        "postalCode" => GenKind::PostalCode,
        "countryCode" => GenKind::CountryCode,
        "fullAddress" => GenKind::FullAddress,
        other => return Err(format!("unknown generator id: {other}")),
    })
}

// ---------------------------------------------------------------------------
// Data-type normalisation
// ---------------------------------------------------------------------------

/// Fold an engine's own type spelling into the PostgreSQL-flavoured
/// vocabulary [`classify_column`] and [`value_bound`] understand.
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

/// Types that hold fractional amounts. Integer types are left out so a
/// `total integer` column is not handed `12.34`.
fn is_fractional_numeric(data_type: &str) -> bool {
    let t = data_type.to_ascii_lowercase();
    let (base, _) = split_parameterized(t.trim());
    matches!(
        base.trim(),
        "numeric"
            | "decimal"
            | "dec"
            | "fixed"
            | "real"
            | "float"
            | "float4"
            | "float8"
            | "double"
            | "double precision"
            | "money"
    )
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
        if n == "middle_name" || n == "middlename" {
            return Some(GenKind::MiddleName);
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
        // Before addresses: `website_address` is a URL.
        if n.contains("url") || n.contains("website") || n.contains("homepage") {
            return Some(GenKind::Url);
        }
        if let Some(kind) = infer_address(&n) {
            return Some(kind);
        }
        if n.ends_with("name") && !names_a_thing(&n) {
            return Some(GenKind::FullName);
        }
        if n.contains("phone")
            || n.contains("mobile")
            || n.contains("msisdn")
            || ends_with_word(&n, "tel")
            || ends_with_word(&n, "fax")
        {
            return Some(GenKind::Phone);
        }
        // Whole name parts only: `message_id`, `content_type` and
        // `content_hash` are identifiers, not prose.
        if [
            "description",
            "desc",
            "bio",
            "biography",
            "notes",
            "note",
            "comment",
            "comments",
            "body",
            "content",
            "message",
            "about",
            "details",
        ]
        .iter()
        .any(|word| ends_with_word(&n, word))
        {
            return Some(GenKind::Paragraph);
        }
        if ["summary", "title", "subject", "headline", "caption"]
            .iter()
            .any(|word| ends_with_word(&n, word))
        {
            return Some(GenKind::Sentence);
        }
        if n.ends_with("name") {
            return Some(GenKind::Word);
        }
    }
    if is_fractional_numeric(data_type)
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

/// Address-part columns. `state` is left alone: it is as often a status
/// (`order.state`) as a region, so it needs an explicit generator.
fn infer_address(n: &str) -> Option<GenKind> {
    let any = |words: &[&str]| words.iter().any(|w| ends_with_word(n, w));
    let exactly = |words: &[&str]| words.contains(&n);
    if any(&[
        "country_code",
        "country_iso",
        "iso_country",
        "country_iso2",
        "country_alpha2",
    ]) {
        return Some(GenKind::CountryCode);
    }
    if any(&["country", "country_name"]) {
        return Some(GenKind::Country);
    }
    if any(&[
        "zip",
        "zipcode",
        "zip_code",
        "postcode",
        "post_code",
        "postal_code",
        "postalcode",
        "pincode",
        "pin_code",
        "cep",
        "plz",
    ]) {
        return Some(GenKind::PostalCode);
    }
    // Plain `region` (`aws_region`) and `state` (`order.state`) are as often
    // something else, so they need an explicit generator.
    if any(&[
        "province",
        "county",
        "prefecture",
        "state_province",
        "state_name",
    ]) {
        return Some(GenKind::Region);
    }
    if any(&["city", "town", "locality", "city_name"]) {
        return Some(GenKind::City);
    }
    // Only the address forms: `test_suite` is not an apartment.
    if any(&["address2", "address_2", "address_line2", "address_line_2"])
        || exactly(&[
            "line2",
            "line_2",
            "apartment",
            "apt",
            "apt_number",
            "suite",
            "suite_number",
        ])
    {
        return Some(GenKind::AddressLine2);
    }
    // `ip_address`, `mac_address`, `wallet_address` are not postal; their
    // `_country` and `_city` siblings were handled above.
    let network = [
        "ip", "mac", "wallet", "contract", "memory", "host", "remote", "server", "bitcoin",
    ]
    .iter()
    .any(|w| n.starts_with(&format!("{w}_")) || n.starts_with(&format!("{w}address")));
    if network {
        return None;
    }
    if any(&[
        "full_address",
        "mailing_address",
        "postal_address",
        "billing_address",
        "shipping_address",
        "home_address",
        "work_address",
        "delivery_address",
    ]) {
        return Some(GenKind::FullAddress);
    }
    if any(&[
        "address",
        "address1",
        "address_1",
        "address_line1",
        "address_line_1",
        "line1",
        "line_1",
        "street",
        "street_address",
        "street_name",
    ]) {
        return Some(GenKind::StreetAddress);
    }
    None
}

/// `name` is `word` or ends with `_word`.
fn ends_with_word(name: &str, word: &str) -> bool {
    name.strip_suffix(word)
        .is_some_and(|head| head.is_empty() || head.ends_with('_'))
}

/// A `*name` column that names an object rather than a person:
/// `file_name`, `hostname`, `product_name`.
fn names_a_thing(name: &str) -> bool {
    const THINGS: &[&str] = &[
        "file",
        "host",
        "domain",
        "table",
        "schema",
        "column",
        "db",
        "database",
        "product",
        "brand",
        "app",
        "application",
        "bucket",
        "key",
        "tag",
        "class",
        "type",
        "event",
        "role",
        "group",
        "project",
        "team",
        "category",
        "item",
        "service",
        "queue",
        "topic",
        "device",
        "branch",
        "repo",
        "package",
        "module",
        "field",
        "index",
        "template",
        "plan",
        "feature",
        "path",
        "dir",
        "folder",
        "image",
        "model",
        "machine",
        "server",
        "cluster",
        "region",
        "zone",
        "env",
        "environment",
        "job",
        "task",
        "metric",
        "setting",
        "config",
        "channel",
        "street",
        "city",
        "country",
        "state",
        "store",
        "shop",
        "sku",
        "variant",
    ];
    let head = name.trim_end_matches("name").trim_end_matches(['_', '-']);
    let last = head.rsplit(['_', '-']).next().unwrap_or(head);
    THINGS.contains(&last) || THINGS.contains(&head)
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

/// What a column's declared type allows a generated value to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ValueBound {
    Unbounded,
    /// `varchar(n)` / `char(n)`: at most `n` characters.
    Chars(usize),
    /// `numeric(p, s)`: at most `precision` digits, `scale` of them
    /// after the decimal point.
    Numeric {
        precision: u32,
        scale: u32,
    },
}

/// Largest scale a generated decimal is rendered with. The database pads
/// to the declared scale, and more random digits add nothing to test data.
const MAX_GENERATED_SCALE: u32 = 4;

/// The bound a normalised data type puts on generated values. Only
/// character and fixed-point types carry one: the `(3)` in
/// `timestamp(3)` or `time(0)` is a fractional-second precision, not a
/// length.
pub(crate) fn value_bound(data_type: &str) -> ValueBound {
    let (base, args) = split_parameterized(data_type.trim());
    let Some(args) = args else {
        return ValueBound::Unbounded;
    };
    match base.trim() {
        "character varying"
        | "varchar"
        | "character"
        | "char"
        | "nchar"
        | "nvarchar"
        | "national character"
        | "national character varying"
        | "national char"
        | "national varchar" => match args.trim().parse() {
            Ok(length) => ValueBound::Chars(length),
            Err(_) => ValueBound::Unbounded,
        },
        "numeric" | "decimal" | "dec" | "fixed" => {
            let mut parts = args.split(',').map(|part| part.trim().parse::<u32>());
            let precision = match parts.next() {
                Some(Ok(precision)) if precision > 0 => precision,
                _ => return ValueBound::Unbounded,
            };
            let scale = match parts.next() {
                None => 0,
                Some(Ok(scale)) if scale <= precision => scale,
                _ => return ValueBound::Unbounded,
            };
            if parts.next().is_some() {
                return ValueBound::Unbounded;
            }
            ValueBound::Numeric { precision, scale }
        }
        _ => ValueBound::Unbounded,
    }
}

/// Exclusive upper bound on the integer part of a `numeric(p, s)` value,
/// or `None` when it exceeds every value the generators produce.
pub(crate) fn numeric_integer_limit(precision: u32, scale: u32) -> Option<i64> {
    10i64.checked_pow(precision - scale)
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
        /// Length or precision bound from the declared type.
        bound: ValueBound,
        /// Character-length window for text kinds, already intersected
        /// with `bound` and the generator's own limits.
        length: Option<LengthPlan>,
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
    /// Locales and realism for names, emails and other contact data.
    pub options: GenOptions,
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

#[cfg(feature = "isolated-profile")]
impl PlanDraft {
    pub(crate) fn columns(&self) -> &[ColumnPlan] {
        &self.columns
    }
}

/// Why a Seed Spec cannot be planned against a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanError {
    pub kind: PlanErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanErrorKind {
    /// A column has no usable value source.
    Unsupported,
    /// A requested length range cannot fit the column or generator.
    Length,
}

impl PlanError {
    fn length(message: String) -> Self {
        Self {
            kind: PlanErrorKind::Length,
            message,
        }
    }
}

impl From<String> for PlanError {
    fn from(message: String) -> Self {
        Self {
            kind: PlanErrorKind::Unsupported,
            message,
        }
    }
}

impl From<PlanError> for String {
    fn from(error: PlanError) -> Self {
        error.message
    }
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Intersect a column's requested length range with its capacity and the
/// generator's own limits. Text kinds always get a window; an inferred
/// generator too long for its column falls back to plain words.
fn plan_length(
    column: &str,
    kind: GenKind,
    explicit: bool,
    bound: ValueBound,
    spec: Option<&SeedColumnSpec>,
) -> Result<(GenKind, Option<LengthPlan>), PlanError> {
    let requested = spec
        .filter(|s| s.min_length.is_some() || s.max_length.is_some())
        .map(|s| (s.min_length, s.max_length));
    let Some((min, max)) = kind.text_limits() else {
        return match requested {
            Some(_) => Err(PlanError::length(format!(
                "column \"{column}\": a length range applies only to text generators"
            ))),
            None => Ok((kind, None)),
        };
    };
    let capacity = match bound {
        ValueBound::Chars(n) => n,
        _ => usize::MAX,
    };
    let hi_limit = max.min(capacity);
    if hi_limit < min {
        if !explicit && requested.is_none() && kind != GenKind::Word {
            return plan_length(column, GenKind::Word, false, bound, spec);
        }
        return Err(PlanError::length(format!(
            "column \"{column}\" holds at most {capacity} characters, but {kind:?} values need at least {min}"
        )));
    }
    let Some((lo, hi)) = requested else {
        return Ok((kind, Some(LengthPlan::natural(min, hi_limit))));
    };
    let lo = lo.unwrap_or(min).max(min);
    let hi = hi.unwrap_or(hi_limit).min(hi_limit);
    if lo > hi {
        return Err(PlanError::length(format!(
            "column \"{column}\": the length range does not fit; the column and {kind:?} generator allow {min} to {hi_limit} characters"
        )));
    }
    Ok((
        kind,
        Some(LengthPlan {
            lo,
            hi,
            uniform: true,
        }),
    ))
}

/// A check that needs the run's row count, which a [`PlanDraft`] does
/// not know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapacityError {
    /// Unique `numeric(p, s)` values would outgrow the integer digits.
    Numeric,
    /// Unique text values would outgrow the length window once tagged.
    Length,
    /// A unique phone column needs more numbers than its ranges hold.
    Phone,
}

impl From<CapacityError> for String {
    fn from(error: CapacityError) -> Self {
        match error {
            CapacityError::Numeric => {
                "a unique numeric column cannot hold this many distinct values".to_string()
            }
            CapacityError::Length => {
                "a unique text column's length range is too short to hold this many distinct values"
                    .to_string()
            }
            CapacityError::Phone => "a unique phone column needs more numbers than the \
                 fictional-safe ranges hold; allow realistic values or seed fewer rows"
                .to_string(),
        }
    }
}

/// Largest size one generated value of `column` can have, in bytes. Text
/// kinds use their length window: prose, words, emails and usernames are
/// ASCII, names may take up to four UTF-8 bytes per character. Every other
/// generated kind renders well under 128 bytes.
pub(crate) fn worst_case_value_bytes(plan: &SeedPlan, column: &ColumnPlan) -> usize {
    match &column.source {
        ColumnSource::Skip => 0,
        ColumnSource::Constant(value) => value.as_ref().map_or(0, String::len),
        ColumnSource::ValueList(values) => values.iter().map(String::len).max().unwrap_or(0),
        ColumnSource::FkPool { pool, member } => plan.fk_pools[*pool]
            .iter()
            .map(|row| row[*member].len())
            .max()
            .unwrap_or(0),
        ColumnSource::Generated { kind, length, .. } => match (kind.text_limits(), length) {
            (Some((min, max)), length) => {
                let length = length.unwrap_or(LengthPlan::natural(min, max));
                let bytes_per_char = if kind.ascii() { 1 } else { 4 };
                kind.worst_case_chars(length).saturating_mul(bytes_per_char)
            }
            (None, _) => 128,
        },
    }
}

/// Rows whose generated values fit in `budget` bytes, at least one.
pub(crate) fn rows_within(plan: &SeedPlan, budget: usize) -> usize {
    let row = plan
        .columns
        .iter()
        .map(|column| worst_case_value_bytes(plan, column))
        .fold(0usize, usize::saturating_add)
        .max(1);
    (budget / row).max(1)
}

/// Generated bytes one INSERT batch of the bounded seed paths may carry.
pub(crate) const BATCH_VALUE_BYTES: usize = 8 * 1024 * 1024;

/// Verify unique columns can hold `row_count` distinct values.
pub(crate) fn check_capacity(
    draft: &PlanDraft,
    row_count: u32,
    options: &GenOptions,
) -> Result<(), CapacityError> {
    let digits = row_count.max(1).to_string().len();
    for column in &draft.columns {
        let ColumnSource::Generated {
            kind,
            unique: true,
            bound,
            length,
            ..
        } = column.source
        else {
            continue;
        };
        if let (true, ValueBound::Numeric { precision, scale }) = (is_fractional_kind(kind), bound)
        {
            if numeric_integer_limit(precision, scale)
                .is_some_and(|limit| i64::from(row_count) >= limit)
            {
                return Err(CapacityError::Numeric);
            }
        }
        if kind == GenKind::Phone {
            let length = length.unwrap_or(LengthPlan::natural(
                phone::MIN_PHONE_CHARS,
                phone::MAX_PHONE_CHARS,
            ));
            let phones = phone::UniquePhones::new(&options.locales, options.realism, length);
            if u64::from(row_count) > phones.capacity() {
                return Err(CapacityError::Phone);
            }
            continue;
        }
        if let (Some((min, _)), Some(length)) = (kind.text_limits(), length) {
            if length.hi < min + digits + kind.unique_tag_overhead() {
                return Err(CapacityError::Length);
            }
        }
    }
    Ok(())
}

/// Per column, whether its Auto source is a text generator and so takes a
/// length range. Foreign-key, database-supplied and enum columns never do.
pub(crate) fn auto_takes_length(dialect: SeedDialect, structure: &TableStructure) -> Vec<bool> {
    structure
        .columns
        .iter()
        .map(|column| {
            let data_type = normalize_data_type(dialect, &column.data_type);
            let foreign = structure
                .foreign_keys
                .iter()
                .any(|fk| fk.columns.contains(&column.name));
            !foreign
                && !is_database_supplied(dialect, column, structure, &data_type)
                && enum_members(&data_type).is_none()
                && classify_column(&column.name, &data_type)
                    .is_some_and(|kind| kind.text_limits().is_some())
        })
        .collect()
}

/// Decide every column's value source from the table's structure plus
/// the Seed Spec. Pure: no database access, no clock.
pub(crate) fn analyze_plan(
    dialect: SeedDialect,
    structure: &TableStructure,
    specs: &[SeedColumnSpec],
) -> Result<PlanDraft, PlanError> {
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
                return Err(format!("column \"{}\" has an empty value list", column.name).into());
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
                    )
                    .into());
                }
            },
        };
        let explicit = spec.and_then(|s| s.generator.as_deref()).is_some();
        let bound = value_bound(&data_type);
        let (kind, length) = plan_length(&column.name, kind, explicit, bound, spec)?;

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
                bound,
                length,
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
        options: GenOptions::default(),
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

/// One cell of a preview: what the run would put in the column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewCell {
    Value(String),
    Null,
    /// Omitted from the INSERT; the database supplies its default.
    Default,
    /// Sampled from the parent table's rows at run time.
    Parent,
}

/// A preview's columns: each name with its sampled cells.
pub(crate) type PreviewColumns = Vec<(String, Vec<PreviewCell>)>;

/// Stands in for a parent row's value in a preview. No generator emits NUL.
const PARENT_MARKER: &str = "\u{0}parent";

/// Sample rows for a draft without touching a database. Each foreign key
/// gets a one-row placeholder pool, so the preview draws the RNG exactly as
/// the run does and every other column shows what the run would insert with
/// the same seed and clock. Only unique sequences differ: the run starts
/// them above the table's current maximum. Returns `None` when the rows'
/// worst-case size exceeds `budget` bytes.
pub(crate) fn preview_rows(
    draft: PlanDraft,
    structure: &TableStructure,
    options: &GenOptions,
    now_epoch_secs: i64,
    seed: u64,
    rows: u32,
    budget: usize,
) -> Result<Option<PreviewColumns>, PlanError> {
    let pools = structure
        .foreign_keys
        .iter()
        .enumerate()
        .map(|(index, fk)| {
            draft
                .needed_pools
                .contains(&index)
                .then(|| vec![vec![PARENT_MARKER.to_string(); fk.columns.len()]])
        })
        .collect();
    let mut plan = finalize_plan(structure, draft, pools, &[], now_epoch_secs)?;
    plan.options = options.clone();
    if rows_within(&plan, budget) < rows as usize {
        return Ok(None);
    }
    let generated = generate_rows_from(&plan, 0, rows, &mut SeedRng::new(seed));
    let mut columns: Vec<(String, Vec<PreviewCell>)> = Vec::with_capacity(plan.columns.len());
    let mut slot = 0;
    for column in &plan.columns {
        let cells = if matches!(column.source, ColumnSource::Skip) {
            vec![PreviewCell::Default; rows as usize]
        } else {
            let cells = generated
                .iter()
                .map(|row| match &row[slot] {
                    Some(value) if value == PARENT_MARKER => PreviewCell::Parent,
                    Some(value) => PreviewCell::Value(value.clone()),
                    None => PreviewCell::Null,
                })
                .collect();
            slot += 1;
            cells
        };
        columns.push((column.name.clone(), cells));
    }
    Ok(Some(columns))
}

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
    let needs = Needs::of(plan);
    // Unique phone columns get their number blocks once per chunk, not
    // once per row.
    let unique_phones: Vec<Option<phone::UniquePhones>> = plan
        .columns
        .iter()
        .map(|c| match c.source {
            ColumnSource::Generated {
                kind: GenKind::Phone,
                unique: true,
                length,
                ..
            } => Some(phone::UniquePhones::new(
                &plan.options.locales,
                plan.options.realism,
                length.unwrap_or(LengthPlan::natural(
                    phone::MIN_PHONE_CHARS,
                    phone::MAX_PHONE_CHARS,
                )),
            )),
            _ => None,
        })
        .collect();
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
        let context = Row::draw(rng, &plan.options, needs);

        let mut row = Vec::new();
        for (column, phones) in plan.columns.iter().zip(&unique_phones) {
            if matches!(column.source, ColumnSource::Skip) {
                continue;
            }
            if column.null_rate > 0.0 && rng.next_f64() < column.null_rate {
                row.push(None);
                continue;
            }
            row.push(generate_cell(
                plan,
                column,
                row_index,
                &pool_picks,
                &context,
                phones.as_ref(),
                rng,
            ));
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
    context: &Row,
    unique_phones: Option<&phone::UniquePhones>,
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
            bound,
            length,
        } => {
            if let Some((min, max)) = kind.text_limits() {
                let length = length.unwrap_or_else(|| {
                    let hi = match bound {
                        ValueBound::Chars(n) => (*n).clamp(min, max),
                        _ => max,
                    };
                    LengthPlan::natural(min, hi)
                });
                return Some(generate_text(
                    *kind,
                    rng,
                    row_index,
                    *unique,
                    length,
                    context,
                    &plan.options,
                    unique_phones,
                ));
            }
            let fixed_point = numeric_bound(*bound);
            if is_fractional_kind(*kind) && (fixed_point.is_some() || *unique) {
                return Some(generate_fixed_point(
                    *kind,
                    rng,
                    row_index,
                    *unique,
                    *unique_base,
                    *min,
                    *max,
                    fixed_point,
                ));
            }
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
            Some(match bound {
                ValueBound::Chars(limit) => truncate_chars(&value, *limit),
                _ => value,
            })
        }
    }
}

/// A text value inside `length`. Unique columns get the row number as a
/// tag ([`GenKind::unique_tag`]); unique phones walk distinct number blocks
/// instead. `row` is the row's shared person, place and locale; a kind
/// only draws what it uses.
#[allow(clippy::too_many_arguments)]
fn generate_text(
    kind: GenKind,
    rng: &mut SeedRng,
    row_index: u32,
    unique: bool,
    length: LengthPlan,
    row: &Row,
    options: &GenOptions,
    unique_phones: Option<&phone::UniquePhones>,
) -> String {
    let sequence = u64::from(row_index) + 1;
    let tag = if unique && kind != GenKind::Phone {
        kind.unique_tag(sequence)
    } else {
        String::new()
    };
    let locale = |rng: &mut SeedRng| row.locale.unwrap_or_else(|| *rng.pick(&options.locales));

    // Email and usernames place the tag themselves; phones are unique by
    // construction.
    match kind {
        GenKind::Email => {
            return with_persona(rng, row, options, |rng, persona| {
                text::email(rng, persona, options.realism, length, &tag)
            })
        }
        GenKind::UserName => {
            return with_persona(rng, row, options, |rng, persona| {
                text::user_name(rng, persona, length, &tag)
            })
        }
        GenKind::Phone => {
            let locale = locale(rng);
            let unique = unique_phones.map(|phones| (phones, u64::from(row_index)));
            return phone::phone(rng, locale, options.realism, length, unique);
        }
        _ => {}
    }

    let floor = kind.text_limits().map_or(1, |(min, _)| min);
    let length = length.reserve(tag.chars().count(), floor);
    let mut value = if kind.uses_persona() {
        with_persona(rng, row, options, |rng, persona| match kind {
            GenKind::FirstName => text::first_name(rng, persona, length),
            GenKind::MiddleName => text::middle_name(rng, persona, length),
            GenKind::LastName => text::last_name(rng, persona, length),
            _ => text::full_name(rng, persona, length),
        })
    } else if kind.uses_place() {
        let drawn;
        let place = match &row.place {
            Some(place) => place,
            None => {
                let locale = locale(rng);
                drawn = Place::draw(rng, locale);
                &drawn
            }
        };
        match kind {
            GenKind::StreetAddress => place::street(rng, place, length),
            GenKind::AddressLine2 => place::line2(rng, place, length),
            GenKind::City => place::locality(rng, place, length),
            GenKind::Region => place::region(rng, place, length),
            GenKind::PostalCode => place::postcode(rng, place, length),
            _ => place::full_address(rng, place, length),
        }
    } else {
        match kind {
            GenKind::Company => {
                let locale = locale(rng);
                place::company(rng, locale, length)
            }
            GenKind::Url => {
                let locale = locale(rng);
                place::url(rng, locale, options.realism, length)
            }
            GenKind::Country => {
                let locale = locale(rng);
                place::country(rng, locale, length)
            }
            GenKind::CountryCode => {
                let locale = locale(rng);
                place::country_code(rng, locale, length)
            }
            GenKind::Paragraph => text::prose(rng, length, (2, 6)),
            GenKind::Sentence => text::prose(rng, length, (1, 1)),
            _ => text::word(rng, length),
        }
    };
    value.push_str(&tag);
    value
}

/// Run `build` with the row's persona, drawing one for callers that
/// generate a single cell without a row.
fn with_persona<T>(
    rng: &mut SeedRng,
    row: &Row,
    options: &GenOptions,
    build: impl FnOnce(&mut SeedRng, &Persona) -> T,
) -> T {
    match &row.persona {
        Some(persona) => build(rng, persona),
        None => {
            let persona = Persona::draw(rng, options);
            build(rng, &persona)
        }
    }
}

fn numeric_bound(bound: ValueBound) -> Option<(u32, u32)> {
    match bound {
        ValueBound::Numeric { precision, scale } => Some((precision, scale)),
        _ => None,
    }
}

fn is_fractional_kind(kind: GenKind) -> bool {
    matches!(kind, GenKind::Float | GenKind::Decimal | GenKind::Price)
}

/// A decimal that fits `numeric(precision, scale)` when the type declares
/// one, built from integer units of the rendered scale so rounding can
/// never carry it past the declared precision. Draws exactly one random
/// value, like the unbounded decimal kinds, so other columns see the same
/// RNG stream.
#[allow(clippy::too_many_arguments)]
fn generate_fixed_point(
    kind: GenKind,
    rng: &mut SeedRng,
    row_index: u32,
    unique: bool,
    unique_base: i64,
    min: Option<f64>,
    max: Option<f64>,
    fixed_point: Option<(u32, u32)>,
) -> String {
    let kind_scale = match kind {
        GenKind::Price => 2,
        _ => MAX_GENERATED_SCALE,
    };
    let rendered_scale = fixed_point.map_or(kind_scale, |(_, scale)| scale.min(kind_scale));
    let unit = 10i64.pow(rendered_scale);
    // Largest representable magnitude, in units of the rendered scale.
    let capacity = fixed_point
        .and_then(|(precision, scale)| numeric_integer_limit(precision, scale))
        .and_then(|limit| limit.checked_mul(unit))
        .map(|units| units - 1)
        .unwrap_or(i64::MAX);
    let (default_lo, default_hi) = match kind {
        GenKind::Price => (1.0, 1_000.0),
        _ => (0.0, 10_000.0),
    };
    // Float-to-int `as` casts saturate, so huge user bounds stay in range.
    let to_units = |value: f64| (value * unit as f64).round() as i64;
    let lo = to_units(min.unwrap_or(default_lo)).clamp(-capacity, capacity);
    let hi = to_units(max.unwrap_or(default_hi)).clamp(-capacity, capacity);
    let units = if unique {
        // Integer part from the row sequence, fraction from the RNG: each
        // row owns a disjoint interval, so values never collide in a run.
        let fraction = rng.range_i64(0, unit - 1);
        (unique_base + row_index as i64 + 1)
            .saturating_mul(unit)
            .saturating_add(fraction)
    } else {
        rng.range_i64(lo.min(hi), hi.max(lo))
    };
    format_units(units, rendered_scale)
}

fn format_units(units: i64, scale: u32) -> String {
    let sign = if units < 0 { "-" } else { "" };
    let magnitude = units.unsigned_abs();
    if scale == 0 {
        return format!("{sign}{magnitude}");
    }
    let unit = 10u64.pow(scale);
    format!(
        "{sign}{}.{:0width$}",
        magnitude / unit,
        magnitude % unit,
        width = scale as usize
    )
}

/// Cut to at most `limit` characters.
pub(crate) fn truncate_chars(value: &str, limit: usize) -> String {
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
        GenKind::Word
        | GenKind::Sentence
        | GenKind::Paragraph
        | GenKind::Email
        | GenKind::FirstName
        | GenKind::MiddleName
        | GenKind::LastName
        | GenKind::FullName
        | GenKind::UserName
        | GenKind::Url
        | GenKind::Phone
        | GenKind::City
        | GenKind::Country
        | GenKind::StreetAddress
        | GenKind::Company
        | GenKind::AddressLine2
        | GenKind::Region
        | GenKind::PostalCode
        | GenKind::CountryCode
        | GenKind::FullAddress => unreachable!("text kinds are built by generate_text"),
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
            options: GenOptions::default(),
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
                bound: ValueBound::Unbounded,
                length: None,
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
                bound: ValueBound::Unbounded,
                length: None,
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
                    bound: ValueBound::Unbounded,
                    length: None,
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
            options: GenOptions::default(),
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
    fn money_names_on_integer_columns_stay_integers() {
        assert_eq!(classify_column("total", "integer"), Some(GenKind::Integer));
        assert_eq!(classify_column("amount", "bigint"), Some(GenKind::BigInt));
        assert_eq!(classify_column("amount", "numeric"), Some(GenKind::Price));
        assert_eq!(
            classify_column("total_interval", "interval"),
            Some(GenKind::Interval)
        );
    }

    #[test]
    fn only_character_and_fixed_point_types_carry_a_bound() {
        let cases = [
            ("character varying(40)", ValueBound::Chars(40)),
            ("character(5)", ValueBound::Chars(5)),
            ("varchar(120)", ValueBound::Chars(120)),
            (
                "numeric(5,2)",
                ValueBound::Numeric {
                    precision: 5,
                    scale: 2,
                },
            ),
            (
                "decimal(10, 0)",
                ValueBound::Numeric {
                    precision: 10,
                    scale: 0,
                },
            ),
            (
                "numeric(4)",
                ValueBound::Numeric {
                    precision: 4,
                    scale: 0,
                },
            ),
            ("numeric", ValueBound::Unbounded),
            ("numeric(2,5)", ValueBound::Unbounded),
            ("timestamp(3) without time zone", ValueBound::Unbounded),
            ("time(0) with time zone", ValueBound::Unbounded),
            ("bit varying(8)", ValueBound::Unbounded),
            ("text", ValueBound::Unbounded),
        ];
        for (data_type, expected) in cases {
            assert_eq!(value_bound(data_type), expected, "{data_type}");
        }
    }

    fn bounded(kind: GenKind, bound: ValueBound, unique: bool) -> ColumnPlan {
        ColumnPlan {
            name: "value".to_string(),
            source: ColumnSource::Generated {
                kind,
                unique,
                unique_base: 0,
                min: None,
                max: None,
                bound,
                length: None,
            },
            null_rate: 0.0,
        }
    }

    fn decimals(rows: &[Vec<Option<String>>]) -> Vec<String> {
        rows.iter().map(|row| row[0].clone().unwrap()).collect()
    }

    fn assert_fits_numeric(value: &str, precision: u32, scale: u32) {
        let digits = value.trim_start_matches('-');
        let (integer, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        assert!(
            fraction.len() as u32 <= scale,
            "{value} has too many fraction digits for numeric({precision},{scale})"
        );
        let integer = integer.trim_start_matches('0');
        assert!(
            integer.len() as u32 <= precision - scale,
            "{value} overflows numeric({precision},{scale})"
        );
    }

    #[test]
    fn fractional_kinds_fit_the_declared_precision_and_scale() {
        for (precision, scale) in [(5, 2), (3, 3), (4, 0), (6, 6), (18, 4), (38, 10)] {
            for kind in [GenKind::Float, GenKind::Decimal, GenKind::Price] {
                let plan = plan_with(vec![bounded(
                    kind,
                    ValueBound::Numeric { precision, scale },
                    false,
                )]);
                for value in decimals(&generate_rows(&plan, 500, &mut SeedRng::new(11))) {
                    assert_fits_numeric(&value, precision, scale);
                }
            }
        }
    }

    #[test]
    fn user_ranges_are_clamped_to_the_declared_precision() {
        let plan = plan_with(vec![ColumnPlan {
            name: "value".to_string(),
            source: ColumnSource::Generated {
                kind: GenKind::Decimal,
                unique: false,
                unique_base: 0,
                min: Some(-1e300),
                max: Some(1e300),
                bound: ValueBound::Numeric {
                    precision: 4,
                    scale: 1,
                },
                length: None,
            },
            null_rate: 0.0,
        }]);
        for value in decimals(&generate_rows(&plan, 500, &mut SeedRng::new(3))) {
            assert_fits_numeric(&value, 4, 1);
        }
    }

    #[test]
    fn unique_decimals_never_collide_and_keep_their_scale() {
        for bound in [
            ValueBound::Unbounded,
            ValueBound::Numeric {
                precision: 8,
                scale: 2,
            },
        ] {
            for kind in [GenKind::Decimal, GenKind::Price] {
                let plan = plan_with(vec![bounded(kind, bound, true)]);
                let values = decimals(&generate_rows(&plan, 2_000, &mut SeedRng::new(5)));
                let distinct: std::collections::HashSet<_> = values.iter().collect();
                assert_eq!(distinct.len(), values.len(), "{kind:?} {bound:?}");
                if let ValueBound::Numeric { precision, scale } = bound {
                    for value in &values {
                        assert_fits_numeric(value, precision, scale);
                    }
                }
            }
        }
    }

    #[test]
    fn fixed_point_rendering_is_exact() {
        assert_eq!(format_units(12_345, 2), "123.45");
        assert_eq!(format_units(-5, 2), "-0.05");
        assert_eq!(format_units(7, 0), "7");
        assert_eq!(format_units(0, 4), "0.0000");
    }

    #[test]
    fn varchar_length_is_respected() {
        assert_eq!(value_bound("character varying(12)"), ValueBound::Chars(12));
        assert_eq!(value_bound("text"), ValueBound::Unbounded);
        let plan = plan_with(vec![ColumnPlan {
            name: "code".to_string(),
            source: ColumnSource::Generated {
                kind: GenKind::Sentence,
                unique: false,
                unique_base: 0,
                min: None,
                max: None,
                bound: ValueBound::Chars(12),
                length: None,
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
    fn precision_suffixes_on_temporal_types_do_not_truncate_values() {
        let table = structure(vec![
            column("created_at", "timestamp(3) without time zone"),
            column("opens_at", "time(0) without time zone"),
        ]);
        let draft = run(SeedDialect::Postgres, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 1_750_000_000).expect("plan");
        let rows = generate_rows(&plan, 20, &mut SeedRng::new(5));
        for row in &rows {
            assert!(row[0].as_ref().unwrap().len() >= "2025-01-01 00:00:00".len());
            assert_eq!(row[1].as_ref().unwrap().len(), "00:00:00".len());
        }
    }

    #[test]
    fn mysql_decimal_columns_respect_precision() {
        let table = structure(vec![column("unit_price", "decimal(5,2) unsigned")]);
        let draft = run(SeedDialect::MySql, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        for row in generate_rows(&plan, 300, &mut SeedRng::new(2)) {
            assert_fits_numeric(row[0].as_ref().unwrap(), 5, 2);
        }
    }

    fn assert_fits_numeric(value: &str, precision: u32, scale: u32) {
        let digits = value.trim_start_matches('-');
        let (integer, fraction) = digits.split_once('.').unwrap_or((digits, ""));
        assert!(fraction.len() as u32 <= scale, "{value}");
        assert!(
            integer.trim_start_matches('0').len() as u32 <= precision - scale,
            "{value}"
        );
    }

    fn spec(
        column: &str,
        generator: Option<&str>,
        length: (Option<usize>, Option<usize>),
    ) -> SeedColumnSpec {
        SeedColumnSpec {
            column: column.to_string(),
            skip: false,
            constant: None,
            values: None,
            generator: generator.map(str::to_string),
            min: None,
            max: None,
            min_length: length.0,
            max_length: length.1,
            null_rate: Some(0.0),
        }
    }

    fn length_of(draft: &PlanDraft, name: &str) -> (GenKind, Option<LengthPlan>) {
        match source_of(draft, name) {
            ColumnSource::Generated { kind, length, .. } => (*kind, *length),
            other => panic!("expected a generated column, got {other:?}"),
        }
    }

    #[test]
    fn requested_lengths_intersect_the_column_and_generator() {
        let table = structure(vec![
            column("email", "character varying(40)"),
            column("bio", "text"),
            column("nick", "character varying(20)"),
        ]);
        let specs = [
            spec("email", None, (Some(3), Some(100))),
            spec("bio", None, (Some(10), Some(5_000))),
        ];
        let draft = analyze_plan(SeedDialect::Postgres, &table, &specs).expect("plan");
        assert_eq!(
            length_of(&draft, "email"),
            (
                GenKind::Email,
                Some(LengthPlan {
                    lo: 8,
                    hi: 40,
                    uniform: true
                })
            )
        );
        assert_eq!(
            length_of(&draft, "bio"),
            (
                GenKind::Paragraph,
                Some(LengthPlan {
                    lo: 10,
                    hi: 5_000,
                    uniform: true
                })
            )
        );
        // No request: natural lengths bounded by the column.
        assert_eq!(
            length_of(&draft, "nick").1,
            Some(LengthPlan::natural(1, 20))
        );
    }

    #[test]
    fn impossible_lengths_are_length_errors() {
        let table = structure(vec![
            column("email", "character varying(40)"),
            column("age", "integer"),
        ]);
        for specs in [
            vec![spec("email", None, (Some(50), Some(60)))],
            vec![spec("email", None, (Some(30), Some(20)))],
            vec![spec("age", None, (Some(1), Some(3)))],
        ] {
            let error = analyze_plan(SeedDialect::Postgres, &table, &specs)
                .err()
                .expect("error");
            assert_eq!(error.kind, PlanErrorKind::Length, "{}", error.message);
        }
    }

    #[test]
    fn an_inferred_generator_too_long_for_its_column_falls_back_to_words() {
        let table = structure(vec![column("email", "character varying(5)")]);
        let draft = run(SeedDialect::Postgres, &table);
        assert_eq!(length_of(&draft, "email").0, GenKind::Word);
        let explicit = [spec("email", Some("email"), (None, None))];
        let error = analyze_plan(SeedDialect::Postgres, &table, &explicit)
            .err()
            .expect("error");
        assert_eq!(error.kind, PlanErrorKind::Length);
    }

    #[test]
    fn a_rows_name_and_email_describe_the_same_person() {
        let table = structure(vec![
            column("first_name", "text"),
            column("last_name", "text"),
            column("email", "text"),
        ]);
        let draft = run(SeedDialect::Postgres, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        let token = |s: &str| -> String {
            s.chars()
                .filter(char::is_ascii_alphanumeric)
                .collect::<String>()
                .to_ascii_lowercase()
        };
        let mut matched = 0;
        let rows = generate_rows(&plan, 200, &mut SeedRng::new(3));
        for row in &rows {
            let (first, last, email) = (
                token(row[0].as_ref().unwrap()),
                token(row[1].as_ref().unwrap()),
                row[2].as_ref().unwrap().to_ascii_lowercase(),
            );
            if email.contains(&last) || email.contains(&first) {
                matched += 1;
            }
        }
        // Names with letters outside ASCII fold differently; the rest match.
        assert!(matched >= 190, "{matched} of 200 rows agree");
    }

    #[test]
    fn locales_choose_the_name_data() {
        let table = structure(vec![column("first_name", "text")]);
        let draft = run(SeedDialect::Postgres, &table);
        let mut plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        plan.options.locales = vec![Locale::Ja];
        for row in generate_rows(&plan, 50, &mut SeedRng::new(1)) {
            assert!(!row[0].as_ref().unwrap().is_ascii());
        }
    }

    #[test]
    fn unique_text_stays_unique_inside_its_length_window() {
        for generator in [
            "email",
            "userName",
            "firstName",
            "lastName",
            "fullName",
            "word",
            "sentence",
            "paragraph",
        ] {
            let mut table = structure(vec![column("value", "character varying(60)")]);
            table.primary_key = Some(vec!["value".to_string()]);
            let specs = [spec("value", Some(generator), (Some(20), Some(40)))];
            let draft = analyze_plan(SeedDialect::Postgres, &table, &specs).expect("plan");
            check_capacity(&draft, 3_000, &GenOptions::default()).expect("capacity");
            let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
            let rows = generate_rows(&plan, 3_000, &mut SeedRng::new(9));
            let mut seen = std::collections::HashSet::new();
            for row in rows {
                let value = row[0].clone().unwrap();
                assert!(
                    (20..=40).contains(&value.chars().count()),
                    "{generator}: {value}"
                );
                assert!(seen.insert(value.clone()), "{generator} repeated {value}");
            }
        }
    }

    #[test]
    fn text_without_a_name_column_spends_no_draws_on_a_persona() {
        let table = structure(vec![column("title", "text")]);
        let draft = run(SeedDialect::Postgres, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        let mut alone = SeedRng::new(4);
        generate_rows(&plan, 1, &mut alone);
        let mut expected = SeedRng::new(4);
        let words = text::prose(
            &mut expected,
            LengthPlan::natural(1, MAX_GENERATED_CHARS),
            (1, 1),
        );
        let mut rng = SeedRng::new(4);
        assert_eq!(
            generate_rows(&plan, 1, &mut rng)[0][0].as_deref(),
            Some(words.as_str())
        );
        assert_eq!(alone.next_u64(), expected.next_u64());
    }

    #[test]
    fn batches_are_sized_by_the_widest_generated_values() {
        let table = structure(vec![column("body", "text"), column("age", "integer")]);
        let natural = finalize_plan(
            &table,
            run(SeedDialect::Postgres, &table),
            Vec::new(),
            &[],
            0,
        )
        .expect("plan");
        // Up to six natural sentences and an integer: under 2 KiB a row.
        assert!(rows_within(&natural, BATCH_VALUE_BYTES) > 4_000);
        let specs = [spec("body", None, (Some(50_000), Some(100_000)))];
        let draft = analyze_plan(SeedDialect::Postgres, &table, &specs).expect("plan");
        let long = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        assert_eq!(
            rows_within(&long, BATCH_VALUE_BYTES),
            BATCH_VALUE_BYTES / (100_016 + 128)
        );
    }

    #[test]
    fn unique_text_needs_room_for_its_row_tag() {
        let mut table = structure(vec![column("email", "character varying(10)")]);
        table.primary_key = Some(vec!["email".to_string()]);
        let draft = run(SeedDialect::Postgres, &table);
        assert_eq!(check_capacity(&draft, 9, &GenOptions::default()), Ok(()));
        assert_eq!(
            check_capacity(&draft, 10, &GenOptions::default()),
            Err(CapacityError::Length)
        );
    }

    #[test]
    fn object_names_are_not_people() {
        for (name, kind) in [
            ("file_name", GenKind::Word),
            ("hostname", GenKind::Word),
            ("product_name", GenKind::Word),
            ("customer_name", GenKind::FullName),
            ("name", GenKind::FullName),
            ("middle_name", GenKind::MiddleName),
            ("bio", GenKind::Paragraph),
            ("description", GenKind::Paragraph),
            ("title", GenKind::Sentence),
            ("msisdn", GenKind::Phone),
            ("message", GenKind::Paragraph),
            ("commit_message", GenKind::Paragraph),
            ("details_url", GenKind::Url),
        ] {
            assert_eq!(classify_column(name, "text"), Some(kind), "{name}");
        }
        // Identifiers that merely contain a prose word stay plain text.
        for name in [
            "message_id",
            "message_status",
            "content_type",
            "content_hash",
            "title_id",
        ] {
            assert_eq!(
                classify_column(name, "character varying(64)"),
                Some(GenKind::Word),
                "{name}"
            );
        }
    }

    #[test]
    fn address_columns_are_recognised_by_name() {
        for (name, kind) in [
            ("address", GenKind::StreetAddress),
            ("address_line1", GenKind::StreetAddress),
            ("street", GenKind::StreetAddress),
            ("address_line2", GenKind::AddressLine2),
            ("apt", GenKind::AddressLine2),
            ("billing_address", GenKind::FullAddress),
            ("city", GenKind::City),
            ("town", GenKind::City),
            ("province", GenKind::Region),
            ("county", GenKind::Region),
            ("zip", GenKind::PostalCode),
            ("postal_code", GenKind::PostalCode),
            ("pincode", GenKind::PostalCode),
            ("country", GenKind::Country),
            ("country_code", GenKind::CountryCode),
            ("fax", GenKind::Phone),
            ("home_tel", GenKind::Phone),
            ("email_address", GenKind::Email),
        ] {
            assert_eq!(classify_column(name, "text"), Some(kind), "{name}");
        }
        for (name, kind) in [
            ("ip_country", GenKind::Country),
            ("ip_city", GenKind::City),
            ("contract_city", GenKind::City),
            ("website_address", GenKind::Url),
            ("homepage_address", GenKind::Url),
            ("suite", GenKind::AddressLine2),
        ] {
            assert_eq!(classify_column(name, "text"), Some(kind), "{name}");
        }
        // Not postal: a status, network addresses, a cloud region, a test suite.
        for name in [
            "state",
            "ip_address",
            "mac_address",
            "host_address",
            "region",
            "aws_region",
            "test_suite",
        ] {
            let kind = classify_column(name, "character varying(64)");
            assert_eq!(kind, Some(GenKind::Word), "{name}");
        }
    }

    #[test]
    fn a_rows_address_parts_and_phone_describe_one_place() {
        let table = structure(vec![
            column("street", "text"),
            column("city", "text"),
            column("province", "text"),
            column("zip", "text"),
            column("country_code", "text"),
            column("phone", "text"),
            column("full_address", "text"),
        ]);
        let draft = run(SeedDialect::Postgres, &table);
        let mut plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        plan.options.locales = Locale::ALL.to_vec();
        for row in generate_rows(&plan, 400, &mut SeedRng::new(8)) {
            let cell = |i: usize| row[i].clone().unwrap();
            let (street, city, zip, code, phone, full) =
                (cell(0), cell(1), cell(3), cell(4), cell(5), cell(6));
            let locale = Locale::ALL
                .into_iter()
                .find(|l| l.country() == code)
                .expect("a seed country");
            let entry = data::places(locale)
                .entries
                .iter()
                .find(|e| e.postal == zip && e.locality == city)
                .unwrap_or_else(|| panic!("{zip} {city} is not one {locale:?} place"));
            assert_eq!(cell(2), entry.region);
            assert!(full.contains(&street) && full.contains(&zip), "{full}");
            // Fiction numbers such as Ofcom's 01632 have no region, so the
            // calling code is what ties the phone to the country.
            let number = phonenumber::parse(None, &phone).expect("phone parses");
            let calling_code = phonenumber::metadata::DATABASE
                .by_id(code.as_str())
                .expect("seed country")
                .country_code();
            assert_eq!(number.code().value(), calling_code, "{phone} for {code}");
        }
    }

    #[test]
    fn unique_phones_are_bounded_by_the_fiction_ranges() {
        let mut table = structure(vec![column("phone", "text")]);
        table.primary_key = Some(vec!["phone".to_string()]);
        let draft = run(SeedDialect::Postgres, &table);
        let us = GenOptions {
            locales: vec![Locale::EnUs],
            realism: Realism::FictionalSafe,
        };
        assert_eq!(check_capacity(&draft, 3_200, &us), Ok(()));
        assert_eq!(
            check_capacity(&draft, 3_201, &us),
            Err(CapacityError::Phone)
        );
        let realistic = GenOptions {
            realism: Realism::Realistic,
            ..us.clone()
        };
        assert_eq!(check_capacity(&draft, 1_000_000, &realistic), Ok(()));

        let mut plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        plan.options = us;
        let values: std::collections::HashSet<_> =
            generate_rows(&plan, 3_200, &mut SeedRng::new(2))
                .into_iter()
                .map(|row| row[0].clone().unwrap())
                .collect();
        assert_eq!(values.len(), 3_200);
    }

    #[test]
    fn unique_phones_keep_the_rows_country() {
        let mut table = structure(vec![
            column("country_code", "text"),
            column("phone", "text"),
        ]);
        table.primary_key = Some(vec!["phone".to_string()]);
        let draft = run(SeedDialect::Postgres, &table);
        let mut plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        plan.options.locales = vec![Locale::EnUs, Locale::Fr];
        let mut seen = std::collections::HashSet::new();
        for row in generate_rows(&plan, 3_000, &mut SeedRng::new(4)) {
            let (code, phone) = (row[0].clone().unwrap(), row[1].clone().unwrap());
            let expected = if code == "US" { "+1" } else { "+33" };
            assert!(phone.starts_with(expected), "{code}: {phone}");
            assert!(seen.insert(phone));
        }
    }

    #[test]
    fn country_columns_draw_no_place() {
        let table = structure(vec![column("country_code", "text")]);
        let draft = run(SeedDialect::Postgres, &table);
        let plan = finalize_plan(&table, draft, Vec::new(), &[], 0).expect("plan");
        let mut rng = SeedRng::new(6);
        assert_eq!(
            generate_rows(&plan, 1, &mut rng)[0][0].as_deref(),
            Some("US")
        );
        // One draw picks the row's locale; nothing else is spent.
        let mut expected = SeedRng::new(6);
        expected.next_u64();
        assert_eq!(rng.next_u64(), expected.next_u64());
    }

    #[test]
    fn previews_mark_defaults_and_parent_rows_without_a_database() {
        let mut table = structure(vec![
            column("id", "integer"),
            column("customer_id", "integer"),
            column("email", "text"),
        ]);
        table.columns[0].default_value = Some("nextval('seq')".into());
        table.foreign_keys.push(ForeignKeyInfo {
            name: "fk".into(),
            columns: vec!["customer_id".into()],
            referenced_schema: "public".into(),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        });
        let draft = run(SeedDialect::Postgres, &table);
        let preview = preview_rows(
            draft,
            &table,
            &GenOptions::default(),
            0,
            1,
            4,
            BATCH_VALUE_BYTES,
        )
        .expect("preview")
        .expect("within budget");
        assert_eq!(preview.len(), 3);
        assert!(preview[0].1.iter().all(|c| *c == PreviewCell::Default));
        assert!(preview[1].1.iter().all(|c| *c == PreviewCell::Parent));
        assert!(preview[2]
            .1
            .iter()
            .all(|c| matches!(c, PreviewCell::Value(v) if v.contains('@'))));
        // The same seed previews the same values.
        let again = preview_rows(
            run(SeedDialect::Postgres, &table),
            &table,
            &GenOptions::default(),
            0,
            1,
            4,
            BATCH_VALUE_BYTES,
        )
        .expect("preview")
        .expect("within budget");
        assert_eq!(preview, again);
    }

    #[test]
    fn previews_show_exactly_what_the_run_inserts_with_the_same_seed() {
        let mut table = structure(vec![
            column("customer_id", "integer"),
            column("email", "text"),
            column("joined", "date"),
        ]);
        table.columns[0].nullable = true;
        table.foreign_keys.push(ForeignKeyInfo {
            name: "fk".into(),
            columns: vec!["customer_id".into()],
            referenced_schema: "public".into(),
            referenced_table: "customers".into(),
            referenced_columns: vec!["id".into()],
            on_update: None,
            on_delete: None,
        });
        let specs = [SeedColumnSpec {
            null_rate: Some(0.5),
            ..spec("customer_id", None, (None, None))
        }];
        let clock = 1_750_000_000;
        let draft = analyze_plan(SeedDialect::Postgres, &table, &specs).expect("plan");
        let parents = vec![Some(vec![
            vec!["10".to_string()],
            vec!["11".to_string()],
            vec!["12".to_string()],
        ])];
        let plan = finalize_plan(&table, draft, parents, &[], clock).expect("plan");
        let run_rows = generate_rows(&plan, 8, &mut SeedRng::new(7));

        let draft = analyze_plan(SeedDialect::Postgres, &table, &specs).expect("plan");
        let preview = preview_rows(
            draft,
            &table,
            &GenOptions::default(),
            clock,
            7,
            8,
            BATCH_VALUE_BYTES,
        )
        .expect("preview")
        .expect("within budget");
        let mut nulls = 0;
        for (row, run_row) in run_rows.iter().enumerate() {
            match (&preview[0].1[row], &run_row[0]) {
                (PreviewCell::Null, None) => nulls += 1,
                (PreviewCell::Parent, Some(_)) => {}
                (cell, value) => panic!("row {row}: preview {cell:?}, run {value:?}"),
            }
            for column in [1, 2] {
                assert_eq!(
                    preview[column].1[row],
                    PreviewCell::Value(run_row[column].clone().unwrap()),
                    "row {row} column {column}"
                );
            }
        }
        assert!((1..8).contains(&nulls), "the NULL draws were exercised");
    }

    #[test]
    fn previews_refuse_rows_over_their_byte_budget() {
        let table = structure(vec![column("body", "text")]);
        let specs = [spec("body", None, (Some(90_000), Some(100_000)))];
        let draft = analyze_plan(SeedDialect::Postgres, &table, &specs).expect("plan");
        let preview =
            preview_rows(draft, &table, &GenOptions::default(), 0, 1, 8, 400_000).expect("plan");
        assert!(preview.is_none());
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
