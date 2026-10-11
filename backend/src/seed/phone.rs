//! Phone numbers from libphonenumber metadata (ADR-0014, "Seed data variety").
//!
//! Numbers are sampled from each region's national-number patterns with
//! the seed's own RNG and kept only when `phonenumber::is_valid` accepts
//! them. In fictional-safe mode, regions whose regulator reserves numbers
//! for fiction (US, GB, DE, FR) draw only from those reserved blocks; the
//! other regions have no such ranges and get valid numbers either way.

use super::data::Locale;
use super::text::{LengthPlan, Realism};
use super::SeedRng;
use phonenumber::{metadata::DATABASE, Mode, PhoneNumber};
use regex_syntax::hir::{Class, Hir, HirKind};
use std::sync::OnceLock;

/// Shortest and longest rendered phone number the generator can produce.
/// Twelve is the shortest E.164 number of any seed country (`+12015550100`),
/// so every window from here up can be filled.
pub(crate) const MIN_PHONE_CHARS: usize = 12;
pub(crate) const MAX_PHONE_CHARS: usize = 32;

/// One output format of a block: the number rendered with every free
/// digit as a slot, so a row only fills in digits.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Template {
    text: Vec<char>,
    slots: Vec<usize>,
}

impl Template {
    fn len(&self) -> usize {
        self.text.len()
    }

    fn fill(&self, digits: &str) -> String {
        let mut text = self.text.clone();
        for (slot, digit) in self.slots.iter().zip(digits.chars()) {
            text[*slot] = digit;
        }
        text.into_iter().collect()
    }
}

/// A run of numbers sharing a national prefix: `prefix` followed by every
/// `free_digits`-digit suffix. A single number has no free digits. Its
/// validity and formats are worked out once, when the block is built, so
/// generating a number costs no libphonenumber call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Block {
    pub locale: Locale,
    pub prefix: String,
    pub free_digits: u32,
    /// E.164, international, national and RFC 3966, in [`MODES`] order.
    templates: Vec<Template>,
}

const MODES: [Mode; 4] = [
    Mode::E164,
    Mode::International,
    Mode::National,
    Mode::Rfc3966,
];

