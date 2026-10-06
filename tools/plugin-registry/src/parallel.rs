use anyhow::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    workers: usize,
    work: impl Fn(&T) -> Result<R> + Sync,
) -> Result<Vec<R>> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<Result<R>>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..workers.clamp(1, items.len().max(1)) {
            scope.spawn(|| loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(index) else {
                    break;
                };
                let result = work(item);
                if let Ok(mut slot) = results[index].lock() {
                    *slot = Some(result);
                }
            });
        }
    });
    results
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .ok()
                .flatten()
                .unwrap_or_else(|| Err(anyhow::anyhow!("a parallel task stopped without a result")))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_input_order_and_reports_the_first_failure() {
        let items: Vec<u32> = (0..20).collect();
        let doubled = parallel_map(&items, 4, |n| Ok(n * 2)).unwrap();
        assert_eq!(doubled, items.iter().map(|n| n * 2).collect::<Vec<_>>());

        let failed = parallel_map(&items, 4, |n| {
            if *n == 7 {
                anyhow::bail!("seven")
            }
            Ok(*n)
        });
        assert_eq!(failed.unwrap_err().to_string(), "seven");
        assert!(parallel_map(&Vec::<u32>::new(), 4, |n| Ok(*n))
            .unwrap()
            .is_empty());
    }
}
