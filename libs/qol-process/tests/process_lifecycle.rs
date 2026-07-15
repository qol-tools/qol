use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
use std::time::Instant;

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
fn terminate_owned_accepts_an_already_reaped_child() {
    let mut child = exit_command(0).spawn().unwrap();
    assert!(child.wait().unwrap().success());

    qol_process::terminate_owned(&mut child, Duration::ZERO).unwrap();
}

#[test]
fn termination_ignores_an_invalid_pid() {
    qol_process::terminate_pid(0, Duration::ZERO);
    qol_process::terminate_group(0, Duration::ZERO);
}

#[test]
fn process_tree_requires_an_assigned_child() {
    let guard = qol_process::own_current_process_tree().unwrap();

    assert_eq!(
        guard
            .terminate_and_wait(Duration::from_millis(20))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotConnected
    );
}

#[cfg(unix)]
fn wait_for_path(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists());
}

#[cfg(unix)]
fn isolated_command(script: &str) -> Command {
    use std::os::unix::process::CommandExt;

    let mut command = Command::new("sh");
    command.args(["-c", script]).process_group(0);
    command
}

#[cfg(unix)]
#[test]
fn process_tree_accepts_a_child_in_the_callers_group_without_claiming_it() {
    let mut child = long_running_command().spawn().unwrap();
    let guard = qol_process::own_current_process_tree().unwrap();

    guard.assign(&child).unwrap();
    assert_eq!(
        guard
            .terminate_and_wait(Duration::from_millis(20))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::TimedOut
    );
    assert!(qol_process::is_pid_alive(child.id()));
    assert!(qol_process::is_pid_alive(std::process::id()));
    qol_process::terminate_owned(&mut child, Duration::from_millis(20)).unwrap();
    let _proof = guard.terminate_and_wait(Duration::from_millis(20)).unwrap();
}

#[cfg(unix)]
#[test]
fn current_process_tree_guard_disarms_idempotently() {
    let mut guard = qol_process::guard_current_process_tree().unwrap();

    guard.disarm().unwrap();
    guard.disarm().unwrap();
}

#[cfg(unix)]
#[test]
fn process_tree_assignment_is_single_owner() {
    let mut first = isolated_command("exec sleep 30").spawn().unwrap();
    let mut second = isolated_command("exec sleep 30").spawn().unwrap();
    let guard = qol_process::own_current_process_tree().unwrap();

    guard.assign(&first).unwrap();
    assert_eq!(
        guard.assign(&second).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );

    qol_process::terminate_owned(&mut first, Duration::from_millis(20)).unwrap();
    qol_process::terminate_owned(&mut second, Duration::from_millis(20)).unwrap();
}

#[cfg(unix)]
#[test]
fn process_tree_terminates_descendants_after_the_leader_exits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let script = "printf ready > \"$QOL_PROCESS_TREE_ROOT/ready\"; while [ ! -f \"$QOL_PROCESS_TREE_ROOT/release\" ]; do sleep 0.01; done; ( trap 'kill -TERM \"$worker\" 2>/dev/null; wait \"$worker\"; exit 0' TERM; sleep 30 & worker=$!; printf '%s' \"$worker\" > \"$QOL_PROCESS_TREE_ROOT/descendant\"; wait \"$worker\" ) & while [ ! -s \"$QOL_PROCESS_TREE_ROOT/descendant\" ]; do sleep 0.01; done";
    let mut command = isolated_command(script);
    command
        .env("QOL_PROCESS_TREE_ROOT", root)
        .stderr(std::process::Stdio::null());
    let mut leader = command.spawn().unwrap();
    let leader_pid = leader.id();
    let guard = qol_process::own_current_process_tree().unwrap();

    wait_for_path(&root.join("ready"));
    guard.assign(&leader).unwrap();
    std::fs::write(root.join("release"), "release").unwrap();
    assert!(leader.wait().unwrap().success());
    wait_for_path(&root.join("descendant"));
    let descendant_pid = std::fs::read_to_string(root.join("descendant"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert!(qol_process::is_pid_alive(descendant_pid));
    assert!(qol_process::is_group_alive(leader_pid));

    let _proof = guard.terminate_and_wait(Duration::from_secs(2)).unwrap();

    assert!(!qol_process::is_pid_alive(descendant_pid));
    assert!(!qol_process::is_group_alive(leader_pid));
}

#[cfg(unix)]
#[test]
fn process_tree_escalates_and_verifies_for_a_term_resistant_group() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let mut command =
        isolated_command("trap '' TERM; printf ready > \"$QOL_PROCESS_TREE_READY\"; exec sleep 30");
    command.env("QOL_PROCESS_TREE_READY", &ready);
    let child = command.spawn().unwrap();
    let pid = child.id();
    let guard = qol_process::own_current_process_tree().unwrap();

    wait_for_path(&ready);
    guard.assign(&child).unwrap();
    let waiter = std::thread::spawn(move || {
        let mut child = child;
        child.wait().unwrap()
    });

    let _proof = guard.terminate_and_wait(Duration::from_secs(1)).unwrap();
    let status = waiter.join().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_group_alive(pid));
}
