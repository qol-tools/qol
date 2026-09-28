use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use super::*;

#[derive(Clone, Copy)]
enum Receipt {
    Complete,
    Failed,
    Pending,
    Delayed(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Unregister,
    Shutdown,
    Status,
}

struct FakeDaemon {
    unregister: Receipt,
    shutdown: Receipt,
    statuses: Mutex<VecDeque<Receipt>>,
    calls: Mutex<Vec<(Command, Instant)>>,
    outstanding: Arc<AtomicUsize>,
}

struct ReceiptOwner(Arc<AtomicUsize>);

impl Drop for ReceiptOwner {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl FakeDaemon {
    fn new(unregister: Receipt, shutdown: Receipt, statuses: Vec<Receipt>) -> Self {
        Self {
            unregister,
            shutdown,
            statuses: Mutex::new(statuses.into()),
            calls: Mutex::new(Vec::new()),
            outstanding: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn receipt(&self, command: Command, receipt: Receipt) -> DiscoveryFuture {
        self.calls.lock().unwrap().push((command, Instant::now()));
        self.outstanding.fetch_add(1, Ordering::SeqCst);
        let owner = ReceiptOwner(self.outstanding.clone());
        Box::pin(async move {
            let _owner = owner;
            match receipt {
                Receipt::Complete => Ok(()),
                Receipt::Failed => Err(NetworkFailure::Cleanup),
                Receipt::Pending => std::future::pending().await,
                Receipt::Delayed(delay) => {
                    tokio::time::sleep(delay).await;
                    Ok(())
                }
            }
        })
    }
}

impl CleanupDaemon for FakeDaemon {
    fn unregister(&self, _: &str) -> DiscoveryFuture {
        self.receipt(Command::Unregister, self.unregister)
    }

    fn shutdown(&self) -> DiscoveryFuture {
        self.receipt(Command::Shutdown, self.shutdown)
    }

    fn status(&self) -> DiscoveryFuture {
        let receipt = self
            .statuses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Receipt::Failed);
        self.receipt(Command::Status, receipt)
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_receipts_have_one_total_deadline_and_shutdown_is_always_issued() {
    for (name, unregister, shutdown, statuses) in [
        (
            "both stalled",
            Receipt::Pending,
            Receipt::Pending,
            vec![Receipt::Pending],
        ),
        (
            "shutdown stalled",
            Receipt::Complete,
            Receipt::Pending,
            vec![Receipt::Failed],
        ),
        (
            "enqueue failed",
            Receipt::Failed,
            Receipt::Failed,
            vec![Receipt::Failed],
        ),
        (
            "status stalled",
            Receipt::Complete,
            Receipt::Failed,
            vec![Receipt::Pending; 30],
        ),
    ] {
        let daemon = FakeDaemon::new(unregister, shutdown, statuses);
        let started = Instant::now();
        assert_eq!(
            close_owned(&daemon, "peer").await,
            Err(NetworkFailure::Cleanup),
            "{name}"
        );
        assert_eq!(Instant::now() - started, CLOSE_BUDGET, "{name}");
        let calls = daemon.calls.lock().unwrap();
        assert_eq!(
            &calls[..2],
            &[(Command::Unregister, started), (Command::Shutdown, started)],
            "{name}"
        );
        assert!(calls.len() <= 32, "{name}: {calls:?}");
        assert_eq!(daemon.outstanding.load(Ordering::SeqCst), 0, "{name}");
    }
}

#[tokio::test(start_paused = true)]
async fn shutdown_completion_proves_cleanup_even_when_unregister_receipt_is_lost() {
    for unregister in [Receipt::Complete, Receipt::Failed, Receipt::Pending] {
        let daemon = FakeDaemon::new(unregister, Receipt::Delayed(Duration::from_secs(1)), vec![]);
        let started = Instant::now();
        assert_eq!(close_owned(&daemon, "peer").await, Ok(()));
        assert_eq!(Instant::now() - started, Duration::from_secs(1));
        assert_eq!(daemon.calls.lock().unwrap().len(), 2);
        assert_eq!(daemon.outstanding.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test(start_paused = true)]
async fn already_stopped_status_makes_repeated_close_idempotent() {
    let daemon = FakeDaemon::new(Receipt::Failed, Receipt::Failed, vec![Receipt::Complete; 2]);
    let started = Instant::now();
    for _ in 0..2 {
        assert_eq!(close_owned(&daemon, "peer").await, Ok(()));
    }
    assert_eq!(Instant::now(), started);
    assert_eq!(daemon.calls.lock().unwrap().len(), 6);
    assert_eq!(daemon.outstanding.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn command_error_after_possible_enqueue_requires_later_completion_status() {
    let daemon = FakeDaemon::new(
        Receipt::Failed,
        Receipt::Failed,
        vec![Receipt::Failed, Receipt::Failed, Receipt::Complete],
    );
    let started = Instant::now();
    assert_eq!(close_owned(&daemon, "peer").await, Ok(()));
    assert_eq!(Instant::now() - started, STATUS_BUDGET * 2);
    assert_eq!(daemon.calls.lock().unwrap().len(), 5);
    assert_eq!(daemon.outstanding.load(Ordering::SeqCst), 0);
}
