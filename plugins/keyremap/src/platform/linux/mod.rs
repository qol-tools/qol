use anyhow::Result;
use qol_headless::CommandResult;

use super::{ConfigInspection, PlatformAdapter, TrustStatus};

#[derive(Clone, Copy)]
pub(crate) struct Adapter;

impl PlatformAdapter for Adapter {
    fn name(&self) -> &'static str {
        "Linux"
    }

    fn supported(&self) -> bool {
        false
    }

    fn launch(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn reload(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn kill(&self) -> Result<CommandResult> {
        Ok(unsupported())
    }

    fn inspect_config(&self) -> Result<ConfigInspection> {
        anyhow::bail!("typed key-remap configuration is only available on macOS")
    }

    fn trust_status(&self) -> TrustStatus {
        TrustStatus::from_trusted(false)
    }
}

fn unsupported() -> CommandResult {
    CommandResult::runtime_error(
        "keyremap: only macOS is supported (requires CGEventTap and Accessibility APIs)",
    )
}