impl Block {
    /// Build a block whose rendering differs only in the free digits for
    /// its lowest and highest suffix; `parse` decides which numbers count.
    fn new(
        locale: Locale,
        prefix: &str,
        free_digits: u32,
        parse: impl Fn(u16, &str) -> Option<PhoneNumber>,
    ) -> Option<Block> {
        let code = region(locale).code;
        let width = free_digits as usize;
        let low = parse(code, &format!("{prefix}{}", "0".repeat(width)))?;
        let high = parse(code, &format!("{prefix}{}", "9".repeat(width)))?;
        if width > 0 {
            parse(code, &format!("{prefix}5{}", "0".repeat(width - 1)))?;
        }
        let templates = MODES
            .iter()
            .map(|mode| {
                let low: Vec<char> = low.format().mode(*mode).to_string().chars().collect();
                let high: Vec<char> = high.format().mode(*mode).to_string().chars().collect();
                let slots: Vec<usize> = (0..low.len())
                    .filter(|&i| low.get(i) != high.get(i))
                    .collect();
                (low.len() == high.len() && slots.len() == width)
                    .then_some(Template { text: low, slots })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Block {
            locale,
            prefix: prefix.to_string(),
            free_digits,
            templates,
        })
    }

    fn size(&self) -> u64 {
        10u64.pow(self.free_digits)
    }

    /// Whether some format of this block fits the window.
    fn fits(&self, plan: LengthPlan) -> bool {
        self.templates.iter().any(|t| plan.contains(t.len()))
    }

    fn digits(&self, offset: u64) -> String {
        if self.free_digits == 0 {
            return String::new();
        }
        format!("{:0width$}", offset, width = self.free_digits as usize)
    }

    /// The number `offset` within the block, in a format that fits the
    /// window: E.164 for natural values, any fitting format otherwise.
    fn render(&self, offset: u64, plan: LengthPlan, rng: &mut SeedRng) -> Option<String> {
        let digits = self.digits(offset);
        if !plan.uniform && plan.contains(self.templates[0].len()) {
            return Some(self.templates[0].fill(&digits));
        }
        let start = rng.range_i64(0, MODES.len() as i64 - 1) as usize;
        (0..MODES.len())
            .map(|i| &self.templates[(start + i) % MODES.len()])
            .find(|template| plan.contains(template.len()))
            .map(|template| template.fill(&digits))
    }
}

/// US area codes the fictional 555-0100..0199 range is placed in.
const US_AREA_CODES: &[&str] = &[
    "201", "202", "206", "212", "213", "214", "215", "248", "303", "305", "310", "312", "313",
    "404", "415", "480", "503", "512", "602", "612", "617", "702", "704", "713", "718", "801",
    "813", "816", "904", "916", "919", "971",
];

/// Ofcom "numbers for drama": 1,000 numbers per national prefix.
const GB_DRAMA: &[&str] = &[
    "1134960", "1144960", "1154960", "1164960", "1174960", "1184960", "1214960", "1314960",
    "1414960", "1514960", "1614960", "2079460", "1914980", "2896496", "2920180", "1632960",
    "7700900", "8081570", "9098790", "3069990",
];

/// Bundesnetzagentur Mitteilung 148/2021: five 1,000-number city blocks,
/// two 100-number mobile blocks and ten single mobile numbers.
const DE_DRAMA_BLOCKS: &[(&str, u32)] = &[
    ("3023125", 3),
    ("6990009", 3),
    ("4066969", 3),
    ("2214710", 3),
    ("8999998", 3),
    ("17139200", 2),
    ("17604069", 2),
];
const DE_DRAMA_NUMBERS: &[&str] = &[
    "15228817386",
    "15228895456",
    "15254599371",
    "1729925904",
    "1729968532",
    "1729973185",
    "1729973186",
    "1729980752",
    "1749091317",
    "1749464308",
];

/// ARCEP Décision 2018-0881: six 10,000-number blocks for audiovisual works.
const FR_FICTION: &[&str] = &["19900", "26191", "35301", "46571", "53649", "63998"];

/// The regulator-reserved fiction blocks of a locale's country, if any.
/// They are used as published even where libphonenumber does not accept
/// them as valid: it rejects France's (ARCEP classes them as technical
/// numbers) and some Ofcom drama blocks.
fn fiction_blocks(locale: Locale) -> &'static [Block] {
    static BLOCKS: OnceLock<Vec<(Locale, Vec<Block>)>> = OnceLock::new();
    let all = BLOCKS.get_or_init(|| {
        Locale::ALL
            .iter()
            .map(|&locale| {
                let ranges: Vec<(String, u32)> = match locale {
                    Locale::EnUs => US_AREA_CODES
                        .iter()
                        .map(|npa| (format!("{npa}55501"), 2))
                        .collect(),
                    Locale::EnGb => GB_DRAMA.iter().map(|p| (p.to_string(), 3)).collect(),
                    Locale::De => DE_DRAMA_BLOCKS
                        .iter()
                        .map(|(p, free)| (p.to_string(), *free))
                        .chain(DE_DRAMA_NUMBERS.iter().map(|n| (n.to_string(), 0)))
                        .collect(),
                    Locale::Fr => FR_FICTION.iter().map(|p| (p.to_string(), 4)).collect(),
                    Locale::EnIn | Locale::Es | Locale::PtBr | Locale::Ja => Vec::new(),
                };
                let blocks = ranges
                    .iter()
                    .map(|(prefix, free)| {
                        Block::new(locale, prefix, *free, reserved)
                            .expect("published fiction ranges parse and format")
                    })
                    .collect();
                (locale, blocks)
            })
            .collect()
    });
    &all.iter().find(|(l, _)| *l == locale).expect("loaded").1
}

// ---------------------------------------------------------------------------
// Pattern sampling
// ---------------------------------------------------------------------------

