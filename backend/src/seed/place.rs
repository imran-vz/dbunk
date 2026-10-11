//! Addresses, companies and URLs (ADR-0014, "Seed data variety").
//!
//! A row's address columns come from one [`Place`]: a real postcode with
//! the locality and region it belongs to, plus a made-up street in the
//! locale's own style. Each builder keeps the place's natural value when
//! it fits the column's window and otherwise builds one to fit.

use super::data::{self, Locale, PlaceEntry, WeightedList};
use super::text::{
    chars, compose, domain_of_length, fit_part, natural_domain, LengthPlan, Realism,
};
use super::SeedRng;
use std::cell::OnceCell;

pub(crate) const MIN_STREET_CHARS: usize = 3;
pub(crate) const MIN_POSTCODE_CHARS: usize = 5;
pub(crate) const MAX_POSTCODE_CHARS: usize = 10;
pub(crate) const MIN_FULL_ADDRESS_CHARS: usize = 20;
pub(crate) const MIN_URL_CHARS: usize = 14;

/// Where a street name's variable part comes from.
#[derive(Clone, Copy)]
enum Root {
    Family,
    Word,
}

impl Root {
    fn list(self, locale: Locale) -> &'static WeightedList {
        match self {
            Root::Family => &data::names(locale).family,
            Root::Word => &data::text().words,
        }
    }

    /// A natural root: one family name, or one capitalised word.
    fn natural(self, rng: &mut SeedRng, locale: Locale) -> String {
        capitalize(self.list(locale).pick(rng).text)
    }

    /// A root of exactly `len` characters.
    fn exact(self, rng: &mut SeedRng, locale: Locale, len: usize) -> String {
        capitalize(&compose(self.list(locale), rng, len, "-"))
    }
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A street as `before + root + after`, so its length can be set through
/// the root alone.
struct Style {
    before: String,
    after: String,
    root: Root,
}

fn number(rng: &mut SeedRng, max: i64) -> i64 {
    // Small house numbers are more common than large ones.
    let digits = rng.range_i64(1, max.to_string().len() as i64);
    rng.range_i64(1, 10i64.pow(digits as u32).min(max + 1) - 1)
}

fn street_style(rng: &mut SeedRng, locale: Locale) -> Style {
    let pick = |rng: &mut SeedRng, items: &[&str]| -> String { (*rng.pick(items)).to_string() };
    let root = if rng.next_f64() < 0.5 {
        Root::Family
    } else {
        Root::Word
    };
    match locale {
        Locale::EnUs => Style {
            before: format!("{} ", number(rng, 9999)),
            after: format!(
                " {}",
                pick(
                    rng,
                    &[
                        "Street",
                        "Avenue",
                        "Road",
                        "Drive",
                        "Lane",
                        "Court",
                        "Boulevard",
                        "Way",
                        "Place",
                        "St",
                        "Ave",
                        "Rd",
                        "Dr",
                        "Blvd"
                    ],
                )
            ),
            root,
        },
        Locale::EnGb => Style {
            before: format!("{} ", number(rng, 250)),
            after: format!(
                " {}",
                pick(
                    rng,
                    &[
                        "Road", "Street", "Lane", "Close", "Avenue", "Gardens", "Crescent",
                        "Drive", "Way", "Grove", "Terrace"
                    ],
                )
            ),
            root,
        },
        Locale::EnIn => Style {
            before: format!("{}, ", number(rng, 999)),
            after: format!(
                " {}",
                pick(rng, &["Road", "Nagar", "Marg", "Colony", "Street", "Lane"])
            ),
            root: Root::Family,
        },
        Locale::De => Style {
            before: String::new(),
            after: format!(
                "{} {}",
                pick(rng, &["straße", "weg", "allee", "ring", "gasse", "str."]),
                number(rng, 199)
            ),
            root: Root::Family,
        },
        Locale::Fr => Style {
            before: format!(
                "{} {} ",
                number(rng, 199),
                pick(
                    rng,
                    &[
                        "rue",
                        "avenue",
                        "boulevard",
                        "place",
                        "impasse",
                        "allée",
                        "chemin"
                    ]
                )
            ),
            after: String::new(),
            root: Root::Family,
        },
        Locale::Es => Style {
            before: format!(
                "{} de ",
                pick(rng, &["Calle", "Avenida", "Plaza", "Paseo", "Camino"])
            ),
            after: format!(", {}", number(rng, 199)),
            root: Root::Family,
        },
        Locale::PtBr => Style {
            before: format!(
                "{} ",
                pick(rng, &["Rua", "Avenida", "Travessa", "Alameda", "Praça"])
            ),
            after: format!(", {}", number(rng, 2999)),
            root: Root::Family,
        },
        Locale::Ja => Style {
            before: String::new(),
            after: format!(
                "{}丁目{}-{}",
                rng.range_i64(1, 9),
                rng.range_i64(1, 30),
                rng.range_i64(1, 20)
            ),
            root: Root::Family,
        },
    }
}

