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
    )
}

fn launch_item_with<OpenFile, LaunchApp>(
    item: &search::ResultItem<'_>,
    mut open_file: OpenFile,
    mut launch_app: LaunchApp,
) -> Result<(), LaunchError>
where
    OpenFile: FnMut(&Path) -> io::Result<()>,
    LaunchApp: FnMut(&Path, &[String]) -> io::Result<()>,
{
    match item {
        search::ResultItem::App(entry) => {
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
        )
        .unwrap_err();

        assert!(matches!(error, LaunchError::AppFailed { .. }));
        assert!(error.to_string().contains("executable not found"));
    }
}
