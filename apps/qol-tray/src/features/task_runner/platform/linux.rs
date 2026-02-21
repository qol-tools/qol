use tokio::process::Command;

pub(super) fn shell_command(script: &str) -> Command {
    super::unix_common::shell_command(script)
}
