#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::Result;
use qol_host_fixes::policy::cli::{parse_args, ParsedCommand, ResidentCommand};
use qol_host_fixes::residency::HostResidency;

use super::{
    apply_residency, emit_request, emit_result, escalate, NoopPhaseRecorder, PhaseRecorder,
};

pub fn run_cli(args: &[String]) -> i32 {
    run_cli_with(args, escalate, &mut NoopPhaseRecorder)
}

pub(super) fn run_cli_with<R>(
    args: &[String],
    escalate_command: impl FnOnce(&ResidentCommand) -> Result<()>,
    recorder: &mut R,
) -> i32
where
    R: PhaseRecorder,
{
    let carrier = emit_request(args, recorder);
    let parsed = match parse_args(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("resident-policy: {error:#}");
            let reason = qol_host_fixes::policy::trace::sanitize_reason(&format!("{error:#}"));
            emit_result(args, &carrier, "invalid", &reason, recorder);
            return 2;
        }
    };
    let command = parsed.command;
    if let ResidentCommand::Residency { resident } = command {
        let target = if resident {
            HostResidency::Resident
        } else {
            HostResidency::Portable
        };
        let result = apply_residency(target);
        if let Err(error) = &result {
            eprintln!("resident-policy: {error:#}");
        }
        let code = if result.is_ok() { 0 } else { 1 };
        let outcome = if result.is_ok() { "ok" } else { "error" };
        let reason = qol_host_fixes::policy::trace::sanitize_reason(
            &result
                .as_ref()
                .err()
                .map(|error| format!("{error:#}"))
                .unwrap_or_default(),
        );
        emit_result(args, &carrier, outcome, &reason, recorder);
        return code;
    }
    if matches!(command, ResidentCommand::Status | ResidentCommand::Help) {
        let result = qol_host_fixes::policy::nvidia::run_resident_policy_cli(args);
        if let Err(error) = &result {
            eprintln!("resident-policy: {error:#}");
        }
        let code = result.as_ref().copied().unwrap_or(1);
        let outcome = qol_host_fixes::policy::trace::outcome_of(&result);
        let reason = qol_host_fixes::policy::trace::error_reason(&result);
        emit_result(args, &carrier, outcome, &reason, recorder);
        return code;
    }
    if qol_host_fixes::privilege::is_elevated() {
        let result = qol_host_fixes::policy::nvidia::run_resident_policy_cli(args);
        if let Err(error) = &result {
            eprintln!("resident-policy: {error:#}");
        }
        let code = result.as_ref().copied().unwrap_or(1);
        let outcome = qol_host_fixes::policy::trace::outcome_of(&result);
        let reason = qol_host_fixes::policy::trace::error_reason(&result);
        emit_result(args, &carrier, outcome, &reason, recorder);
        return code;
    }
    match escalate_command(&command) {
        Ok(()) => {
            emit_result(args, &carrier, "ok", "", recorder);
            0
        }
        Err(error) => {
            eprintln!("resident-policy: {error:#}");
            let reason = qol_host_fixes::policy::trace::sanitize_reason(&format!("{error:#}"));
            emit_result(args, &carrier, "error", &reason, recorder);
            1
        }
    }
}

pub fn run_hidden(raw_args: &[String]) -> i32 {
    run_hidden_with(raw_args, &mut NoopPhaseRecorder)
}

pub(super) fn run_hidden_with<R>(raw_args: &[String], recorder: &mut R) -> i32
where
    R: PhaseRecorder,
{
    let carrier = emit_request(raw_args, recorder);
    let parsed: ParsedCommand = match parse_args(raw_args) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("resident-policy: {error:#}");
            let reason = qol_host_fixes::policy::trace::sanitize_reason(&format!("{error:#}"));
            emit_result(raw_args, &carrier, "invalid", &reason, recorder);
            return 2;
        }
    };
    if !parsed.hidden {
        eprintln!("hidden residency route requires the __resident-policy-<op> shape");
        emit_result(
            raw_args,
            &carrier,
            "refused",
            "hidden residency route requires the __resident-policy-<op> shape",
            recorder,
        );
        return 2;
    }
    if !qol_host_fixes::privilege::is_elevated() {
        eprintln!("hidden residency operation requires root");
        emit_result(
            raw_args,
            &carrier,
            "refused",
            "hidden residency operation requires root",
            recorder,
        );
        return 2;
    }
    let result = qol_host_fixes::policy::nvidia::run_resident_policy_cli(raw_args);
    if let Err(error) = &result {
        eprintln!("resident-policy: {error:#}");
    }
    let code = result.as_ref().copied().unwrap_or(1);
    let outcome = qol_host_fixes::policy::trace::outcome_of(&result);
    let reason = qol_host_fixes::policy::trace::error_reason(&result);
    emit_result(raw_args, &carrier, outcome, &reason, recorder);
    code
}
