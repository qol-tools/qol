use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::discovery::search;

mod platform;

#[derive(Debug, PartialEq, Eq)]
pub enum LaunchError {
    NotFound { path: PathBuf },
    OpenFailed { path: PathBuf, message: String },
    AppFailed { name: String, message: String },
}

impl fmt::Display for LaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => {
                write!(formatter, "file no longer exists: {}", path.display())
            }
            Self::OpenFailed { message, .. } => write!(formatter, "Could not open file: {message}"),
            Self::AppFailed { name, message } => {
                write!(formatter, "Could not launch {name}: {message}")
            }
        }
    }
}

pub fn open_path(path: &Path) -> io::Result<()> {
    qol_apps::desktop_integration::open_with_default_app(path)
}

pub fn launch_item(item: &search::ResultItem<'_>) -> Result<(), LaunchError> {
    launch_item_with(
        item,
        |path| qol_apps::desktop_integration::open_with_default_app_checked(path),
        platform::launch_app,
        |path, exec| {
            platform::daemon_action_args(path, exec)
                .map(|(target, action)| qol_plugin_api::host_exec::run_exec(&target, &action))
        },
    )
}

fn launch_item_with<OpenFile, LaunchApp, DaemonLaunch>(
    item: &search::ResultItem<'_>,
    mut open_file: OpenFile,
    mut launch_app: LaunchApp,
    mut daemon_launch: DaemonLaunch,
) -> Result<(), LaunchError>
where
    OpenFile: FnMut(&Path) -> io::Result<()>,
    LaunchApp: FnMut(&Path, &[String]) -> io::Result<()>,
    DaemonLaunch: FnMut(&Path, &[String]) -> Option<i32>,
{
    match item {
        search::ResultItem::App(entry) => {
            if let Some(code) = daemon_launch(&entry.path, &entry.exec) {
                if code == 0 {
                    return Ok(());
                }
                return Err(LaunchError::AppFailed {
                    name: entry.name.clone(),
                    message: format!("qol daemon action failed with exit code {code}"),
                });
            }
            eprintln!("[launch] app: {:?} exec: {:?}", entry.name, entry.exec);
            launch_app(&entry.path, &entry.exec).map_err(|error| LaunchError::AppFailed {
                name: entry.name.clone(),
                message: error.to_string(),
            })
        }
        search::ResultItem::File(entry) => {
            if !entry.path.exists() {
                return Err(LaunchError::NotFound {
                    path: entry.path.clone(),
                });
            }
            open_file(&entry.path).map_err(|error| LaunchError::OpenFailed {
                path: entry.path.clone(),
                message: error.to_string(),
            })
        }
        search::ResultItem::Flow(entry) => Err(LaunchError::AppFailed {
            name: entry.title.clone(),
            message: "flow entries open inside the launcher flow session".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{launch_item_with, LaunchError};
    use crate::discovery::search::ResultItem;
    use crate::discovery::{AppEntry, FileEntry};
    use std::io;
    use std::path::PathBuf;

    #[test]
    fn missing_file_returns_not_found_without_calling_opener() {
        let temp = tempfile::tempdir().unwrap();
        let entry = FileEntry {
            name: "missing.ods".to_string(),
            path: temp.path().join("missing.ods"),
        };

        let error = launch_item_with(
            &ResultItem::File(&entry),
            |_| panic!("missing files must not reach the opener"),
            |_, _| Ok(()),
            |_, _| None,
        )
        .unwrap_err();

        assert!(matches!(error, LaunchError::NotFound { .. }));
        assert!(error.to_string().contains("file no longer exists"));
    }

    #[test]
    fn existing_unhandled_file_returns_open_failed_with_reason() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("document.unhandled");
        std::fs::write(&path, "content").unwrap();
        let entry = FileEntry {
            name: "document.unhandled".to_string(),
            path,
        };

        let error = launch_item_with(
            &ResultItem::File(&entry),
            |_| Err(io::Error::other("no default handler")),
            |_, _| Ok(()),
            |_, _| None,
        )
        .unwrap_err();

        assert!(matches!(error, LaunchError::OpenFailed { .. }));
        assert!(error.to_string().contains("no default handler"));
    }

    #[test]
    fn missing_app_executable_returns_app_failure_with_reason() {
        let entry = AppEntry {
            name: "Missing App".to_string(),
            exec: vec!["missing-app-executable".to_string()],
            path: PathBuf::from("/apps/missing.desktop"),
        };

        let error = launch_item_with(
            &ResultItem::App(&entry),
            |_| Ok(()),
            |_, _| {
                Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "executable not found",
                ))
            },
            |_, _| None,
        )
        .unwrap_err();

        assert!(matches!(error, LaunchError::AppFailed { .. }));
        assert!(error.to_string().contains("executable not found"));
    }

    #[test]
    fn qol_daemon_entry_launches_in_process_without_calling_os() {
        let entry = AppEntry {
            name: "Monitor Settings".to_string(),
            exec: vec![
                "/Applications/qol-tray.app/Contents/MacOS/qol-courier".to_string(),
                "exec".to_string(),
                "plugin-monitor".to_string(),
                "settings".to_string(),
            ],
            path: PathBuf::from("/Applications/Monitor Settings.app"),
        };

        let result = launch_item_with(
            &ResultItem::App(&entry),
            |_| panic!("files must not reach the opener"),
            |_, _| panic!("qol daemon entries must not reach the OS launcher"),
            |_, _| Some(0),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn failed_qol_daemon_action_returns_app_failure_with_reason() {
        let entry = AppEntry {
            name: "Monitor Settings".to_string(),
            exec: vec![
                "/Applications/qol-tray.app/Contents/MacOS/qol-courier".to_string(),
                "exec".to_string(),
                "plugin-monitor".to_string(),
                "settings".to_string(),
            ],
            path: PathBuf::from("/Applications/Monitor Settings.app"),
        };

        let error = launch_item_with(
            &ResultItem::App(&entry),
            |_| panic!("files must not reach the opener"),
            |_, _| panic!("failed qol daemon entries must not reach the OS launcher"),
            |_, _| Some(1),
        )
        .unwrap_err();

        assert!(matches!(error, LaunchError::AppFailed { .. }));
        assert!(error.to_string().contains("exit code 1"));
    }

    #[test]
    fn non_qol_app_entry_falls_back_to_os_launch() {
        let entry = AppEntry {
            name: "Firefox".to_string(),
            exec: vec!["/usr/bin/firefox".to_string()],
            path: PathBuf::from("/Applications/Firefox.app"),
        };
        let mut os_launched = false;

        let result = launch_item_with(
            &ResultItem::App(&entry),
            |_| Ok(()),
            |path, exec| {
                os_launched = true;
                assert_eq!(path, &entry.path);
                assert_eq!(exec, &entry.exec);
                Ok(())
            },
            |_, _| None,
        );

        assert!(result.is_ok());
        assert!(os_launched);
    }
}