/// A street of exactly `target` characters in the locale's style, or a
/// plain `Root 1` when the style's fixed parts leave no room.
fn street_of_length(rng: &mut SeedRng, locale: Locale, target: usize) -> String {
    for _ in 0..8 {
        let style = street_style(rng, locale);
        let fixed = chars(&style.before) + chars(&style.after);
        if target > fixed {
            let root = style.root.exact(rng, locale, target - fixed);
            return format!("{}{root}{}", style.before, style.after);
        }
    }
    let root = Root::Family.exact(rng, locale, target.saturating_sub(2).max(1));
    format!("{root} 1")
}

fn natural_street(rng: &mut SeedRng, locale: Locale) -> String {
    let style = street_style(rng, locale);
    let root = style.root.natural(rng, locale);
    format!("{}{root}{}", style.before, style.after)
}

fn line2_prefixes(locale: Locale) -> &'static [&'static str] {
    match locale {
        Locale::EnUs => &["Apt ", "Suite ", "Unit ", "#", "Floor "],
        Locale::EnGb => &["Flat ", "Apartment ", "Unit "],
        Locale::EnIn => &["Flat ", "Floor ", "Block "],
        Locale::De => &["Wohnung ", "Etage "],
        Locale::Fr => &["Appartement ", "Bâtiment ", "Étage "],
        Locale::Es => &["Piso ", "Puerta ", "Escalera "],
        Locale::PtBr => &["Apto ", "Bloco ", "Casa "],
        Locale::Ja => &["ハイツ", "マンション", "コーポ"],
    }
}

fn natural_line2(rng: &mut SeedRng, locale: Locale) -> String {
    let prefix = rng.pick(line2_prefixes(locale));
    let value = rng.range_i64(1, 450);
    if locale == Locale::Ja {
        format!(
            "{}{prefix}{value}号室",
            data::names(locale).family.pick(rng).text
        )
    } else {
        format!("{prefix}{value}")
    }
}

fn digits(rng: &mut SeedRng, count: usize) -> String {
    let mut out = String::with_capacity(count);
    out.push(char::from(b'1' + rng.range_i64(0, 8) as u8));
    for _ in 1..count {
        out.push(char::from(b'0' + rng.range_i64(0, 9) as u8));
    }
    out
}

fn line2_of_length(rng: &mut SeedRng, locale: Locale, target: usize) -> String {
    let prefixes: Vec<&str> = line2_prefixes(locale)
        .iter()
        .copied()
        .filter(|p| chars(p) < target)
        .collect();
    let Some(prefix) = (!prefixes.is_empty()).then(|| *rng.pick(&prefixes)) else {
        return digits(rng, target);
    };
    let room = target - chars(prefix);
    if room <= 5 {
        return format!("{prefix}{}", digits(rng, room));
    }
    // Long windows: a unit number and the building's name.
    let unit = digits(rng, 2);
    let name = Root::Family.exact(rng, locale, room - 4);
    format!("{prefix}{unit}, {name}")
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// One made-up address per row, so a row's street, locality, region,
/// postcode and country columns describe the same place.
pub(crate) struct Place {
    pub locale: Locale,
    pub entry: &'static PlaceEntry,
    /// Drawn by the first street-like column, so a row that only needs a
    /// locality or postcode spends nothing on a street.
    extras: OnceCell<Extras>,
}

/// The made-up parts of a place.
pub(crate) struct Extras {
    pub street: String,
    pub line2: Option<String>,
    with_country: bool,
}

impl Place {
    pub(crate) fn draw(rng: &mut SeedRng, locale: Locale) -> Self {
        Self {
            locale,
            entry: rng.pick(&data::places(locale).entries),
            extras: OnceCell::new(),
        }
    }

    pub(crate) fn extras(&self, rng: &mut SeedRng) -> &Extras {
        self.extras.get_or_init(|| Extras {
            street: natural_street(rng, self.locale),
            line2: (rng.next_f64() < 0.3).then(|| natural_line2(rng, self.locale)),
            with_country: rng.next_f64() < 0.5,
        })
    }

    fn places(&self) -> &'static data::Places {
        data::places(self.locale)
    }
}

