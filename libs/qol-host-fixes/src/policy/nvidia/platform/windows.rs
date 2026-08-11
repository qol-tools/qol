use super::{NvidiaPolicyBackend, PolicyStatusView};
use crate::policy::cli::{self, ResidentCommand};
use crate::policy::nvidia::{print_help, ActiveFileFingerprint, NVIDIA_POLICY_ID};
use crate::policy::{PolicyError, ResidencyOwnerId, ResidentPolicy};
use anyhow::Result;

pub(crate) struct WindowsNvidia;

impl NvidiaPolicyBackend for WindowsNvidia {
    fn status(_policy: &ResidentPolicy) -> Result<PolicyStatusView> {
        Err(PolicyError::PlatformUnsupported {
            policy: NVIDIA_POLICY_ID.to_string(),
        }
        .into())
    }

    fn enable(_policy: &ResidentPolicy, _owner: &ResidencyOwnerId) -> Result<()> {
        Err(PolicyError::PlatformUnsupported {
            policy: NVIDIA_POLICY_ID.to_string(),
        }
        .into())
    }

    fn disable(_policy: &ResidentPolicy, _owner: &ResidencyOwnerId) -> Result<()> {
        Err(PolicyError::PlatformUnsupported {
            policy: NVIDIA_POLICY_ID.to_string(),
        }
        .into())
    }

    fn join(_policy: &ResidentPolicy, _owner: &ResidencyOwnerId) -> Result<()> {
        Err(PolicyError::PlatformUnsupported {
            policy: NVIDIA_POLICY_ID.to_string(),
        }
        .into())
    }

    fn transfer(_policy: &ResidentPolicy, _new_owner: &ResidencyOwnerId) -> Result<()> {
        Err(PolicyError::PlatformUnsupported {
            policy: NVIDIA_POLICY_ID.to_string(),
        }
        .into())
    }

    fn run_resident_policy_cli(args: &[String]) -> Result<i32> {
        let command = cli::parse_args(args)?.command;
        match command {
            ResidentCommand::Help => {
                print_help();
                Ok(0)
            }
            ResidentCommand::Status
            | ResidentCommand::Enable
            | ResidentCommand::Disable { .. }
            | ResidentCommand::Join { .. }
            | ResidentCommand::Transfer { .. } => Err(PolicyError::PlatformUnsupported {
                policy: NVIDIA_POLICY_ID.to_string(),
            }
            .into()),
        }
    }

    fn crash_point(_point: &str) -> Result<()> {
        Ok(())
    }

    fn validate_fingerprint_owner(_fingerprint: &ActiveFileFingerprint) -> Result<()> {
        Ok(())
    }
}
