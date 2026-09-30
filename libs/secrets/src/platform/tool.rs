use std::process::Command;

use anyhow::{bail, Context, Result};

pub(super) fn secret_from(mut command: Command, service: &str, store: &str) -> Result<String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .with_context(|| format!("failed to run `{program}` to read `{service}` from {store}"))?;
    if !output.status.success() {
        bail!(
            "{store} has no readable secret `{service}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("the secret `{service}` in {store} is not UTF-8"))
}