/// A pattern compiled for sampling: every branch and repetition count is
/// weighted by how many strings it can produce, so each matching number is
/// about equally likely and narrow branches are not over-drawn.
enum Node {
    Literal(String),
    Class(Vec<(char, char)>),
    Concat(Vec<Node>),
    /// Branches with cumulative weights.
    Alternation(Vec<(Node, f64)>),
    /// Counts `min..=max` with cumulative weights.
    Repeat(Box<Node>, Vec<(u32, f64)>),
}

impl Node {
    fn from_hir(hir: &Hir) -> Node {
        match hir.kind() {
            HirKind::Empty | HirKind::Look(_) => Node::Literal(String::new()),
            HirKind::Literal(literal) => Node::Literal(String::from_utf8_lossy(&literal.0).into()),
            HirKind::Class(Class::Bytes(class)) => Node::Class(
                class
                    .ranges()
                    .iter()
                    .map(|r| (char::from(r.start()), char::from(r.end())))
                    .collect(),
            ),
            HirKind::Class(Class::Unicode(class)) => Node::Class(
                class
                    .ranges()
                    .iter()
                    .map(|r| (r.start(), r.end()))
                    .collect(),
            ),
            HirKind::Capture(capture) => Node::from_hir(&capture.sub),
            HirKind::Concat(parts) => Node::Concat(parts.iter().map(Node::from_hir).collect()),
            HirKind::Alternation(branches) => {
                let mut total = 0.0;
                Node::Alternation(
                    branches
                        .iter()
                        .map(|b| {
                            let node = Node::from_hir(b);
                            total += node.size();
                            (node, total)
                        })
                        .collect(),
                )
            }
            HirKind::Repetition(repetition) => {
                let sub = Node::from_hir(&repetition.sub);
                let unit = sub.size();
                let max = repetition.max.unwrap_or(repetition.min + 4);
                let mut total = 0.0;
                let counts = (repetition.min..=max)
                    .map(|k| {
                        total += unit.powi(k as i32);
                        (k, total)
                    })
                    .collect();
                Node::Repeat(Box::new(sub), counts)
            }
        }
    }

    /// How many strings this node matches, as a float (it only weighs).
    fn size(&self) -> f64 {
        match self {
            Node::Literal(_) => 1.0,
            Node::Class(ranges) => ranges
                .iter()
                .map(|(a, b)| f64::from(*b as u32 - *a as u32 + 1))
                .sum(),
            Node::Concat(parts) => parts.iter().map(Node::size).product(),
            Node::Alternation(branches) => branches.last().map_or(0.0, |(_, t)| *t),
            Node::Repeat(_, counts) => counts.last().map_or(0.0, |(_, t)| *t),
        }
    }

    fn sample(&self, rng: &mut SeedRng, out: &mut String) {
        let weighted = |rng: &mut SeedRng, total: f64| rng.next_f64() * total;
        match self {
            Node::Literal(text) => out.push_str(text),
            Node::Class(ranges) => {
                let total: u64 = ranges
                    .iter()
                    .map(|(a, b)| u64::from(*b as u32 - *a as u32) + 1)
                    .sum();
                let mut pick = rng.next_u64() % total.max(1);
                for (start, end) in ranges {
                    let size = u64::from(*end as u32 - *start as u32) + 1;
                    if pick < size {
                        if let Some(c) = char::from_u32(*start as u32 + pick as u32) {
                            out.push(c);
                        }
                        return;
                    }
                    pick -= size;
                }
            }
            Node::Concat(parts) => {
                for part in parts {
                    part.sample(rng, out);
                }
            }
            Node::Alternation(branches) => {
                let target = weighted(rng, self.size());
                let branch = branches
                    .iter()
                    .find(|(_, cumulative)| target < *cumulative)
                    .or(branches.last());
                if let Some((node, _)) = branch {
                    node.sample(rng, out);
                }
            }
            Node::Repeat(sub, counts) => {
                // Counts are drawn evenly, not by how many strings each
                // produces, which would favour the longest numbers a
                // pattern allows (15-digit German fixed lines).
                let count = counts
                    .get(rng.range_i64(0, counts.len() as i64 - 1) as usize)
                    .map_or(0, |(k, _)| *k);
                for _ in 0..count {
                    sub.sample(rng, out);
                }
            }
        }
    }
}

