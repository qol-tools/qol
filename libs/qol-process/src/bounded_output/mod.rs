//! Bounded command-output capture and guarded execution.

mod guarded;

pub use guarded::run_guarded_with_output_timeout;
use std::process::ExitStatus;

#[derive(Debug)]
pub struct CapturedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

impl CapturedOutput {
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug)]
pub struct CompletedCommandOutput {
    pub status: ExitStatus,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
}

#[derive(Debug)]
pub enum BoundedCommandOutput {
    Completed(CompletedCommandOutput),
    TimedOut {
        stdout: CapturedOutput,
        stderr: CapturedOutput,
    },
}
