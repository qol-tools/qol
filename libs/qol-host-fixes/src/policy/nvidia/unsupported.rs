use super::{print_help, PolicyStatusView};
use crate::policy::cli::{self, ResidentCommand};
use crate::policy::{PolicyError, ResidentPolicy};
use anyhow::Result;

pub fn status(policy: &ResidentPolicy) -> Result<PolicyStatusView> {
    Err(PolicyError::PlatformUnsupported {
        policy: policy.id().to_string(),
    }
    .into())
}

pub fn enable(_policy: &ResidentPolicy, _owner: &crate::policy::ResidencyOwnerId) -> Result<()> {
    Err(PolicyError::PlatformUnsupported {
        policy: _policy.id().to_string(),
    }
    .into())
}

pub fn disable(_policy: &ResidentPolicy, _owner: &crate::policy::ResidencyOwnerId) -> Result<()> {
    Err(PolicyError::PlatformUnsupported {
        policy: _policy.id().to_string(),
    }
    .into())
}

pub fn join(_policy: &ResidentPolicy, _owner: &crate::policy::ResidencyOwnerId) -> Result<()> {
    Err(PolicyError::PlatformUnsupported {
        policy: _policy.id().to_string(),
    }
    .into())
}

pub fn transfer(
    _policy: &ResidentPolicy,
    _new_owner: &crate::policy::ResidencyOwnerId,
) -> Result<()> {
    Err(PolicyError::PlatformUnsupported {
        policy: _policy.id().to_string(),
    }
    .into())
}

pub fn run_resident_policy_cli(args: &[String]) -> Result<i32> {
    let command = cli::parse_args(args)?.command;
    match command {
        ResidentCommand::Help => {
            print_help();
            Ok(0)
        }
        ResidentCommand::Status => Err(PolicyError::PlatformUnsupported {
            policy: ResidentPolicy::nvidia().id().to_string(),
        }
        .into()),
        ResidentCommand::Enable
        | ResidentCommand::Disable { .. }
        | ResidentCommand::Join { .. }
        | ResidentCommand::Transfer { .. } => Err(PolicyError::PlatformUnsupported {
            policy: ResidentPolicy::nvidia().id().to_string(),
        }
        .into()),
    }
}

pub fn run_resident_policy_cli_traced(args: &[String]) -> Result<i32> {
    run_resident_policy_cli_traced_with(args, &mut crate::policy::trace::NoopEmissionRecorder)
}

pub(crate) fn run_resident_policy_cli_traced_with<R>(
    args: &[String],
    recorder: &mut R,
) -> Result<i32>
where
    R: crate::policy::trace::EmissionRecorder,
{
    let carrier = crate::policy::trace::cli_request(args);
    recorder.on_request();
    let result = run_resident_policy_cli(args);
    let outcome = crate::policy::trace::outcome_of(&result);
    let reason = crate::policy::trace::error_reason(&result);
    crate::policy::trace::cli_result(args, &carrier, outcome, &reason);
    recorder.on_result();
    result
}

pub fn crash_point(_point: &str) -> Result<()> {
    Ok(())
}