fn natural_or<F>(natural: &str, plan: LengthPlan, build: F) -> String
where
    F: FnOnce() -> String,
{
    if !plan.uniform && plan.contains(chars(natural)) {
        natural.to_string()
    } else {
        build()
    }
}

pub(crate) fn street(rng: &mut SeedRng, place: &Place, plan: LengthPlan) -> String {
    let natural = place.extras(rng).street.clone();
    natural_or(&natural, plan, || {
        let target = plan
            .target(rng)
            .unwrap_or_else(|| chars(&natural).clamp(plan.lo, plan.hi));
        street_of_length(rng, place.locale, target)
    })
}

pub(crate) fn line2(rng: &mut SeedRng, place: &Place, plan: LengthPlan) -> String {
    let natural = match &place.extras(rng).line2 {
        Some(line2) => line2.clone(),
        None => natural_line2(rng, place.locale),
    };
    natural_or(&natural, plan, || {
        let target = plan
            .target(rng)
            .unwrap_or_else(|| chars(&natural).clamp(plan.lo, plan.hi));
        line2_of_length(rng, place.locale, target)
    })
}

pub(crate) fn locality(rng: &mut SeedRng, place: &Place, plan: LengthPlan) -> String {
    natural_or(place.entry.locality, plan, || {
        fit_part(&place.places().localities, rng, plan, "-", None)
    })
}

pub(crate) fn region(rng: &mut SeedRng, place: &Place, plan: LengthPlan) -> String {
    natural_or(place.entry.region, plan, || {
        fit_part(&place.places().regions, rng, plan, "-", None)
    })
}

/// A real postcode inside the window: the place's own, another of the
/// same locale, a US ZIP+4, or one from another locale.
pub(crate) fn postcode(rng: &mut SeedRng, place: &Place, plan: LengthPlan) -> String {
    natural_or(place.entry.postal, plan, || {
        let (lo, hi) = match plan.target(rng) {
            Some(target) => (target, target),
            None => (plan.lo, plan.hi),
        };
        if let Some(entry) = place.places().postcodes.pick_within(rng, lo, hi) {
            return entry.text.to_string();
        }
        if place.locale == Locale::EnUs && (lo..=hi).contains(&10) {
            return format!("{}-{}", place.entry.postal, digits(rng, 4));
        }
        Locale::ALL
            .iter()
            .find_map(|&l| data::places(l).postcodes.pick_within(rng, plan.lo, plan.hi))
            .map_or_else(|| place.entry.postal.to_string(), |e| e.text.to_string())
    })
}

