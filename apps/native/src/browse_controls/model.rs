//! Pure filter-bar and sort models (Plan 032 P4). No GPUI: the bar renders
//! these and every applied change still leaves as a `BrowseEvent::Apply`.
use dbunk_lib::backend::data::*;

/// Conditions the filter bar edits at once; further applied conditions stay
/// applied and are listed as kept filters instead of rows.
pub const ROW_LIMIT: usize = 32;
/// Mirrors `BrowseState::validate`.
pub const SORT_LIMIT: usize = 256;
/// One condition value; larger drafts stay editable but cannot be applied.
pub const VALUE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    InList,
    IsNull,
    IsNotNull,
}

impl Operator {
    /// Menu order; the index is the legacy `build_filter` operator number.
    pub const ALL: [Operator; 13] = [
        Operator::Eq,
        Operator::Neq,
        Operator::Gt,
        Operator::Gte,
        Operator::Lt,
        Operator::Lte,
        Operator::Contains,
        Operator::NotContains,
        Operator::StartsWith,
        Operator::EndsWith,
        Operator::InList,
        Operator::IsNull,
        Operator::IsNotNull,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Operator::Eq => "equals",
            Operator::Neq => "not equal",
            Operator::Gt => "greater than",
            Operator::Gte => "greater or equal",
            Operator::Lt => "less than",
            Operator::Lte => "less or equal",
            Operator::Contains => "contains",
            Operator::NotContains => "not contains",
            Operator::StartsWith => "starts with",
            Operator::EndsWith => "ends with",
            Operator::InList => "in list",
            Operator::IsNull => "is null",
            Operator::IsNotNull => "is not null",
        }
    }

    /// The SQL the backend builds, shown as a badge beside the label.
    pub fn sql_hint(self) -> &'static str {
        match self {
            Operator::Eq => "=",
            Operator::Neq => "<>",
            Operator::Gt => ">",
            Operator::Gte => ">=",
            Operator::Lt => "<",
            Operator::Lte => "<=",
            Operator::Contains => "ILIKE %x%",
            Operator::NotContains => "NOT ILIKE %x%",
            Operator::StartsWith => "ILIKE x%",
            Operator::EndsWith => "ILIKE %x",
            Operator::InList => "IN",
            Operator::IsNull => "IS NULL",
            Operator::IsNotNull => "IS NOT NULL",
        }
    }

    pub fn takes_value(self) -> bool {
        !matches!(self, Operator::IsNull | Operator::IsNotNull)
    }

    pub fn index(self) -> usize {
        Operator::ALL
            .iter()
            .position(|operator| *operator == self)
            .expect("operator is listed")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterRowDraft {
    pub column: String,
    pub operator: Operator,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterDraft {
    pub rows: Vec<FilterRowDraft>,
}

impl FilterDraft {
    /// Splits applied filters into editable rows and filters the bar cannot
    /// represent exactly (raw SQL, values a row would trim or re-split).
    pub fn from_filters(filters: &[BrowseFilter]) -> (Self, Vec<BrowseFilter>) {
        let mut rows = Vec::new();
        let mut kept = Vec::new();
        for filter in filters {
            match row_for(filter) {
                Some(row) => rows.push(row),
                None => kept.push(filter.clone()),
            }
        }
        (Self { rows }, kept)
    }

    /// As `from_filters`, keeping at most `cap` rows; the filters of later
    /// rows are appended to the kept ones so none is lost.
    pub fn from_filters_capped(filters: &[BrowseFilter], cap: usize) -> (Self, Vec<BrowseFilter>) {
        let (mut draft, mut kept) = Self::from_filters(filters);
        if draft.rows.len() > cap {
            let overflow = Self {
                rows: draft.rows.split_off(cap),
            };
            // `row_for` only admits rows that rebuild to their filter.
            kept.extend(
                overflow
                    .to_filters()
                    .expect("representable rows rebuild their filters"),
            );
        }
        (draft, kept)
    }

    /// Every row as a typed filter, in row order. Several conditions on one
    /// column are kept and ANDed; the error names the first invalid row.
    pub fn to_filters(&self) -> Result<Vec<BrowseFilter>, (usize, &'static str)> {
        self.rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                build_filter(&row.column, row.operator.index(), &row.value)
                    .map_err(|error| (index, error))
            })
            .collect()
    }

    /// Whether applying would change the typed filters `base` was loaded
    /// from. Whitespace a row trims is not a change; an invalid row is.
    pub fn is_dirty(&self, base: &FilterDraft) -> bool {
        match (self.to_filters(), base.to_filters()) {
            (Ok(current), Ok(applied)) => current != applied,
            _ => self != base,
        }
    }
}

