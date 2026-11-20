use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

use crate::ChordCommand;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliChordFileError {
    message: String,
}

impl CliChordFileError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for CliChordFileError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for CliChordFileError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliTakenChord {
    pub wanted: ChordCommand,
    pub holder: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliChordBinding {
    pub text: Option<String>,
    pub added: Vec<ChordCommand>,
    pub taken: Vec<CliTakenChord>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CliChordRelease {
    pub text: Option<String>,
    pub leaves_nothing: bool,
}

pub trait CliChordBinder: Send + Sync {
    fn file(&self, home: &Path) -> PathBuf;

    fn bind(
        &self,
        current: Option<&str>,
        wanted: &[ChordCommand],
    ) -> Result<CliChordBinding, CliChordFileError>;

    fn release(
        &self,
        current: &str,
        bound: &[ChordCommand],
    ) -> Result<CliChordRelease, CliChordFileError>;
}
