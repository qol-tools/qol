use crate::plugins::PluginManager;
use anyhow::{bail, Context, Result};
use qol_host_fixes::policy::cli::{parse_args, ParsedCommand, ResidentCommand};
use qol_host_fixes::residency::HostResidency;
use std::sync::{Arc, Mutex};

mod give_back;
mod restore;
pub(crate) use give_back::{give_back_all, give_back_one, GiveBackReport};
pub use restore::{restore_all, RestoreEntry, RestoreReport};

pub use qol_host_fixes::policy::nvidia::{fragment_path, NVIDIA_POLICY_ID};

pub fn apply_residency(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    target: HostResidency,
) -> Result<()> {
    apply_residency_with(
        target,
        HostResidency::current(),
        HostResidency::set,
        restore_all,
        || give_back_all(plugin_manager),
    )
}

fn apply_residency_without_plugins(target: HostResidency) -> Result<()> {
    apply_residency_with(
        target,
        HostResidency::current(),
        HostResidency::set,
        restore_all,
        GiveBackReport::empty,
    )
}

fn apply_residency_with(
    target: HostResidency,
    previous: HostResidency,
    set: impl FnOnce(HostResidency) -> Result<()>,
    restore: impl FnOnce() -> RestoreReport,
    give_back: impl FnOnce() -> GiveBackReport,
) -> Result<()> {
    set(target)?;
    if !previous.is_resident() || target.is_resident() {
        return Ok(());
    }
    give_back().log_failures("residency disabled");
    let report = restore();
    let failures: Vec<String> = report
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            RestoreEntry::Failed { policy, error } => Some(format!("{policy}: {error}")),
            RestoreEntry::Restored { .. } => None,
        })
        .collect();
    if failures.is_empty() {
        return Ok(());
    }
    bail!(
        "residency is off but the host was left changed: {}",
        failures.join("; ")
    )
}

trait PhaseRecorder {
    fn on_request(&mut self);
    fn on_result(&mut self);
}

struct NoopPhaseRecorder;

impl PhaseRecorder for NoopPhaseRecorder {
    fn on_request(&mut self) {}
    fn on_result(&mut self) {}
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CountedPhaseRecorder {
    requests: usize,
    results: usize,
}

#[cfg(test)]
impl PhaseRecorder for CountedPhaseRecorder {
    fn on_request(&mut self) {
        self.requests += 1;
    }

