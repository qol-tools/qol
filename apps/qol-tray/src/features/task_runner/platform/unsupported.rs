use tokio::process::Command;

pub(super) fn shell_command(_script: &str) -> Command {
    panic!("task runner platform command adapter is required for this target OS")
}
