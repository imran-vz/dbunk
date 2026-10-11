//! Length-targeted text generators (ADR-0014, "Seed data variety").
//!
//! Every builder takes a [`LengthPlan`] and returns a value whose
//! character count lies inside it. In natural mode a value is drawn as
//! it would occur and only kept inside the window; in uniform mode a
//! target length is drawn first and the value is built to exactly that
//! length. Nothing is truncated to fit a column after the fact.

use super::data::{self, Entry, Locale, WeightedList};
use super::{truncate_chars as cut, SeedRng};
use serde::{Deserialize, Serialize};

/// Largest generated text value, in characters. The public request limit
/// (`MAX_TABLE_SEED_TEXT_CHARS`) is derived from this.
pub(crate) const MAX_GENERATED_CHARS: usize = 100_000;

/// Inclusive character-length window for one column's values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LengthPlan {
    pub lo: usize,
    pub hi: usize,
    /// Spread lengths evenly over the window (the user chose a range)
    /// instead of following the data's own length distribution.
    pub uniform: bool,
}

impl LengthPlan {
    pub(crate) fn natural(lo: usize, hi: usize) -> Self {
        Self {
            lo,
            hi,
            uniform: false,
        }
    }

    fn contains(&self, chars: usize) -> bool {
        (self.lo..=self.hi).contains(&chars)
    }

    fn target(&self, rng: &mut SeedRng) -> Option<usize> {
        self.uniform
            .then(|| rng.range_i64(self.lo as i64, self.hi as i64) as usize)
    }

    /// The window left for a value once `reserved` characters (a unique
    /// tag) are appended to it.
    pub(crate) fn reserve(&self, reserved: usize, floor: usize) -> Self {
        Self {
            lo: self.lo.saturating_sub(reserved).max(floor),
            hi: self.hi.saturating_sub(reserved).max(floor),
            uniform: self.uniform,
        }
    }
}

/// Whether generated values may look like real, assigned contact details.
/// Public as `backend::table_seed::TableSeedRealism`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Realism {
    /// Reserved example domains and fiction-only number ranges.
    #[default]
    FictionalSafe,
    Realistic,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GenOptions {
    /// Never empty. Each row draws one of these.
    pub locales: Vec<Locale>,
    pub realism: Realism,
}

impl Default for GenOptions {
    fn default() -> Self {
        Self {
            locales: vec![Locale::EnUs],
            realism: Realism::FictionalSafe,
        }
    }
}

/// One made-up person per row, so a row's name, email and username
/// columns agree with each other when their length windows allow.
pub(crate) struct Persona {
    pub locale: Locale,
    pub female: bool,
    pub given: &'static Entry,
    pub middle: &'static Entry,
    pub family: &'static Entry,
    pub second_family: Option<&'static Entry>,
    pub middle_in_full_name: bool,
}

impl Persona {
    pub(crate) fn draw(rng: &mut SeedRng, options: &GenOptions) -> Self {
        let locale = *rng.pick(&options.locales);
        let names = data::names(locale);
        let female = rng.next_u64().is_multiple_of(2);
        let given_list = if female { &names.female } else { &names.male };
        let given = given_list.pick(rng);
        let middle = given_list.pick(rng);
        let family = names.family.pick(rng);
        let (rate, _) = locale.compound_family();
        let second_family = (rng.next_f64() < rate).then(|| names.family.pick(rng));
        let middle_in_full_name = rng.next_f64() < locale.middle_name_rate();
        Self {
            locale,
            female,
            given,
            middle,
            family,
            second_family,
            middle_in_full_name,
        }
    }

    fn given_list(&self) -> &'static WeightedList {
        let names = data::names(self.locale);
        if self.female {
            &names.female
        } else {
            &names.male
        }
    }

    fn family_text(&self) -> String {
        match self.second_family {
            Some(second) => {
                let (_, joiner) = self.locale.compound_family();
                format!("{}{joiner}{}", self.family.text, second.text)
            }
            None => self.family.text.to_string(),
        }
    }

    fn full_name(&self) -> String {
        let sep = self.locale.name_separator();
        let family = self.family_text();
        if self.locale.family_first() {
            format!("{family}{sep}{}", self.given.text)
        } else if self.middle_in_full_name && self.middle.text != self.given.text {
            format!("{}{sep}{}{sep}{family}", self.given.text, self.middle.text)
        } else {
            format!("{}{sep}{family}", self.given.text)
        }
    }

    /// Lowercase ASCII tokens for addresses and handles.
    fn tokens(&self) -> (String, String) {
        let family = match self.second_family {
            Some(second) => format!("{}{}", token(self.family.ascii), token(second.ascii)),
            None => token(self.family.ascii),
        };
        (token(self.given.ascii), family)
    }
}