/// A row only when rebuilding it yields the identical filter, so editing
/// never silently rewrites an applied condition.
fn row_for(filter: &BrowseFilter) -> Option<FilterRowDraft> {
    let (column, operator, value) = match filter {
        BrowseFilter::Comparison {
            column,
            operator,
            value,
        } => (
            column,
            match operator {
                ComparisonOperator::Eq => Operator::Eq,
                ComparisonOperator::Neq => Operator::Neq,
                ComparisonOperator::Gt => Operator::Gt,
                ComparisonOperator::Gte => Operator::Gte,
                ComparisonOperator::Lt => Operator::Lt,
                ComparisonOperator::Lte => Operator::Lte,
            },
            value.clone(),
        ),
        BrowseFilter::TextMatch {
            column,
            operator,
            value,
        } => (
            column,
            match operator {
                TextMatchOperator::Contains => Operator::Contains,
                TextMatchOperator::NotContains => Operator::NotContains,
                TextMatchOperator::StartsWith => Operator::StartsWith,
                TextMatchOperator::EndsWith => Operator::EndsWith,
            },
            value.clone(),
        ),
        BrowseFilter::IsNull { column } => (column, Operator::IsNull, String::new()),
        BrowseFilter::IsNotNull { column } => (column, Operator::IsNotNull, String::new()),
        BrowseFilter::InList { column, values } => (column, Operator::InList, values.join(", ")),
        BrowseFilter::RawSql { .. } => return None,
    };
    build_filter(column, operator.index(), &value)
        .is_ok_and(|built| &built == filter)
        .then(|| FilterRowDraft {
            column: column.clone(),
            operator,
            value,
        })
}