/// The country of the row's locale: its common name when it fits, else
/// its official name, another seed country's name, or an ISO code. When
/// nothing reaches the window's lower bound, the longest value under the
/// upper bound wins, so the column's capacity is never exceeded.
pub(crate) fn country(rng: &mut SeedRng, locale: Locale, plan: LengthPlan) -> String {
    let own = [data::places(locale).country_name, locale.official_name()];
    if !plan.uniform && plan.contains(chars(own[0])) {
        return own[0].to_string();
    }
    let fits = |names: &[&'static str]| -> Vec<&'static str> {
        names
            .iter()
            .copied()
            .filter(|n| plan.contains(chars(n)))
            .collect()
    };
    let own_fit = fits(&own);
    if !own_fit.is_empty() {
        return (*rng.pick(&own_fit)).to_string();
    }
    let codes = [locale.country(), locale.country_alpha3()];
    let all: Vec<&'static str> = Locale::ALL
        .iter()
        .flat_map(|&l| [data::places(l).country_name, l.official_name()])
        .collect();
    let others = fits(&all);
    if !others.is_empty() {
        return (*rng.pick(&others)).to_string();
    }
    let code_fit = fits(&codes);
    if !code_fit.is_empty() {
        return code_fit[0].to_string();
    }
    own.iter()
        .chain(&codes)
        .copied()
        .filter(|n| chars(n) <= plan.hi)
        .max_by_key(|n| chars(n))
        .unwrap_or(codes[0])
        .to_string()
}

/// ISO 3166-1 alpha-2, or alpha-3 when the window asks for three
/// characters (or allows both and the length is spread).
pub(crate) fn country_code(rng: &mut SeedRng, locale: Locale, plan: LengthPlan) -> String {
    let alpha2 = locale.country();
    let alpha3 = locale.country_alpha3();
    let code = match (plan.contains(2), plan.contains(3)) {
        (true, true) if plan.uniform && rng.next_f64() < 0.5 => alpha3,
        (true, _) => alpha2,
        (false, true) => alpha3,
        (false, false) => alpha2,
    };
    code.to_string()
}

/// Which optional parts a formatted address carries.
#[derive(Clone, Copy)]
struct Parts {
    line2: bool,
    region: bool,
    country: bool,
}

/// The address on one line in the country's own order (libaddressinput
/// format) for `entry`, with the street set to `street`.
fn format_address(
    locale: Locale,
    entry: &PlaceEntry,
    street: &str,
    line2: Option<&str>,
    parts: Parts,
) -> String {
    let places = data::places(locale);
    let region = match (parts.region, entry.region_code.is_empty()) {
        (false, _) => "",
        (true, true) => entry.region,
        (true, false) => entry.region_code,
    };
    let street_lines = match (line2, parts.line2) {
        (Some(second), true) => format!("{street}, {second}"),
        _ => street.to_string(),
    };
    let separator = if locale == Locale::Ja { " " } else { ", " };
    let mut lines: Vec<String> = places
        .address_format
        .split("%n")
        .map(|line| {
            line.replace("%N", "")
                .replace("%O", "")
                .replace("%X", "")
                .replace("%A", &street_lines)
                .replace("%C", entry.locality)
                .replace("%S", region)
                .replace("%Z", entry.postal)
                .trim_matches([' ', ',', '-'])
                .to_string()
        })
        .filter(|line| !line.is_empty() && line != "〒")
        .collect();
    if parts.country && locale != Locale::Ja {
        lines.push(places.country_name.to_string());
    }
    lines.join(separator)
}

pub(crate) fn full_address(rng: &mut SeedRng, place: &Place, plan: LengthPlan) -> String {
    let extras = place.extras(rng);
    let (street, line2) = (extras.street.clone(), extras.line2.clone());
    let full = Parts {
        line2: true,
        region: true,
        country: extras.with_country,
    };
    let natural = format_address(place.locale, place.entry, &street, line2.as_deref(), full);
    natural_or(&natural, plan, || {
        let natural_len = chars(&natural);
        let target = plan
            .target(rng)
            .unwrap_or_else(|| natural_len.clamp(plan.lo, plan.hi));
        // Fit through the street. When the rest alone is too long, drop the
        // second line, the country and the region, then try places with
        // shorter localities.
        let fallbacks = [
            full,
            Parts {
                line2: false,
                ..full
            },
            Parts {
                line2: false,
                country: false,
                ..full
            },
            Parts {
                line2: false,
                country: false,
                region: false,
            },
        ];
        let entries = &place.places().entries;
        let minimal = Parts {
            line2: false,
            country: false,
            region: false,
        };
        let measure = |entry: &PlaceEntry, parts: Parts| {
            chars(&format_address(
                place.locale,
                entry,
                "X",
                line2.as_deref(),
                parts,
            )) - 1
        };
        for attempt in 0..10 {
            // The row's place, then a few others, then the locale's most
            // compact one.
            let entry = match attempt {
                0 => place.entry,
                9 => entries
                    .iter()
                    .min_by_key(|e| measure(e, minimal))
                    .unwrap_or(place.entry),
                _ => rng.pick(entries),
            };
            for parts in fallbacks {
                // Measured with a one-character street: an empty one would let
                // the separator before the second line be trimmed away.
                let fixed = measure(entry, parts);
                if target >= fixed + MIN_STREET_CHARS {
                    let street = street_of_length(rng, place.locale, target - fixed);
                    return format_address(place.locale, entry, &street, line2.as_deref(), parts);
                }
            }
        }
        natural.clone()
    })
}

// ---------------------------------------------------------------------------
// Companies and URLs
// ---------------------------------------------------------------------------

fn company_forms(locale: Locale) -> &'static [(&'static str, &'static str)] {
    match locale {
        Locale::EnUs => &[
            ("", " Inc."),
            ("", " LLC"),
            ("", " Group"),
            ("", " & Sons"),
            ("", " Holdings"),
            ("", " Partners"),
            ("", " Industries"),
            ("", " Labs"),
        ],
        Locale::EnGb => &[
            ("", " Ltd"),
            ("", " plc"),
            ("", " & Co."),
            ("", " Group"),
            ("", " LLP"),
        ],
        Locale::EnIn => &[
            ("", " Pvt. Ltd."),
            ("", " Industries Ltd."),
            ("", " & Sons"),
            ("", " Enterprises"),
            ("", " Technologies"),
        ],
        Locale::De => &[
            ("", " GmbH"),
            ("", " AG"),
            ("", " KG"),
            ("", " GmbH & Co. KG"),
            ("", " Gruppe"),
        ],
        Locale::Fr => &[
            ("", " SARL"),
            ("", " SA"),
            ("", " SAS"),
            ("Groupe ", ""),
            ("", " et Fils"),
        ],
        Locale::Es => &[
            ("", " S.L."),
            ("", " S.A."),
            ("Grupo ", ""),
            ("", " y Asociados"),
        ],
        Locale::PtBr => &[
            ("", " Ltda."),
            ("", " S.A."),
            ("Grupo ", ""),
            ("", " & Filhos"),
        ],
        Locale::Ja => &[
            ("株式会社", ""),
            ("", "商事"),
            ("", "工業株式会社"),
            ("", "製作所"),
            ("", "電機"),
        ],
    }
}

