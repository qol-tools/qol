use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

#[derive(Clone, Default)]
pub(super) struct RecordingActionController {
    action_active: Rc<Cell<bool>>,
    countdown: Rc<RefCell<CountdownState>>,
}

#[derive(Default)]
struct CountdownState {
    next_id: u64,
    active: Option<ActiveCountdown>,
}

struct ActiveCountdown {
    id: u64,
    cancelled: bool,
    waker: Option<Waker>,
}

pub(super) struct RecordingActionGuard {
    active: Rc<Cell<bool>>,
}

struct CountdownFuture<F> {
    id: u64,
    future: Pin<Box<F>>,
    state: Rc<RefCell<CountdownState>>,
}

impl RecordingActionController {
    pub(super) fn try_begin(&self) -> Option<RecordingActionGuard> {
        if self.action_active.replace(true) {
            return None;
        }
        Some(RecordingActionGuard {
            active: self.action_active.clone(),
        })
    }

    pub(super) fn countdown<F>(&self, future: F) -> impl Future<Output = Option<F::Output>>
    where
        F: Future,
    {
        let id = {
            let mut state = self.countdown.borrow_mut();
            state.next_id = state.next_id.wrapping_add(1);
            let id = state.next_id;
            state.active = Some(ActiveCountdown {
                id,
                cancelled: false,
                waker: None,
            });
            id
        };
        CountdownFuture {
            id,
            future: Box::pin(future),
            state: self.countdown.clone(),
        }
    }

    pub(super) fn cancel_countdown(&self) -> bool {
        let waker = {
            let mut state = self.countdown.borrow_mut();
            let Some(active) = state.active.as_mut() else {
                return false;
            };
            if active.cancelled {
                return false;
            }
            active.cancelled = true;
            active.waker.take()
        };
        if let Some(waker) = waker {
            waker.wake();
        }
        true
    }
}

impl Drop for RecordingActionGuard {
    fn drop(&mut self) {
        self.active.set(false);
    }
}

impl<F> Future for CountdownFuture<F>
where
    F: Future,
{
    type Output = Option<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        {
            let mut state = this.state.borrow_mut();
            let Some(active) = state.active.as_mut() else {
                return Poll::Ready(None);
            };
            if active.id != this.id || active.cancelled {
                state.active = None;
                return Poll::Ready(None);
            }
            active.waker = Some(cx.waker().clone());
        }
        match this.future.as_mut().poll(cx) {
            Poll::Ready(output) => {
                this.clear();
                Poll::Ready(Some(output))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<F> CountdownFuture<F> {
    fn clear(&self) {
        let mut state = self.state.borrow_mut();
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.id == self.id)
        {
            state.active = None;
        }
    }
}

impl<F> Drop for CountdownFuture<F> {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use std::future::{pending, ready, Future};
    use std::pin::pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake, Waker};

    use super::RecordingActionController;

    struct WakeCounter(AtomicUsize);

    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn recording_actions_are_serialized() {
        let controller = RecordingActionController::default();
        let first = controller.try_begin().expect("first action");
        assert!(controller.try_begin().is_none());
        drop(first);
        assert!(controller.try_begin().is_some());
    }

    #[test]
    fn cancellation_wakes_and_finishes_a_pending_countdown() {
        let controller = RecordingActionController::default();
        let mut countdown = pin!(controller.countdown(pending::<()>()));
        let wake_counter = Arc::new(WakeCounter(AtomicUsize::new(0)));
        let waker = Waker::from(wake_counter.clone());
        let mut cx = Context::from_waker(&waker);

        assert_eq!(countdown.as_mut().poll(&mut cx), Poll::Pending);
        assert!(controller.cancel_countdown());
        assert_eq!(wake_counter.0.load(Ordering::SeqCst), 1);
        assert_eq!(countdown.as_mut().poll(&mut cx), Poll::Ready(None));
        assert!(!controller.cancel_countdown());
    }

    #[test]
    fn completed_countdown_returns_output_and_clears_cancellation() {
        let controller = RecordingActionController::default();
        let mut countdown = pin!(controller.countdown(ready(7)));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);

        assert_eq!(countdown.as_mut().poll(&mut cx), Poll::Ready(Some(7)));
        assert!(!controller.cancel_countdown());
    }
}
