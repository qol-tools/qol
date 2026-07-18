use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RevealSource {
    Automatic,
    #[cfg(target_os = "linux")]
    Notification,
    PreviewAction,
    Toast,
}

impl RevealSource {
    fn label(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            #[cfg(target_os = "linux")]
            Self::Notification => "notification",
            Self::PreviewAction => "preview-action",
            Self::Toast => "toast",
        }
    }
}

#[derive(Clone)]
pub(crate) struct RevealTarget {
    state: Arc<RevealState>,
}

struct RevealState {
    path: PathBuf,
    opened: Mutex<bool>,
}

impl RevealTarget {
    fn new(path: &Path) -> Self {
        Self {
            state: Arc::new(RevealState {
                path: path.to_path_buf(),
                opened: Mutex::new(false),
            }),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.state.path
    }

    pub(crate) fn open(&self, source: RevealSource) -> Result<()> {
        let mut opened = self
            .state
            .opened
            .lock()
            .map_err(|_| anyhow!("folder reveal state lock was poisoned"))?;
        if *opened {
            trace_reveal(source, "already-opened", self.path());
            return Ok(());
        }
        if let Err(error) = reveal(self.path()) {
            trace_reveal(source, "failed", self.path());
            return Err(error);
        }
        *opened = true;
        trace_reveal(source, "opened", self.path());
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewExit {
    Intentional,
    OpenFolder,
    Pinned,
    LostFocus,
    Superseded,
    Unavailable,
}

impl PreviewExit {
    fn allows_automatic_reveal(self) -> bool {
        matches!(self, Self::Intentional)
    }
}

#[derive(Clone)]
pub(crate) struct SavedAnnouncement {
    pub(crate) title: &'static str,
    pub(crate) message: String,
    pub(crate) target: RevealTarget,
    pub(crate) open_automatically: bool,
}

impl SavedAnnouncement {
    pub(crate) fn reveal_automatically(&self) {
        if let Err(error) = self.target.open(RevealSource::Automatic) {
            eprintln!("[qol-shot] automatic folder reveal failed: {error:#}");
        }
    }
}

#[derive(Clone)]
pub(crate) struct PreviewCompletion {
    target: RevealTarget,
    open_folder_after_save: bool,
    lifecycle: Arc<PreviewLifecycle>,
}

struct PreviewLifecycle {
    announced: AtomicBool,
    pending_automatic: AtomicBool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutomaticReveal {
    Open,
    Pending,
    Suppressed,
}

impl PreviewLifecycle {
    fn announce(&self) -> Option<bool> {
        if self.announced.swap(true, Ordering::AcqRel) {
            return None;
        }
        Some(self.pending_automatic.swap(false, Ordering::AcqRel))
    }

    fn finish(&self, automatic_enabled: bool, exit: PreviewExit) -> AutomaticReveal {
        if !automatic_enabled || !exit.allows_automatic_reveal() {
            return AutomaticReveal::Suppressed;
        }
        self.pending_automatic.store(true, Ordering::Release);
        if !self.announced.load(Ordering::Acquire) {
            return AutomaticReveal::Pending;
        }
        if self.pending_automatic.swap(false, Ordering::AcqRel) {
            return AutomaticReveal::Open;
        }
        AutomaticReveal::Suppressed
    }
}

impl PreviewCompletion {
    pub(crate) fn new(path: &Path, open_folder_after_save: bool) -> Self {
        Self {
            target: RevealTarget::new(path),
            open_folder_after_save,
            lifecycle: Arc::new(PreviewLifecycle {
                announced: AtomicBool::new(false),
                pending_automatic: AtomicBool::new(false),
            }),
        }
    }

    pub(crate) fn announce(&self) -> Option<SavedAnnouncement> {
        let open_automatically = self.lifecycle.announce()?;
        Some(SavedAnnouncement {
            title: "Screenshot saved",
            message: file_label(self.target.path()),
            target: self.target.clone(),
            open_automatically,
        })
    }

    pub(crate) fn announce_saved(&self) {
        let Some(announcement) = self.announce() else {
            return;
        };
        crate::platform::show_saved_notification(
            announcement.title,
            &announcement.message,
            8_000,
            announcement.target.clone(),
        );
        if announcement.open_automatically {
            announcement.reveal_automatically();
        }
    }

    pub(crate) fn open(&self, source: RevealSource) -> Result<()> {
        self.target.open(source)
    }

    pub(crate) fn finish(self, exit: PreviewExit) {
        let decision = self.lifecycle.finish(self.open_folder_after_save, exit);
        match decision {
            AutomaticReveal::Open => self.open_automatically(),
            AutomaticReveal::Pending | AutomaticReveal::Suppressed => qol_runtime::probe!(
                "SHOT_SAVED_REVEAL",
                "kind=preview result={decision:?} exit={exit:?} file={}",
                file_label(self.target.path())
            ),
        }
    }

    fn open_automatically(&self) {
        if let Err(error) = self.target.open(RevealSource::Automatic) {
            eprintln!("[qol-shot] automatic folder reveal failed: {error:#}");
        }
    }
}

pub(crate) fn background_saved(
    title: &str,
    message: &str,
    path: &Path,
    open_folder_after_save: bool,
) {
    let target = RevealTarget::new(path);
    crate::platform::show_saved_notification(title, message, 8_000, target.clone());
    if !open_folder_after_save {
        trace_reveal(RevealSource::Automatic, "disabled", path);
        return;
    }
    if let Err(error) = target.open(RevealSource::Automatic) {
        eprintln!("[qol-shot] automatic folder reveal failed: {error:#}");
    }
}

pub(crate) fn reveal(path: &Path) -> Result<()> {
    qol_apps::desktop_integration::reveal_in_file_manager(path)
        .with_context(|| format!("failed to open containing folder for {}", path.display()))
}

fn trace_reveal(source: RevealSource, result: &str, path: &Path) {
    qol_runtime::probe!(
        "SHOT_SAVED_REVEAL",
        "kind={} result={result} file={}",
        source.label(),
        file_label(path)
    );
}

pub(crate) fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("capture")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{AutomaticReveal, PreviewExit, PreviewLifecycle};
    use std::sync::atomic::AtomicBool;

    fn lifecycle() -> PreviewLifecycle {
        PreviewLifecycle {
            announced: AtomicBool::new(false),
            pending_automatic: AtomicBool::new(false),
        }
    }

    #[test]
    fn preview_exit_controls_automatic_reveal() {
        let cases = [
            (PreviewExit::Intentional, true),
            (PreviewExit::Unavailable, false),
            (PreviewExit::OpenFolder, false),
            (PreviewExit::Pinned, false),
            (PreviewExit::LostFocus, false),
            (PreviewExit::Superseded, false),
        ];

        for (exit, expected) in cases {
            assert_eq!(exit.allows_automatic_reveal(), expected, "{exit:?}");
        }
    }

    #[test]
    fn non_user_exit_cannot_reveal_after_saved_announcement() {
        for exit in [
            PreviewExit::LostFocus,
            PreviewExit::Superseded,
            PreviewExit::Unavailable,
        ] {
            let lifecycle = lifecycle();
            assert_eq!(lifecycle.announce(), Some(false));
            assert_eq!(
                lifecycle.finish(true, exit),
                AutomaticReveal::Suppressed,
                "{exit:?}"
            );
        }
    }

    #[test]
    fn intentional_exit_releases_reveal_only_after_save() {
        let before_save = lifecycle();
        assert_eq!(
            before_save.finish(true, PreviewExit::Intentional),
            AutomaticReveal::Pending
        );
        assert_eq!(before_save.announce(), Some(true));

        let after_save = lifecycle();
        assert_eq!(after_save.announce(), Some(false));
        assert_eq!(
            after_save.finish(true, PreviewExit::Intentional),
            AutomaticReveal::Open
        );
    }

    #[test]
    fn disabled_automatic_reveal_never_becomes_pending() {
        let lifecycle = lifecycle();
        assert_eq!(
            lifecycle.finish(false, PreviewExit::Intentional),
            AutomaticReveal::Suppressed
        );
        assert_eq!(lifecycle.announce(), Some(false));
    }
}
