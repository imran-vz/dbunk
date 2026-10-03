//! Open Anything index and ranking, ported from the baseline palette rules:
//! every token must match, tiers never cross, and per-kind caps are disclosed.
use dbunk_lib::backend::objects::PgObjectRef;
use std::collections::HashMap;

const RECENT_LIMIT: usize = 6;
/// Below the smallest tier gap, so frecency reorders only within a tier.
const FRECENCY_BOOST_CAP: u32 = 50;
const FRECENCY_KEYS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ItemKind {
    Command,
    Tab,
    Connection,
    Schema,
    Relation,
    Object,
}
impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Command => "Command",
            Self::Tab => "Tab",
            Self::Connection => "Connection",
            Self::Schema => "Schema",
            Self::Relation => "Relation",
            Self::Object => "Object",
        }
    }
    fn cap(self) -> Option<usize> {
        match self {
            Self::Connection | Self::Schema => Some(20),
            Self::Relation | Self::Object => Some(200),
            Self::Command | Self::Tab => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target<C> {
    Command(C),
    Tab(String),
    Connection(String),
    Schema {
        connection: String,
        schema: String,
    },
    Relation {
        connection: String,
        schema: String,
        name: String,
    },
    Object {
        connection: String,
        reference: PgObjectRef,
    },
}

#[derive(Clone, Debug)]
pub struct Item<C> {
    /// Stable identity and frecency key.
    pub key: String,
    pub kind: ItemKind,
    pub label: String,
    pub description: String,
    keywords: String,
    pub target: Target<C>,
}
impl<C> Item<C> {
    pub fn new(
        key: String,
        kind: ItemKind,
        label: String,
        description: String,
        keywords: &str,
        target: Target<C>,
    ) -> Self {
        let keywords = format!("{label} {keywords} {description}").to_lowercase();
        Self {
            key,
            kind,
            label,
            description,
            keywords,
            target,
        }
    }
}

fn word_boundary(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(at, _)| {
        haystack[..at]
            .chars()
            .next_back()
            // Baseline `/[a-z0-9]/` on the lowercased haystack.
            .is_none_or(|before| !(before.is_ascii_lowercase() || before.is_ascii_digit()))
    })
}
fn subsequence(haystack: &str, needle: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|wanted| chars.any(|c| c == wanted))
}
fn score<C>(item: &Item<C>, token: &str) -> u32 {
    let label = item.label.to_lowercase();
    if label == token {
        400
    } else if label.starts_with(token) {
        300
    } else if word_boundary(&item.keywords, token) {
        200
    } else if item.keywords.contains(token) {
        100
    } else if subsequence(&label, token) {
        40
    } else {
        0
    }
}

/// Session-local usage counts. Bounded; the least recent key is evicted.
#[derive(Clone, Default)]
pub struct Frecency {
    entries: HashMap<String, (u32, u64)>,
    clock: u64,
}
impl Frecency {
    pub fn record(&mut self, key: &str) {
        self.clock += 1;
        let entry = self.entries.entry(key.to_owned()).or_insert((0, 0));
        entry.0 = entry.0.saturating_add(1);
        entry.1 = self.clock;
        if self.entries.len() > FRECENCY_KEYS
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last))| *last)
                .map(|(key, _)| key.clone())
        {
            self.entries.remove(&oldest);
        }
    }
    fn count(&self, key: &str) -> u32 {
        self.entries.get(key).map_or(0, |entry| entry.0)
    }
}

pub struct Ranked {
    /// Indices into the index, best first.
    pub items: Vec<usize>,
    /// Matches cut by each kind's cap.
    pub truncated: Vec<(ItemKind, usize)>,
}