pub fn build_filter(
    column: &str,
    operator: usize,
    input: &str,
) -> Result<BrowseFilter, &'static str> {
    if column.is_empty() || input.len() > VALUE_BYTES {
        return Err("Filter is empty or exceeds 64 KiB");
    }
    if operator == 11 {
        return Ok(BrowseFilter::IsNull {
            column: column.into(),
        });
    }
    if operator == 12 {
        return Ok(BrowseFilter::IsNotNull {
            column: column.into(),
        });
    }
    let value = input.trim();
    if value.is_empty() {
        return Err("Enter a filter value");
    }
    if operator == 10 {
        let values = value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return Err("Enter one or more comma-separated values");
        }
        return Ok(BrowseFilter::InList {
            column: column.into(),
            values,
        });
    }
    if operator >= 6 {
        let operator = match operator {
            6 => TextMatchOperator::Contains,
            7 => TextMatchOperator::NotContains,
            8 => TextMatchOperator::StartsWith,
            9 => TextMatchOperator::EndsWith,
            _ => return Err("Unknown operator"),
        };
        return Ok(BrowseFilter::TextMatch {
            column: column.into(),
            operator,
            value: value.into(),
        });
    }
    let operator = match operator {
        0 => ComparisonOperator::Eq,
        1 => ComparisonOperator::Neq,
        2 => ComparisonOperator::Gt,
        3 => ComparisonOperator::Gte,
        4 => ComparisonOperator::Lt,
        5 => ComparisonOperator::Lte,
        _ => return Err("Unknown operator"),
    };
    Ok(BrowseFilter::Comparison {
        column: column.into(),
        operator,
        value: value.into(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderSort {
    Asc,
    Desc,
    Clear,
}

/// Header-menu sort. Asc or Desc replaces the whole sort with that column
/// (keeping its NULL placement when it was already sorted); Clear removes
/// only that column.
pub fn header_sort(
    current: &[BrowseSortKey],
    column: &str,
    choice: HeaderSort,
) -> Vec<BrowseSortKey> {
    let direction = match choice {
        HeaderSort::Asc => BrowseSortDirection::Asc,
        HeaderSort::Desc => BrowseSortDirection::Desc,
        HeaderSort::Clear => {
            return current
                .iter()
                .filter(|key| key.column != column)
                .cloned()
                .collect();
        }
    };
    let nulls = current
        .iter()
        .find(|key| key.column == column)
        .map_or(BrowseNulls::Default, |key| key.nulls);
    vec![BrowseSortKey {
        column: column.into(),
        direction,
        nulls,
    }]
}

/// Sort-popover edits. Each returns whether `sort` changed; other keys keep
/// their order, direction and NULL placement.
pub mod sort_edit {
    use super::SORT_LIMIT;
    use dbunk_lib::backend::data::*;

    /// Appends an ascending key; refuses duplicates and the 256-key limit.
    pub fn append(sort: &mut Vec<BrowseSortKey>, column: &str) -> bool {
        if column.is_empty()
            || sort.len() >= SORT_LIMIT
            || sort.iter().any(|key| key.column == column)
        {
            return false;
        }
        sort.push(BrowseSortKey {
            column: column.into(),
            direction: BrowseSortDirection::Asc,
            nulls: BrowseNulls::Default,
        });
        true
    }

    pub fn toggle_direction(sort: &mut [BrowseSortKey], index: usize) -> bool {
        let Some(key) = sort.get_mut(index) else {
            return false;
        };
        key.direction = match key.direction {
            BrowseSortDirection::Asc => BrowseSortDirection::Desc,
            BrowseSortDirection::Desc => BrowseSortDirection::Asc,
        };
        true
    }

    /// Default, then NULLS FIRST, then NULLS LAST.
    pub fn cycle_nulls(sort: &mut [BrowseSortKey], index: usize) -> bool {
        let Some(key) = sort.get_mut(index) else {
            return false;
        };
        key.nulls = match key.nulls {
            BrowseNulls::Default => BrowseNulls::First,
            BrowseNulls::First => BrowseNulls::Last,
            BrowseNulls::Last => BrowseNulls::Default,
        };
        true
    }

    pub fn remove(sort: &mut Vec<BrowseSortKey>, index: usize) -> bool {
        if index >= sort.len() {
            return false;
        }
        sort.remove(index);
        true
    }

    pub fn move_up(sort: &mut [BrowseSortKey], index: usize) -> bool {
        if index == 0 || index >= sort.len() {
            return false;
        }
        sort.swap(index - 1, index);
        true
    }

    pub fn move_down(sort: &mut [BrowseSortKey], index: usize) -> bool {
        if index + 1 >= sort.len() {
            return false;
        }
        sort.swap(index, index + 1);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(column: &str, operator: Operator, value: &str) -> FilterRowDraft {
        FilterRowDraft {
            column: column.into(),
            operator,
            value: value.into(),
        }
    }

    fn key(column: &str, direction: BrowseSortDirection, nulls: BrowseNulls) -> BrowseSortKey {
        BrowseSortKey {
            column: column.into(),
            direction,
            nulls,
        }
    }

    fn one_of_each() -> Vec<BrowseFilter> {
        Operator::ALL
            .iter()
            .map(|operator| {
                let value = match operator {
                    Operator::InList => "1, 東京, x y",
                    _ => "a b",
                };
                build_filter("col", operator.index(), value).unwrap()
            })
            .collect()
    }

    #[test]
    fn every_operator_round_trips_through_rows() {
        let filters = one_of_each();
        let (draft, kept) = FilterDraft::from_filters(&filters);
        assert!(kept.is_empty());
        assert_eq!(draft.rows.len(), 13);
        for (row, operator) in draft.rows.iter().zip(Operator::ALL) {
            assert_eq!(row.operator, operator);
            assert!(!operator.label().is_empty() && !operator.sql_hint().is_empty());
        }
        assert_eq!(draft.to_filters().unwrap(), filters);
        assert_eq!(Operator::ALL[11].index(), 11);
        assert!(!Operator::IsNull.takes_value() && !Operator::IsNotNull.takes_value());
        assert!(Operator::InList.takes_value());
    }

    #[test]
    fn two_conditions_on_one_column_are_both_kept() {
        let draft = FilterDraft {
            rows: vec![
                row("total", Operator::Gt, "10"),
                row("total", Operator::Lt, "100"),
            ],
        };
        let filters = draft.to_filters().unwrap();
        assert_eq!(filters.len(), 2);
        let (back, kept) = FilterDraft::from_filters(&filters);
        assert!(kept.is_empty());
        assert_eq!(back, draft);
    }

    #[test]
    fn invalid_rows_report_their_index() {
        let draft = FilterDraft {
            rows: vec![
                row("id", Operator::Eq, "1"),
                row("id", Operator::InList, " , ,"),
            ],
        };
        assert_eq!(
            draft.to_filters(),
            Err((1, "Enter one or more comma-separated values"))
        );
        let blank = FilterDraft {
            rows: vec![row("id", Operator::Eq, "  ")],
        };
        assert_eq!(blank.to_filters(), Err((0, "Enter a filter value")));
        let oversized = FilterDraft {
            rows: vec![row("id", Operator::Eq, &"x".repeat(VALUE_BYTES + 1))],
        };
        assert!(matches!(oversized.to_filters(), Err((0, _))));
    }

    #[test]
    fn null_operators_ignore_the_value() {
        let draft = FilterDraft {
            rows: vec![
                row("name", Operator::IsNull, "ignored"),
                row("name", Operator::IsNotNull, ""),
            ],
        };
        assert_eq!(
            draft.to_filters().unwrap(),
            vec![
                BrowseFilter::IsNull {
                    column: "name".into()
                },
                BrowseFilter::IsNotNull {
                    column: "name".into()
                },
            ]
        );
    }

    #[test]
    fn raw_sql_and_lossy_filters_stay_out_of_the_rows() {
        let raw = BrowseFilter::RawSql {
            text: "id > 1".into(),
        };
        let comma = BrowseFilter::InList {
            column: "tag".into(),
            values: vec!["a,b".into(), "c".into()],
        };
        let padded = BrowseFilter::Comparison {
            column: "name".into(),
            operator: ComparisonOperator::Eq,
            value: " padded ".into(),
        };
        let plain = build_filter("id", 0, "7").unwrap();
        let (draft, kept) =
            FilterDraft::from_filters(&[raw.clone(), plain, comma.clone(), padded.clone()]);
        assert_eq!(draft.rows, vec![row("id", Operator::Eq, "7")]);
        assert_eq!(kept, vec![raw, comma, padded]);
    }

    #[test]
    fn rows_beyond_the_cap_are_kept_not_dropped() {
        let filters = (0..ROW_LIMIT + 3)
            .map(|index| build_filter("id", 0, &index.to_string()).unwrap())
            .collect::<Vec<_>>();
        let (draft, kept) = FilterDraft::from_filters_capped(&filters, ROW_LIMIT);
        assert_eq!(draft.rows.len(), ROW_LIMIT);
        assert_eq!(kept, filters[ROW_LIMIT..].to_vec());
        let mut rebuilt = draft.to_filters().unwrap();
        rebuilt.extend(kept);
        assert_eq!(rebuilt, filters);
    }

    #[test]
    fn dirty_detection_compares_applied_filters() {
        let (base, _) = FilterDraft::from_filters(&[build_filter("id", 2, "250").unwrap()]);
        let mut draft = base.clone();
        assert!(!draft.is_dirty(&base));
        draft.rows[0].value = " 250 ".into();
        assert!(!draft.is_dirty(&base), "trimmed whitespace is not a change");
        draft.rows[0].value = "251".into();
        assert!(draft.is_dirty(&base));
        draft.rows[0].value = "250".into();
        draft.rows[0].operator = Operator::Gte;
        assert!(draft.is_dirty(&base));
        let mut added = base.clone();
        added.rows.push(row("id", Operator::Eq, ""));
        assert!(added.is_dirty(&base), "a new blank row is unapplied");
        let mut removed = base.clone();
        removed.rows.clear();
        assert!(removed.is_dirty(&base));
        assert!(!FilterDraft::default().is_dirty(&FilterDraft::default()));
    }

    #[test]
    fn header_sort_replaces_or_clears_one_column() {
        use BrowseNulls::{Default as Unset, First, Last};
        use BrowseSortDirection::{Asc, Desc};
        let current = vec![
            key("a", Asc, Unset),
            key("b", Desc, Last),
            key("c", Asc, First),
        ];
        assert_eq!(
            header_sort(&current, "b", HeaderSort::Asc),
            vec![key("b", Asc, Last)]
        );
        assert_eq!(
            header_sort(&current, "z", HeaderSort::Desc),
            vec![key("z", Desc, Unset)]
        );
        assert_eq!(
            header_sort(&current, "b", HeaderSort::Clear),
            vec![key("a", Asc, Unset), key("c", Asc, First)]
        );
        assert_eq!(header_sort(&current, "z", HeaderSort::Clear), current);
        assert!(header_sort(&[], "z", HeaderSort::Clear).is_empty());
    }

    #[test]
    fn sort_edits_keep_order_and_null_placement() {
        use BrowseNulls::{Default as Unset, First, Last};
        use BrowseSortDirection::{Asc, Desc};
        let mut sort = vec![key("a", Asc, First), key("b", Desc, Last)];
        assert!(sort_edit::append(&mut sort, "c"));
        assert!(!sort_edit::append(&mut sort, "a"), "duplicates refused");
        assert!(!sort_edit::append(&mut sort, ""));
        assert_eq!(sort[2], key("c", Asc, Unset));
        assert!(sort_edit::toggle_direction(&mut sort, 0));
        assert_eq!(sort[0], key("a", Desc, First));
        assert!(sort_edit::cycle_nulls(&mut sort, 2));
        assert_eq!(sort[2].nulls, First);
        assert!(sort_edit::cycle_nulls(&mut sort, 2));
        assert_eq!(sort[2].nulls, Last);
        assert!(sort_edit::cycle_nulls(&mut sort, 2));
        assert_eq!(sort[2].nulls, Unset);
        assert!(
            !sort_edit::move_up(&mut sort, 0),
            "first key cannot move up"
        );
        assert!(
            !sort_edit::move_down(&mut sort, 2),
            "last key cannot move down"
        );
        assert!(sort_edit::move_up(&mut sort, 1));
        assert_eq!(
            sort,
            vec![
                key("b", Desc, Last),
                key("a", Desc, First),
                key("c", Asc, Unset)
            ]
        );
        assert!(sort_edit::move_down(&mut sort, 0));
        assert_eq!(sort[1], key("b", Desc, Last));
        assert!(sort_edit::remove(&mut sort, 0));
        assert_eq!(sort, vec![key("b", Desc, Last), key("c", Asc, Unset)]);
        assert!(!sort_edit::remove(&mut sort, 5));
        assert!(!sort_edit::toggle_direction(&mut sort, 5));
        assert!(!sort_edit::cycle_nulls(&mut sort, 5));
    }

    #[test]
    fn sort_append_respects_the_key_limit() {
        let mut sort = (0..SORT_LIMIT)
            .map(|index| {
                key(
                    &index.to_string(),
                    BrowseSortDirection::Asc,
                    BrowseNulls::Default,
                )
            })
            .collect::<Vec<_>>();
        assert!(!sort_edit::append(&mut sort, "extra"));
        assert_eq!(sort.len(), SORT_LIMIT);
        sort.pop();
        assert!(sort_edit::append(&mut sort, "extra"));
        assert_eq!(sort.len(), SORT_LIMIT);
    }
}
