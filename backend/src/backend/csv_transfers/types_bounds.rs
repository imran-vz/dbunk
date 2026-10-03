use super::types::*;
use std::mem::size_of;
fn text(value: &str, max: usize) -> bool {
    value.len() <= max && !value.contains('\0')
}
fn add(n: &mut usize, value: usize) -> Option<()> {
    *n = n.checked_add(value)?;
    Some(())
}
fn string(n: &mut usize, value: &String, max: usize) -> Option<()> {
    if !text(value, max) {
        return None;
    }
    add(n, value.capacity())
}
pub(super) fn encoded<T: serde::Serialize>(value: &T, limit: usize) -> Option<usize> {
    struct Count {
        used: usize,
        limit: usize,
    }
    impl std::io::Write for Count {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.used = self
                .used
                .checked_add(b.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("CSV bound"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count { used: 0, limit };
    serde_json::to_writer(&mut count, value).ok()?;
    Some(count.used)
}
impl CsvInspectionData {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.source_columns.len() > MAX_CSV_COLUMNS
            || self.target_columns.len() > MAX_CSV_COLUMNS
            || self.sample_rows.len() > 50
        {
            return None;
        }
        let mut n = size_of::<Self>();
        if let Some(source) = &self.workbook {
            if self.direction != CsvDirection::Import {
                return None;
            }
            add(&mut n, source.checked_heap_bytes()?)?;
        }
        string(&mut n, &self.connection_id, 128)?;
        add(&mut n, self.target.checked_heap_bytes()?)?;
        add(&mut n, self.options.checked_heap_bytes()?)?;
        add(&mut n, self.connection.checked_heap_bytes()?)?;
        string(&mut n, &self.expires_at, 64)?;
        if let Some(name) = &self.file_name {
            string(&mut n, name, 1024)?;
        }
        add(
            &mut n,
            self.source_columns
                .capacity()
                .checked_mul(size_of::<CsvSourceColumn>())?,
        )?;
        let mut labels = 0usize;
        for (i, column) in self.source_columns.iter().enumerate() {
            if column.index != i {
                return None;
            }
            labels = labels.checked_add(column.name.len())?;
            string(&mut n, &column.name, 64 * 1024)?;
        }
        if labels > 64 * 1024 {
            return None;
        }
        add(
            &mut n,
            self.target_columns
                .capacity()
                .checked_mul(size_of::<CsvTargetColumn>())?,
        )?;
        let mut metadata = 0usize;
        for (i, column) in self.target_columns.iter().enumerate() {
            if !identifier(&column.name)
                || self.target_columns[..i]
                    .iter()
                    .any(|c| c.name == column.name)
            {
                return None;
            }
            string(&mut n, &column.name, 63)?;
            string(&mut n, &column.data_type, 8192)?;
            metadata = metadata
                .checked_add(column.name.len())?
                .checked_add(column.data_type.len())?;
        }
        if metadata > 256 * 1024 {
            return None;
        }
        add(
            &mut n,
            self.sample_rows
                .capacity()
                .checked_mul(size_of::<Vec<Option<String>>>())?,
        )?;
        let mut values = 0usize;
        for row in &self.sample_rows {
            if row.len() != self.source_columns.len() {
                return None;
            }
            add(
                &mut n,
                row.capacity().checked_mul(size_of::<Option<String>>())?,
            )?;
            for value in row.iter().flatten() {
                values = values.checked_add(value.len())?;
                add(&mut n, value.capacity())?;
            }
        }
        if values > 64 * 1024 || n > MAX_CSV_INSPECTION_BYTES {
            return None;
        }
        encoded(self, MAX_CSV_INSPECTION_BYTES)?;
        Some(n)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self, MAX_CSV_INSPECTION_BYTES)
    }
}
impl CsvInspectionObservation {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut n = size_of::<Self>();
        string(&mut n, &self.connection_id, 128)?;
        add(&mut n, self.target.checked_heap_bytes()?)?;
        if let Some(expiry) = &self.expires_at {
            string(&mut n, expiry, 64)?;
        }
        if let Some(d) = &self.diagnostic {
            add(&mut n, d.checked_heap_bytes()?)?;
        }
        Some(n)
    }
}
impl CsvTransferObservation {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut n = size_of::<Self>();
        if let Some(source) = &self.workbook {
            if self.direction != CsvDirection::Import {
                return None;
            }
            add(&mut n, source.checked_heap_bytes()?)?;
        }
        string(&mut n, &self.connection_id, 128)?;
        add(&mut n, self.target.checked_heap_bytes()?)?;
        string(&mut n, &self.file_name, 1024)?;
        string(&mut n, &self.started_at, 64)?;
        if let Some(time) = &self.finished_at {
            string(&mut n, time, 64)?;
        }
        if self.phase.terminal() != self.finished_at.is_some() {
            return None;
        }
        if let Some(d) = &self.diagnostic {
            add(&mut n, d.checked_heap_bytes()?)?;
        }
        let changed = self.direction == CsvDirection::Import
            && self.phase.terminal()
            && matches!(self.effect, CsvEffect::Succeeded | CsvEffect::Unknown);
        if changed != self.import_change_revision.is_some()
            || self.import_change_revision == Some(0)
        {
            return None;
        }
        Some(n)
    }
}
impl CsvTransferList {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.jobs.len() > MAX_CSV_ACTIVE + MAX_CSV_TERMINAL
            || self.execution_reserved_bytes > MAX_CSV_EXECUTION_POOL_BYTES
            || !self
                .execution_reserved_bytes
                .is_multiple_of(MAX_CSV_EXECUTION_BYTES)
        {
            return None;
        }
        let mut n = size_of::<Self>().checked_add(
            self.jobs
                .capacity()
                .checked_mul(size_of::<CsvTransferObservation>())?,
        )?;
        for (i, row) in self.jobs.iter().enumerate() {
            add(&mut n, row.checked_heap_bytes()?)?;
            if self.jobs[..i]
                .iter()
                .any(|other| other.attempt_id == row.attempt_id)
            {
                return None;
            }
            if let Some(revision) = row.import_change_revision {
                if revision > self.import_change_revision
                    || self.jobs[..i]
                        .iter()
                        .any(|other| other.import_change_revision == Some(revision))
                {
                    return None;
                }
            }
        }
        if n > MAX_CSV_LIST_BYTES {
            return None;
        }
        self.encoded_bytes()?;
        Some(n)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self, MAX_CSV_LIST_BYTES)
    }
}
impl CsvInspectionList {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.inspections.len() > MAX_CSV_INSPECTIONS {
            return None;
        }
        let mut n = size_of::<Self>().checked_add(
            self.inspections
                .capacity()
                .checked_mul(size_of::<CsvInspectionObservation>())?,
        )?;
        for (i, row) in self.inspections.iter().enumerate() {
            add(&mut n, row.checked_heap_bytes()?)?;
            if self.inspections[..i]
                .iter()
                .any(|other| other.inspection_id == row.inspection_id)
            {
                return None;
            }
        }
        if n > MAX_CSV_LIST_BYTES {
            return None;
        }
        encoded(self, MAX_CSV_LIST_BYTES)?;
        Some(n)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self, MAX_CSV_LIST_BYTES)
    }
}