    fn on_result(&mut self) {
        self.results += 1;
    }
}

fn emit_request<R: PhaseRecorder>(
    args: &[String],
    recorder: &mut R,
) -> qol_host_fixes::policy::trace::CarrierObservation {
    let carrier = qol_host_fixes::policy::trace::cli_request(args);
    recorder.on_request();
    carrier
}

fn emit_result<R: PhaseRecorder>(
    args: &[String],
    carrier: &qol_host_fixes::policy::trace::CarrierObservation,
    outcome: &str,
    reason: &str,
    recorder: &mut R,
) {
    qol_host_fixes::policy::trace::cli_result(args, carrier, outcome, reason);
    recorder.on_result();
}

pub fn run_cli(args: &[String]) -> i32 {
    run_cli_with(args, escalate, &mut NoopPhaseRecorder)
}

fn run_cli_with<R>(
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
        let result = apply_residency_without_plugins(target);
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

fn run_hidden_with<R>(raw_args: &[String], recorder: &mut R) -> i32
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

fn escalate(command: &ResidentCommand) -> Result<()> {
    let executable = std::env::current_exe().context("failed to resolve the tray executable")?;
    let mut args = vec![
        format!("__resident-policy-{}", command.operation()),
        "--policy".to_string(),
        NVIDIA_POLICY_ID.to_string(),
    ];
    if let Some(owner) = command.owner() {
        args.push("--owner".to_string());
        args.push(owner.as_str().to_string());
    }
    let mut child = qol_host_fixes::elevation::spawn_privileged(
        "qol-resident-policy",
        &executable,
        &args
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>(),
    )
    .context("failed to escalate the residency policy operation")?;
    let status = child
        .wait()
        .context("failed to wait for the privileged residency operation")?;
    if !status.success() {
        bail!("privileged residency operation exited {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::give_back::{GiveBackEntry, GiveBackOutcome};
    use super::*;
    use qol_host_fixes::policy::cli::parse_args;
    use std::sync::{Mutex, OnceLock};

    fn serialized() -> std::sync::MutexGuard<'static, ()> {
        static GUARD: OnceLock<Mutex<()>> = OnceLock::new();
        GUARD
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn mutation_args() -> Vec<String> {
        vec![
            "disable".to_string(),
            "--policy".to_string(),
            "nvidia-driver-version-pin".to_string(),
        ]
    }

    #[test]
    fn escalation_success_emits_exactly_one_request_and_one_result() {
        let _serial = serialized();
        let mut recorder = CountedPhaseRecorder::default();
        let args = mutation_args();
        let code = run_cli_with(&args, |_| Ok(()), &mut recorder);
        assert_eq!(code, 0);
        assert_eq!(
            recorder.requests, 1,
            "a successful escalation must emit exactly one request"
        );
        assert_eq!(
            recorder.results, 1,
            "a successful escalation must emit exactly one result"
        );
    }

    #[test]
    fn escalation_failure_emits_exactly_one_request_and_one_result() {
        let _serial = serialized();
        let mut recorder = CountedPhaseRecorder::default();
        let args = mutation_args();
        let code = run_cli_with(
            &args,
            |_| Err(anyhow::anyhow!("pkexec refused")),
            &mut recorder,
        );
        assert_eq!(code, 1);
        assert_eq!(recorder.requests, 1);
        assert_eq!(recorder.results, 1);
    }

    #[test]
    fn malformed_input_emits_exactly_one_request_and_one_result() {
        let _serial = serialized();
        let mut recorder = CountedPhaseRecorder::default();
        let args = vec!["disable".to_string(), "--bogus".to_string()];
        let code = run_cli_with(
            &args,
            |_| unreachable!("must not reach escalation"),
            &mut recorder,
        );
        assert_eq!(code, 2);
        assert_eq!(recorder.requests, 1);
        assert_eq!(recorder.results, 1);
    }

    #[test]
    fn escalation_uses_only_the_parsed_command() {
        let args = vec![
            "disable".to_string(),
            "--owner".to_string(),
            "qol-resident-abc".to_string(),
        ];
        let command = parse_args(&args).unwrap();
        assert_eq!(command.operation(), "disable");
        let owner = command.owner().unwrap();
        assert_eq!(owner.as_str(), "qol-resident-abc");
    }

    #[test]
    fn only_known_operations_can_reach_elevation() {
        for values in [
            vec!["bogus"],
            vec!["status", "--owner", "owner-a"],
            vec!["enable", "--owner", "owner-a"],
            vec!["join"],
            vec!["transfer"],
        ] {
            let args = values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            assert!(
                parse_args(&args).is_err(),
                "malformed command must fail before elevation: {values:?}"
            );
        }
    }

    #[test]
    fn join_requires_an_explicit_owner() {
        let args = vec!["join".to_string()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn hidden_routing_validates_before_dispatch() {
        let args = vec![
            "__resident-policy-disable".to_string(),
            "--policy".to_string(),
            NVIDIA_POLICY_ID.to_string(),
        ];
        let command = parse_args(&args).unwrap();
        assert!(command.hidden);
        assert_eq!(command.command, ResidentCommand::Disable { owner: None });
        let args = vec![
            "__resident-policy-disable".to_string(),
            "--policy".to_string(),
            "other-policy".to_string(),
        ];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn malformed_hidden_commands_are_rejected_before_the_root_check() {
        let _serial = serialized();
        let malformed: Vec<Vec<&str>> = vec![
            vec!["__resident-policy-disable", "--bogus"],
            vec!["__resident-policy-disable", "--policy"],
            vec![
                "__resident-policy-disable",
                "--policy",
                "nvidia-driver-version-pin",
                "--policy",
                "nvidia-driver-version-pin",
            ],
            vec!["__resident-policy-enable", "--owner", "owner-a"],
            vec![
                "__resident-policy-disable",
                "--policy",
                "nvidia-driver-version-pin",
                "trailing",
            ],
            vec![
                "__resident-policy-bogus",
                "--policy",
                "nvidia-driver-version-pin",
            ],
            vec!["__resident-policy-disable", "--policy", "other-policy"],
            vec!["__resident-policy-disable", "--owner", "owner-a"],
            vec!["__resident-policy-disable"],
            vec!["__resident-policy-join"],
            vec![
                "__resident-policy-disable",
                "--policy",
                "nvidia-driver-version-pin",
                "--owner",
                "bad owner!",
            ],
        ];
        for values in malformed {
            let mut recorder = CountedPhaseRecorder::default();
            let args = values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>();
            let code = run_hidden_with(&args, &mut recorder);
            assert_eq!(
                code, 2,
                "malformed hidden command must fail before any privilege check: {values:?}"
            );
            assert_eq!(recorder.requests, 1, "{values:?}");
            assert_eq!(recorder.results, 1, "{values:?}");
        }
        let valid = vec![
            "__resident-policy-disable".to_string(),
            "--policy".to_string(),
            NVIDIA_POLICY_ID.to_string(),
        ];
        let mut recorder = CountedPhaseRecorder::default();
        assert_eq!(
            run_hidden_with(&valid, &mut recorder),
            2,
            "a valid hidden command as an unprivileged process must hit the root check"
        );
        assert_eq!(recorder.requests, 1);
        assert_eq!(recorder.results, 1);
    }

    #[test]
    fn parsed_owners_are_validated_identities() {
        let args = vec![
            "join".to_string(),
            "--owner".to_string(),
            "bad owner!".to_string(),
        ];
        assert!(parse_args(&args).is_err());
        let args = vec![
            "join".to_string(),
            "--owner".to_string(),
            "qol-resident-abc".to_string(),
        ];
        let command = parse_args(&args).unwrap();
        assert!(matches!(command.command, ResidentCommand::Join { .. }));
    }

    #[test]
    fn leaving_residency_restores_the_policies_it_owned() {
        let mut restored = false;
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Resident,
            |_| Ok(()),
            || {
                restored = true;
                RestoreReport::from_entries(Vec::new())
            },
            || GiveBackReport::from_entries(Vec::new()),
        );
        assert!(outcome.is_ok());
        assert!(restored, "disabling residency must restore the host");
    }

    #[test]
    fn staying_portable_restores_nothing() {
        let mut restored = false;
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Portable,
            |_| Ok(()),
            || {
                restored = true;
                RestoreReport::from_entries(Vec::new())
            },
            || GiveBackReport::from_entries(Vec::new()),
        );
        assert!(outcome.is_ok());
        assert!(!restored, "a portable host has nothing to restore");
    }

    #[test]
    fn a_failed_restore_is_reported_to_the_caller() {
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Resident,
            |_| Ok(()),
            || {
                RestoreReport::from_entries(vec![RestoreEntry::Failed {
                    policy: "nvidia",
                    error: "permission denied".to_string(),
                }])
            },
            || GiveBackReport::from_entries(Vec::new()),
        );
        let error = outcome.expect_err("a failed restore must not report success");
        assert!(format!("{error:#}").contains("nvidia: permission denied"));
    }

    #[test]
    fn leaving_residency_asks_the_plugins_to_give_back() {
        let mut asked = false;
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Resident,
            |_| Ok(()),
            || RestoreReport::from_entries(Vec::new()),
            || {
                asked = true;
                GiveBackReport::from_entries(Vec::new())
            },
        );
        assert!(outcome.is_ok());
        assert!(
            asked,
            "disabling residency must ask the plugins to give back"
        );
    }

    #[test]
    fn staying_portable_asks_nothing_to_give_back() {
        let mut asked = false;
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Portable,
            |_| Ok(()),
            || RestoreReport::from_entries(Vec::new()),
            || {
                asked = true;
                GiveBackReport::from_entries(Vec::new())
            },
        );
        assert!(outcome.is_ok());
        assert!(!asked, "a portable host owns no plugin state to hand back");
    }

    #[test]
    fn becoming_resident_asks_nothing_to_give_back() {
        let mut asked = false;
        let outcome = apply_residency_with(
            HostResidency::Resident,
            HostResidency::Portable,
            |_| Ok(()),
            || RestoreReport::from_entries(Vec::new()),
            || {
                asked = true;
                GiveBackReport::from_entries(Vec::new())
            },
        );
        assert!(outcome.is_ok());
        assert!(!asked, "taking residency must not hand plugin state back");
    }

    #[test]
    fn give_back_runs_before_the_host_is_restored() {
        let order = std::cell::RefCell::new(Vec::new());
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Resident,
            |_| Ok(()),
            || {
                order.borrow_mut().push("restore");
                RestoreReport::from_entries(Vec::new())
            },
            || {
                order.borrow_mut().push("give_back");
                GiveBackReport::from_entries(Vec::new())
            },
        );
        assert!(outcome.is_ok());
        assert_eq!(
            order.into_inner(),
            vec!["give_back", "restore"],
            "plugins give back before restore reverts the host policy"
        );
    }

    #[test]
    fn a_refused_give_back_does_not_block_the_residency_change() {
        let mut restored = false;
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Resident,
            |_| Ok(()),
            || {
                restored = true;
                RestoreReport::from_entries(Vec::new())
            },
            || {
                GiveBackReport::from_entries(vec![GiveBackEntry {
                    plugin_id: "plugin-sound".to_string(),
                    outcome: GiveBackOutcome::Refused {
                        error: "the output is busy".to_string(),
                    },
                }])
            },
        );
        assert!(
            outcome.is_ok(),
            "a plugin refusal must not abort the residency change"
        );
        assert!(
            restored,
            "the host restore still runs after a refused give back"
        );
    }

