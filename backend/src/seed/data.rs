//! Compiled-in Table Seeding data (ADR-0014, "Seed data variety").
//!
//! The files under `backend/data/seed/` are produced by
//! `tools/seed-data/build.py`; see its `MANIFEST.md` for sources and
//! licenses. They are parsed once, on first use, into length-indexed
//! weighted lists so a generator can draw "a family name of 4 to 9
//! characters" in time proportional to the number of distinct lengths.

use super::SeedRng;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Version of the files under `backend/data/seed/`. Must match
/// `DATA_VERSION` in `tools/seed-data/build.py`; bumping it also requires
/// bumping [`super::SEED_GENERATOR_VERSION`].
pub(crate) const DATA_VERSION: u32 = 2;

macro_rules! names {
    ($locale:literal) => {
        (
            include_str!(concat!("../../data/seed/names/", $locale, ".given.tsv")),
            include_str!(concat!("../../data/seed/names/", $locale, ".family.tsv")),
        )
    };
}

/// A locale the built-in name data covers. Public as
/// `backend::table_seed::TableSeedLocale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
    EnUs,
    EnGb,
    EnIn,
    De,
    Fr,
    Es,
    PtBr,
    Ja,
}

impl Locale {
    pub const ALL: [Locale; 8] = [
        Locale::EnUs,
        Locale::EnGb,
        Locale::EnIn,
        Locale::De,
        Locale::Fr,
        Locale::Es,
        Locale::PtBr,
        Locale::Ja,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Locale::EnUs => "en-US",
            Locale::EnGb => "en-GB",
            Locale::EnIn => "en-IN",
            Locale::De => "de",
            Locale::Fr => "fr",
            Locale::Es => "es",
            Locale::PtBr => "pt-BR",
            Locale::Ja => "ja",
        }
    }

    /// The locale's `(given, family)` name files.
    fn files(self) -> (&'static str, &'static str) {
        match self {
            Locale::EnUs => names!("en-US"),
            Locale::EnGb => names!("en-GB"),
            Locale::EnIn => names!("en-IN"),
            Locale::De => names!("de"),
            Locale::Fr => names!("fr"),
            Locale::Es => names!("es"),
            Locale::PtBr => names!("pt-BR"),
            Locale::Ja => names!("ja"),
        }
    }

    /// Japanese writes the family name first and, in kanji, without a space.
    pub(crate) fn family_first(self) -> bool {
        self == Locale::Ja
    }

    pub(crate) fn name_separator(self) -> &'static str {
        if self == Locale::Ja {
            ""
        } else {
            " "
        }
    }

    /// Chance that a family name is a compound of two, and how the two are
    /// joined: Spanish and Brazilian naming carries both parents' names.
    pub(crate) fn compound_family(self) -> (f64, &'static str) {
        match self {
            Locale::Es => (0.6, " "),
            Locale::PtBr => (0.45, " "),
            Locale::Ja => (0.0, ""),
            _ => (0.03, "-"),
        }
    }

    /// Chance that a full name carries a middle name.
    pub(crate) fn middle_name_rate(self) -> f64 {
        match self {
            Locale::EnUs | Locale::EnGb => 0.25,
            Locale::De | Locale::Fr | Locale::PtBr | Locale::Es => 0.1,
            Locale::EnIn => 0.05,
            Locale::Ja => 0.0,
        }
    }

    /// ISO 3166-1 alpha-2 country the locale's places and phones belong to.
    pub(crate) fn country(self) -> &'static str {
        match self {
            Locale::EnUs => "US",
            Locale::EnGb => "GB",
            Locale::EnIn => "IN",
            Locale::De => "DE",
            Locale::Fr => "FR",
            Locale::Es => "ES",
            Locale::PtBr => "BR",
            Locale::Ja => "JP",
        }
    }

    /// The country's official English name.
    pub(crate) fn official_name(self) -> &'static str {
        match self {
            Locale::EnUs => "United States of America",
            Locale::EnGb => "United Kingdom of Great Britain and Northern Ireland",
            Locale::EnIn => "Republic of India",
            Locale::De => "Federal Republic of Germany",
            Locale::Fr => "French Republic",
            Locale::Es => "Kingdom of Spain",
            Locale::PtBr => "Federative Republic of Brazil",
            Locale::Ja => "Japan",
        }
    }

    /// ISO 3166-1 alpha-3 code of [`Locale::country`].
    pub(crate) fn country_alpha3(self) -> &'static str {
        match self {
            Locale::EnUs => "USA",
            Locale::EnGb => "GBR",
            Locale::EnIn => "IND",
            Locale::De => "DEU",
            Locale::Fr => "FRA",
            Locale::Es => "ESP",
            Locale::PtBr => "BRA",
            Locale::Ja => "JPN",
        }
    }

    fn places_file(self) -> &'static str {
        match self {
            Locale::EnUs => include_str!("../../data/seed/places/en-US.tsv"),
            Locale::EnGb => include_str!("../../data/seed/places/en-GB.tsv"),
            Locale::EnIn => include_str!("../../data/seed/places/en-IN.tsv"),
            Locale::De => include_str!("../../data/seed/places/de.tsv"),
            Locale::Fr => include_str!("../../data/seed/places/fr.tsv"),
            Locale::Es => include_str!("../../data/seed/places/es.tsv"),
            Locale::PtBr => include_str!("../../data/seed/places/pt-BR.tsv"),
            Locale::Ja => include_str!("../../data/seed/places/ja.tsv"),
        }
    }

    /// Mail providers used only when realistic values are opted into.
    pub(crate) fn mail_providers(self) -> &'static [&'static str] {
        match self {
            Locale::EnUs => &[
                "gmail.com",
                "yahoo.com",
                "outlook.com",
                "icloud.com",
                "aol.com",
            ],
            Locale::EnGb => &[
                "gmail.com",
                "outlook.com",
                "yahoo.co.uk",
                "btinternet.com",
                "icloud.com",
            ],
            Locale::EnIn => &["gmail.com", "yahoo.co.in", "rediffmail.com", "outlook.com"],
            Locale::De => &["gmail.com", "web.de", "gmx.de", "t-online.de", "outlook.de"],
            Locale::Fr => &[
                "gmail.com",
                "orange.fr",
                "free.fr",
                "laposte.net",
                "outlook.fr",
            ],
            Locale::Es => &[
                "gmail.com",
                "hotmail.es",
                "yahoo.es",
                "outlook.es",
                "telefonica.net",
            ],
            Locale::PtBr => &[
                "gmail.com",
                "hotmail.com",
                "uol.com.br",
                "bol.com.br",
                "outlook.com",
            ],
            Locale::Ja => &[
                "gmail.com",
                "yahoo.co.jp",
                "docomo.ne.jp",
                "icloud.com",
                "outlook.jp",
            ],
        }
    }
}

