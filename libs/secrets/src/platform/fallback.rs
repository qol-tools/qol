use anyhow::{bail, Result};

pub(crate) fn read(service: &str) -> Result<String> {
    bail!("reading the secret `{service}` is not supported on this platform")
}

pub(crate) fn store(service: &str, _secret: &str) -> Result<()> {
    bail!("storing the secret `{service}` is not supported on this platform")
}
