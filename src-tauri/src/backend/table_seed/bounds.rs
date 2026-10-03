use super::*;
pub(super) fn encoded<T: serde::Serialize>(value: &T, limit: usize) -> Option<usize> {
    struct Counter {
        used: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.used = self
                .used
                .checked_add(b.len())
                .filter(|n| *n <= self.limit)
                .ok_or_else(|| std::io::Error::other("seed bound"))?;
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { used: 0, limit };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.used)
}
impl TableSeedDiagnostic {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut n = std::mem::size_of::<Self>();
        for value in [
            &self.sqlstate,
            &self.constraint,
            &self.column,
            &self.parent_schema,
            &self.parent_table,
        ]
        .into_iter()
        .flatten()
        {
            if value.len() > 63 || value.contains('\0') {
                return None;
            }
            n = n.checked_add(value.capacity())?;
        }
        if self.sqlstate.as_ref().is_some_and(|s| {
            s.len() != 5
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        }) {
            return None;
        }
        (n <= 1024).then_some(n)
    }
}
impl TableSeedReceipt {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.outcome == TableSeedOutcome::Pending {
            return None;
        }
        let mut n =
            std::mem::size_of::<Self>().checked_add(self.description.checked_heap_bytes()?)?;
        if let TableSeedOutcome::Completed { rows } = self.outcome {
            if rows > u64::from(self.description.row_count)
                || self.failure.is_some_and(|f| f != TableSeedError::Cleanup)
            {
                return None;
            }
        }
        if let Some(d) = &self.diagnostic {
            n = n.checked_add(d.checked_heap_bytes()?)?;
        }
        Some(n)
    }
}
impl TableSeedObservation {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !(1..=MAX_TABLE_SEED_ROWS).contains(&self.row_count)
            || self.rows_generated > u64::from(self.row_count)
        {
            return None;
        }
        let mut n = std::mem::size_of::<Self>().checked_add(self.endpoint.checked_heap_bytes()?)?;
        if let Some(d) = &self.diagnostic {
            n = n.checked_add(d.checked_heap_bytes()?)?;
        }
        if let Some(r) = &self.receipt {
            if r.attempt_id != self.attempt_id
                || r.description.endpoint != self.endpoint
                || r.description.row_count != self.row_count
                || r.outcome != self.outcome
            {
                return None;
            }
            n = n.checked_add(r.checked_heap_bytes()?)?;
        }
        if matches!(
            self.outcome,
            TableSeedOutcome::Completed { .. } | TableSeedOutcome::OutcomeUnknown
        ) != self.change_revision.is_some()
            || self.change_revision == Some(0)
        {
            return None;
        }
        Some(n)
    }
}
impl TableSeedList {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.jobs.len() > MAX_TABLE_SEED_ACTIVE + MAX_TABLE_SEED_TERMINAL {
            return None;
        }
        let mut n = std::mem::size_of::<Self>().checked_add(
            self.jobs
                .capacity()
                .checked_mul(std::mem::size_of::<TableSeedObservation>())?,
        )?;
        for (i, j) in self.jobs.iter().enumerate() {
            if self.jobs[..i].iter().any(|p| p.attempt_id == j.attempt_id)
                || j.change_revision.is_some_and(|r| r > self.change_revision)
            {
                return None;
            }
            n = n.checked_add(j.checked_heap_bytes()?)?;
        }
        self.encoded_bytes()?;
        (n <= MAX_TABLE_SEED_LIST_BYTES).then_some(n)
    }
    pub fn encoded_bytes(&self) -> Option<usize> {
        encoded(self, MAX_TABLE_SEED_LIST_BYTES)
    }
}
macro_rules! redacted{($($ty:ty),*)=>{$(impl std::fmt::Debug for $ty{fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result{f.write_str(concat!(stringify!($ty)," { .. }"))}})*}}
redacted!(
    TableSeedDescription,
    TableSeedReceipt,
    TableSeedDiagnostic,
    TableSeedColumn,
    TableSeedColumnSpec,
    TableSeedSource,
    TableSeedObservation,
    TableSeedList,
    TableSeedInspection,
    TableSeedReview,
    TableSeedConfirmation,
    TableSeedSubmission
);