pub(crate) fn company(rng: &mut SeedRng, locale: Locale, plan: LengthPlan) -> String {
    let forms = company_forms(locale);
    let families = &data::names(locale).family;
    let natural = {
        let (before, after) = rng.pick(forms);
        let root = if locale != Locale::Ja && rng.next_f64() < 0.15 {
            format!("{} & {}", families.pick(rng).text, families.pick(rng).text)
        } else {
            families.pick(rng).text.to_string()
        };
        format!("{before}{root}{after}")
    };
    natural_or(&natural, plan, || {
        let target = plan
            .target(rng)
            .unwrap_or_else(|| chars(&natural).clamp(plan.lo, plan.hi));
        let fitting: Vec<&(&str, &str)> = forms
            .iter()
            .filter(|(b, a)| chars(b) + chars(a) < target)
            .collect();
        match fitting.is_empty() {
            true => compose(families, rng, target, "-"),
            false => {
                let (before, after) = rng.pick(&fitting);
                let root = compose(families, rng, target - chars(before) - chars(after), "-");
                format!("{before}{root}{after}")
            }
        }
    })
}

pub(crate) fn url(rng: &mut SeedRng, locale: Locale, realism: Realism, plan: LengthPlan) -> String {
    let words = &data::text().words;
    let host = match (realism, rng.next_f64() < 0.5) {
        (Realism::FictionalSafe, true) => format!("www.{}", natural_domain(rng, realism, locale)),
        (Realism::FictionalSafe, false) => natural_domain(rng, realism, locale),
        (Realism::Realistic, _) => format!(
            "www.{}{}.com",
            words.pick_within(rng, 3, 8).map_or("site", |e| e.text),
            words.pick_within(rng, 3, 8).map_or("hub", |e| e.text)
        ),
    };
    let natural = match rng.range_i64(0, 4) {
        0 | 1 => format!("https://{host}"),
        2 | 3 => format!("https://{host}/{}", words.pick(rng).text),
        _ => format!(
            "https://{host}/{}/{}",
            words.pick(rng).text,
            words.pick(rng).text
        ),
    };
    natural_or(&natural, plan, || {
        let target = plan
            .target(rng)
            .unwrap_or_else(|| natural.len().clamp(plan.lo, plan.hi));
        let scheme = "https://";
        let host = if scheme.len() + host.len() <= target {
            host
        } else {
            domain_of_length(rng, realism, target - scheme.len())
        };
        let room = target - scheme.len() - host.len();
        match room {
            0 => format!("{scheme}{host}"),
            1 => format!("{scheme}{host}/"),
            _ => format!("{scheme}{host}/{}", compose(words, rng, room - 1, "/")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(lo: usize, hi: usize) -> LengthPlan {
        LengthPlan {
            lo,
            hi,
            uniform: true,
        }
    }

    #[test]
    fn natural_places_keep_their_postcode_locality_and_region_together() {
        let mut rng = SeedRng::new(1);
        for locale in Locale::ALL {
            let place = Place::draw(&mut rng, locale);
            let wide = LengthPlan::natural(1, 255);
            assert_eq!(locality(&mut rng, &place, wide), place.entry.locality);
            assert_eq!(region(&mut rng, &place, wide), place.entry.region);
            assert_eq!(
                postcode(&mut rng, &place, LengthPlan::natural(5, 10)),
                place.entry.postal
            );
            let full = full_address(
                &mut rng,
                &place,
                LengthPlan::natural(MIN_FULL_ADDRESS_CHARS, 1000),
            );
            assert!(full.contains(place.entry.postal), "{full}");
            assert!(full.contains(place.entry.locality), "{full}");
        }
    }

    #[test]
    fn postcodes_match_the_country_pattern() {
        let mut rng = SeedRng::new(2);
        for locale in Locale::ALL {
            let regex =
                regex::Regex::new(&format!("^(?:{})$", data::places(locale).postcode_pattern))
                    .expect("pattern compiles");
            for _ in 0..300 {
                let place = Place::draw(&mut rng, locale);
                let code = postcode(&mut rng, &place, LengthPlan::natural(5, 10));
                assert!(regex.is_match(&code), "{locale:?}: {code}");
            }
        }
    }

    #[test]
    fn address_parts_fit_uniform_windows_in_every_locale() {
        let mut rng = SeedRng::new(3);
        for locale in Locale::ALL {
            for seed in 0..60 {
                let place = Place::draw(&mut SeedRng::new(seed), locale);
                for (lo, hi) in [(3, 12), (10, 40), (40, 255)] {
                    let plan = uniform(lo, hi);
                    for value in [
                        street(&mut rng, &place, plan),
                        line2(&mut rng, &place, uniform(lo.max(1), hi)),
                        locality(&mut rng, &place, plan),
                        region(&mut rng, &place, plan),
                        company(&mut rng, locale, plan),
                    ] {
                        assert!(
                            plan.contains(chars(&value)),
                            "{locale:?} {lo}..{hi}: {value:?}"
                        );
                        assert_eq!(value.trim(), value, "{locale:?}: {value:?}");
                    }
                }
                for (lo, hi) in [(20, 40), (30, 60), (60, 200), (200, 1000)] {
                    let value = full_address(&mut rng, &place, uniform(lo, hi));
                    assert!(
                        (lo..=hi).contains(&chars(&value)),
                        "{locale:?} {lo}..{hi}: {value}"
                    );
                }
                for (lo, hi) in [(5, 5), (6, 8), (9, 10)] {
                    let value = postcode(&mut rng, &place, uniform(lo, hi));
                    assert!(
                        (lo..=hi).contains(&value.len()),
                        "{locale:?} {lo}..{hi}: {value}"
                    );
                }
            }
        }
    }

    #[test]
    fn urls_are_well_formed_at_every_length() {
        let mut rng = SeedRng::new(4);
        for realism in [Realism::FictionalSafe, Realism::Realistic] {
            for target in MIN_URL_CHARS..=300 {
                let value = url(&mut rng, Locale::EnUs, realism, uniform(target, target));
                assert_eq!(value.len(), target, "{value}");
                assert!(
                    value.starts_with("https://") && !value.ends_with('.'),
                    "{value}"
                );
                let host = value["https://".len()..].split('/').next().unwrap();
                assert!(host.contains('.') && !host.starts_with('.'), "{value}");
                if realism == Realism::FictionalSafe {
                    assert!(
                        host.ends_with(".test")
                            || host.ends_with(".example")
                            || host.contains("example."),
                        "{value}"
                    );
                }
            }
        }
    }

    #[test]
    fn country_codes_and_names() {
        let mut rng = SeedRng::new(5);
        let de = Locale::De;
        assert_eq!(country_code(&mut rng, de, LengthPlan::natural(2, 3)), "DE");
        assert_eq!(country_code(&mut rng, de, uniform(3, 3)), "DEU");
        assert_eq!(country(&mut rng, de, LengthPlan::natural(2, 60)), "Germany");
        for _ in 0..200 {
            let value = country(&mut rng, de, uniform(12, 20));
            assert!((12..=20).contains(&value.len()), "{value}");
            let value = country(&mut rng, de, uniform(25, 30));
            assert_eq!(value, "Federal Republic of Germany");
            // Nothing is 18 to 23 characters long: the longest value that
            // still fits the column wins.
            let value = country(&mut rng, de, uniform(18, 23));
            assert!(value.len() <= 23, "{value}");
        }
    }
}