/// One name or word with its precomputed lengths.
#[derive(Debug)]
pub(crate) struct Entry {
    pub text: &'static str,
    /// ASCII form for emails and usernames: diacritics folded, or the
    /// romaji reading for Japanese.
    pub ascii: &'static str,
    pub chars: usize,
}

/// Entries of one character length with cumulative draw weights.
#[derive(Debug, Default)]
struct Bucket {
    entries: Vec<usize>,
    cumulative: Vec<u64>,
}

impl Bucket {
    fn total(&self) -> u64 {
        self.cumulative.last().copied().unwrap_or(0)
    }
}

/// A weighted list indexed by character length.
#[derive(Debug, Default)]
pub(crate) struct WeightedList {
    entries: Vec<Entry>,
    /// `buckets[n]` holds the entries `n` characters long.
    buckets: Vec<Bucket>,
}

impl WeightedList {
    fn push(&mut self, text: &'static str, ascii: &'static str, count: u64) {
        let chars = text.chars().count();
        if chars == 0 {
            return;
        }
        // Square-root damping keeps common names common without letting a
        // handful of them crowd out the long tail.
        let weight = ((count.max(1) as f64).sqrt().ceil() as u64).max(1);
        if self.buckets.len() <= chars {
            self.buckets.resize_with(chars + 1, Bucket::default);
        }
        let bucket = &mut self.buckets[chars];
        let total = bucket.total();
        bucket.entries.push(self.entries.len());
        bucket.cumulative.push(total + weight);
        self.entries.push(Entry { text, ascii, chars });
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn min_chars(&self) -> usize {
        self.buckets
            .iter()
            .position(|b| !b.entries.is_empty())
            .unwrap_or(0)
    }

    pub(crate) fn max_chars(&self) -> usize {
        self.buckets.len().saturating_sub(1)
    }

    pub(crate) fn has_length(&self, chars: usize) -> bool {
        self.buckets
            .get(chars)
            .is_some_and(|b| !b.entries.is_empty())
    }

    /// Weighted draw over every entry.
    pub(crate) fn pick(&self, rng: &mut SeedRng) -> &Entry {
        self.pick_within(rng, 0, usize::MAX)
            .expect("seed data lists are never empty")
    }

    /// Weighted draw among entries `lo..=hi` characters long.
    pub(crate) fn pick_within(&self, rng: &mut SeedRng, lo: usize, hi: usize) -> Option<&Entry> {
        let hi = hi.min(self.max_chars());
        if lo > hi {
            return None;
        }
        let total: u64 = self.buckets[lo..=hi].iter().map(Bucket::total).sum();
        if total == 0 {
            return None;
        }
        let mut target = rng.next_u64() % total;
        for bucket in &self.buckets[lo..=hi] {
            if target < bucket.total() {
                let slot = bucket.cumulative.partition_point(|&c| c <= target);
                return Some(&self.entries[bucket.entries[slot]]);
            }
            target -= bucket.total();
        }
        None
    }
}

pub(crate) struct LocaleNames {
    pub male: WeightedList,
    pub female: WeightedList,
    /// Both genders, for columns that hold one given name on its own.
    pub given: WeightedList,
    pub family: WeightedList,
}

pub(crate) struct Text {
    pub sentences: Vec<&'static str>,
    pub words: WeightedList,
}

const SENTENCES: &str = include_str!("../../data/seed/text/en.sentences.txt");
const WORDS: &str = include_str!("../../data/seed/text/en.words.tsv");

fn parse_names(given: &'static str, family: &'static str) -> Result<LocaleNames, String> {
    let mut names = LocaleNames {
        male: WeightedList::default(),
        female: WeightedList::default(),
        given: WeightedList::default(),
        family: WeightedList::default(),
    };
    for line in given.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let [text, ascii, count, gender] = fields[..] else {
            return Err(format!("given-name line needs 4 fields: {line:?}"));
        };
        let count = parse_count(count, line)?;
        match gender {
            "f" => names.female.push(text, ascii, count),
            "m" => names.male.push(text, ascii, count),
            _ => return Err(format!("unknown gender {gender:?}: {line:?}")),
        }
        names.given.push(text, ascii, count);
    }
    for line in family.lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let [text, ascii, count] = fields[..] else {
            return Err(format!("family-name line needs 3 fields: {line:?}"));
        };
        names.family.push(text, ascii, parse_count(count, line)?);
    }
    Ok(names)
}

