use anyhow::Result;
use std::ffi::OsString;

pub(crate) fn run(args: &[OsString]) -> Result<()> {
    super::trace_rs::run_as("trace", args)
}