/// A `>` prefix restricts the query to commands, like the baseline palette.
pub fn rank<C>(items: &[Item<C>], query: &str, frecency: &Frecency) -> Ranked {
    let (commands_only, query) = match query.trim_start().strip_prefix('>') {
        Some(rest) => (true, rest),
        None => (false, query),
    };
    let query = query.to_lowercase();
    let tokens: Vec<&str> = query.split_whitespace().collect();
    let eligible = |item: &Item<C>| !commands_only || matches!(item.kind, ItemKind::Command);
    if tokens.is_empty() {
        let mut recents: Vec<usize> = (0..items.len())
            .filter(|&i| eligible(&items[i]) && frecency.count(&items[i].key) > 0)
            .collect();
        recents.sort_by_key(|&i| std::cmp::Reverse(frecency.count(&items[i].key)));
        recents.truncate(RECENT_LIMIT);
        let ambient = (0..items.len()).filter(|i| {
            eligible(&items[*i])
                && (commands_only || matches!(items[*i].kind, ItemKind::Tab | ItemKind::Command))
                && !recents.contains(i)
        });
        let mut ranked = recents.clone();
        ranked.extend(ambient);
        return Ranked {
            items: ranked,
            truncated: Vec::new(),
        };
    }
    let mut scored: Vec<(usize, u32)> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| eligible(item))
        .filter_map(|(index, item)| {
            let mut total = 0;
            for token in &tokens {
                match score(item, token) {
                    0 => return None,
                    value => total += value,
                }
            }
            Some((
                index,
                total + frecency.count(&item.key).min(FRECENCY_BOOST_CAP),
            ))
        })
        .collect();
    scored.sort_by(|(a, left), (b, right)| {
        right
            .cmp(left)
            .then_with(|| items[*a].label.cmp(&items[*b].label))
    });
    let mut seen: HashMap<ItemKind, usize> = HashMap::new();
    let mut cut: HashMap<ItemKind, usize> = HashMap::new();
    let mut ranked = Vec::new();
    for (index, _) in scored {
        let kind = items[index].kind;
        let count = seen.entry(kind).or_default();
        if kind.cap().is_some_and(|cap| *count >= cap) {
            *cut.entry(kind).or_default() += 1;
            continue;
        }
        *count += 1;
        ranked.push(index);
    }
    let mut truncated: Vec<_> = cut.into_iter().collect();
    truncated.sort();
    Ranked {
        items: ranked,
        truncated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::objects::PgObjectKind;

    fn item(key: &str, kind: ItemKind, label: &str, keywords: &str) -> Item<u8> {
        Item::new(
            key.into(),
            kind,
            label.into(),
            String::new(),
            keywords,
            Target::Command(0),
        )
    }

    #[test]
    fn tiers_and_and_semantics_rank_like_the_baseline() {
        let items = vec![
            item("a", ItemKind::Relation, "orders", "public.orders table"),
            item(
                "b",
                ItemKind::Relation,
                "order_items",
                "public.order_items table",
            ),
            item(
                "c",
                ItemKind::Relation,
                "customers",
                "sales.customers table",
            ),
            item(
                "d",
                ItemKind::Relation,
                "big_orders",
                "public.big_orders table",
            ),
            item(
                "e",
                ItemKind::Relation,
                "xoyrdzers",
                "public.xoyrdzers table",
            ),
        ];
        let frecency = Frecency::default();
        let ranked = rank(&items, "orders", &frecency);
        let labels: Vec<_> = ranked
            .items
            .iter()
            .map(|&i| items[i].label.as_str())
            .collect();
        // Exact 400, word boundary 200, then subsequence 40 ties by label.
        assert_eq!(labels, ["orders", "big_orders", "order_items", "xoyrdzers"]);
        let both = rank(&items, "public ord", &frecency);
        assert!(
            both.items
                .iter()
                .all(|&i| items[i].keywords.contains("public"))
        );
        assert!(!both.items.contains(&2));
        assert!(rank(&items, "orders nomatch", &frecency).items.is_empty());
    }

    #[test]
    fn frecency_reorders_within_a_tier_but_never_across() {
        let items = vec![
            item("exact", ItemKind::Relation, "users", ""),
            item("prefix", ItemKind::Relation, "users_archive", ""),
            item("prefix2", ItemKind::Relation, "users_audit", ""),
        ];
        let mut frecency = Frecency::default();
        for _ in 0..500 {
            frecency.record("prefix2");
        }
        let ranked = rank(&items, "users", &frecency);
        assert_eq!(ranked.items, [0, 2, 1]);
    }

    #[test]
    fn empty_query_shows_recents_then_tabs_and_commands() {
        let items = vec![
            item("cmd", ItemKind::Command, "New query", ""),
            item("rel", ItemKind::Relation, "orders", ""),
            item("tab", ItemKind::Tab, "Query 1", ""),
            item("rel2", ItemKind::Relation, "users", ""),
        ];
        let mut frecency = Frecency::default();
        frecency.record("rel2");
        frecency.record("rel2");
        frecency.record("cmd");
        assert_eq!(rank(&items, "  ", &frecency).items, [3, 0, 2]);
        // `>` restricts both empty and non-empty queries to commands.
        assert_eq!(rank(&items, ">", &frecency).items, [0]);
        assert_eq!(rank(&items, "> new", &frecency).items, [0]);
        assert!(rank(&items, ">orders", &frecency).items.is_empty());
    }

    #[test]
    fn per_kind_caps_are_disclosed_and_frecency_stays_bounded() {
        let items: Vec<_> = (0..230)
            .map(|i| {
                item(
                    &format!("r{i}"),
                    ItemKind::Relation,
                    &format!("t{i:03}"),
                    "",
                )
            })
            .chain(
                (0..25).map(|i| item(&format!("s{i}"), ItemKind::Schema, &format!("t_s{i}"), "")),
            )
            .collect();
        let ranked = rank(&items, "t", &Frecency::default());
        assert_eq!(ranked.items.len(), 220);
        assert_eq!(
            ranked.truncated,
            [(ItemKind::Schema, 5), (ItemKind::Relation, 30)]
        );
        let mut frecency = Frecency::default();
        for i in 0..(FRECENCY_KEYS + 10) {
            frecency.record(&format!("k{i}"));
        }
        assert_eq!(frecency.entries.len(), FRECENCY_KEYS);
        assert_eq!(frecency.count("k0"), 0);
        assert_eq!(frecency.count(&format!("k{}", FRECENCY_KEYS + 9)), 1);
    }

    #[test]
    fn unicode_and_overloaded_objects_keep_exact_targets() {
        let reference = PgObjectRef {
            kind: PgObjectKind::Function,
            schema: Some("público".into()),
            name: "total".into(),
            identity_args: Some("text".into()),
        };
        let items = vec![Item::new(
            "object:c:fn".into(),
            ItemKind::Object,
            "total(text)".into(),
            "Function · público".into(),
            "público.total",
            Target::<u8>::Object {
                connection: "c".into(),
                reference: reference.clone(),
            },
        )];
        let ranked = rank(&items, "PÚBLICO total", &Frecency::default());
        assert_eq!(ranked.items, [0]);
        assert_eq!(
            items[0].target,
            Target::Object {
                connection: "c".into(),
                reference
            }
        );
    }
}
