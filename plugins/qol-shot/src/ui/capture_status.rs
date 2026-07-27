use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::toast::{Toast, ToastHost, ToastLayout, ToastTone};
use std::time::Duration;

pub(crate) struct CaptureStatus {
    context: &'static str,
    stage: &'static str,
    title: String,
    subtitle: String,
    tone: ToastTone,
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
            tone: ToastTone::Neutral,
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

    pub(crate) fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
        self
    }

    fn into_toast(self) -> Toast {
        let toast = Toast::new(self.title, self.subtitle)
            .layout(ToastLayout::Status)
            .tone(self.tone);
        match self.timeout {
            Some(timeout) => toast.timeout(timeout),
            None => toast.persistent(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct CaptureStatusUi {
    host: ToastHost,
}

impl CaptureStatusUi {
    pub(crate) fn new(tracker: MonitorTracker) -> Self {
        Self {
            host: ToastHost::new(tracker),
        }
    }

    pub(crate) fn hide(&self, cx: &mut App) {
        self.host.dismiss(cx);
    }

    pub(crate) fn prepare_selector(&self, cx: &mut App) {
        self.hide(cx);
    }

    pub(crate) fn show(&self, status: CaptureStatus, cx: &mut App) -> bool {
        let context = status.context;
        let stage = status.stage;
        let shown = self.host.show(status.into_toast(), cx).is_ok();
        qol_runtime::probe!(
            "SHOT_CAPTURE_STATUS",
            "context={context} stage={stage} surface=shared-toast shown={shown}"
        );
        shown
    }

    pub(crate) async fn wait_until_hidden(
        &self,
        cx: &mut AsyncApp,
    ) -> qol_gpui::popup_window::HiddenWindowsBarrier {
        self.host.wait_until_hidden(cx).await
    }
}