struct Pattern {
    node: Node,
    lengths: Vec<u16>,
}

struct Region {
    code: u16,
    /// Mobile first, then fixed line; either may be absent.
    patterns: Vec<Pattern>,
}

fn compile(pattern: &str, lengths: &[u16]) -> Option<Pattern> {
    let hir = regex_syntax::ParserBuilder::new()
        .ignore_whitespace(true)
        .unicode(false)
        .utf8(false)
        .build()
        .parse(pattern)
        .ok()?;
    Some(Pattern {
        node: Node::from_hir(&hir),
        lengths: lengths.to_vec(),
    })
}

fn region(locale: Locale) -> &'static Region {
    static REGIONS: OnceLock<Vec<(Locale, Region)>> = OnceLock::new();
    let all = REGIONS.get_or_init(|| {
        Locale::ALL
            .iter()
            .map(|&locale| {
                let metadata = DATABASE
                    .by_id(locale.country())
                    .expect("libphonenumber covers every seed locale");
                let descriptors = metadata.descriptors();
                let patterns = [descriptors.mobile(), descriptors.fixed_line()]
                    .into_iter()
                    .flatten()
                    .filter_map(|d| compile(d.national_number().as_str(), d.possible_length()))
                    .collect();
                (
                    locale,
                    Region {
                        code: metadata.country_code(),
                        patterns,
                    },
                )
            })
            .collect()
    });
    &all.iter().find(|(l, _)| *l == locale).expect("loaded").1
}

fn parse(code: u16, national: &str) -> Option<PhoneNumber> {
    let number = reserved(code, national)?;
    phonenumber::is_valid(&number).then_some(number)
}

/// A regulator-reserved fiction number, parsed but not validated. The
/// published ranges win over the validator, which rejects France's (ARCEP
/// classes them as technical numbers) and some Ofcom drama blocks.
fn reserved(code: u16, national: &str) -> Option<PhoneNumber> {
    phonenumber::parse(None, format!("+{code}{national}")).ok()
}

