use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};

pub(super) struct CargoChild {
    pub(super) child: Child,
    pub(super) stdout: ChildStdout,
    pub(super) stderr: ChildStderr,
}

pub(super) fn spawn_piped(command: &mut Command) -> Result<CargoChild, std::io::Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    Ok(CargoChild {
        child,
        stdout,
        stderr,
    })
}
