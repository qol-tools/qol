pub(crate) struct CaptureGuard {
    action: &'static str,
}

pub(crate) fn try_acquire(action: &'static str) -> Option<CaptureGuard> {
    qol_runtime::probe!(
        "SHOT_CAPTURE_LOCK",
        "action={action} result=acquired platform=noop"
    );
    Some(CaptureGuard { action })
}

impl Drop for CaptureGuard {
    fn drop(&mut self) {
        qol_runtime::probe!(
            "SHOT_CAPTURE_LOCK",
            "action={} result=released",
            self.action
        );
    }
}
