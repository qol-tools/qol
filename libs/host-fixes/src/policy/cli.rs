#![allow(clippy::print_stdout, clippy::print_stderr)]

use super::nvidia::{PolicyStatusView, NVIDIA_POLICY_ID};
use super::{ResidencyOwnerId, ResidentPolicy};
use anyhow::{anyhow, bail, Result};

pub fn run_standalone(args: &[String]) -> i32 {
    match super::nvidia::run_resident_policy_cli_traced(args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("resident-policy: {error:#}");
            1
        }
    }
}

pub fn run_guardian() -> i32 {
    match qol_process::run_process_tree_guardian_entry() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("resident-policy: process-tree guardian entry failed: {error}");
            1
        }
    }
}

pub fn print_status(view: &PolicyStatusView) {
    let module = view.expected_module_version.as_deref().unwrap_or("-");
    println!(
        "policy={} state={} owners={} module={} {}",
        view.policy,
        view.state.as_str(),
        view.owners.join(","),
        module,
        view.detail
    );
}

pub fn print_help() {
    println!("qol-tray resident-policy");
    println!();
    println!("Manage a durable, host-local residency policy. Enabling pins the exact");
    println!("installed NVIDIA driver versions with APT preferences and is an explicit,");
    println!("machine-scoped mutation; disabling restores the exact owned state.");
    println!();
    println!("USAGE:");
    println!("    qol-tray resident-policy status                 Read-only state (no elevation)");
    println!("    qol-tray resident-policy help                   This message (no elevation)");
    println!("    qol-tray resident-policy enable                 Adopt the NVIDIA policy");
    println!("    qol-tray resident-policy disable [--owner <id>] Release this owner's state");
    println!("    qol-tray resident-policy join --owner <id>      Join an active policy");
    println!("    qol-tray resident-policy transfer --owner <id>  Replace the owner set");
    println!("    qol-tray resident-policy residency --resident   Mark THIS device resident");
    println!("    qol-tray resident-policy residency --portable   Mark THIS device portable");
    println!();
    println!("Mutations require elevation (pkexec) and root. Status is read-only and");
    println!("never elevates. The residency toggle writes the per-device status into the");
    println!("active profile and never elevates. Only the fixed nvidia-driver-version-pin");
    println!("policy is known.");
    println!("Activation succeeds only from a managed install; raw and portable artifacts");
    println!("cannot create resident state.");
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidentCommand {
    Status,
    Help,
    Enable,
    Disable { owner: Option<ResidencyOwnerId> },
    Join { owner: ResidencyOwnerId },
    Transfer { owner: ResidencyOwnerId },
    Residency { resident: bool },
}

impl ResidentCommand {
    pub fn operation(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Help => "help",
            Self::Enable => "enable",
            Self::Disable { .. } => "disable",
            Self::Join { .. } => "join",
            Self::Transfer { .. } => "transfer",
            Self::Residency { .. } => "residency",
        }
    }

    pub fn owner(&self) -> Option<&ResidencyOwnerId> {
        match self {
            Self::Disable { owner } => owner.as_ref(),
            Self::Join { owner } | Self::Transfer { owner } => Some(owner),
            Self::Status | Self::Help | Self::Enable | Self::Residency { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub hidden: bool,
    pub policy: ResidentPolicy,
    pub command: ResidentCommand,
}

impl ParsedCommand {
    pub fn operation(&self) -> &'static str {
        self.command.operation()
    }

    pub fn owner(&self) -> Option<&ResidencyOwnerId> {
        self.command.owner()
    }
}

pub fn parse_args(args: &[String]) -> Result<ParsedCommand> {
    let mut tokens = args.iter();
    let first = tokens.next().map(String::as_str);
    let (operation, hidden) = match first {
        Some(token) => match token.strip_prefix("__resident-policy-") {
            Some(operation) => (operation, true),
            None => (token, false),
        },
        None => ("status", false),
    };
    let mut policy: Option<ResidentPolicy> = None;
    let mut owner_seen = false;
    let mut owner_value: Option<ResidencyOwnerId> = None;
    let mut residency_seen = false;
    let mut residency_value: Option<bool> = None;
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--policy" => {
                if policy.is_some() {
                    bail!("duplicate --policy flag");
                }
                let value = tokens
                    .next()
                    .ok_or_else(|| anyhow!("--policy requires a value"))?;
                policy = Some(ResidentPolicy::from_id(value)?);
            }
            "--owner" => {
                if owner_seen {
                    bail!("duplicate --owner flag");
                }
                owner_seen = true;
                let value = tokens
                    .next()
                    .ok_or_else(|| anyhow!("--owner requires a value"))?;
                owner_value = Some(ResidencyOwnerId::parse(value)?);
            }
            "--resident" => {
                if residency_seen {
                    bail!("duplicate residency --resident flag");
                }
                residency_seen = true;
                residency_value = Some(true);
            }
            "--portable" => {
                if residency_seen {
                    bail!("duplicate residency --portable flag");
                }
                residency_seen = true;
                residency_value = Some(false);
            }
            other if other.starts_with('-') => {
                bail!("unknown flag `{other}`");
            }
            other => bail!("unexpected argument `{other}`"),
        }
    }
    if hidden {
        match policy.as_ref().map(ResidentPolicy::id) {
            Some(NVIDIA_POLICY_ID) => {}
            Some(other) => {
                bail!(
                    "the hidden command requires the fixed policy `{NVIDIA_POLICY_ID}`, got `{other}`"
                );
            }
            None => {
                bail!("the hidden command requires exactly one --policy {NVIDIA_POLICY_ID}");
            }
        }
    }
    let policy = policy.unwrap_or_else(ResidentPolicy::nvidia);
    if matches!(operation, "help" | "--help" | "-h") {
        if hidden {
            if owner_seen {
                bail!("--owner is not valid for the hidden `help` command");
            }
        } else if args.len() > 1 {
            bail!("flags are not valid for `help`");
        }
        return Ok(ParsedCommand {
            hidden,
            policy,
            command: ResidentCommand::Help,
        });
    }
    if residency_seen && operation != "residency" {
        bail!("--resident/--portable are only valid for the `residency` operation");
    }
    if hidden && operation == "residency" {
        bail!("the hidden route does not accept the `residency` operation");
    }
    match operation {
        "status" | "enable" | "residency" => {
            if owner_seen {
                bail!("--owner is not valid for `{operation}`");
            }
        }
        "disable" | "join" | "transfer" => {}
        other => bail!("unknown resident-policy operation `{other}`"),
    }
    let command = match operation {
        "status" => ResidentCommand::Status,
        "enable" => ResidentCommand::Enable,
        "disable" => ResidentCommand::Disable { owner: owner_value },
        "join" => {
            let owner = owner_value.ok_or_else(|| anyhow!("join requires an explicit --owner"))?;
            ResidentCommand::Join { owner }
        }
        "transfer" => {
            let owner =
                owner_value.ok_or_else(|| anyhow!("transfer requires an explicit --owner"))?;
            ResidentCommand::Transfer { owner }
        }
        "residency" => ResidentCommand::Residency {
            resident: residency_value
                .ok_or_else(|| anyhow!("residency requires --resident or --portable"))?,
        },
        _ => unreachable!("operation was validated above"),
    };
    Ok(ParsedCommand {
        hidden,
        policy,
        command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(values: &[&str]) -> Result<ParsedCommand> {
        parse_args(
            &values
                .iter()
                .map(|value| value.to_string())
                .collect::<Vec<_>>(),
        )
    }

    fn hidden(values: &[&str]) -> Result<ParsedCommand> {
        parse(&[&["__resident-policy-disable"], values].concat())
    }

    #[test]
    fn the_requested_policy_survives_parsing() {
        let parsed = parse(&["status", "--policy", crate::udev::UDEV_UACCESS_POLICY_ID]).unwrap();
        assert_eq!(parsed.policy, ResidentPolicy::UdevUaccess);
        let parsed = parse(&["status", "--policy", NVIDIA_POLICY_ID]).unwrap();
        assert_eq!(parsed.policy, ResidentPolicy::NvidiaDriverVersionPin);
    }

    #[test]
    fn an_absent_policy_flag_still_means_the_driver_pin() {
        assert_eq!(
            parse(&["status"]).unwrap().policy,
            ResidentPolicy::NvidiaDriverVersionPin
        );
    }

    #[test]
    fn public_status_needs_no_flags() {
        let parsed = parse(&[]).unwrap();
        assert!(!parsed.hidden);
        assert_eq!(parsed.command, ResidentCommand::Status);
        let parsed = parse(&["status"]).unwrap();
        assert_eq!(parsed.command, ResidentCommand::Status);
    }

    #[test]
    fn public_commands_keep_ergonomic_policy_flags() {
        let parsed = parse(&["enable", "--policy", NVIDIA_POLICY_ID]).unwrap();
        assert!(!parsed.hidden);
        assert_eq!(parsed.command, ResidentCommand::Enable);
        let parsed = parse(&["enable"]).unwrap();
        assert_eq!(parsed.command, ResidentCommand::Enable);
        let parsed = parse(&["disable", "--owner", "owner-a"]).unwrap();
        assert!(matches!(
            parsed.command,
            ResidentCommand::Disable { owner: Some(_) }
        ));
    }

    #[test]
    fn hidden_commands_require_exactly_the_fixed_policy() {
        let parsed = hidden(&["--policy", NVIDIA_POLICY_ID]).unwrap();
        assert!(parsed.hidden);
        assert_eq!(parsed.command, ResidentCommand::Disable { owner: None });

        for values in [
            &["--policy", "other-policy"][..],
            &["--policy"][..],
            &["--policy", NVIDIA_POLICY_ID, "--policy", NVIDIA_POLICY_ID][..],
        ] {
            assert!(hidden(values).is_err(), "{}", values.join(" "));
        }
        let missing = hidden(&["--policy"]).unwrap_err();
        assert!(format!("{missing:#}").contains("--policy"), "{missing:#}");
        let duplicate =
            hidden(&["--policy", NVIDIA_POLICY_ID, "--policy", NVIDIA_POLICY_ID]).unwrap_err();
        assert!(
            format!("{duplicate:#}").contains("--policy"),
            "{duplicate:#}"
        );
    }

    #[test]
    fn a_hidden_command_without_policy_is_rejected_before_any_dispatch() {
        for values in [
            &["--owner", "owner-a"][..],
            &[][..],
            &["trailing"][..],
            &["--bogus"][..],
        ] {
            assert!(
                hidden(values).is_err(),
                "{} must be rejected",
                values.join(" ")
            );
        }
    }

    #[test]
    fn hidden_commands_still_validate_owners_and_operations() {
        assert!(hidden(&["--policy", NVIDIA_POLICY_ID, "--owner", "bad owner!"]).is_err());
        assert!(hidden(&["--policy", NVIDIA_POLICY_ID, "--owner", "owner-a"]).is_ok());
        assert!(parse(&["__resident-policy-join", "--policy", NVIDIA_POLICY_ID]).is_err());
        assert!(parse(&["__resident-policy-bogus", "--policy", NVIDIA_POLICY_ID]).is_err());
    }

    #[test]
    fn owner_flags_remain_operation_bound() {
        assert!(parse(&["status", "--owner", "owner-a"]).is_err());
        assert!(parse(&["enable", "--owner", "owner-a"]).is_err());
        assert!(parse(&["join"]).is_err());
        assert!(parse(&["transfer"]).is_err());
        assert!(parse(&["join", "--owner", "owner-a"]).is_ok());
        assert!(parse(&["transfer", "--owner", "owner-a"]).is_ok());
    }

    #[test]
    fn residency_parses_from_explicit_flags() {
        assert_eq!(
            parse(&["residency", "--resident"]).unwrap().command,
            ResidentCommand::Residency { resident: true }
        );
        assert_eq!(
            parse(&["residency", "--portable"]).unwrap().command,
            ResidentCommand::Residency { resident: false }
        );
    }

    #[test]
    fn residency_requires_exactly_one_mode_flag() {
        let missing = parse(&["residency"]).unwrap_err();
        assert!(
            format!("{missing:#}").contains("--resident or --portable"),
            "{missing:#}"
        );
        let duplicate = parse(&["residency", "--resident", "--portable"]).unwrap_err();
        assert!(
            format!("{duplicate:#}").contains("duplicate"),
            "{duplicate:#}"
        );
        assert!(parse(&["status", "--resident"]).is_err());
        assert!(parse(&["residency", "--owner", "owner-a"]).is_err());
    }

    #[test]
    fn residency_is_not_a_hidden_elevated_route() {
        assert!(parse(&["__resident-policy-residency", "--resident"]).is_err());
        assert!(parse(&["__resident-policy-residency", "--policy", NVIDIA_POLICY_ID]).is_err());
    }
}
