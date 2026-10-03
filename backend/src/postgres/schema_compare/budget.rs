use super::protocol::{CompareError, Limit};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

pub const GLOBAL_BYTES: usize = 256 * 1024 * 1024;
/// Fixed headroom for the bounded owner's counters, scope Arcs and control
/// records. Dynamic definition/result containers are charged separately.
pub const CONTROL_BYTES: usize = 64 * 1024;
pub const FIELD_BYTES: usize = 256 * 1024;
pub const ENDPOINT_BYTES: usize = 16 * 1024 * 1024;
pub const RESULT_BYTES: usize = 32 * 1024 * 1024;
pub const PAGE_BYTES: usize = 1024 * 1024;
pub const CHUNK_BYTES: usize = 64 * 1024;
pub const PAGE_ITEMS: usize = 100;
pub const SERIALIZER_SCRATCH: usize = 8 * 1024 * 1024;
pub const MAX_VALUES: usize = 50_000;
pub const MAX_RESULT_VALUES: usize = 2 * MAX_VALUES;
pub const INVENTORY_ENTRIES: usize = 2_000;
pub const TABLE_ENTRIES: usize = 1_000;

struct Counters {
    used: AtomicUsize,
    peak: AtomicUsize,
    serializers: AtomicUsize,
    limit: usize,
}

#[derive(Clone)]
pub struct Budget {
    counters: Arc<Counters>,
    /// Present only for a result scope: the combined retained bytes of one
    /// result's values, inventory and diff.
    result_scope: Option<Arc<AtomicUsize>>,
}

impl Default for Budget {
    fn default() -> Self {
        Self::new(GLOBAL_BYTES - CONTROL_BYTES)
    }
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            counters: Arc::new(Counters {
                used: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                serializers: AtomicUsize::new(0),
                limit,
            }),
            result_scope: None,
        }
    }

    #[cfg(test)]
    pub fn used(&self) -> usize {
        self.counters.used.load(Ordering::Acquire)
    }

    /// Highest reserved total observed since creation or the last reset. This
    /// is accounting evidence for the runtime measurements, not a limit.
    #[cfg(test)]
    pub fn peak(&self) -> usize {
        self.counters.peak.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn reset_peak(&self) {
        self.counters.peak.store(self.used(), Ordering::Release);
    }

    /// Share this scope across the values, inventory and diff owned by one
    /// result. Their combined retained allocation, not each component alone,
    /// is capped at 32 MiB while also consuming the global budget.
    pub fn result_scope(&self) -> Self {
        if self.result_scope.is_some() {
            self.clone()
        } else {
            Self {
                counters: self.counters.clone(),
                result_scope: Some(Arc::new(AtomicUsize::new(0))),
            }
        }
    }

    /// Capture and diff must retain the same result counter, not merely the
    /// same global allocator. A fresh scope cannot reset a result's budget.
    pub(crate) fn same_result_scope(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.counters, &other.counters)
            && matches!((&self.result_scope, &other.result_scope), (Some(a), Some(b)) if Arc::ptr_eq(a, b))
    }

    pub fn scratch(&self, bytes: usize) -> Result<Reservation, CompareError> {
        Self {
            counters: self.counters.clone(),
            result_scope: None,
        }
        .reserve(bytes)
    }

    /// Reserve before requesting an allocation. Charge owned capacities and
    /// containers; allocator-internal overhead is measured separately as RSS.
    pub fn reserve(&self, bytes: usize) -> Result<Reservation, CompareError> {
        let previous = self
            .counters
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= self.counters.limit)
            })
            .map_err(|_| CompareError::LimitExceeded {
                limit: Limit::Allocation,
            })?;
        self.counters
            .peak
            .fetch_max(previous + bytes, Ordering::AcqRel);
        if let Some(scope) = &self.result_scope {
            if scope
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(bytes).filter(|next| *next <= RESULT_BYTES)
                })
                .is_err()
            {
                self.counters.used.fetch_sub(bytes, Ordering::AcqRel);
                return Err(CompareError::LimitExceeded {
                    limit: Limit::ResultBytes,
                });
            }
        }
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }

    /// No waiter queue. The returned lease must survive serialization and
    /// transport handoff; dropping it inside IpcResponse::body is too early.
    pub fn serializer(&self) -> Result<SerializerLease, CompareError> {
        self.counters
            .serializers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| CompareError::Busy)?;
        match self.scratch(SERIALIZER_SCRATCH) {
            Ok(reservation) => Ok(SerializerLease { reservation }),
            Err(error) => {
                self.counters.serializers.fetch_sub(1, Ordering::AcqRel);
                Err(error)
            }
        }
    }
}

pub struct Reservation {
    budget: Budget,
    bytes: usize,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget
            .counters
            .used
            .fetch_sub(self.bytes, Ordering::AcqRel);
        if let Some(scope) = &self.budget.result_scope {
            scope.fetch_sub(self.bytes, Ordering::AcqRel);
        }
    }
}

