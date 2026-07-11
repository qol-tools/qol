use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
fn exit_command(code: i32) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", &format!("exit {code}")]);
    command
}

#[cfg(windows)]
fn exit_command(code: i32) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", &format!("exit {code}")]);
    command
}

#[cfg(unix)]
fn long_running_command() -> Command {
    let mut command = Command::new("sleep");
    command.arg("30");
    command
}

#[cfg(windows)]
fn long_running_command() -> Command {
    let mut command = Command::new("cmd");
    command.args(["/C", "ping -n 31 127.0.0.1 >NUL"]);
    command
}

#[test]
#[allow(clippy::zombie_processes)]
fn wait_pid_preserves_the_child_exit_status() {
    let child = exit_command(7).spawn().unwrap();
    let status = qol_process::wait_pid(child.id()).unwrap();

    assert_eq!(status.code(), Some(7));
    assert!(!qol_process::is_pid_alive(child.id()));
}

#[test]
#[allow(clippy::zombie_processes)]
fn kill_pid_stops_and_reaps_a_running_child() {
    let child = long_running_command().spawn().unwrap();
    let pid = child.id();

    assert!(qol_process::is_pid_alive(pid));
    qol_process::kill_pid(pid).unwrap();
    let status = qol_process::wait_pid(pid).unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_pid_alive(pid));
}

#[test]
fn terminate_owned_stops_a_regular_child() {
    let mut child = long_running_command().spawn().unwrap();
    let pid = child.id();

    qol_process::terminate_owned(&mut child, Duration::from_millis(20)).unwrap();

    assert!(!qol_process::is_pid_alive(pid));
}

#[test]
fn termination_ignores_an_invalid_pid() {
    qol_process::terminate_pid(0, Duration::ZERO);
    qol_process::terminate_group(0, Duration::ZERO);
}
