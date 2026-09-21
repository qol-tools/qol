use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(super) struct TtlCell<T> {
    cell: Mutex<Option<(Instant, T)>>,
}

impl<T> TtlCell<T> {
    pub(super) fn new() -> Self {
        Self {
            cell: Mutex::new(None),
        }
    }

    pub(super) fn get_or_compute_at(
        &self,
        now: Instant,
        ttl: Duration,
        compute: impl FnOnce() -> T,
    ) -> T
    where
        T: Clone,
    {
        let mut cell = self
            .cell
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((at, cached)) = cell.as_ref() {
            if now.saturating_duration_since(*at) < ttl {
                return cached.clone();
            }
        }
        let value = compute();
        *cell = Some((now, value.clone()));
        value
    }

    pub(super) fn get_or_compute(&self, ttl: Duration, compute: impl FnOnce() -> T) -> T
    where
        T: Clone,
    {
        self.get_or_compute_at(Instant::now(), ttl, compute)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_value_served_within_ttl_and_recomputed_after() {
        let cell = TtlCell::new();
        let now = Instant::now();
        let ttl = Duration::from_secs(60);
        let mut computes = 0;

        let first = cell.get_or_compute_at(now, ttl, || {
            computes += 1;
            7
        });
        assert_eq!(first, 7);
        assert_eq!(computes, 1, "the first call runs the closure");

        let second = cell.get_or_compute_at(now, ttl, || {
            computes += 1;
            7
        });
        assert_eq!(second, 7);
        assert_eq!(
            computes, 1,
            "a second call at the same instant must reuse the cached value"
        );

        let stale = cell.get_or_compute_at(now + ttl + Duration::from_secs(1), ttl, || {
            computes += 1;
            9
        });
        assert_eq!(stale, 9);
        assert_eq!(computes, 2, "a call past the ttl must recompute");
    }
}
