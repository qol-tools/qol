use std::process::Command;

pub(crate) fn shell(script: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }
}

pub(crate) fn exiting() -> Command {
    if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "exit 0"]);
        command
    } else {
        Command::new("true")
    }
}

pub(crate) fn sleeping(seconds: u32) -> Command {
    if cfg!(windows) {
        shell(&format!("Start-Sleep -Seconds {seconds}"))
    } else {
        let mut command = Command::new("sleep");
        command.arg(seconds.to_string());
        command
    }
}

pub(crate) fn absolute(path: &'static str) -> &'static str {
    if cfg!(windows) {
        Box::leak(format!("C:{path}").into_boxed_str())
    } else {
        path
    }
}
