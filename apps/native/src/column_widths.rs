//! Baseline grid sizing uses UTF-16 character counts and the first displayed
//! line. Stop counting once the width clamp makes further text irrelevant.
use std::{borrow::Cow, ops::Range};

pub const SAMPLE_ROWS: usize = 100;
pub const MIN_WIDTH: f32 = 60.;
pub const INITIAL_MAX: f32 = 400.;
pub const AUTO_FIT_MAX: f32 = 500.;
const CHARACTER_WIDTH: f32 = 7.3;
const CELL_CHROME: f32 = 18.;
const MAX_UNITS: usize = 70;

pub fn heading(name: Option<&str>, index: usize) -> Cow<'_, str> {
    name.map(Cow::Borrowed)
        .unwrap_or_else(|| Cow::Owned(format!("Column {} (name omitted)", index + 1)))
}

fn width(units: usize, maximum: f32) -> f32 {
    (units as f32 * CHARACTER_WIDTH + CELL_CHROME)
        .clamp(MIN_WIDTH, maximum)
        .round()
}

pub fn header_width(name: &str, maximum: f32) -> f32 {
    width(name.encode_utf16().take(MAX_UNITS).count() + 3, maximum)
}

fn cell_width(text: &str, maximum: f32) -> f32 {
    let mut units = 0;
    for unit in text.encode_utf16().take(MAX_UNITS) {
        if unit == u16::from(b'\n') {
            units += 2;
            break;
        }
        units += 1;
    }
    width(units, maximum)
}

/// Fit only already-retained cells. No value clones or database reads occur.
pub fn fit<'a>(name: &str, cells: impl Iterator<Item = Option<&'a str>>, maximum: f32) -> f32 {
    let mut longest = header_width(name, maximum);
    if longest >= maximum {
        return longest;
    }
    for cell in cells.flatten() {
        longest = longest.max(cell_width(cell, maximum));
        if longest >= maximum {
            break;
        }
    }
    longest
}