fn parse_count(count: &str, line: &str) -> Result<u64, String> {
    count
        .parse()
        .map_err(|_| format!("weight {count:?} is not a count: {line:?}"))
}

fn parse_words(words: &'static str) -> Result<WeightedList, String> {
    let mut list = WeightedList::default();
    for line in words.lines() {
        let Some((word, count)) = line.split_once('\t') else {
            return Err(format!("word line needs 2 fields: {line:?}"));
        };
        list.push(word, word, parse_count(count, line)?);
    }
    Ok(list)
}

/// The data is compiled in and every file is parsed by the tests below,
/// so a parse failure here is a build defect, not a runtime condition.
const VALIDATED: &str = "built-in seed data is validated by data::tests";

pub(crate) fn names(locale: Locale) -> &'static LocaleNames {
    static NAMES: OnceLock<Vec<(Locale, LocaleNames)>> = OnceLock::new();
    let all = NAMES.get_or_init(|| {
        Locale::ALL
            .iter()
            .map(|&l| {
                let (given, family) = l.files();
                (l, parse_names(given, family).expect(VALIDATED))
            })
            .collect()
    });
    &all.iter()
        .find(|(l, _)| *l == locale)
        .expect("every locale is loaded")
        .1
}

/// One postcode with the locality and region it belongs to.
#[derive(Debug)]
pub(crate) struct PlaceEntry {
    pub postal: &'static str,
    pub locality: &'static str,
    pub region: &'static str,
    /// State or UF abbreviation where addresses use one (US, BR).
    pub region_code: &'static str,
}

/// A locale's places plus length-indexed views for windowed draws.
pub(crate) struct Places {
    pub entries: Vec<PlaceEntry>,
    pub localities: WeightedList,
    pub regions: WeightedList,
    pub postcodes: WeightedList,
    /// English country name.
    pub country_name: &'static str,
    /// libaddressinput format: `%A` street, `%C` locality, `%S` region,
    /// `%Z` postcode, `%n` line break.
    pub address_format: &'static str,
    /// libaddressinput postcode pattern.
    pub postcode_pattern: &'static str,
}

const COUNTRIES: &str = include_str!("../../data/seed/places/countries.tsv");

