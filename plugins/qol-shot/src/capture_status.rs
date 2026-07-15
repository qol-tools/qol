use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

pub(crate) struct CaptureStatus {
    context: &'static str,
    stage: &'static str,
    title: String,
    subtitle: String,
    timeout: Option<Duration>,
}

impl CaptureStatus {
    pub(crate) fn persistent(
        context: &'static str,
        stage: &'static str,
        title: impl Into<String>,
        subtitle: impl Into<String>,
    ) -> Self {
        Self {
            context,
            stage,
            title: title.into(),
            subtitle: subtitle.into(),
            timeout: None,
        }
    }

    pub(crate) fn timed(
        context: &'static str,
        stage: &'static str,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        timeout: Duration,
    ) -> Self {
        Self {
            timeout: Some(timeout),
            ..Self::persistent(context, stage, title, subtitle)
        }
    }
}

#[derive(Clone)]
pub(crate) struct CaptureStatusUi {
    generation: Rc<Cell<u64>>,
    tracker: MonitorTracker,
}

impl CaptureStatusUi {
    pub(crate) fn new(tracker: MonitorTracker) -> Self {
        Self {
            generation: Rc::new(Cell::new(0)),
            tracker,
        }
    }

    pub(crate) fn prepare_selector(&self, cx: &mut App) {
        self.next_generation();
        crate::platform::hide_capture_status(cx);
    }

    pub(crate) fn show(&self, status: CaptureStatus, cx: &mut App) {
        let Some((monitor, _)) = self.tracker.snapshot_cursor() else {
            return;
        };
        let generation = self.next_generation();
        let shown = crate::platform::show_capture_status(
            monitor.bounds(),
            status.title,
            status.subtitle,
            cx,
        );
        qol_runtime::probe!(
            "SHOT_CAPTURE_STATUS",
            "context={} stage={} surface=selector-guide shown={shown} monitor={},{}",
            status.context,
            status.stage,
            monitor.bounds().origin.x.to_f64(),
            monitor.bounds().origin.y.to_f64()
        );
        if let Some(timeout) = status.timeout {
            self.dismiss_after(generation, timeout, cx);
        }
    }

    fn next_generation(&self) -> u64 {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        generation
    }

    fn dismiss_after(&self, generation: u64, timeout: Duration, cx: &mut App) {
        let current_generation = self.generation.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor().timer(timeout).await;
            if current_generation.get() != generation {
                return;
            }
            let _ = cx.update(crate::platform::hide_capture_status);
        })
        .detach();
    }
}