pub struct SerializerLease {
    reservation: Reservation,
}

impl Drop for SerializerLease {
    fn drop(&mut self) {
        self.reservation
            .budget
            .counters
            .serializers
            .fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocation_and_serializer_admission_are_atomic_and_release_on_drop() {
        let budget = Budget::new(2 * SERIALIZER_SCRATCH);
        let first = budget.serializer().unwrap();
        let second = budget.serializer().unwrap();
        assert!(matches!(budget.serializer(), Err(CompareError::Busy)));
        assert!(budget.reserve(1).is_err());
        assert_eq!(budget.used(), 2 * SERIALIZER_SCRATCH);
        drop(first);
        assert!(budget.serializer().is_ok());
        drop(second);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn one_result_shares_its_cap_across_components_and_failure_rolls_back() {
        let global = Budget::default();
        let result = global.result_scope();
        let values = result.reserve(RESULT_BYTES / 2).unwrap();
        let inventory_and_diff = result.reserve(RESULT_BYTES / 2).unwrap();
        assert!(matches!(
            result.reserve(1),
            Err(CompareError::LimitExceeded {
                limit: Limit::ResultBytes
            })
        ));
        assert_eq!(global.used(), RESULT_BYTES);
        assert!(result.serializer().is_ok());
        drop(values);
        drop(inventory_and_diff);
        assert_eq!(global.used(), 0);
    }

    #[test]
    fn nested_result_scope_preserves_the_combined_cap_and_rolls_back_global() {
        let global = Budget::default();
        let result = global.result_scope();
        let nested = result.result_scope();
        let values = result.reserve(RESULT_BYTES / 2).unwrap();
        let inventory_and_diff = nested.reserve(RESULT_BYTES / 2).unwrap();

        assert!(matches!(
            nested.reserve(1),
            Err(CompareError::LimitExceeded {
                limit: Limit::ResultBytes
            })
        ));
        assert_eq!(global.used(), RESULT_BYTES);

        drop(values);
        drop(inventory_and_diff);
        assert_eq!(global.used(), 0);
    }

    #[test]
    fn simultaneous_serializers_have_two_winners_without_a_queue() {
        let budget = Budget::default();
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..3)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        let lease = budget.serializer();
                        barrier.wait();
                        barrier.wait();
                        lease.is_ok()
                    })
                })
                .collect();
            barrier.wait();
            barrier.wait();
            assert_eq!(budget.used(), 2 * SERIALIZER_SCRATCH);
            barrier.wait();
            assert_eq!(
                workers
                    .into_iter()
                    .map(|worker| worker.join().unwrap())
                    .filter(|winner| *winner)
                    .count(),
                2
            );
        });
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn two_jobs_two_results_and_two_serializers_fit_the_global_ceiling_with_a_known_margin() {
        // Manager setup/transport scratch and native capture scratch are each
        // 32 MiB per active job; see manager::start_with_timing and
        // capture::CAPTURE_SCRATCH. Retained results share RESULT_BYTES each.
        const JOB_SCRATCH: usize = 2 * 32 * 1024 * 1024;
        const WORST_CASE: usize = 2 * (JOB_SCRATCH + RESULT_BYTES) + 2 * SERIALIZER_SCRATCH;
        const MARGIN: usize = GLOBAL_BYTES - CONTROL_BYTES - WORST_CASE;
        assert_eq!(WORST_CASE, 208 * 1024 * 1024);

        let budget = Budget::default();
        let mut held = Vec::new();
        let mut results = Vec::new();
        for _ in 0..2 {
            held.push(budget.scratch(JOB_SCRATCH / 2).unwrap());
            held.push(budget.scratch(JOB_SCRATCH / 2).unwrap());
            let result = budget.result_scope();
            held.push(result.reserve(RESULT_BYTES).unwrap());
            results.push(result);
        }
        let serializers = [budget.serializer().unwrap(), budget.serializer().unwrap()];
        assert_eq!(budget.used(), WORST_CASE);
        assert!(matches!(budget.serializer(), Err(CompareError::Busy)));
        // Each retained result is already at its own cap while global room remains.
        assert!(matches!(
            results[0].reserve(1),
            Err(CompareError::LimitExceeded {
                limit: Limit::ResultBytes
            })
        ));
        let margin = budget.reserve(MARGIN).unwrap();
        assert!(matches!(
            budget.reserve(1),
            Err(CompareError::LimitExceeded {
                limit: Limit::Allocation
            })
        ));
        assert_eq!(budget.peak(), GLOBAL_BYTES - CONTROL_BYTES);
        drop(margin);
        drop(serializers);
        drop(held);
        assert_eq!(budget.used(), 0);
        budget.reset_peak();
        assert_eq!(budget.peak(), 0);
    }
}
