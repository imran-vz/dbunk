use super::*;
use std::{io, mem::size_of};
pub(super) fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && !s.contains('\0')
}
pub(super) fn encoded(value: &impl serde::Serialize, limit: usize) -> Option<usize> {
    struct Count {
        bytes: usize,
        limit: usize,
    }
    impl io::Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes = self
                .bytes
                .checked_add(bytes.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| io::Error::other("export bound"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { bytes: 0, limit };
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.bytes)
}
pub(super) fn row_limit(columns: usize) -> usize {
    MAX_TABLE_EXPORT_CELLS
        .checked_div(columns)
        .map_or(MAX_TABLE_EXPORT_ROWS, |cells| {
            MAX_TABLE_EXPORT_ROWS.min(cells.saturating_sub(1))
        })
}
pub(super) fn header_heap(data: &TableExportData) -> Option<usize> {
    if data.connection_id.is_empty()
        || data.connection_id.len() > 128
        || data.connection_id.contains('\0')
        || !name(&data.database)
        || !name(&data.schema)
        || !name(&data.table)
        || data.identity.database_oid == 0
        || data.identity.relation_oid == 0
        || data.schema_oid == 0
        || data.captured_start.len() > 64
        || data.captured_end.len() > 64
        || data.columns.len() > MAX_TABLE_EXPORT_COLUMNS
    {
        return None;
    }
    let mut heap = size_of::<TableExportData>() + 2 * size_of::<usize>();
    for value in [
        &data.connection_id,
        &data.database,
        &data.schema,
        &data.table,
        &data.captured_start,
        &data.captured_end,
    ] {
        heap = heap.checked_add(value.capacity())?;
    }
    heap = heap.checked_add(
        data.columns
            .capacity()
            .checked_mul(size_of::<TableExportColumn>())?,
    )?;
    let mut attnum = 0;
    for column in &data.columns {
        if !name(&column.name) || column.attnum <= attnum || column.type_oid == 0 {
            return None;
        }
        attnum = column.attnum;
        heap = heap.checked_add(column.name.capacity())?;
    }
    Some(heap)
}
pub(super) fn checked(data: &TableExportData) -> Option<usize> {
    let mut heap = header_heap(data)?;
    if data.rows.len() > row_limit(data.columns.len()) {
        return None;
    }
    heap = heap.checked_add(
        data.rows
            .capacity()
            .checked_mul(size_of::<Vec<Option<String>>>())?,
    )?;
    let mut text = data.columns.iter().map(|c| c.name.len()).sum::<usize>();
    for row in &data.rows {
        if row.len() != data.columns.len() {
            return None;
        }
        heap = heap.checked_add(row.capacity().checked_mul(size_of::<Option<String>>())?)?;
        for value in row.iter().flatten() {
            if value.len() > MAX_TABLE_EXPORT_FIELD_BYTES {
                return None;
            }
            text = text.checked_add(value.len())?;
            heap = heap.checked_add(value.capacity())?;
        }
    }
    if text > MAX_TABLE_EXPORT_TEXT_BYTES || heap > MAX_TABLE_EXPORT_HEAP_BYTES {
        return None;
    }
    data.encoded_bytes()?;
    Some(heap)
}
