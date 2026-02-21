use tokio::process::Command;

pub(super) fn shell_command(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}