fn token(ascii: &str) -> String {
    ascii
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn chars(value: &str) -> usize {
    value.chars().count()
}

/// Build a string of exactly `target` characters from entries of `list`
/// joined by `joiner`: "Anna-Lena", "harbour.lantern", a double surname.
pub(crate) fn compose(
    list: &WeightedList,
    rng: &mut SeedRng,
    target: usize,
    joiner: &str,
) -> String {
    let joiner_chars = chars(joiner);
    let min = list.min_chars().max(1);
    let mut out = String::new();
    let mut len = 0;
    loop {
        let gap = if out.is_empty() { 0 } else { joiner_chars };
        let remaining = target.saturating_sub(len + gap);
        if remaining == 0 {
            return cut(&out, target);
        }
        if list.has_length(remaining) {
            let entry = list
                .pick_within(rng, remaining, remaining)
                .expect("bucket exists");
            out.push_str(if gap > 0 { joiner } else { "" });
            out.push_str(entry.text);
            return out;
        }
        // Leave room for a joiner and a final entry of at least `min`.
        let room = remaining.saturating_sub(joiner_chars + min);
        match (room >= min)
            .then(|| list.pick_within(rng, min, room))
            .flatten()
        {
            Some(entry) => {
                out.push_str(if gap > 0 { joiner } else { "" });
                out.push_str(entry.text);
                len += gap + entry.chars;
            }
            None => {
                // No exact fit: shorten a longer entry to the remainder.
                let entry = list
                    .pick_within(rng, remaining, usize::MAX)
                    .unwrap_or_else(|| list.pick(rng));
                out.push_str(if gap > 0 { joiner } else { "" });
                out.push_str(&cut(entry.text, remaining));
                return cut(&out, target);
            }
        }
    }
}

/// A single name or word inside the plan's window, preferring `preferred`
/// (the row persona's value) when it fits.
fn fit_part(
    list: &WeightedList,
    rng: &mut SeedRng,
    plan: LengthPlan,
    joiner: &str,
    preferred: Option<&Entry>,
) -> String {
    match plan.target(rng) {
        None => {
            if let Some(entry) = preferred.filter(|e| plan.contains(e.chars)) {
                return entry.text.to_string();
            }
            if let Some(entry) = list.pick_within(rng, plan.lo, plan.hi) {
                return entry.text.to_string();
            }
            let hi = plan.hi.min(plan.lo + 12);
            let target = rng.range_i64(plan.lo as i64, hi as i64) as usize;
            compose(list, rng, target, joiner)
        }
        Some(target) => {
            if let Some(entry) = preferred.filter(|e| e.chars == target) {
                return entry.text.to_string();
            }
            compose(list, rng, target, joiner)
        }
    }
}

pub(crate) fn first_name(rng: &mut SeedRng, persona: &Persona, plan: LengthPlan) -> String {
    fit_part(persona.given_list(), rng, plan, "-", Some(persona.given))
}

pub(crate) fn middle_name(rng: &mut SeedRng, persona: &Persona, plan: LengthPlan) -> String {
    fit_part(persona.given_list(), rng, plan, " ", Some(persona.middle))
}

pub(crate) fn last_name(rng: &mut SeedRng, persona: &Persona, plan: LengthPlan) -> String {
    let natural = persona.family_text();
    if !plan.uniform && plan.contains(chars(&natural)) {
        return natural;
    }
    let (_, joiner) = persona.locale.compound_family();
    let joiner = if joiner.is_empty() { "-" } else { joiner };
    fit_part(
        &data::names(persona.locale).family,
        rng,
        plan,
        joiner,
        Some(persona.family),
    )
}

pub(crate) fn full_name(rng: &mut SeedRng, persona: &Persona, plan: LengthPlan) -> String {
    let natural = persona.full_name();
    let target = match plan.target(rng) {
        None if plan.contains(chars(&natural)) => return natural,
        None => chars(&natural).clamp(plan.lo, plan.hi),
        Some(target) => target,
    };
    if chars(&natural) == target {
        return natural;
    }
    let names = data::names(persona.locale);
    let sep = persona.locale.name_separator();
    let sep_chars = chars(sep);
    let order = |given: &str, middle: &str, family: &str| {
        if persona.locale.family_first() {
            format!("{family}{sep}{given}{middle}")
        } else if middle.is_empty() {
            format!("{given}{sep}{family}")
        } else {
            format!("{given}{sep}{middle}{sep}{family}")
        }
    };
    // Find a given and family name that, with separators, leave room for
    // a middle part of exactly the remaining length (or none at all).
    for _ in 0..32 {
        let family = names.family.pick(rng);
        let given = persona.given_list().pick(rng);
        let base = given.chars + sep_chars + family.chars;
        if base == target {
            return order(given.text, "", family.text);
        }
        if persona.locale.family_first() {
            continue;
        }
        if base + sep_chars < target {
            let middle_len = target - base - sep_chars;
            let middle = match middle_len {
                1 => initial(persona.middle),
                2 => format!("{}.", initial(persona.middle)),
                n => compose(persona.given_list(), rng, n, " "),
            };
            return order(given.text, &middle, family.text);
        }
    }
    // Too short for two names and a separator: one given name.
    if target < 2 + sep_chars {
        return compose(persona.given_list(), rng, target, "-");
    }
    // Short or Japanese targets: a family name and given name built to
    // split the target exactly between them.
    let family_len = (target - sep_chars) / 2;
    let given_len = target - sep_chars - family_len;
    let family = compose(&names.family, rng, family_len, "-");
    let given = compose(persona.given_list(), rng, given_len, "-");
    order(&given, "", &family)
}

fn initial(entry: &Entry) -> String {
    entry
        .text
        .chars()
        .next()
        .map(String::from)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Email and usernames
// ---------------------------------------------------------------------------

/// RFC 2606 / RFC 6761 names that can never reach a real mailbox.
const FICTIONAL_DOMAINS: &[&str] = &["example.com", "example.org", "example.net"];
const FICTIONAL_TLDS: &[&str] = &[".test", ".example"];
const REALISTIC_TLDS: &[&str] = &[".com", ".net", ".org", ".io"];

/// Shortest email the generator can build: `a@b.test`.
pub(crate) const MIN_EMAIL_CHARS: usize = 8;
pub(crate) const MAX_EMAIL_CHARS: usize = 254;
const MAX_LOCAL_CHARS: usize = 64;
const MAX_LABEL_CHARS: usize = 63;

fn natural_domain(rng: &mut SeedRng, realism: Realism, locale: Locale) -> String {
    let words = &data::text().words;
    match realism {
        Realism::FictionalSafe => {
            if rng.next_f64() < 0.5 {
                (*rng.pick(FICTIONAL_DOMAINS)).to_string()
            } else {
                let word = words.pick_within(rng, 4, 10).map_or("mail", |e| e.text);
                format!("{word}{}", rng.pick(FICTIONAL_TLDS))
            }
        }
        Realism::Realistic => {
            if rng.next_f64() < 0.7 {
                (*rng.pick(locale.mail_providers())).to_string()
            } else {
                let word = words.pick_within(rng, 4, 10).map_or("mail", |e| e.text);
                format!("{word}{}", rng.pick(REALISTIC_TLDS))
            }
        }
    }
}

/// A domain of exactly `target` characters (at least 6).
fn domain_of_length(rng: &mut SeedRng, realism: Realism, target: usize) -> String {
    let words = &data::text().words;
    let tld = match realism {
        Realism::FictionalSafe => ".test",
        Realism::Realistic => *rng.pick(REALISTIC_TLDS),
    };
    let mut remaining = target.saturating_sub(tld.len()).max(1);
    let mut labels = Vec::new();
    while remaining > MAX_LABEL_CHARS {
        let len = rng.range_i64(20, MAX_LABEL_CHARS.min(remaining - 2) as i64) as usize;
        labels.push(compose(words, rng, len, "-"));
        remaining -= len + 1;
    }
    labels.push(compose(words, rng, remaining, "-"));
    format!("{}{tld}", labels.join("."))
}

/// Local-part shapes people actually use, from the persona's names.
fn local_patterns(given: &str, family: &str, middle: &str) -> Vec<String> {
    let g0 = &given[..given.len().min(1)];
    let f0 = &family[..family.len().min(1)];
    let m0 = &middle[..middle.len().min(1)];
    let mut patterns: Vec<String> = vec![
        format!("{given}.{family}"),
        format!("{given}{family}"),
        format!("{given}_{family}"),
        format!("{g0}{family}"),
        format!("{g0}.{family}"),
        format!("{family}.{given}"),
        format!("{given}.{m0}.{family}"),
        format!("{given}{f0}"),
        given.to_string(),
        family.to_string(),
    ]
    .into_iter()
    .filter(|p| {
        !p.is_empty() && !p.starts_with(['.', '_']) && !p.ends_with(['.', '_']) && !p.contains("..")
    })
    .collect();
    if patterns.is_empty() {
        patterns.push("user".to_string());
    }
    patterns
}

/// A local part of exactly `target` characters (1 to 64).
fn local_of_length(rng: &mut SeedRng, patterns: &[String], target: usize, sep: &str) -> String {
    let fitting: Vec<&String> = patterns.iter().filter(|p| p.len() <= target).collect();
    let mut local = match fitting.is_empty() {
        true => cut(&patterns[0].replace(['.', '_'], ""), target),
        false => (*rng.pick(&fitting)).clone(),
    };
    let remaining = target - local.len();
    if remaining == 0 {
        return local;
    }
    // Short gaps take digits; a separator and a word need more room.
    if remaining <= 4 || (remaining <= 6 && rng.next_f64() < 0.3) {
        for _ in 0..remaining {
            local.push(char::from(b'0' + rng.range_i64(0, 9) as u8));
        }
        return local;
    }
    local.push_str(sep);
    local.push_str(&compose(
        &data::text().words,
        rng,
        remaining - sep.len(),
        sep,
    ));
    local
}

fn natural_local(rng: &mut SeedRng, patterns: &[String]) -> String {
    let mut local = rng.pick(patterns).clone();
    let roll = rng.next_f64();
    if roll < 0.25 {
        local.push_str(&rng.range_i64(1, 99).to_string());
    } else if roll < 0.35 {
        local.push_str(&rng.range_i64(1960, 2008).to_string());
    }
    local
}

/// An email address inside the plan's window. `tag` (`.17`) is appended
/// to the local part so unique columns stay unique.
pub(crate) fn email(
    rng: &mut SeedRng,
    persona: &Persona,
    realism: Realism,
    plan: LengthPlan,
    tag: &str,
) -> String {
    // The shortest address grows by the tag; check_capacity keeps `hi`
    // at or above that floor.
    let plan = LengthPlan {
        lo: plan.lo.max(MIN_EMAIL_CHARS + tag.len()).min(plan.hi),
        ..plan
    };
    let (given, family) = persona.tokens();
    let middle = token(persona.middle.ascii);
    let patterns = local_patterns(&given, &family, &middle);
    let domain = natural_domain(rng, realism, persona.locale);
    let target = match plan.target(rng) {
        Some(target) => target,
        None => {
            let local = natural_local(rng, &patterns);
            let value = format!("{local}{tag}@{domain}");
            if plan.contains(value.len()) && local.len() + tag.len() <= MAX_LOCAL_CHARS {
                return value;
            }
            value.len().clamp(plan.lo, plan.hi)
        }
    };
    // Split the target between the local part and the domain.
    let mut local_len = target.saturating_sub(1 + domain.len());
    let domain = if (1 + tag.len()..=MAX_LOCAL_CHARS).contains(&local_len) {
        domain
    } else {
        local_len = local_len.clamp(1 + tag.len(), MAX_LOCAL_CHARS);
        let mut domain_len = target.saturating_sub(1 + local_len);
        if domain_len < 6 {
            local_len = local_len.saturating_sub(6 - domain_len).max(1 + tag.len());
            domain_len = target.saturating_sub(1 + local_len);
        }
        domain_of_length(rng, realism, domain_len)
    };
    let local = local_of_length(rng, &patterns, local_len - tag.len(), ".");
    format!("{local}{tag}@{domain}")
}

/// A username inside the plan's window, ending with `tag` when unique.
pub(crate) fn user_name(
    rng: &mut SeedRng,
    persona: &Persona,
    plan: LengthPlan,
    tag: &str,
) -> String {
    let (given, family) = persona.tokens();
    let words = &data::text().words;
    let word = words.pick_within(rng, 3, 8).map_or("dev", |e| e.text);
    let patterns: Vec<String> = [
        format!("{given}{family}"),
        format!("{given}_{family}"),
        format!("{given}{}", &family[..family.len().min(1)]),
        format!("{}{family}", &given[..given.len().min(1)]),
        format!("{given}_{word}"),
        format!("{word}_{given}"),
        given.clone(),
    ]
    .into_iter()
    .filter(|p| !p.is_empty() && !p.starts_with('_') && !p.ends_with('_'))
    .chain(std::iter::once(word.to_string()))
    .collect();
    let plan = plan.reserve(tag.len(), 1);
    let base = match plan.target(rng) {
        None => {
            let mut value = rng.pick(&patterns).clone();
            if rng.next_f64() < 0.4 {
                value.push_str(&rng.range_i64(1, 999).to_string());
            }
            if plan.contains(value.len()) {
                value
            } else {
                let target = value.len().clamp(plan.lo, plan.hi);
                local_of_length(rng, &patterns, target, "_")
            }
        }
        Some(target) => local_of_length(rng, &patterns, target, "_"),
    };
    format!("{base}{tag}")
}

// ---------------------------------------------------------------------------
// Words and prose
// ---------------------------------------------------------------------------

pub(crate) fn word(rng: &mut SeedRng, plan: LengthPlan) -> String {
    fit_part(&data::text().words, rng, plan, "-", None)
}

/// Below this many characters prose is built from words, not sentences.
const SHORT_PROSE_CHARS: usize = 24;

/// Sentences of English prose inside the plan's window. `sentences` is
/// the natural count range; long targets are broken into paragraphs.
pub(crate) fn prose(rng: &mut SeedRng, plan: LengthPlan, sentences: (usize, usize)) -> String {
    let corpus = &data::text().sentences;
    let target = match plan.target(rng) {
        Some(target) => target,
        None => {
            let count = rng.range_i64(sentences.0 as i64, sentences.1 as i64) as usize;
            let value = (0..count)
                .map(|_| *rng.pick(corpus))
                .collect::<Vec<_>>()
                .join(" ");
            if plan.contains(value.len()) {
                return value;
            }
            value.len().clamp(plan.lo, plan.hi)
        }
    };
    // Too short for a sentence: whole words read better than a cut one.
    if target < SHORT_PROSE_CHARS {
        let words = compose(&data::text().words, rng, target, " ");
        let mut chars = words.chars();
        return match chars.next() {
            Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
            None => words,
        };
    }
    let mut value = String::with_capacity(target + 256);
    let mut in_paragraph = 0usize;
    let mut paragraph_len = rng.range_i64(3, 7) as usize;
    while value.len() < target {
        if !value.is_empty() {
            if target > 600 && in_paragraph >= paragraph_len {
                value.push_str("\n\n");
                in_paragraph = 0;
                paragraph_len = rng.range_i64(3, 7) as usize;
            } else {
                value.push(' ');
            }
        }
        value.push_str(rng.pick(corpus));
        in_paragraph += 1;
    }
    cut_at_word(&value, plan.lo, target)
}

/// Shorten ASCII prose to at most `target` characters (and at least `lo`),
/// ending on a word boundary when one is in range. The result never ends
/// in whitespace, so a unique tag or the column value stays clean.
fn cut_at_word(value: &str, lo: usize, target: usize) -> String {
    if value.len() <= target {
        return value.trim_end().to_string();
    }
    let lo = lo.max(1);
    let prefix = &value[..target];
    let boundary = if value.as_bytes()[target].is_ascii_whitespace() {
        Some(target)
    } else {
        prefix.rfind(char::is_whitespace)
    };
    if let Some(end) = boundary {
        let mut out = value[..end]
            .trim_end_matches(|c: char| c.is_whitespace() || ",;:".contains(c))
            .to_string();
        if out.len() >= lo {
            if out.len() < target && !out.ends_with(['.', '!', '?']) {
                out.push('.');
            }
            return out;
        }
    }
    // No boundary in range: cut mid-word, and if the cut lands after
    // whitespace keep the exact length with stops instead.
    let mut out = prefix.trim_end().to_string();
    while out.len() < lo {
        out.push('.');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn persona(seed: u64, locale: Locale) -> Persona {
        Persona::draw(
            &mut SeedRng::new(seed),
            &GenOptions {
                locales: vec![locale],
                realism: Realism::FictionalSafe,
            },
        )
    }

    fn uniform(lo: usize, hi: usize) -> LengthPlan {
        LengthPlan {
            lo,
            hi,
            uniform: true,
        }
    }

    #[test]
    fn compose_hits_every_target_exactly() {
        let list = &data::names(Locale::EnUs).family;
        let mut rng = SeedRng::new(9);
        for target in 1..=300 {
            let value = compose(list, &mut rng, target, "-");
            assert_eq!(chars(&value), target, "{value}");
            assert!(!value.starts_with('-') && !value.ends_with('-'), "{value}");
        }
    }

    #[test]
    fn name_builders_respect_uniform_windows_in_every_locale() {
        for locale in Locale::ALL {
            for seed in 0..200 {
                let p = persona(seed, locale);
                let mut rng = SeedRng::new(seed);
                for (lo, hi) in [(1, 3), (2, 12), (10, 50), (40, 255)] {
                    let plan = uniform(lo, hi);
                    for value in [
                        first_name(&mut rng, &p, plan),
                        middle_name(&mut rng, &p, plan),
                        last_name(&mut rng, &p, plan),
                    ] {
                        assert!(
                            plan.contains(chars(&value)),
                            "{locale:?} {lo}..{hi}: {value}"
                        );
                    }
                    let value = full_name(&mut rng, &p, plan);
                    assert!(
                        plan.contains(chars(&value)),
                        "{locale:?} {lo}..{hi}: {value}"
                    );
                    assert_eq!(value.trim(), value, "{locale:?} {lo}..{hi}: {value:?}");
                }
            }
        }
    }

    #[test]
    fn uniform_windows_spread_lengths_across_the_range() {
        let p = persona(1, Locale::EnUs);
        let mut rng = SeedRng::new(4);
        let lengths: std::collections::HashSet<usize> = (0..2_000)
            .map(|_| email(&mut rng, &p, Realism::FictionalSafe, uniform(10, 50), "").len())
            .collect();
        assert_eq!(lengths.len(), 41, "every length from 10 to 50 occurs");
    }

    #[test]
    fn natural_names_follow_the_row_persona() {
        let p = persona(7, Locale::De);
        let mut rng = SeedRng::new(1);
        let plan = LengthPlan::natural(1, 255);
        assert_eq!(first_name(&mut rng, &p, plan), p.given.text);
        assert_eq!(last_name(&mut rng, &p, plan), p.family_text());
        let email = email(
            &mut rng,
            &p,
            Realism::FictionalSafe,
            LengthPlan::natural(8, 254),
            "",
        );
        let (given, family) = p.tokens();
        assert!(email.contains(&given) || email.contains(&family), "{email}");
    }

    fn assert_valid_email(value: &str) {
        let (local, domain) = value.rsplit_once('@').expect("@");
        assert!((1..=64).contains(&local.len()), "{value}");
        assert!(value.len() <= 254, "{value}");
        assert!(value.is_ascii(), "{value}");
        assert!(
            !local.starts_with('.') && !local.ends_with('.') && !local.contains(".."),
            "{value}"
        );
        assert!(
            local
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c)),
            "{value}"
        );
        let labels: Vec<&str> = domain.split('.').collect();
        assert!(labels.len() >= 2, "{value}");
        for label in labels {
            assert!((1..=63).contains(&label.len()), "{value}");
            assert!(!label.starts_with('-') && !label.ends_with('-'), "{value}");
            assert!(
                label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "{value}"
            );
        }
    }

    #[test]
    fn emails_are_well_formed_at_every_length_and_locale() {
        for locale in Locale::ALL {
            let p = persona(3, locale);
            let mut rng = SeedRng::new(11);
            for target in MIN_EMAIL_CHARS..=MAX_EMAIL_CHARS {
                let plan = uniform(target, target);
                for realism in [Realism::FictionalSafe, Realism::Realistic] {
                    let value = email(&mut rng, &p, realism, plan, "");
                    assert_eq!(value.len(), target, "{value}");
                    assert_valid_email(&value);
                }
                if target >= MIN_EMAIL_CHARS + 3 {
                    let value = email(&mut rng, &p, Realism::FictionalSafe, plan, ".42");
                    assert_eq!(value.len(), target, "{value}");
                    assert!(value.contains(".42@"), "{value}");
                    assert_valid_email(&value);
                }
            }
        }
    }

    #[test]
    fn tagged_emails_in_a_tight_window_leave_room_for_the_tag() {
        let p = persona(5, Locale::EnUs);
        let mut rng = SeedRng::new(6);
        for _ in 0..500 {
            let value = email(&mut rng, &p, Realism::FictionalSafe, uniform(8, 12), ".42");
            assert!((11..=12).contains(&value.len()), "{value}");
            assert_valid_email(&value);
        }
    }

    #[test]
    fn fictional_emails_never_use_a_real_domain() {
        let mut rng = SeedRng::new(2);
        for seed in 0..500 {
            let p = persona(seed, Locale::EnGb);
            let plan = if seed % 2 == 0 {
                LengthPlan::natural(8, 254)
            } else {
                uniform(8, 254)
            };
            let value = email(&mut rng, &p, Realism::FictionalSafe, plan, "");
            let domain = value.rsplit_once('@').unwrap().1;
            assert!(
                domain.ends_with(".test")
                    || domain.ends_with(".example")
                    || FICTIONAL_DOMAINS.contains(&domain),
                "{value}"
            );
        }
    }

    #[test]
    fn prose_fits_long_and_short_windows() {
        let mut rng = SeedRng::new(5);
        for (lo, hi) in [(3, 4), (1, 30), (1000, 5000), (90_000, 100_000)] {
            for _ in 0..20 {
                let value = prose(&mut rng, uniform(lo, hi), (1, 3));
                assert!(
                    (lo..=hi).contains(&value.len()),
                    "{lo}..{hi}: {}",
                    value.len()
                );
            }
        }
        let long = prose(&mut rng, uniform(4000, 4000), (1, 3));
        assert!(long.contains("\n\n"), "long text is broken into paragraphs");
        for target in 24..2_000 {
            let value = prose(&mut rng, uniform(target / 2, target), (1, 3));
            assert_eq!(value.trim_end(), value, "{target}: {value:?}");
            assert!((target / 2..=target).contains(&value.len()), "{target}");
        }
    }

    #[test]
    fn cuts_never_end_in_whitespace() {
        let text = "One two.\n\nThree four five.";
        for target in 1..text.len() {
            for lo in [1, target] {
                let value = cut_at_word(text, lo, target);
                assert_eq!(value.trim_end(), value, "{lo}..{target}: {value:?}");
                assert!(
                    (lo..=target).contains(&value.len()),
                    "{lo}..{target}: {value:?}"
                );
            }
        }
    }

    #[test]
    fn usernames_and_words_fit_their_windows() {
        let mut rng = SeedRng::new(8);
        for seed in 0..200 {
            let p = persona(seed, Locale::Ja);
            for (lo, hi) in [(3, 8), (8, 30), (60, 255)] {
                let plan = uniform(lo, hi);
                let value = user_name(&mut rng, &p, plan, "");
                assert!(plan.contains(value.len()) && value.is_ascii(), "{value}");
                let value = user_name(&mut rng, &p, plan, "_99");
                assert!(
                    plan.contains(value.len()) && value.ends_with("_99"),
                    "{value}"
                );
                let value = word(&mut rng, plan);
                assert!(plan.contains(chars(&value)), "{value}");
            }
        }
    }
}
