use std::process::Command;

use anyhow::Result;

const STORE: &str = "the Secret Service (secret-tool, from libsecret-tools)";

pub(crate) fn read(service: &str) -> Result<String> {
    let mut command = Command::new("secret-tool");
    command.args(["lookup", "service", service]);
    super::tool::secret_from(command, service, STORE)
}

pub(crate) fn store(service: &str, secret: &str) -> Result<()> {
    super::tool::store_through_stdin(
        store_command(service),
        secret.as_bytes(),
        &[secret],
        service,
        STORE,
    )
}

fn store_command(service: &str) -> Command {
    let mut command = Command::new("secret-tool");
    command
        .arg("store")
        .arg(format!("--label={service}"))
        .args(["service", service]);
    command
}

#[cfg(test)]
mod tests {
    use super::store_command;

    #[test]
    fn the_entry_carries_the_attributes_and_label_of_secret_tool_store_by_hand() {
        let command = store_command("claude-oauth-work");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(command.get_program(), "secret-tool");
        assert_eq!(
            args,
            [
                "store",
                "--label=claude-oauth-work",
                "service",
                "claude-oauth-work"
            ]
        );
    }
}
