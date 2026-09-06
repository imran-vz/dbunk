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
    serializers: AtomicUsize,
    limit: usize,
}

#[derive(Clone)]
pub struct Budget(Arc<Counters>, Option<Arc<AtomicUsize>>);

impl Default for Budget {
    fn default() -> Self {
        Self::new(GLOBAL_BYTES - CONTROL_BYTES)
    }
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self(
            Arc::new(Counters {
                used: AtomicUsize::new(0),
                serializers: AtomicUsize::new(0),
                limit,
            }),
            None,
        )
    }

    pub fn used(&self) -> usize {
        self.0.used.load(Ordering::Acquire)
    }

    /// Share this scope across the values, inventory and diff owned by one
    /// result. Their combined retained allocation, not each component alone,
    /// is capped at 32 MiB while also consuming the global budget.
    pub fn result_scope(&self) -> Self {
        if self.1.is_some() {
            self.clone()
        } else {
            Self(self.0.clone(), Some(Arc::new(AtomicUsize::new(0))))
        }
    }

    /// Capture and diff must retain the same result counter, not merely the
    /// same global allocator. A fresh scope cannot reset a result's budget.
    pub(crate) fn same_result_scope(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
            && matches!((&self.1, &other.1), (Some(a), Some(b)) if Arc::ptr_eq(a, b))
    }

    pub fn scratch(&self, bytes: usize) -> Result<Reservation, CompareError> {
        Self(self.0.clone(), None).reserve(bytes)
    }

    /// Reserve before requesting an allocation. Charge owned capacities and
    /// containers; allocator-internal overhead is measured separately as RSS.
    pub fn reserve(&self, bytes: usize) -> Result<Reservation, CompareError> {
        self.0
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|next| *next <= self.0.limit)
            })
            .map_err(|_| CompareError::LimitExceeded {
                limit: Limit::Allocation,
            })?;
        if let Some(scope) = &self.1 {
            if scope
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                    used.checked_add(bytes).filter(|next| *next <= RESULT_BYTES)
                })
                .is_err()
            {
                self.0.used.fetch_sub(bytes, Ordering::AcqRel);
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
        self.0
            .serializers
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| CompareError::Busy)?;
        match self.scratch(SERIALIZER_SCRATCH) {
            Ok(reservation) => Ok(SerializerLease { reservation }),
            Err(error) => {
                self.0.serializers.fetch_sub(1, Ordering::AcqRel);
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
        self.budget.0.used.fetch_sub(self.bytes, Ordering::AcqRel);
        if let Some(scope) = &self.budget.1 {
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
            .0
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
}
