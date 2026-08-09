use super::nvidia::NVIDIA_POLICY_ID;
use super::{ResidencyOwnerId, ResidentPolicy};
use anyhow::{anyhow, bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidentCommand {
    Status,
    Help,
    Enable,
    Disable { owner: Option<ResidencyOwnerId> },
    Join { owner: ResidencyOwnerId },
    Transfer { owner: ResidencyOwnerId },
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
        }
    }

    pub fn owner(&self) -> Option<&ResidencyOwnerId> {
        match self {
            Self::Disable { owner } => owner.as_ref(),
            Self::Join { owner } | Self::Transfer { owner } => Some(owner),
            Self::Status | Self::Help | Self::Enable => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCommand {
    pub hidden: bool,
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
    let mut policy_value: Option<String> = None;
    let mut owner_seen = false;
    let mut owner_value: Option<ResidencyOwnerId> = None;
    while let Some(token) = tokens.next() {
        match token.as_str() {
            "--policy" => {
                if policy_value.is_some() {
                    bail!("duplicate --policy flag");
                }
                let value = tokens
                    .next()
                    .ok_or_else(|| anyhow!("--policy requires a value"))?;
                ResidentPolicy::from_id(value)?;
                policy_value = Some(value.clone());
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
            other if other.starts_with('-') => {
                bail!("unknown flag `{other}`");
            }
            other => bail!("unexpected argument `{other}`"),
        }
    }
    if hidden {
        match policy_value.as_deref() {
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
            command: ResidentCommand::Help,
        });
    }
    match operation {
        "status" | "enable" => {
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
        _ => unreachable!("operation was validated above"),
    };
    Ok(ParsedCommand { hidden, command })
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
}
