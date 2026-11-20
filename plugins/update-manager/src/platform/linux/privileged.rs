use std::ffi::OsString;
use std::path::Path;
use std::process::{Child, ExitStatus};

use qol_host_fixes::elevation::{
    run_privileged, spawn_privileged_piped, PrivilegedExit, PromptTimedOut,
};

use crate::updates::Refusal;

const SHELL: &str = "/bin/sh";
const LABEL: &str = "qol-update-manager";

pub(crate) fn spawn(script: &str, args: &[String]) -> Result<Child, Refusal> {
    let mut all: Vec<OsString> = vec!["-c".into(), script.into(), LABEL.into()];
    all.extend(args.iter().map(OsString::from));
    spawn_privileged_piped(LABEL, Path::new(SHELL), &all)
        .map_err(|error| Refusal::Failed(format!("{error:#}")))
}

pub(crate) fn refusal(status: ExitStatus) -> Refusal {
    match PrivilegedExit::check(status) {
        Ok(()) => Refusal::Failed("the helper stopped".into()),
        Err(error) => classify(&error),
    }
}

pub(crate) fn classify(error: &anyhow::Error) -> Refusal {
    if error.is::<PromptTimedOut>() {
        return Refusal::TimedOut;
    }
    match error.downcast_ref::<PrivilegedExit>() {
        Some(exit) if exit.dismissed() => Refusal::Dismissed,
        Some(exit) if exit.not_authorized() => Refusal::Denied,
        _ => Refusal::Failed(format!("{error:#}")),
    }
}

pub(crate) fn refused(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("not authorized")
        || stderr.contains("access denied")
        || stderr.contains("interactive authentication required")
}

pub(crate) fn run_script(script: &str, args: &[String]) -> Result<(), Refusal> {
    run_privileged(LABEL, script, args).map_err(|error| classify(&error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polkit_refusals_are_recognised() {
        assert!(refused(
            "Failed to reboot system via logind: Interactive authentication required."
        ));
        assert!(refused("Access denied"));
        assert!(!refused("Unit not found"));
    }

    #[test]
    fn a_password_prompt_left_open_is_a_timed_out_refusal() {
        assert_eq!(
            classify(&anyhow::Error::from(PromptTimedOut)),
            Refusal::TimedOut
        );
    }
}
