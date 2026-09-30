use anyhow::{bail, Result};

pub(crate) fn read(service: &str) -> Result<String> {
    bail!("reading the secret `{service}` is not supported on this platform")
}
