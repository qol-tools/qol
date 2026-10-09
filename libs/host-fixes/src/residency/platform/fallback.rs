use anyhow::{bail, Result};

pub(crate) fn device_id() -> Result<String> {
    bail!("no device residency identity derived on this platform")
}
