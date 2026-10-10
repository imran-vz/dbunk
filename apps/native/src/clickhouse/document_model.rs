//! Pure state for ClickHouse documents: data paging and sort, conversion of a
//! bounded result into the shared grid's page shape, and status text.
use crate::structure_table::{
    ColumnSpec, SectionTable, Tone, clip, faint, fill, fixed, mono, optional, or_dash, ordinal,
    tag, tags, text,
};
use crate::ui::{grouped, offset_range_label};
use dbunk_lib::backend::clickhouse::{
    ClickHouseError, ClickHouseRows, ClickHouseStructure, ClickHouseTruncation,
};
use dbunk_lib::backend::data::{
    BrowseColumn, BrowseCount, BrowseCountKind, BrowseIdentity, BrowseIdentityKind,
    BrowseInspection, BrowsePageInfo, BrowsePageMode, BrowseTableResult,
};

/// Rows per data page. One extra row is requested to learn whether more
/// follow, so this stays below the backend's per-page bound.
pub const PAGE_ROWS: usize = 100;
/// Longest error text kept in a document.
const ERROR_CHARS: usize = 4_000;

/// Offset paging with an optional single-column sort.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paging {
    pub offset: u64,
    /// Column and descending flag.
    pub sort: Option<(String, bool)>,
}

impl Paging {
    pub fn page(&self) -> u64 {
        self.offset / PAGE_ROWS as u64 + 1
    }
    pub fn next(&mut self, has_more: bool) -> bool {
        if !has_more {
            return false;
        }
        self.offset += PAGE_ROWS as u64;
        true
    }
    pub fn previous(&mut self) -> bool {
        if self.offset == 0 {
            return false;
        }
        self.offset = self.offset.saturating_sub(PAGE_ROWS as u64);
        true
    }
    /// Header click: ascending, then descending, then unsorted. Any sort
    /// change returns to the first page.
    pub fn sort_by(&mut self, column: &str) {
        self.sort = match self.sort.take() {
            Some((current, false)) if current == column => Some((current, true)),
            Some((current, true)) if current == column => None,
            _ => Some((column.to_owned(), false)),
        };
        self.offset = 0;
    }
}

/// Splits the look-ahead row off a data page.
pub fn take_page(mut rows: ClickHouseRows) -> (ClickHouseRows, bool) {
    let has_more = rows.rows.len() > PAGE_ROWS;
    rows.rows.truncate(PAGE_ROWS);
    (rows, has_more)
}

/// The shared grid's page shape. ClickHouse rows have no identity, so the
/// grid stays read-only.
pub fn grid_page(
    rows: ClickHouseRows,
    request_id: u64,
    page: Option<u32>,
    has_more: bool,
    sql: String,
) -> BrowseTableResult {
    BrowseTableResult {
        request_id,
        columns: rows
            .columns
            .into_iter()
            .map(|column| BrowseColumn {
                nullable: column.type_name.starts_with("Nullable("),
                name: column.name,
                cast_type: column.type_name,
            })
            .collect(),
        rows: rows.rows,
        identity: BrowseIdentity {
            kind: BrowseIdentityKind::None,
            columns: Vec::new(),
        },
        row_identity: None,
        page_info: BrowsePageInfo {
            mode: BrowsePageMode::Offset,
            page,
            has_more,
            next_cursor: None,
        },
        count: BrowseCount {
            kind: BrowseCountKind::Unknown,
            value: None,
        },
        inspection: BrowseInspection {
            sql,
            params: Vec::new(),
        },
        omitted_rows: 0,
        truncated_cells: 0,
        runtime_ms: rows.runtime_ms,
    }
}

fn plural(value: usize, one: &str) -> String {
    format!(
        "{} {one}{}",
        grouped(value as u64),
        if value == 1 { "" } else { "s" }
    )
}

/// Footer text for a query result, naming any bound that stopped it.
pub fn query_summary(rows: &ClickHouseRows) -> String {
    let runtime = format!("{} ms", rows.runtime_ms);
    if rows.columns.is_empty() {
        return match rows.written_rows {
            Some(written) => format!("{} written · {runtime}", plural(written as usize, "row")),
            None => format!("Statement completed · {runtime}"),
        };
    }
    let shown = plural(rows.rows.len(), "row");
    match rows.truncated {
        None => format!("{shown} · {runtime}"),
        Some(ClickHouseTruncation::Rows) => {
            format!("First {shown}; more were not read (row limit) · {runtime}")
        }
        Some(ClickHouseTruncation::Bytes) => {
            format!("First {shown}; more were not read (size limit) · {runtime}")
        }
    }
}

