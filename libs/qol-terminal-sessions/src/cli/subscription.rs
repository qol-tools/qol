use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

use super::CliSessionChangeHandler;

pub struct CliSessionSubscription {
    _guard: Box<dyn Send>,
}

impl CliSessionSubscription {
    pub fn from_guard(guard: impl Send + 'static) -> Self {
        Self {
            _guard: Box::new(guard),
        }
    }

    pub fn watch_file(
        path: impl AsRef<Path>,
        on_change: CliSessionChangeHandler,
    ) -> Result<Self, CliSessionSubscriptionError> {
        let watch = qol_watch::watch_file(path, move || on_change())
            .map_err(CliSessionSubscriptionError::Watcher)?;
        Ok(Self::from_guard(watch))
    }

    pub fn watch_dir(
        path: impl AsRef<Path>,
        on_change: CliSessionChangeHandler,
    ) -> Result<Self, CliSessionSubscriptionError> {
        let roots = [qol_watch::WatchRoot::shallow(path.as_ref())];
        let watch = qol_watch::watch(&roots, move |notice| {
            if matches!(notice, qol_watch::WatchNotice::Changed(_)) {
                on_change();
            }
        })
        .map_err(CliSessionSubscriptionError::Watcher)?;
        Ok(Self::from_guard(watch))
    }
}

#[derive(Debug)]
pub enum CliSessionSubscriptionError {
    Watcher(qol_watch::WatchError),
}

impl Display for CliSessionSubscriptionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Watcher(error) => {
                write!(formatter, "failed to watch CLI session metadata: {error}")
            }
        }
    }
}

impl Error for CliSessionSubscriptionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Watcher(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::CliSessionSubscription;

    #[test]
    fn file_subscription_emits_when_metadata_changes() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("metadata.jsonl");
        std::fs::write(&path, "old\n").unwrap();
        let (changed, events) = mpsc::channel();
        let _subscription = CliSessionSubscription::watch_file(
            &path,
            Arc::new(move || {
                let _ = changed.send(());
            }),
        )
        .unwrap();

        std::fs::write(path, "new\n").unwrap();

        events.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn directory_subscription_emits_when_a_session_file_appears() {
        let root = TempDir::new().unwrap();
        let (changed, events) = mpsc::channel();
        let _subscription = CliSessionSubscription::watch_dir(
            root.path(),
            Arc::new(move || {
                let _ = changed.send(());
            }),
        )
        .unwrap();

        std::fs::write(root.path().join("2026-08-27T09-00-00-000Z_lane.jsonl"), "").unwrap();

        events.recv_timeout(Duration::from_secs(3)).unwrap();
    }
}
