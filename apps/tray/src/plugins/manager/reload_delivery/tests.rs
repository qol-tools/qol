use super::*;
use std::time::{Duration, Instant};

fn instance(pid: u32, incarnation: u64) -> DaemonInstance {
    DaemonInstance { pid, incarnation }
}

#[test]
fn begin_registers_pending_delivery_without_a_published_generation() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);

    assert!(deliveries.is_pending("plugin-a"));
    assert!(!deliveries.is_pending("plugin-b"));
    assert_eq!(ticket.plugin_id(), "plugin-a");
    assert_eq!(ticket.daemon_instance(), Some(instance(10, 1)));
    assert!(ticket.request_id() > 0);
}

#[test]
fn handled_completion_acknowledges_the_recorded_generation() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&ticket, 7));

    assert_eq!(
        deliveries.complete(
            &ticket,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Acknowledged { generation: 7 }
    );
    assert!(!deliveries.is_pending("plugin-a"));
}

#[test]
fn completing_one_delivery_leaves_the_other_request_pending() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let first = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    let second = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&first, 1));
    assert!(deliveries.record_saved(&second, 2));

    assert_eq!(
        deliveries.complete(
            &first,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Acknowledged { generation: 1 }
    );
    assert!(deliveries.is_pending("plugin-a"));
    assert_eq!(
        deliveries.complete(
            &second,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Acknowledged { generation: 2 }
    );
    assert!(!deliveries.is_pending("plugin-a"));
}

#[test]
fn completion_for_an_unknown_request_is_superseded() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    deliveries.supersede_plugin("plugin-a");

    assert_eq!(
        deliveries.complete(
            &ticket,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
}

#[test]
fn replaced_daemon_with_the_same_pid_rejects_handled_completion() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&ticket, 7));

    assert_eq!(
        deliveries.complete(
            &ticket,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 2))
        ),
        DeliverySettlement::Superseded
    );
    assert!(!deliveries.is_pending("plugin-a"));
}

#[test]
fn replaced_daemon_with_the_same_pid_rejects_failure_restart() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&ticket, 7));

    assert_eq!(
        deliveries.complete(
            &ticket,
            ReloadDeliveryOutcome::Failed,
            Some(instance(10, 2))
        ),
        DeliverySettlement::Superseded
    );
    assert!(!deliveries.is_pending("plugin-a"));
}

#[test]
fn failure_restarts_only_when_no_other_delivery_is_pending() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let first = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    let second = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&first, 1));
    assert!(deliveries.record_saved(&second, 2));

    assert_eq!(
        deliveries.complete(&first, ReloadDeliveryOutcome::Failed, Some(instance(10, 1))),
        DeliverySettlement::Superseded
    );
    assert!(deliveries.is_pending("plugin-a"));
    assert_eq!(
        deliveries.complete(
            &second,
            ReloadDeliveryOutcome::Failed,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Restart {
            generation: Some(2)
        }
    );
    assert!(!deliveries.is_pending("plugin-a"));
}

#[test]
fn failing_larger_request_does_not_restart_while_a_smaller_request_is_pending() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let smaller = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    let larger = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&larger, 1));
    assert!(deliveries.record_saved(&smaller, 2));

    assert_eq!(
        deliveries.complete(
            &larger,
            ReloadDeliveryOutcome::Failed,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
    assert!(deliveries.is_pending("plugin-a"));
    assert_eq!(
        deliveries.complete(
            &smaller,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Acknowledged { generation: 2 }
    );
}

#[test]
fn save_failure_and_snapshot_failure_and_missing_daemon_retire_only_their_own_request() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let save_failed = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    let snapshot_failed = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    let no_daemon = deliveries.begin("plugin-a", None, now);

    assert_eq!(
        deliveries.complete(
            &save_failed,
            ReloadDeliveryOutcome::SaveFailed,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
    assert_eq!(
        deliveries.complete(
            &snapshot_failed,
            ReloadDeliveryOutcome::SnapshotFailed,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
    assert!(deliveries.is_pending("plugin-a"));
    assert_eq!(
        deliveries.complete(&no_daemon, ReloadDeliveryOutcome::NoDaemon, None),
        DeliverySettlement::Superseded
    );
    assert!(!deliveries.is_pending("plugin-a"));
}

#[test]
fn unpublished_handled_completion_is_superseded() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);

    assert_eq!(
        deliveries.complete(
            &ticket,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
}

#[test]
fn expiry_removes_only_requests_past_the_ttl() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let ttl = Duration::from_secs(15);
    let ticket = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    assert!(deliveries.record_saved(&ticket, 7));

    assert_eq!(
        deliveries.expire_stale(now + ttl - Duration::from_secs(1), ttl),
        0
    );
    assert!(deliveries.is_pending("plugin-a"));
    assert_eq!(deliveries.expire_stale(now + ttl, ttl), 1);
    assert!(!deliveries.is_pending("plugin-a"));
    assert_eq!(
        deliveries.complete(
            &ticket,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
}

#[test]
fn supersede_all_clears_every_pending_delivery() {
    let mut deliveries = ReloadDeliveries::default();
    let now = Instant::now();
    let first = deliveries.begin("plugin-a", Some(instance(10, 1)), now);
    let second = deliveries.begin("plugin-b", None, now);

    deliveries.supersede_all();

    assert!(!deliveries.is_pending("plugin-a"));
    assert!(!deliveries.is_pending("plugin-b"));
    assert_eq!(
        deliveries.complete(
            &first,
            ReloadDeliveryOutcome::Handled,
            Some(instance(10, 1))
        ),
        DeliverySettlement::Superseded
    );
    assert_eq!(
        deliveries.complete(&second, ReloadDeliveryOutcome::SaveFailed, None),
        DeliverySettlement::Superseded
    );
}