/// Appended to a data page's footer when the table has no sorting key and no
/// column is chosen: ClickHouse returns rows in no fixed order, so offset
/// pages may repeat or skip rows.
pub const APPROXIMATE_ORDER_NOTE: &str =
    " · Order is approximate (no sorting key); sort a column for stable pages";

/// Footer text for a data page.
pub fn page_summary(paging: &Paging, shown: usize, has_more: bool, runtime_ms: u64) -> String {
    if shown == 0 {
        return if paging.offset == 0 {
            format!("No rows · {runtime_ms} ms")
        } else {
            format!(
                "No rows after row {} · {runtime_ms} ms",
                grouped(paging.offset)
            )
        };
    }
    format!(
        "Rows {}{} · {runtime_ms} ms",
        offset_range_label(paging.offset, shown),
        if has_more { " of more" } else { "" }
    )
}

/// Error text for a document: bounded, first line first.
pub fn error_text(error: &ClickHouseError) -> String {
    let text = error.message.trim();
    if text.chars().count() <= ERROR_CHARS {
        return text.to_owned();
    }
    let mut kept = text.chars().take(ERROR_CHARS).collect::<String>();
    kept.push('…');
    kept
}

pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024. && unit < UNITS.len() - 1 {
        size /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

const OVERVIEW: &[ColumnSpec] = &[
    mono(fill("Engine", 140.)),
    mono(fill("Sorting key", 140.)),
    mono(fill("Partition by", 120.)),
    mono(fill("Sample by", 100.)),
    fixed("Rows", 110.),
    fixed("Size", 90.),
];
const COLUMNS: &[ColumnSpec] = &[
    mono(fixed("#", 40.)),
    fill("Name", 140.),
    mono(fill("Type", 140.)),
    mono(fill("Default", 140.)),
    fixed("Keys", 110.),
];
const SKIP_INDEXES: &[ColumnSpec] = &[
    fill("Name", 160.),
    mono(fill("Expression", 200.)),
    fixed("Type", 120.),
];
const CONSTRAINTS: &[ColumnSpec] = &[fill("Name", 160.), mono(fill("Check", 260.))];

/// The Structure page's section tables, in the PostgreSQL page's order.
pub fn structure_tables(structure: &ClickHouseStructure) -> Vec<SectionTable> {
    let overview = vec![
        or_dash(&structure.engine),
        or_dash(&structure.sorting_key.join(", ")),
        or_dash(structure.partition_by.as_deref().unwrap_or_default()),
        or_dash(structure.sample_by.as_deref().unwrap_or_default()),
        structure
            .total_rows
            .map_or_else(|| faint("—"), |rows| text(grouped(rows))),
        structure
            .total_bytes
            .map_or_else(|| faint("—"), |size| text(bytes(size))),
    ];
    let columns = structure
        .columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            vec![
                ordinal(index),
                text(clip(&column.name)),
                text(clip(&column.type_name)),
                optional(&column.default),
                tags([column.in_sorting_key.then(|| tag("sorting key", Tone::Key))]),
            ]
        })
        .collect();
    let skip_indexes = structure
        .skip_indexes
        .iter()
        .map(|index| {
            vec![
                text(clip(&index.name)),
                text(clip(&index.expression)),
                text(clip(&index.kind)),
            ]
        })
        .collect();
    let constraints = structure
        .constraints
        .iter()
        .map(|(name, check)| vec![text(clip(name)), text(clip(check))])
        .collect();
    vec![
        SectionTable::new("Overview", OVERVIEW, vec![overview], ""),
        SectionTable::new("Columns", COLUMNS, columns, "No columns"),
        SectionTable::new(
            "Skip indexes",
            SKIP_INDEXES,
            skip_indexes,
            "No skip indexes",
        ),
        SectionTable::new(
            "Constraints",
            CONSTRAINTS,
            constraints,
            "No check constraints",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::clickhouse::{ClickHouseColumn, ClickHouseErrorKind};

    fn result(rows: usize) -> ClickHouseRows {
        ClickHouseRows {
            columns: vec![ClickHouseColumn {
                name: "n".into(),
                type_name: "Nullable(UInt64)".into(),
            }],
            rows: (0..rows).map(|n| vec![Some(n.to_string())]).collect(),
            runtime_ms: 7,
            ..Default::default()
        }
    }

    #[test]
    fn paging_moves_by_pages_and_sort_cycles_back_to_first_page() {
        let mut paging = Paging::default();
        assert!(!paging.previous());
        assert!(!paging.next(false));
        assert!(paging.next(true));
        assert_eq!((paging.offset, paging.page()), (PAGE_ROWS as u64, 2));
        paging.sort_by("n");
        assert_eq!(
            paging,
            Paging {
                offset: 0,
                sort: Some(("n".into(), false))
            }
        );
        paging.next(true);
        paging.sort_by("n");
        assert_eq!(
            paging,
            Paging {
                offset: 0,
                sort: Some(("n".into(), true))
            }
        );
        paging.sort_by("n");
        assert_eq!(paging.sort, None);
        paging.sort_by("a");
        paging.sort_by("b");
        assert_eq!(paging.sort, Some(("b".into(), false)));
    }

    #[test]
    fn look_ahead_row_decides_has_more_and_is_never_shown() {
        let (page, more) = take_page(result(PAGE_ROWS + 1));
        assert!(more);
        assert_eq!(page.rows.len(), PAGE_ROWS);
        let (page, more) = take_page(result(3));
        assert!(!more);
        let grid = grid_page(page, 4, Some(1), more, "SELECT".into());
        assert_eq!(grid.request_id, 4);
        assert!(grid.columns[0].nullable);
        assert_eq!(grid.identity.kind, BrowseIdentityKind::None);
        assert_eq!(grid.rows.len(), 3);
    }

    #[test]
    fn summaries_name_the_bound_that_stopped_a_result() {
        assert_eq!(query_summary(&result(1)), "1 row · 7 ms");
        let mut capped = result(10_000);
        capped.truncated = Some(ClickHouseTruncation::Rows);
        assert_eq!(
            query_summary(&capped),
            "First 10,000 rows; more were not read (row limit) · 7 ms"
        );
        capped.truncated = Some(ClickHouseTruncation::Bytes);
        assert!(query_summary(&capped).contains("size limit"));
        let written = ClickHouseRows {
            written_rows: Some(1_234),
            runtime_ms: 3,
            ..Default::default()
        };
        assert_eq!(query_summary(&written), "1,234 rows written · 3 ms");
        assert_eq!(
            query_summary(&ClickHouseRows::default()),
            "Statement completed · 0 ms"
        );
        let paging = Paging {
            offset: 100,
            sort: None,
        };
        assert_eq!(
            page_summary(&paging, 100, true, 5),
            "Rows 101–200 of more · 5 ms"
        );
        assert_eq!(
            page_summary(&paging, 0, false, 5),
            "No rows after row 100 · 5 ms"
        );
    }

    #[test]
    fn errors_and_sizes_are_bounded_text() {
        let long = ClickHouseError {
            kind: ClickHouseErrorKind::Server,
            message: "x".repeat(ERROR_CHARS + 10),
        };
        assert_eq!(error_text(&long).chars().count(), ERROR_CHARS + 1);
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
    }

    #[test]
    fn structure_tables_show_engine_keys_and_one_cell_per_column() {
        use crate::structure_table::Shown;
        use dbunk_lib::backend::clickhouse::{ClickHouseSkipIndex, ClickHouseStructureColumn};
        let structure = ClickHouseStructure {
            engine: "MergeTree".into(),
            total_rows: Some(18_204),
            total_bytes: Some(2048),
            sorting_key: vec!["day".into(), "id".into()],
            columns: vec![
                ClickHouseStructureColumn {
                    name: "day".into(),
                    type_name: "Date".into(),
                    default: None,
                    in_sorting_key: true,
                },
                ClickHouseStructureColumn {
                    name: "note".into(),
                    type_name: "String".into(),
                    default: Some("DEFAULT ''".into()),
                    in_sorting_key: false,
                },
            ],
            skip_indexes: vec![ClickHouseSkipIndex {
                name: "note_idx".into(),
                expression: "note".into(),
                kind: "bloom_filter".into(),
            }],
            ..Default::default()
        };
        let tables = structure_tables(&structure);
        let overview = &tables[0].rows()[0];
        assert_eq!(overview[1], Shown::Text("day, id".into()));
        assert_eq!(overview[2], Shown::Faint("—".into()));
        assert_eq!(overview[4], Shown::Text("18,204".into()));
        assert_eq!(overview[5], Shown::Text("2.0 KiB".into()));
        assert_eq!(
            tables[1].rows()[0][4],
            Shown::Tags(vec![crate::structure_table::tag("sorting key", Tone::Key)])
        );
        assert_eq!(tables[1].rows()[1][4], Shown::Tags(Vec::new()));
        assert_eq!(tables[2].rows().len(), 1);
        assert!(tables[3].rows().is_empty());
    }
}
