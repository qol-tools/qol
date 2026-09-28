use std::cell::{Cell, RefCell};
use std::future::poll_fn;
use std::pin::Pin;
use std::task::Poll;

use tokio::sync::{broadcast, oneshot};

use super::*;

const OPERATIONS: [&str; 2] = ["Self recompile restart", "Rolling dev restart"];

fn attempt(operation: &'static str) -> (PendingRestart, Arc<DevRuntimeService>, Arc<EventBus>) {
    let runtime = Arc::new(DevRuntimeService::new());
    let events = Arc::new(EventBus::new());
    let pending = PendingRestart::begin(runtime.clone(), events.clone(), operation).unwrap();
    (pending, runtime, events)
}

async fn assert_pending(mut future: Pin<&mut impl Future>, operation: &str) {
    assert!(
        poll_fn(|context| Poll::Ready(future.as_mut().poll(context).is_pending())).await,
        "{operation} must wait for cleanup completion"
    );
}

fn assert_failure(receiver: &mut broadcast::Receiver<DaemonEvent>, expected: &str) {
    match receiver.try_recv().unwrap() {
        DaemonEvent::SelfRecompileFailed { message } => assert_eq!(message, expected),
        event => panic!("expected restart failure, received {event:?}"),
    }
    assert!(matches!(
        receiver.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn restart_waits_for_cleanup_receipt_then_executes_once() {
    for operation in OPERATIONS {
        let (pending, runtime, events) = attempt(operation);
        let mut receiver = events.subscribe();
        let calls = RefCell::new(Vec::new());
        let (complete, completion) = oneshot::channel();
        let mut restart = Box::pin(pending.run(
            async {
                calls.borrow_mut().push("cleanup started");
                completion.await.unwrap();
                calls.borrow_mut().push("cleanup completed");
                Ok(())
            },
            || {
                calls.borrow_mut().push("exec");
                Ok(())
            },
        ));

        assert_pending(restart.as_mut(), operation).await;
        assert_eq!(*calls.borrow(), ["cleanup started"], "{operation}");
        assert!(!runtime.try_mark_restart_pending(), "{operation}");
        complete.send(()).unwrap();
        assert_eq!(restart.await, Ok(()), "{operation}");
        assert_eq!(
            *calls.borrow(),
            ["cleanup started", "cleanup completed", "exec"],
            "{operation}"
        );
        assert!(runtime.try_mark_restart_pending(), "{operation}");
        runtime.clear_restart_pending();
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }
}

#[tokio::test]
async fn cleanup_failure_prevents_exec_and_reports_once() {
    for operation in OPERATIONS {
        for message in [
            "core peer unavailable",
            "peer cleanup failed",
            "plugin cleanup failed",
        ] {
            let (pending, runtime, events) = attempt(operation);
            let mut receiver = events.subscribe();
            let executions = Cell::new(0);
            let result = pending
                .run(async { Err(message.to_string()) }, || {
                    executions.set(executions.get() + 1);
                    Ok(())
                })
                .await;

            assert_eq!(result, Err(message.to_string()), "{operation}: {message}");
            assert_eq!(executions.get(), 0, "{operation}: {message}");
            assert_failure(&mut receiver, message);
            assert!(runtime.try_mark_restart_pending(), "{operation}: {message}");
            runtime.clear_restart_pending();
        }
    }
}

#[tokio::test]
async fn cancelled_waiting_never_executes_and_releases_pending() {
    for operation in OPERATIONS {
        let (pending, runtime, events) = attempt(operation);
        let mut receiver = events.subscribe();
        let executions = Cell::new(0);
        let (complete, completion) = oneshot::channel();
        let mut restart = Box::pin(pending.run(
            async {
                completion.await.unwrap();
                Ok(())
            },
            || {
                executions.set(executions.get() + 1);
                Ok(())
            },
        ));

        assert_pending(restart.as_mut(), operation).await;
        drop(restart);

        assert!(complete.send(()).is_err(), "{operation}");
        assert_eq!(executions.get(), 0, "{operation}");
        assert_failure(
            &mut receiver,
            &format!("{operation} cancelled before process replacement"),
        );
        assert!(runtime.try_mark_restart_pending(), "{operation}");
        runtime.clear_restart_pending();
    }
}

#[test]
fn dropping_an_unpolled_restart_releases_its_claim() {
    for operation in OPERATIONS {
        let (pending, runtime, events) = attempt(operation);
        let mut receiver = events.subscribe();
        let executions = Cell::new(0);
        let restart = pending.run(async { Ok(()) }, || {
            executions.set(executions.get() + 1);
            Ok(())
        });
        drop(restart);

        assert_eq!(executions.get(), 0, "{operation}");
        assert_failure(
            &mut receiver,
            &format!("{operation} cancelled before process replacement"),
        );
        assert!(runtime.try_mark_restart_pending(), "{operation}");
        runtime.clear_restart_pending();
    }
}

#[tokio::test]
async fn exec_failure_is_reported_without_retry() {
    for operation in OPERATIONS {
        let (pending, runtime, events) = attempt(operation);
        let mut receiver = events.subscribe();
        let executions = Cell::new(0);
        let result = pending
            .run(async { Ok(()) }, || {
                executions.set(executions.get() + 1);
                Err("exec failed".to_string())
            })
            .await;

        assert_eq!(result, Err("exec failed".to_string()), "{operation}");
        assert_eq!(executions.get(), 1, "{operation}");
        assert_failure(&mut receiver, "exec failed");
        assert!(runtime.try_mark_restart_pending(), "{operation}");
        runtime.clear_restart_pending();
    }
}

#[test]
fn failure_drop_does_not_release_a_later_restart_claim() {
    for operation in OPERATIONS {
        let (mut pending, runtime, events) = attempt(operation);
        let mut receiver = events.subscribe();
        assert!(matches!(
            PendingRestart::begin(runtime.clone(), events.clone(), operation),
            Err("Restart already pending")
        ));
        pending.fail("staging failed".to_string());
        assert_failure(&mut receiver, "staging failed");

        let mut next = PendingRestart::begin(runtime.clone(), events.clone(), operation).unwrap();
        drop(pending);
        assert!(!runtime.try_mark_restart_pending(), "{operation}");
        next.fail("next attempt failed".to_string());
        assert_failure(&mut receiver, "next attempt failed");
        assert!(runtime.try_mark_restart_pending(), "{operation}");
        runtime.clear_restart_pending();
    }
}
