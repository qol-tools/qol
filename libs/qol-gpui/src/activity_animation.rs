use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Context, ElementId, IntoElement, RenderOnce, Task, Window};

pub const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);
const CYCLE: Duration = Duration::from_millis(1200);

#[derive(IntoElement)]
pub struct ActivityAnimation {
    id: ElementId,
    active: bool,
    interval: Duration,
    child: AnyElement,
}

impl ActivityAnimation {
    pub fn new(id: impl Into<ElementId>, active: bool, child: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            active,
            interval: FRAME_INTERVAL,
            child: child.into_any_element(),
        }
    }

    pub fn interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }
}

#[derive(Default)]
struct FrameClock {
    pending: bool,
    task: Option<Task<()>>,
}

impl FrameClock {
    fn schedule(&mut self, interval: Duration, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(interval).await;
            let _ = this.update(cx, |clock, cx| {
                clock.pending = false;
                cx.notify();
            });
        }));
    }
}

impl RenderOnce for ActivityAnimation {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.active {
            let clock = window.use_keyed_state(self.id, cx, |_, _| FrameClock::default());
            clock.update(cx, |clock, cx| clock.schedule(self.interval, cx));
        }
        self.child
    }
}

pub(crate) fn progress() -> f32 {
    progress_of(CYCLE)
}

pub(crate) fn progress_of(cycle: Duration) -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    cycle_progress(START.get_or_init(Instant::now).elapsed(), cycle)
}

fn cycle_progress(elapsed: Duration, cycle: Duration) -> f32 {
    (elapsed.as_secs_f64() % cycle.as_secs_f64() / cycle.as_secs_f64()) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_phase_wraps_without_depending_on_frame_count() {
        for (millis, expected) in [(0, 0.0), (300, 0.25), (600, 0.5), (1200, 0.0), (1500, 0.25)] {
            assert!(
                (cycle_progress(Duration::from_millis(millis), CYCLE) - expected).abs() < 0.001
            );
        }
    }
}
