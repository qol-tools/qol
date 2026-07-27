use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::Path;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

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
        let path = path.as_ref();
        let file_name = path
            .file_name()
            .ok_or(CliSessionSubscriptionError::InvalidPath)?;
        let watched_directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or(CliSessionSubscriptionError::InvalidPath)?;
        let watched_directory = std::fs::canonicalize(watched_directory)
            .unwrap_or_else(|_| watched_directory.to_path_buf());
        let expected_path = watched_directory.join(file_name);
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(
            move |result: notify::Result<notify::Event>| match result {
                Ok(event)
                    if relevant_kind(&event.kind)
                        && event.paths.iter().any(|changed| changed == &expected_path) =>
                {
                    on_change();
                }
                Ok(_) => {}
                Err(error) => {
                    qol_runtime::probe!(
                        "CLI_SESSION_INTERPRETATION",
                        "event=subscription_error source=file error={error}"
                    );
                }
            },
        )
        .map_err(CliSessionSubscriptionError::Watcher)?;
        watcher
            .watch(&watched_directory, RecursiveMode::NonRecursive)
            .map_err(CliSessionSubscriptionError::Watcher)?;
        Ok(Self::from_guard(watcher))
    }
}

fn relevant_kind(kind: &EventKind) -> bool {
    kind.is_create() || kind.is_modify() || kind.is_remove()
}

#[derive(Debug)]
pub enum CliSessionSubscriptionError {
    InvalidPath,
    Watcher(notify::Error),
}

impl Display for CliSessionSubscriptionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPath => write!(formatter, "CLI session metadata path has no parent"),
            Self::Watcher(error) => {
                write!(formatter, "failed to watch CLI session metadata: {error}")
            }
        }
    }
}

impl Error for CliSessionSubscriptionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidPath => None,
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
}
