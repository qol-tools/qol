use std::io::Write;
use std::process::{Command, Stdio};

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

pub(super) fn store_through_stdin(
    mut command: Command,
    input: &[u8],
    hidden: &[&str],
    service: &str,
    store: &str,
) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to run `{program}` to store `{service}` in {store}"))?;
    let written = child
        .stdin
        .take()
        .context("the child has no stdin pipe")
        .and_then(|mut stdin| stdin.write_all(input).map_err(Into::into));
    let output = child
        .wait_with_output()
        .with_context(|| format!("`{program}` did not finish storing `{service}` in {store}"))?;
    if !output.status.success() {
        bail!(
            "{store} did not store the secret `{service}` ({}): {}",
            output.status,
            redacted(&String::from_utf8_lossy(&output.stderr), hidden).trim()
        );
    }
    written.with_context(|| format!("failed to hand the secret `{service}` to `{program}`"))
}

fn redacted(text: &str, hidden: &[&str]) -> String {
    hidden
        .iter()
        .filter(|value| !value.is_empty())
        .fold(text.to_owned(), |text, value| {
            text.replace(value, "<secret>")
        })
}

#[cfg(test)]
mod tests {
    use super::redacted;

    #[test]
    fn every_hidden_value_leaves_the_tool_output() {
        let text = "failed: abc123 then 616263313233 and abc123 again";
        assert_eq!(
            redacted(text, &["abc123", "616263313233", ""]),
            "failed: <secret> then <secret> and <secret> again"
        );
    }
}