/// A valid number sampled from the region's mobile or fixed-line pattern.
fn sampled(locale: Locale, rng: &mut SeedRng) -> Option<PhoneNumber> {
    let region = region(locale);
    for _ in 0..64 {
        let pattern = rng.pick(&region.patterns);
        let mut national = String::new();
        pattern.node.sample(rng, &mut national);
        if !pattern.lengths.is_empty() && !pattern.lengths.contains(&(national.len() as u16)) {
            continue;
        }
        if let Some(number) = parse(region.code, &national) {
            return Some(number);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Realistic number pools
// ---------------------------------------------------------------------------

/// Blocks per locale: 256 prefixes of 10,000 numbers each.
const POOL_BLOCKS: usize = 256;
const POOL_FREE_DIGITS: u32 = 4;

/// Distinct prefixes of valid numbers whose lowest, middle and highest
/// 4-digit suffixes are valid too. Seeded from the locale alone, so every
/// run sees the same pool.
fn pool(locale: Locale) -> &'static [Block] {
    static POOLS: OnceLock<Vec<(Locale, OnceLock<Vec<Block>>)>> = OnceLock::new();
    let all = POOLS.get_or_init(|| Locale::ALL.iter().map(|&l| (l, OnceLock::new())).collect());
    let slot = &all.iter().find(|(l, _)| *l == locale).expect("loaded").1;
    slot.get_or_init(|| {
        let mut rng = SeedRng::new(0x5EED_0000 + locale as u64);
        let mut blocks: Vec<Block> = Vec::with_capacity(POOL_BLOCKS);
        for _ in 0..POOL_BLOCKS * 32 {
            if blocks.len() == POOL_BLOCKS {
                break;
            }
            let Some(number) = sampled(locale, &mut rng) else {
                continue;
            };
            let national = number.national().value().to_string();
            if national.len() < 7 {
                continue;
            }
            let prefix = &national[..national.len() - POOL_FREE_DIGITS as usize];
            if blocks.iter().any(|b| b.prefix == prefix) {
                continue;
            }
            if let Some(block) = Block::new(locale, prefix, POOL_FREE_DIGITS, parse) {
                blocks.push(block);
            }
        }
        blocks
    })
}

/// The blocks numbers for `locale` are drawn from under `realism`.
fn blocks(locale: Locale, realism: Realism) -> &'static [Block] {
    let fiction = fiction_blocks(locale);
    if realism == Realism::FictionalSafe && !fiction.is_empty() {
        fiction
    } else {
        pool(locale)
    }
}

/// One locale's blocks for a unique column, with the lookup worked out
/// once: cumulative offsets for a binary search, and an affine
/// permutation (a bijection, since the multiplier is coprime with the
/// capacity) that spreads consecutive rows across blocks.
struct BlockSet {
    blocks: Vec<&'static Block>,
    /// `starts[i]` is the first position of `blocks[i]`.
    starts: Vec<u64>,
    capacity: u64,
    multiplier: u64,
}

impl BlockSet {
    fn new(blocks: Vec<&'static Block>) -> Option<BlockSet> {
        let mut starts = Vec::with_capacity(blocks.len());
        let mut capacity = 0u64;
        for block in &blocks {
            starts.push(capacity);
            capacity += block.size();
        }
        if capacity == 0 {
            return None;
        }
        let mut multiplier = (0x9E37_79B9_7F4A_7C15u64 % capacity).max(1);
        while gcd(multiplier, capacity) != 1 {
            multiplier += 1;
        }
        Some(BlockSet {
            blocks,
            starts,
            capacity,
            multiplier,
        })
    }

    fn position(&self, index: u64) -> Option<(&'static Block, u64)> {
        if index >= self.capacity {
            return None;
        }
        let position =
            ((u128::from(index) * u128::from(self.multiplier)) % u128::from(self.capacity)) as u64;
        let slot = self.starts.partition_point(|&start| start <= position) - 1;
        Some((self.blocks[slot], position - self.starts[slot]))
    }
}

/// The number blocks of a unique phone column, per locale and limited to
/// blocks with a format that fits the column's window. Built once per
/// column, not per row.
pub(crate) struct UniquePhones {
    sets: Vec<(Locale, BlockSet)>,
}

impl UniquePhones {
    pub(crate) fn new(locales: &[Locale], realism: Realism, plan: LengthPlan) -> Self {
        let sets = locales
            .iter()
            .filter_map(|&locale| {
                let fitting = blocks(locale, realism)
                    .iter()
                    .filter(|b| b.fits(plan))
                    .collect();
                BlockSet::new(fitting).map(|set| (locale, set))
            })
            .collect();
        Self { sets }
    }

    /// Rows a column can hold. Each row numbers itself by its index within
    /// its own locale's set, so the smallest set bounds the run.
    pub(crate) fn capacity(&self) -> u64 {
        self.sets
            .iter()
            .map(|(_, set)| set.capacity)
            .min()
            .unwrap_or(0)
    }

    /// The row's own locale, or the first locale with fitting blocks when
    /// none of its numbers fit the window. Rows sharing a set get distinct
    /// numbers because each uses its distinct row index.
    fn set(&self, locale: Locale) -> Option<&BlockSet> {
        self.sets
            .iter()
            .find(|(l, _)| *l == locale)
            .or(self.sets.first())
            .map(|(_, set)| set)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// A phone number for a row of `locale` inside the plan's window. Unique
/// columns pass `unique` (the number blocks and the row index).
pub(crate) fn phone(
    rng: &mut SeedRng,
    locale: Locale,
    realism: Realism,
    plan: LengthPlan,
    unique: Option<(&UniquePhones, u64)>,
) -> String {
    if let Some((phones, index)) = unique {
        if let Some((block, offset)) = phones.set(locale).and_then(|set| set.position(index)) {
            return block
                .render(offset, plan, rng)
                .unwrap_or_else(|| block.templates[0].fill(&block.digits(offset)));
        }
    }
    // Try the row's locale first, then any locale, until a format fits.
    let mut last = None;
    for candidate in std::iter::once(locale).chain(Locale::ALL) {
        let pool = blocks(candidate, realism);
        if pool.is_empty() {
            continue;
        }
        for _ in 0..4 {
            let block = rng.pick(pool);
            let offset = rng.next_u64() % block.size();
            if let Some(value) = block.render(offset, plan, rng) {
                return value;
            }
            let shortest = block
                .templates
                .iter()
                .min_by_key(|t| t.len())
                .expect("four formats");
            let value = shortest.fill(&block.digits(offset));
            if last.as_ref().is_none_or(|l: &String| value.len() < l.len()) {
                last = Some(value);
            }
        }
    }
    // No format fits: the shortest number seen, which stays under the
    // column's capacity even if it misses the lower bound.
    last.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn natural() -> LengthPlan {
        LengthPlan::natural(MIN_PHONE_CHARS, MAX_PHONE_CHARS)
    }

    fn parsed(value: &str) -> PhoneNumber {
        let number = phonenumber::parse(None, value).unwrap_or_else(|e| panic!("{value}: {e:?}"));
        assert!(phonenumber::is_valid(&number), "{value} is not valid");
        number
    }

    /// Fiction numbers are used as published; libphonenumber rejects some
    /// (France's, and a few Ofcom drama blocks), so only parsing is checked.
    fn fiction(value: &str) -> PhoneNumber {
        phonenumber::parse(None, value).unwrap_or_else(|e| panic!("{value}: {e:?}"))
    }

    #[test]
    fn every_region_has_a_mobile_or_fixed_pattern() {
        for locale in Locale::ALL {
            assert!(!region(locale).patterns.is_empty(), "{locale:?}");
        }
    }

    #[test]
    fn realistic_numbers_are_valid_in_their_country() {
        let mut rng = SeedRng::new(1);
        for locale in Locale::ALL {
            let mut distinct = std::collections::HashSet::new();
            for _ in 0..1_000 {
                let value = phone(&mut rng, locale, Realism::Realistic, natural(), None);
                let number = parsed(&value);
                assert_eq!(
                    number.country().id().map(|id| id.as_ref().to_string()),
                    Some(locale.country().to_string()),
                    "{value}"
                );
                distinct.insert(value);
            }
            assert!(
                distinct.len() > 950,
                "{locale:?}: {} distinct",
                distinct.len()
            );
        }
    }

    #[test]
    fn fictional_numbers_stay_inside_the_reserved_blocks() {
        let mut rng = SeedRng::new(2);
        for locale in [Locale::EnUs, Locale::EnGb, Locale::De, Locale::Fr] {
            let blocks = fiction_blocks(locale);
            for _ in 0..500 {
                let value = phone(&mut rng, locale, Realism::FictionalSafe, natural(), None);
                let number = fiction(&value);
                assert_eq!(number.code().value(), region(locale).code, "{value}");
                let national = number.national().value().to_string();
                assert!(
                    blocks.iter().any(|b| national.starts_with(&b.prefix)
                        && national.len() == b.prefix.len() + b.free_digits as usize),
                    "{locale:?}: {value} is outside the fiction ranges"
                );
            }
        }
    }

    #[test]
    fn regions_without_fiction_ranges_get_valid_numbers() {
        let mut rng = SeedRng::new(3);
        for locale in [Locale::EnIn, Locale::Es, Locale::PtBr, Locale::Ja] {
            assert!(fiction_blocks(locale).is_empty());
            for _ in 0..200 {
                parsed(&phone(
                    &mut rng,
                    locale,
                    Realism::FictionalSafe,
                    natural(),
                    None,
                ));
            }
        }
    }

    #[test]
    fn spread_windows_pick_formats_that_fit() {
        let mut rng = SeedRng::new(4);
        for locale in Locale::ALL {
            for (lo, hi) in [(10, 14), (15, 20), (20, 32)] {
                let plan = LengthPlan {
                    lo,
                    hi,
                    uniform: true,
                };
                for _ in 0..100 {
                    let value = phone(&mut rng, locale, Realism::Realistic, plan, None);
                    assert!(
                        (lo..=hi).contains(&value.len()),
                        "{locale:?} {lo}..{hi}: {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn unique_numbers_never_repeat_and_stop_at_capacity() {
        let phones = UniquePhones::new(
            &[Locale::EnUs, Locale::Fr],
            Realism::FictionalSafe,
            natural(),
        );
        // Rows number themselves within their own locale's set, so the
        // smaller one (32 US area codes x 100) bounds the column.
        assert_eq!(phones.capacity(), 32 * 100);
        for locale in [Locale::EnUs, Locale::Fr] {
            let set = phones.set(locale).expect("set");
            let mut seen = std::collections::HashSet::new();
            for index in 0..set.capacity {
                let (block, offset) = set.position(index).expect("inside capacity");
                assert!(
                    seen.insert(format!("{}{}", block.prefix, block.digits(offset))),
                    "repeat at {index}"
                );
            }
            assert!(set.position(set.capacity).is_none());
        }
    }

    #[test]
    fn unique_numbers_follow_the_row_locale_and_window() {
        let window = LengthPlan {
            lo: 12,
            hi: 12,
            uniform: true,
        };
        let phones = UniquePhones::new(&[Locale::EnUs, Locale::De], Realism::FictionalSafe, window);
        let mut rng = SeedRng::new(1);
        let mut seen = std::collections::HashSet::new();
        for index in 0..3_000 {
            let locale = if index % 2 == 0 {
                Locale::EnUs
            } else {
                Locale::De
            };
            let value = phone(
                &mut rng,
                locale,
                Realism::FictionalSafe,
                window,
                Some((&phones, index)),
            );
            assert_eq!(value.len(), 12, "{value}");
            assert!(seen.insert(value.clone()), "{value} repeated");
            // US rows fit as E.164; German ones only in national format.
            if locale == Locale::EnUs {
                assert!(value.starts_with("+1"), "{value}");
            } else {
                assert!(value.starts_with('0'), "{value}");
            }
        }
        let natural_phones = UniquePhones::new(
            &[Locale::EnUs, Locale::De],
            Realism::FictionalSafe,
            natural(),
        );
        let value = phone(
            &mut rng,
            Locale::De,
            Realism::FictionalSafe,
            natural(),
            Some((&natural_phones, 7)),
        );
        assert!(value.starts_with("+49"), "{value}");
    }

    #[test]
    fn templates_render_what_libphonenumber_renders() {
        let mut rng = SeedRng::new(9);
        for locale in Locale::ALL {
            for block in pool(locale).iter().take(16) {
                let offset = rng.next_u64() % block.size();
                let national = format!("{}{}", block.prefix, block.digits(offset));
                let number = parse(region(locale).code, &national).expect("valid");
                for (template, mode) in block.templates.iter().zip(MODES) {
                    assert_eq!(
                        template.fill(&block.digits(offset)),
                        number.format().mode(mode).to_string(),
                        "{locale:?} {national}"
                    );
                }
            }
        }
    }

    #[test]
    fn realistic_unique_blocks_hold_a_million_rows_per_locale() {
        for locale in Locale::ALL {
            let phones = UniquePhones::new(&[locale], Realism::Realistic, natural());
            assert!(phones.capacity() >= 2_000_000, "{locale:?}");
            let mut rng = SeedRng::new(5);
            for index in (0..1_000_000).step_by(9_973) {
                let value = phone(
                    &mut rng,
                    locale,
                    Realism::Realistic,
                    natural(),
                    Some((&phones, index)),
                );
                parsed(&value);
            }
        }
    }
}
