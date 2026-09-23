use anyhow::Result;
use std::ffi::OsString;
use std::path::Path;
use std::process::Child;

mod platform;

pub fn available() -> bool {
    platform::available()
}

pub fn run_privileged(label: &str, script: &str, args: &[String]) -> Result<()> {
    platform::run(label, script, args)
}

pub fn spawn_privileged(label: &str, program: &Path, args: &[OsString]) -> Result<Child> {
    platform::spawn(label, program, args)
}
