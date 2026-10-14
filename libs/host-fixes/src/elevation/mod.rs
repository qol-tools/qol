use anyhow::Result;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::process::{Child, ExitStatus};

mod platform;

const DISMISSED: i32 = 126;
const NOT_AUTHORIZED: i32 = 127;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivilegedExit {
    status: ExitStatus,
}

impl PrivilegedExit {
    pub fn check(status: ExitStatus) -> Result<()> {
        if status.success() {
            return Ok(());
        }
        Err(Self { status }.into())
    }

    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    pub fn dismissed(&self) -> bool {
        self.code() == Some(DISMISSED)
    }

    pub fn not_authorized(&self) -> bool {
        self.code() == Some(NOT_AUTHORIZED)
    }
}

impl fmt::Display for PrivilegedExit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pkexec exited with {}", self.status)
    }
}

impl std::error::Error for PrivilegedExit {}

pub fn available() -> bool {
    platform::available()
}

pub fn run_privileged(label: &str, script: &str, args: &[String]) -> Result<()> {
    platform::run(label, script, args)
}

pub fn spawn_privileged(label: &str, program: &Path, args: &[OsString]) -> Result<Child> {
    platform::spawn(label, program, args, false)
}

pub fn spawn_privileged_piped(label: &str, program: &Path, args: &[OsString]) -> Result<Child> {
    platform::spawn(label, program, args, true)
}
