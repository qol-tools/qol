use std::time::Duration;

use mdns_sd::{DaemonStatus, ServiceDaemon};
use tokio::time::{sleep_until, timeout_at, Instant};

use super::DiscoveryFuture;
use crate::network::NetworkFailure;

const CLOSE_BUDGET: Duration = Duration::from_secs(3);
const UNREGISTER_BUDGET: Duration = Duration::from_millis(500);
const SHUTDOWN_BUDGET: Duration = Duration::from_secs(2);
const STATUS_BUDGET: Duration = Duration::from_millis(100);

trait CleanupDaemon {
    fn unregister(&self, fullname: &str) -> DiscoveryFuture;
    fn shutdown(&self) -> DiscoveryFuture;
    fn status(&self) -> DiscoveryFuture;
}

impl CleanupDaemon for ServiceDaemon {
    fn unregister(&self, fullname: &str) -> DiscoveryFuture {
        let receipt = ServiceDaemon::unregister(self, fullname);
        Box::pin(async move {
            receipt
                .map_err(|_| NetworkFailure::Cleanup)?
                .recv_async()
                .await
                .map_err(|_| NetworkFailure::Cleanup)?;
            Ok(())
        })
    }

    fn shutdown(&self) -> DiscoveryFuture {
        status_receipt(ServiceDaemon::shutdown(self))
    }

    fn status(&self) -> DiscoveryFuture {
        status_receipt(ServiceDaemon::status(self))
    }
}

fn status_receipt(receipt: mdns_sd::Result<mdns_sd::Receiver<DaemonStatus>>) -> DiscoveryFuture {
    Box::pin(async move {
        if matches!(
            receipt
                .map_err(|_| NetworkFailure::Cleanup)?
                .recv_async()
                .await,
            Ok(DaemonStatus::Shutdown)
        ) {
            return Ok(());
        }
        Err(NetworkFailure::Cleanup)
    })
}

pub(super) async fn close(daemon: &ServiceDaemon, fullname: &str) -> Result<(), NetworkFailure> {
    close_owned(daemon, fullname).await
}

async fn close_owned(daemon: &impl CleanupDaemon, fullname: &str) -> Result<(), NetworkFailure> {
    let started = Instant::now();
    let deadline = started + CLOSE_BUDGET;
    let unregister = daemon.unregister(fullname);
    let shutdown = daemon.shutdown();
    let (_, stopped) = tokio::join!(
        timeout_at(started + UNREGISTER_BUDGET, unregister),
        timeout_at(started + SHUTDOWN_BUDGET, shutdown),
    );
    if matches!(stopped, Ok(Ok(()))) {
        return Ok(());
    }
    while Instant::now() < deadline {
        let next = (Instant::now() + STATUS_BUDGET).min(deadline);
        if matches!(timeout_at(next, daemon.status()).await, Ok(Ok(()))) {
            return Ok(());
        }
        sleep_until(next).await;
    }
    Err(NetworkFailure::Cleanup)
}

#[cfg(test)]
#[path = "../tests/cleanup.rs"]
mod tests;
