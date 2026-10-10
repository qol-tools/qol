use anyhow::{Context, Result};
use qol_platform::native::registry::{self, Hive, View};

const CRYPTOGRAPHY_KEY: &str = r"SOFTWARE\Microsoft\Cryptography";
const MACHINE_GUID_VALUE: &str = "MachineGuid";

pub(crate) fn device_id() -> Result<String> {
    let guid = machine_guid()?;
    let owner = crate::policy::host_owner_from_machine_id("residency", &guid)?;
    Ok(owner.as_str().to_string())
}

fn machine_guid() -> Result<String> {
    registry::read_string(
        Hive::LocalMachine,
        CRYPTOGRAPHY_KEY,
        MACHINE_GUID_VALUE,
        View::Native,
    )
    .with_context(|| format!("failed to read HKLM\\{CRYPTOGRAPHY_KEY}\\{MACHINE_GUID_VALUE}"))?
    .filter(|guid| !guid.is_empty())
    .with_context(|| format!("HKLM\\{CRYPTOGRAPHY_KEY}\\{MACHINE_GUID_VALUE} is missing"))
}
