use gpui::*;
use qol_gpui::monitor::MonitorTracker;
use qol_gpui::toast::{Toast, ToastHost, ToastLayout, ToastTone};
use std::path::PathBuf;
use std::time::Duration;

pub(crate) struct CaptureStatus {
    context: &'static str,
    stage: &'static str,
    title: String,
    subtitle: String,
    tone: ToastTone,
    timeout: Option<Duration>,
    layout: ToastLayout,
    saved_file: Option<PathBuf>,
    busy: bool,
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
            layout: ToastLayout::status(),
            saved_file: None,
            busy: false,
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
            layout: ToastLayout::compact(),
            ..Self::persistent(context, stage, title, subtitle)
        }
    }

    pub(crate) fn tone(mut self, tone: ToastTone) -> Self {
        self.tone = tone;
        self
    }

    pub(crate) fn saved_file(mut self, path: PathBuf) -> Self {
        self.saved_file = Some(path);
        self
    }

    pub(crate) fn busy(mut self) -> Self {
        self.busy = true;
        self
    }

    fn into_toast(self) -> Toast {
        let toast = Toast::new(self.title, self.subtitle, self.layout)
            .tone(self.tone)
            .group("qol-shot")
            .key(self.context);
        let toast = if self.busy { toast.busy() } else { toast };
        let toast = match self.saved_file {
            Some(path) => toast.artifact(path),
            None => toast,
        };
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
        qol_runtime::probe!(
            "SHOT_CAPTURE_STATUS",
            "stage=dismissed surface=shared-toast"
        );
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