    #[test]
    fn a_refused_give_back_is_not_folded_into_the_restore_failure() {
        let outcome = apply_residency_with(
            HostResidency::Portable,
            HostResidency::Resident,
            |_| Ok(()),
            || {
                RestoreReport::from_entries(vec![RestoreEntry::Failed {
                    policy: "nvidia",
                    error: "permission denied".to_string(),
                }])
            },
            || {
                GiveBackReport::from_entries(vec![GiveBackEntry {
                    plugin_id: "plugin-sound".to_string(),
                    outcome: GiveBackOutcome::Refused {
                        error: "the output is busy".to_string(),
                    },
                }])
            },
        );
        let error = outcome.expect_err("a failed restore must not report success");
        let text = format!("{error:#}");
        assert!(text.contains("nvidia: permission denied"));
        assert!(
            !text.contains("the output is busy"),
            "a plugin refusal is logged, never a host policy failure"
        );
    }

    #[test]
    fn residency_runs_without_elevation() {
        let _serial = serialized();
        std::env::set_var("QOL_RESIDENCY_DEVICE_ID", "test-device");
        let root = tempfile::tempdir().unwrap();
        std::env::set_var("QOL_RESIDENCY_CONFIG_DIR", root.path());
        let mut recorder = CountedPhaseRecorder::default();
        let resident = vec!["residency".to_string(), "--resident".to_string()];
        let code = run_cli_with(
            &resident,
            |_| panic!("residency must not reach elevation"),
            &mut recorder,
        );
        assert_eq!(code, 0, "the residency toggle succeeds without elevation");
        let portable = vec!["residency".to_string(), "--portable".to_string()];
        let code = run_cli_with(
            &portable,
            |_| panic!("residency must not reach elevation"),
            &mut recorder,
        );
        assert_eq!(code, 0);
        std::env::remove_var("QOL_RESIDENCY_DEVICE_ID");
        std::env::remove_var("QOL_RESIDENCY_CONFIG_DIR");
    }
}
