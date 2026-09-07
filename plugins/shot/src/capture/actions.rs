use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::capture::screenshot::CaptureFileReady;
use crate::{capture::output, platform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotAction {
    Copy,
    CopyPath,
    OpenFolder,
}

impl ShotAction {
    pub const ALL: &'static [ShotAction] = &[
        ShotAction::Copy,
        ShotAction::CopyPath,
        ShotAction::OpenFolder,
    ];
    pub const PINNED: &'static [ShotAction] = &[ShotAction::Copy, ShotAction::CopyPath];

    pub fn perform(self, path: &Path) -> Result<()> {
        match self {
            ShotAction::Copy => platform::copy_image_to_clipboard(path),
            ShotAction::CopyPath => platform::copy_path_to_clipboard(path),
            ShotAction::OpenFolder => crate::capture::completion::reveal(path),
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            ShotAction::Copy => "⧉",
            ShotAction::CopyPath => "∕",
            ShotAction::OpenFolder => "↗",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ShotAction::Copy => "Copy",
            ShotAction::CopyPath => "Copy Path",
            ShotAction::OpenFolder => "Open Folder",
        }
    }

    pub fn accel(self) -> char {
        match self {
            ShotAction::Copy => 'c',
            ShotAction::CopyPath => 'p',
            ShotAction::OpenFolder => 'o',
        }
    }

    pub fn done_message(self) -> &'static str {
        match self {
            ShotAction::Copy => "Copied screenshot to clipboard",
            ShotAction::CopyPath => "Copied screenshot path to clipboard",
            ShotAction::OpenFolder => "Opened screenshot folder",
        }
    }
}

pub fn perform_on_latest(action: ShotAction) -> Result<PathBuf> {
    let path = output::latest_screenshot()?;
    action.perform(&path)?;
    Ok(path)
}

pub(crate) fn perform_when_file_ready(
    surface: &'static str,
    action: ShotAction,
    file_ready: CaptureFileReady,
    perform: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let started = Instant::now();
    qol_runtime::probe!(
        "SHOT_FILE_ACTION",
        "surface={surface} action={} phase=waiting",
        action.label()
    );
    let ready = file_ready.wait();
    qol_runtime::probe!(
        "SHOT_FILE_ACTION",
        "surface={surface} action={} phase=file-ready outcome={} wait_ms={}",
        action.label(),
        if ready.is_ok() { "ok" } else { "failed" },
        started.elapsed().as_millis()
    );
    let result = ready.and_then(|()| perform());
    qol_runtime::probe!(
        "SHOT_FILE_ACTION",
        "surface={surface} action={} phase=complete outcome={} elapsed_ms={}",
        action.label(),
        if result.is_ok() { "ok" } else { "failed" },
        started.elapsed().as_millis()
    );
    if let Err(error) = &result {
        eprintln!("[qol-shot] {surface} action failed: {error:#}");
    }
    result
}

pub(crate) fn spawn_file_action(
    surface: &'static str,
    action: ShotAction,
    file_ready: CaptureFileReady,
    perform: impl FnOnce() -> Result<()> + Send + 'static,
) -> std::io::Result<()> {
    let worker = std::thread::Builder::new()
        .name(format!("qol-shot-{}", action.accel()))
        .spawn(move || {
            let _ = perform_when_file_ready(surface, action, file_ready, perform);
        });
    if let Err(error) = worker {
        qol_runtime::probe!(
            "SHOT_FILE_ACTION",
            "surface={surface} action={} phase=complete outcome=worker-failed",
            action.label()
        );
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{spawn_file_action, ShotAction};
    use crate::capture::screenshot::CaptureFileReady;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn file_action_completes_without_foreground_dispatch() {
        let file_ready = CaptureFileReady::test_pending();
        let signal = file_ready.clone();
        let (performed_tx, performed_rx) = mpsc::channel();
        spawn_file_action("test", ShotAction::Copy, file_ready, move || {
            performed_tx.send(()).unwrap();
            Ok(())
        })
        .unwrap();

        assert!(performed_rx
            .recv_timeout(Duration::from_millis(20))
            .is_err());
        signal.test_complete(Ok(()));
        performed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("file action remained parked after the readiness signal");
    }
}