fn parse_places(locale: Locale) -> Result<Places, String> {
    let country = COUNTRIES
        .lines()
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|fields| fields.first() == Some(&locale.id()))
        .ok_or_else(|| format!("no country record for {}", locale.id()))?;
    let [_, iso, name, format, pattern] = country[..] else {
        return Err(format!("country line needs 5 fields: {country:?}"));
    };
    if iso != locale.country() {
        return Err(format!(
            "{} is {iso}, expected {}",
            locale.id(),
            locale.country()
        ));
    }
    let mut places = Places {
        entries: Vec::new(),
        localities: WeightedList::default(),
        regions: WeightedList::default(),
        postcodes: WeightedList::default(),
        country_name: name,
        address_format: format,
        postcode_pattern: pattern,
    };
    for line in locale.places_file().lines() {
        let fields: Vec<&str> = line.split('\t').collect();
        let [postal, locality, region, region_code] = fields[..] else {
            return Err(format!("place line needs 4 fields: {line:?}"));
        };
        if postal.is_empty() || locality.is_empty() || region.is_empty() {
            return Err(format!("place line has an empty field: {line:?}"));
        }
        places.localities.push(locality, locality, 1);
        places.regions.push(region, region, 1);
        places.postcodes.push(postal, postal, 1);
        places.entries.push(PlaceEntry {
            postal,
            locality,
            region,
            region_code,
        });
    }
    if places.entries.is_empty() {
        return Err(format!("no places for {}", locale.id()));
    }
    Ok(places)
}

pub(crate) fn places(locale: Locale) -> &'static Places {
    static PLACES: OnceLock<Vec<(Locale, Places)>> = OnceLock::new();
    let all = PLACES.get_or_init(|| {
        Locale::ALL
            .iter()
            .map(|&l| (l, parse_places(l).expect(VALIDATED)))
            .collect()
    });
    &all.iter()
        .find(|(l, _)| *l == locale)
        .expect("every locale is loaded")
        .1
}

pub(crate) fn text() -> &'static Text {
    static TEXT: OnceLock<Text> = OnceLock::new();
    TEXT.get_or_init(|| Text {
        sentences: SENTENCES.lines().filter(|l| !l.is_empty()).collect(),
        words: parse_words(WORDS).expect(VALIDATED),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_data_file_parses_strictly() {
        for locale in Locale::ALL {
            let (given, family) = locale.files();
            parse_names(given, family).unwrap_or_else(|e| panic!("{locale:?}: {e}"));
        }
        parse_words(WORDS).expect("words");
        for locale in Locale::ALL {
            let places = parse_places(locale).unwrap_or_else(|e| panic!("{locale:?}: {e}"));
            assert!(places.entries.len() > 1000, "{locale:?}");
        }
        assert!(parse_names("Ann\tAnn\t3\tu\n", "").is_err());
        assert!(parse_names("Ann\tAnn\tmany\tf\n", "").is_err());
        assert!(parse_names("", "Smith\tSmith\n").is_err());
    }

    #[test]
    fn every_locale_has_names_of_both_genders_and_families() {
        for locale in Locale::ALL {
            let names = names(locale);
            assert!(names.male.len() > 100, "{locale:?} male");
            assert!(names.female.len() > 100, "{locale:?} female");
            assert!(names.family.len() > 100, "{locale:?} family");
            assert_eq!(names.given.len(), names.male.len() + names.female.len());
            for entry in names.family.entries.iter().chain(&names.given.entries) {
                assert!(
                    entry.ascii.is_ascii() && !entry.ascii.is_empty(),
                    "{locale:?} {}",
                    entry.text
                );
            }
        }
    }

    #[test]
    fn length_windows_only_return_entries_inside_them() {
        let family = &names(Locale::De).family;
        let mut rng = SeedRng::new(3);
        for _ in 0..500 {
            let entry = family.pick_within(&mut rng, 4, 6).expect("entry");
            assert!((4..=6).contains(&entry.chars), "{}", entry.text);
        }
        assert!(family.pick_within(&mut rng, 200, 300).is_none());
    }

    #[test]
    fn japanese_names_are_kanji_with_romaji() {
        let family = &names(Locale::Ja).family;
        let entry = family.pick(&mut SeedRng::new(1));
        assert!(!entry.text.is_ascii());
        assert!(entry
            .ascii
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '\'' || c == '-' || c == ' '));
    }

    #[test]
    fn text_corpus_is_loaded() {
        let text = text();
        assert!(text.sentences.len() > 1000);
        assert!(text.words.len() > 1000);
        assert!(text.sentences.iter().all(|s| s.is_ascii()));
    }
}
