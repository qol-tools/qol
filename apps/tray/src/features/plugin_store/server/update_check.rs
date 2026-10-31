use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::plugin_services;
use super::types::AppState;

pub(super) struct Claim(Arc<AtomicBool>);

impl Drop for Claim {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

pub(super) fn claim(state: &AppState) -> Option<impl Future<Output = ()> + Send + 'static> {
    let claim = take(&state.update_check_running)?;
    let catalog = plugin_services::claim_catalog_revalidation(state);
    Some(async move {
        let _claim = claim;
        let catalog = async {
            if let Some(revalidation) = catalog {
                revalidation.await;
            }
        };
        tokio::join!(check_host(), catalog);
    })
}

pub(super) async fn check_host() {
    if let Err(error) = crate::updates::check_for_updates().await {
        log::debug!("Update check failed: {}", error);
    }
}

pub(super) fn running(state: &AppState) -> bool {
    state.update_check_running.load(Ordering::SeqCst)
}

pub(super) fn take(flag: &Arc<AtomicBool>) -> Option<Claim> {
    flag.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .ok()
        .map(|_| Claim(flag.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_check_runs_at_a_time_and_its_claim_frees_on_drop() {
        let flag = Arc::new(AtomicBool::new(false));
        let first = take(&flag);
        assert!(first.is_some());
        assert!(take(&flag).is_none());
        drop(first);
        assert!(!flag.load(Ordering::SeqCst));
        assert!(take(&flag).is_some());
    }
}