/// Query-result geometry is admitted with column metadata. Each result set owns
/// its widths; explicit auto-fit is not overwritten by later stream samples.
#[derive(Default)]
pub struct ColumnWidths {
    widths: Vec<f32>,
    offsets: Vec<f32>,
    explicit: Vec<bool>,
    // Query layouts use source indices, including duplicate/omitted headings.
    order: Vec<usize>,
    pinned: usize,
}
impl ColumnWidths {
    pub fn storage_bytes(columns: usize) -> usize {
        columns
            .saturating_mul(2 * size_of::<f32>() + size_of::<bool>() + size_of::<usize>())
            .saturating_add(size_of::<f32>())
    }
    pub fn new(columns: &[Option<String>]) -> Self {
        let mut widths = Vec::with_capacity(columns.len());
        for (index, column) in columns.iter().enumerate() {
            widths.push(header_width(
                &heading(column.as_deref(), index),
                INITIAL_MAX,
            ));
        }
        let mut result = Self {
            widths,
            offsets: Vec::with_capacity(columns.len() + 1),
            explicit: vec![false; columns.len()],
            order: (0..columns.len()).collect(),
            pinned: 0,
        };
        result.rebuild_offsets();
        result
    }
    pub fn sample(&mut self, row: &[Option<String>], retained_index: usize) -> bool {
        if retained_index >= SAMPLE_ROWS {
            return false;
        }
        let mut changed = false;
        for (index, cell) in row.iter().enumerate().take(self.widths.len()) {
            if !self.explicit[index]
                && self.widths[index] < INITIAL_MAX
                && let Some(value) = cell
            {
                let next = self.widths[index].max(cell_width(value, INITIAL_MAX));
                changed |= next != self.widths[index];
                self.widths[index] = next;
            }
        }
        changed
    }
    /// Width samples and explicit fits always address the original source index.
    pub fn set_explicit(&mut self, column: usize, width: f32) {
        if let Some(current) = self.widths.get_mut(column) {
            *current = width.clamp(MIN_WIDTH, AUTO_FIT_MAX);
            self.explicit[column] = true;
        }
    }
    pub fn rebuild_offsets(&mut self) {
        self.offsets.clear();
        self.offsets.push(0.);
        for source in &self.order {
            self.offsets
                .push(self.offsets.last().unwrap() + self.widths[*source]);
        }
    }
    pub fn width(&self, column: usize) -> f32 {
        self.widths[self.order[column]]
    }
    pub fn source(&self, display: usize) -> Option<usize> {
        self.order.get(display).copied()
    }
    pub fn display(&self, source: usize) -> Option<usize> {
        self.order.iter().position(|item| *item == source)
    }
    pub fn pinned_count(&self) -> usize {
        self.pinned
    }
    /// Pins append in selection order; unpin restores source order among the
    /// remaining unpinned columns. The admitted vectors never grow here.
    pub fn toggle_pin(&mut self, display: usize) -> bool {
        if display >= self.order.len() {
            return false;
        }
        let source = self.order.remove(display);
        if display < self.pinned {
            self.pinned -= 1;
            let position =
                self.pinned + self.order[self.pinned..].partition_point(|item| *item < source);
            self.order.insert(position, source);
        } else {
            self.order.insert(self.pinned, source);
            self.pinned += 1;
        }
        self.rebuild_offsets();
        true
    }
    pub fn offset(&self, column: usize) -> f32 {
        self.offsets[column]
    }
    pub fn total_width(&self) -> f32 {
        self.offsets.last().copied().unwrap_or(0.)
    }
    pub fn visible_range(&self, left: f32, width: f32) -> Range<usize> {
        let first = self
            .offsets
            .partition_point(|offset| *offset < left.max(0.))
            .saturating_sub(2);
        let last = self
            .offsets
            .partition_point(|offset| *offset <= left.max(0.) + width.max(0.));
        first.min(self.widths.len())..last.saturating_add(1).min(self.widths.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn baseline_clamps_utf16_and_first_line_do_not_scan_large_values() {
        assert_eq!(fit("c", [Some("ab")].into_iter(), AUTO_FIT_MAX), 60.);
        assert_eq!(
            fit(
                "c",
                [Some("1234567890\nlonger second line ignored")].into_iter(),
                AUTO_FIT_MAX
            ),
            106.
        );
        assert_eq!(
            fit("c", [Some("😀😀😀😀😀")].into_iter(), AUTO_FIT_MAX),
            91.
        );
        assert_eq!(
            fit(
                "c",
                [Some("w".repeat(1_000_000).as_str())].into_iter(),
                AUTO_FIT_MAX
            ),
            500.
        );
        assert_eq!(
            fit(
                "c",
                [Some("w".repeat(1000).as_str())].into_iter(),
                INITIAL_MAX
            ),
            400.
        );
        assert_eq!(fit("c", [None, Some("")].into_iter(), AUTO_FIT_MAX), 60.);
    }
    #[test]
    fn samples_stop_at_100_and_explicit_width_survives_later_stream_rows() {
        let mut widths = ColumnWidths::new(&[Some("a".into()), Some("b".into())]);
        widths.sample(&[Some("1234567890".into()), None], 99);
        widths.sample(&[Some("x".repeat(100)), None], 100);
        widths.rebuild_offsets();
        assert_eq!(widths.width(0), 91.);
        assert_eq!(widths.offset(1), 91.);
        widths.set_explicit(0, 60.);
        widths.sample(&[Some("x".repeat(100)), None], 1);
        widths.rebuild_offsets();
        assert_eq!(widths.total_width(), 120.);
        assert_eq!(widths.visible_range(70., 20.), 0..2);
        let actual = widths.widths.capacity() * size_of::<f32>()
            + widths.offsets.capacity() * size_of::<f32>()
            + widths.explicit.capacity() * size_of::<bool>()
            + widths.order.capacity() * size_of::<usize>();
        assert!(actual <= ColumnWidths::storage_bytes(2));
    }
    #[test]
    fn duplicate_query_headings_pin_by_source_and_width_sampling_keeps_identity() {
        let mut layout = ColumnWidths::new(&[Some("x".into()), Some("x".into()), None]);
        layout.set_explicit(0, 80.);
        layout.set_explicit(1, 120.);
        assert!(layout.toggle_pin(2));
        assert!(layout.toggle_pin(2));
        assert_eq!(layout.order, [2, 1, 0]);
        assert_eq!(layout.pinned_count(), 2);
        assert_eq!(layout.width(1), 120.);
        assert_eq!(layout.source(2), Some(0));
        layout.sample(&[Some("long".repeat(60)), None, Some("long".repeat(60))], 0);
        layout.rebuild_offsets();
        assert_eq!(layout.width(0), 400.);
        assert_eq!(layout.offset(2), 520.);
        assert!(layout.toggle_pin(0));
        assert_eq!(layout.order, [1, 0, 2]);
        assert_eq!(layout.display(0), Some(1));
        assert!(layout.toggle_pin(0));
        assert_eq!(layout.order, [0, 1, 2]);
        assert!(!layout.toggle_pin(3));
        assert_eq!(layout.pinned_count(), 0);
    }
}
