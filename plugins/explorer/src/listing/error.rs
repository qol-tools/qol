use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureReason {
    Permission,
    Missing,
    NotAFolder,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingError {
    pub folder: PathBuf,
    pub reason: FailureReason,
}

impl ListingError {
    pub fn from_io(folder: PathBuf, error: &io::Error) -> Self {
        let reason = match error.kind() {
            io::ErrorKind::PermissionDenied => FailureReason::Permission,
            io::ErrorKind::NotFound => FailureReason::Missing,
            io::ErrorKind::NotADirectory => FailureReason::NotAFolder,
            _ => FailureReason::Other(error.to_string()),
        };
        Self { folder, reason }
    }
}

impl fmt::Display for ListingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let folder = super::display_name(self.folder.as_os_str());
        match &self.reason {
            FailureReason::Permission => write!(f, "No permission to read {folder}"),
            FailureReason::Missing => write!(f, "{folder} does not exist"),
            FailureReason::NotAFolder => write!(f, "{folder} is not a folder"),
            FailureReason::Other(message) => write!(f, "Could not read {folder}: {message}"),
        }
    }
}

impl std::error::Error for ListingError {}
