use std::process::Command;
use std::time::Duration;

#[cfg(any(target_os = "linux", windows))]
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

#[cfg(any(target_os = "linux", windows))]
#[test]
fn process_tree_requires_an_assigned_child() {
    let guard = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();

    assert_eq!(
        guard
            .terminate_and_wait(Duration::from_millis(20))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotConnected
    );
}

#[cfg(target_os = "macos")]
#[test]
fn process_tree_containment_reports_unsupported_before_startup() {
    assert_eq!(
        qol_process::process_tree_containment_support()
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::Unsupported
    );
    assert_eq!(
        qol_process::own_current_process_tree_with_guardian(Command::new(
            std::env::current_exe().unwrap(),
        ))
        .err()
        .unwrap()
        .kind(),
        std::io::ErrorKind::Unsupported
    );
}

#[cfg(any(target_os = "linux", windows))]
fn wait_for_path(path: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists());
}

#[cfg(any(target_os = "linux", windows))]
fn read_pid(path: &std::path::Path) -> u32 {
    std::fs::read_to_string(path)
        .unwrap()
        .parse::<u32>()
        .unwrap()
}

#[cfg(any(target_os = "linux", windows))]
fn wait_for_pids_to_exit(pids: &[u32], timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if pids.iter().all(|pid| !qol_process::is_pid_alive(*pid)) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(any(target_os = "linux", windows))]
fn guardian_command() -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "process_tree_guardian_helper", "--nocapture"]);
    command
}

#[cfg(target_os = "linux")]
fn owned_process_tree() -> qol_process::ProcessTreeGuard {
    qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn process_tree_guardian_helper() {
    if std::env::var_os("QOL_PROCESS_GUARDIAN_PROTOCOL").is_none() {
        return;
    }
    qol_process::run_process_tree_guardian_entry().unwrap();
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn ordinary_descendant_helper() {
    let Some(marker) = std::env::var_os("QOL_PROCESS_ORDINARY_DESCENDANT") else {
        return;
    };
    let mut descendant = long_running_command().spawn().unwrap();
    std::fs::write(marker, descendant.id().to_string()).unwrap();
    let _ = descendant.wait();
}

#[cfg(windows)]
#[test]
#[allow(clippy::zombie_processes)]
fn abrupt_windows_job_owner_helper() {
    let Some(root) = std::env::var_os("QOL_PROCESS_WINDOWS_JOB_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let marker = root.join("descendant");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "ordinary_descendant_helper", "--nocapture"])
        .env("QOL_PROCESS_ORDINARY_DESCENDANT", &marker);
    let guard = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let prepared = guard.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    std::fs::write(root.join("root"), child.id().to_string()).unwrap();
    wait_for_path(&marker);
    std::fs::write(root.join("ready"), "ready").unwrap();
    std::process::exit(0);
}

#[cfg(windows)]
#[test]
fn abrupt_owner_exit_closes_the_job_and_kills_its_entire_tree() {
    let temp = tempfile::tempdir().unwrap();
    let mut owner = Command::new(std::env::current_exe().unwrap());
    owner
        .args(["--exact", "abrupt_windows_job_owner_helper", "--nocapture"])
        .env("QOL_PROCESS_WINDOWS_JOB_ROOT", temp.path());
    let mut owner = owner.spawn().unwrap();
    wait_for_path(&temp.path().join("ready"));
    let root = read_pid(&temp.path().join("root"));
    let descendant = read_pid(&temp.path().join("descendant"));

    assert!(owner.wait().unwrap().success());
    assert!(wait_for_pids_to_exit(
        &[root, descendant],
        Duration::from_secs(3)
    ));
}

#[cfg(any(target_os = "linux", windows))]
fn spawn_prepared_descendant_tree() -> (qol_process::ProcessTreeGuard, std::process::Child, u32, u32)
{
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("descendant");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "ordinary_descendant_helper", "--nocapture"])
        .env("QOL_PROCESS_ORDINARY_DESCENDANT", &marker);
    let guard = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let prepared = guard.prepare_command(command).unwrap();
    let root = prepared.spawn().unwrap();
    let root_pid = root.id();
    wait_for_path(&marker);
    let descendant_pid = std::fs::read_to_string(&marker)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    (guard, root, root_pid, descendant_pid)
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn prepared_tree_terminates_its_root_and_ordinary_descendant() {
    let (guard, mut root, root_pid, descendant_pid) = spawn_prepared_descendant_tree();

    let _proof = guard.terminate_and_wait(Duration::from_secs(2)).unwrap();
    let status = root.wait().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_pid_alive(root_pid));
    assert!(!qol_process::is_pid_alive(descendant_pid));
}

#[cfg(any(target_os = "linux", windows))]
#[test]
fn pending_spawn_recovery_terminates_only_the_prepared_tree() {
    let (guard, mut root, root_pid, descendant_pid) = spawn_prepared_descendant_tree();
    let mut unrelated = long_running_command().spawn().unwrap();
    let unrelated_pid = unrelated.id();

    let _proof = guard.recover_pending_spawn(Duration::from_secs(2)).unwrap();
    let status = root.wait().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_pid_alive(root_pid));
    assert!(!qol_process::is_pid_alive(descendant_pid));
    assert!(qol_process::is_pid_alive(unrelated_pid));
    qol_process::terminate_owned(&mut unrelated, Duration::from_millis(20)).unwrap();
}

#[cfg(target_os = "linux")]
fn isolated_command(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    qol_process::isolate_owned_command(&mut command).unwrap();
    command
}

#[cfg(target_os = "linux")]
fn isolated_session_command(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    qol_process::isolate_owned_session(&mut command).unwrap();
    command
}

#[cfg(target_os = "linux")]
#[test]
fn process_tree_terminates_only_its_child_in_the_callers_group() {
    let command = long_running_command();
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    let child_pid = child.id();

    let waiter = std::thread::spawn(move || {
        let mut child = child;
        child.wait().unwrap()
    });
    let _proof = guard.terminate_and_wait(Duration::from_secs(1)).unwrap();
    let status = waiter.join().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_pid_alive(child_pid));
    assert!(qol_process::is_pid_alive(std::process::id()));
}

#[cfg(target_os = "linux")]
#[test]
fn exact_process_scope_accepts_an_already_reaped_child() {
    let command = exit_command(0);
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(command).unwrap();
    let mut child = prepared.spawn().unwrap();

    assert!(child.wait().unwrap().success());
    let _proof = guard.terminate_and_wait(Duration::from_millis(20)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn preexec_cgroup_owns_an_immediate_exit_before_assignment() {
    let command = exit_command(0);
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(command).unwrap();
    let mut child = prepared.spawn().unwrap();
    std::thread::sleep(Duration::from_millis(100));

    assert!(guard.root_has_exited().unwrap());
    assert!(child.wait().unwrap().success());
    let _proof = guard.terminate_and_wait(Duration::from_secs(1)).unwrap();
}

#[cfg(unix)]
#[test]
fn current_process_tree_guard_disarms_idempotently() {
    let mut guard = qol_process::guard_current_process_tree().unwrap();

    guard.disarm().unwrap();
    guard.disarm().unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn prepared_command_cannot_assign_a_different_exited_child() {
    let first_command = isolated_command("exec sleep 30");
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(first_command).unwrap();
    let mut second = exit_command(0).spawn().unwrap();
    assert!(second.wait().unwrap().success());
    assert_eq!(
        guard
            .prepare_command(isolated_command("exec sleep 30"))
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );
    let first = prepared.spawn().unwrap();
    let first_pid = first.id();
    let waiter = std::thread::spawn(move || {
        let mut first = first;
        first.wait().unwrap()
    });

    let _proof = guard.terminate_and_wait(Duration::from_secs(1)).unwrap();
    assert!(!waiter.join().unwrap().success());
    assert!(!qol_process::is_pid_alive(first_pid));
}

#[cfg(target_os = "linux")]
#[test]
fn process_tree_terminates_descendants_after_the_leader_exits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let script = "printf ready > \"$QOL_PROCESS_TREE_ROOT/ready\"; while [ ! -f \"$QOL_PROCESS_TREE_ROOT/release\" ]; do sleep 0.01; done; ( trap 'kill -TERM \"$worker\" 2>/dev/null; wait \"$worker\"; exit 0' TERM; sleep 30 & worker=$!; printf '%s' \"$worker\" > \"$QOL_PROCESS_TREE_ROOT/descendant\"; wait \"$worker\" ) & while [ ! -s \"$QOL_PROCESS_TREE_ROOT/descendant\" ]; do sleep 0.01; done";
    let mut command = isolated_session_command(script);
    let guard = owned_process_tree();
    command
        .env("QOL_PROCESS_TREE_ROOT", root)
        .stderr(std::process::Stdio::null());
    let prepared = guard.prepare_command(command).unwrap();
    let mut leader = prepared.spawn().unwrap();
    let leader_pid = leader.id();

    wait_for_path(&root.join("ready"));
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

#[cfg(target_os = "linux")]
#[test]
fn owned_session_descendant_helper() {
    let Some(root) = std::env::var_os("QOL_PROCESS_SESSION_TEST_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let mut command = isolated_session_command(
        "trap '' TERM; printf ready > \"$QOL_PROCESS_SESSION_READY\"; exec sleep 30",
    );
    command.env("QOL_PROCESS_SESSION_READY", root.join("ready"));
    let mut child = command.spawn().unwrap();
    std::fs::write(root.join("descendant"), child.id().to_string()).unwrap();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn cgroup_tree_terminates_a_descendant_that_escapes_to_a_new_session() {
    let temp = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "owned_session_descendant_helper", "--nocapture"])
        .env("QOL_PROCESS_SESSION_TEST_ROOT", temp.path());
    qol_process::isolate_owned_session(&mut command).unwrap();
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(command).unwrap();
    let root = prepared.spawn().unwrap();
    let root_pid = root.id();

    wait_for_path(&temp.path().join("ready"));
    let descendant_pid = std::fs::read_to_string(temp.path().join("descendant"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert_eq!(unsafe { libc::getsid(root_pid as i32) }, root_pid as i32);
    assert_eq!(
        unsafe { libc::getsid(descendant_pid as i32) },
        descendant_pid as i32
    );
    assert_eq!(
        unsafe { libc::getpgid(descendant_pid as i32) },
        descendant_pid as i32
    );

    let mut unrelated = isolated_session_command("exec sleep 30").spawn().unwrap();
    let unrelated_pid = unrelated.id();
    let waiter = std::thread::spawn(move || {
        let mut root = root;
        root.wait().unwrap()
    });

    let _proof = guard.terminate_and_wait(Duration::from_secs(2)).unwrap();
    let status = waiter.join().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_pid_alive(root_pid));
    assert!(!qol_process::is_pid_alive(descendant_pid));
    assert!(qol_process::is_pid_alive(unrelated_pid));
    qol_process::terminate_owned(&mut unrelated, Duration::from_millis(20)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn cgroup_kill_catches_a_descendant_forked_from_a_term_handler() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let forked = temp.path().join("forked");
    let script = "trap 'trap \"\" TERM; sleep 30 & printf %s \"$!\" > \"$QOL_PROCESS_FORKED\"' TERM; printf ready > \"$QOL_PROCESS_READY\"; while :; do sleep 0.05; done";
    let mut command = isolated_session_command(script);
    command
        .env("QOL_PROCESS_READY", &ready)
        .env("QOL_PROCESS_FORKED", &forked);
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    wait_for_path(&ready);
    let waiter = std::thread::spawn(move || {
        let mut child = child;
        child.wait().unwrap()
    });

    let _proof = guard.terminate_and_wait(Duration::from_secs(2)).unwrap();
    let status = waiter.join().unwrap();

    assert!(!status.success());
    wait_for_path(&forked);
    let forked_pid = std::fs::read_to_string(&forked)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    assert!(!qol_process::is_pid_alive(forked_pid));
}

#[cfg(target_os = "linux")]
#[test]
fn nested_cgroup_owner_helper() {
    let Some(marker) = std::env::var_os("QOL_PROCESS_NESTED_CGROUP_MARKER") else {
        return;
    };
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap()
        .to_string();
    let arbitrary = std::path::Path::new("/sys/fs/cgroup")
        .join(cgroup.trim_start_matches('/'))
        .join("arbitrary-child");
    std::fs::create_dir(&arbitrary).unwrap();
    std::fs::write(arbitrary.join("cgroup.procs"), b"0\n").unwrap();
    let inner = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let command = isolated_session_command("trap '' TERM; exec sleep 30");
    let prepared = inner.prepare_command(command).unwrap();
    let mut child = prepared.spawn().unwrap();
    let child_cgroup = std::fs::read_to_string(format!("/proc/{}/cgroup", child.id())).unwrap();
    std::fs::write(marker, format!("{}\n{child_cgroup}", child.id())).unwrap();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn parent_cgroup_proof_includes_nested_guards_below_an_arbitrary_child() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("nested-pid");
    let outer = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "nested_cgroup_owner_helper", "--nocapture"])
        .env("QOL_PROCESS_NESTED_CGROUP_MARKER", &marker);
    qol_process::isolate_owned_session(&mut command).unwrap();
    let prepared = outer.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    wait_for_path(&marker);
    let marker = std::fs::read_to_string(&marker).unwrap();
    let (nested_pid, nested_cgroup) = marker.split_once('\n').unwrap();
    let nested_pid = nested_pid.parse::<u32>().unwrap();
    assert!(nested_cgroup.contains("/arbitrary-child/qol-process-v1-"));
    let waiter = std::thread::spawn(move || {
        let mut child = child;
        child.wait().unwrap()
    });

    let _proof = outer.terminate_and_wait(Duration::from_secs(2)).unwrap();
    let status = waiter.join().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_pid_alive(nested_pid));
}

#[cfg(target_os = "linux")]
#[test]
fn abrupt_nested_guard_grandchild_helper() {
    let Some(root) = std::env::var_os("QOL_PROCESS_PDEATH_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    std::fs::write(root.join("child"), std::process::id().to_string()).unwrap();
    let guard = owned_process_tree();
    let command = isolated_session_command("trap '' TERM; exec sleep 30");
    let prepared = guard.prepare_command(command).unwrap();
    let mut grandchild = prepared.spawn().unwrap();
    std::fs::write(root.join("grandchild"), grandchild.id().to_string()).unwrap();
    let _ = grandchild.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn abrupt_nested_guard_owner_helper() {
    let Some(root) = std::env::var_os("QOL_PROCESS_PDEATH_OWNER_ROOT") else {
        return;
    };
    let guard = owned_process_tree();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "abrupt_nested_guard_grandchild_helper",
            "--nocapture",
        ])
        .env("QOL_PROCESS_PDEATH_ROOT", &root);
    qol_process::isolate_owned_session(&mut command).unwrap();
    let prepared = guard.prepare_command(command).unwrap();
    let _child = prepared.spawn().unwrap();
    let root = std::path::PathBuf::from(root);
    wait_for_path(&root.join("grandchild"));
    std::process::exit(0);
}

#[cfg(target_os = "linux")]
#[test]
fn abrupt_nested_guard_owner_exit_kills_child_and_grandchild() {
    let temp = tempfile::tempdir().unwrap();
    let mut owner = Command::new(std::env::current_exe().unwrap());
    owner
        .args(["--exact", "abrupt_nested_guard_owner_helper", "--nocapture"])
        .env("QOL_PROCESS_PDEATH_OWNER_ROOT", temp.path());
    assert!(owner.spawn().unwrap().wait().unwrap().success());
    let child = std::fs::read_to_string(temp.path().join("child"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let grandchild = std::fs::read_to_string(temp.path().join("grandchild"))
        .unwrap()
        .parse::<u32>()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while (qol_process::is_pid_alive(child) || qol_process::is_pid_alive(grandchild))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!qol_process::is_pid_alive(child));
    assert!(!qol_process::is_pid_alive(grandchild));
    drop(owned_process_tree());
}

#[cfg(target_os = "linux")]
#[test]
#[allow(clippy::zombie_processes)]
fn abrupt_guarded_owner_helper() {
    let Some(root) = std::env::var_os("QOL_PROCESS_ABRUPT_GUARD_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let guard = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "ordinary_descendant_helper", "--nocapture"])
        .env("QOL_PROCESS_ORDINARY_DESCENDANT", root.join("descendant"));
    let prepared = guard.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    std::fs::write(root.join("root"), child.id().to_string()).unwrap();
    wait_for_path(&root.join("descendant"));
    let inherited = unsafe { libc::fork() };
    assert_ne!(inherited, -1);
    if inherited == 0 {
        loop {
            unsafe {
                libc::pause();
            }
        }
    }
    std::fs::write(root.join("inherited"), inherited.to_string()).unwrap();
    let dropper = unsafe { libc::fork() };
    assert_ne!(dropper, -1);
    if dropper == 0 {
        drop(guard);
        unsafe { libc::_exit(0) };
    }
    wait_for_raw_child(dropper);
    let descendant = read_pid(&root.join("descendant"));
    let preserved = qol_process::is_pid_alive(child.id()) && qol_process::is_pid_alive(descendant);
    std::fs::write(root.join("fork-drop-preserved"), preserved.to_string()).unwrap();
    std::fs::write(root.join("ready"), "ready").unwrap();
    loop {
        unsafe {
            libc::pause();
        }
    }
}

#[cfg(target_os = "linux")]
fn wait_for_raw_child(pid: libc::pid_t) {
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid, &mut status, 0) };
        if result == pid {
            assert!(libc::WIFEXITED(status));
            assert_eq!(libc::WEXITSTATUS(status), 0);
            return;
        }
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::Interrupted
        );
    }
}

#[cfg(target_os = "linux")]
fn linux_cgroup_path(pid: u32) -> std::path::PathBuf {
    let content = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap();
    let relative = content
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap();
    std::fs::canonicalize(
        std::path::Path::new("/sys/fs/cgroup").join(relative.trim_start_matches('/')),
    )
    .unwrap()
}

#[cfg(target_os = "linux")]
fn private_journal_root(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let journal = root.join(name);
    std::fs::create_dir(&journal).unwrap();
    std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o700)).unwrap();
    journal
}

#[cfg(target_os = "linux")]
#[test]
#[allow(clippy::zombie_processes)]
fn containing_scope_guarded_owner_helper() {
    let Some(root) = std::env::var_os("QOL_PROCESS_SCOPE_GUARD_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    std::fs::write(root.join("owner"), std::process::id().to_string()).unwrap();
    wait_for_path(&root.join("release"));
    let guard = qol_process::own_current_process_tree_with_guardian(guardian_command()).unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "ordinary_descendant_helper", "--nocapture"])
        .env(
            "QOL_PROCESS_ORDINARY_DESCENDANT",
            root.join("scope-descendant"),
        );
    let prepared = guard.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    std::fs::write(root.join("scope-root"), child.id().to_string()).unwrap();
    wait_for_path(&root.join("scope-descendant"));
    let target = linux_cgroup_path(child.id());
    std::fs::write(
        root.join("target-cgroup"),
        target.as_os_str().as_encoded_bytes(),
    )
    .unwrap();
    std::fs::write(root.join("scope-ready"), "ready").unwrap();
    loop {
        unsafe {
            libc::pause();
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn stale_guardian_recovery_helper() {
    if std::env::var_os("QOL_PROCESS_GUARDIAN_RECOVERY").is_none() {
        return;
    }
    drop(owned_process_tree());
}

#[cfg(target_os = "linux")]
fn recover_stale_guardian_journal(
    journal: &std::path::Path,
    cgroup_root: Option<&std::path::Path>,
) {
    let journal_path = std::fs::read_dir(journal)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|value| value.to_str()) == Some("lock"))
        .unwrap();
    let cgroup_path = std::fs::read_to_string(&journal_path)
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("path="))
        .map(std::path::PathBuf::from)
        .unwrap();
    assert!(cgroup_path.exists());
    let mut recovery = Command::new(std::env::current_exe().unwrap());
    recovery
        .args(["--exact", "stale_guardian_recovery_helper", "--nocapture"])
        .env("QOL_PROCESS_GUARDIAN_RECOVERY", "1")
        .env("QOL_PROCESS_CGROUP_JOURNAL_ROOT", journal);
    if let Some(cgroup_root) = cgroup_root {
        recovery.env("QOL_PROCESS_CGROUP_ROOT", cgroup_root);
    }
    let status = recovery.spawn().unwrap().wait().unwrap();
    assert!(status.success());
    assert!(!cgroup_path.exists());
    assert!(!journal_path.exists());
}

#[cfg(target_os = "linux")]
#[test]
fn abrupt_owner_death_kills_an_ordinary_descendant_before_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let journal = private_journal_root(temp.path(), "journal");
    let mut owner = Command::new(std::env::current_exe().unwrap());
    owner
        .args(["--exact", "abrupt_guarded_owner_helper", "--nocapture"])
        .env("QOL_PROCESS_ABRUPT_GUARD_ROOT", temp.path())
        .env("QOL_PROCESS_CGROUP_JOURNAL_ROOT", &journal);
    let mut owner = owner.spawn().unwrap();
    wait_for_path(&temp.path().join("ready"));
    let root_pid = read_pid(&temp.path().join("root"));
    let descendant_pid = read_pid(&temp.path().join("descendant"));
    let inherited_pid = read_pid(&temp.path().join("inherited"));
    let fork_drop_preserved =
        std::fs::read_to_string(temp.path().join("fork-drop-preserved")).unwrap() == "true";

    owner.kill().unwrap();
    let owner_failed = !owner.wait().unwrap().success();
    let tree_died_before_recovery =
        wait_for_pids_to_exit(&[root_pid, descendant_pid], Duration::from_secs(3));
    let inherited_survived_owner = qol_process::is_pid_alive(inherited_pid);

    qol_process::kill_pid(inherited_pid).unwrap();
    let inherited_died = wait_for_pids_to_exit(&[inherited_pid], Duration::from_secs(2));

    recover_stale_guardian_journal(&journal, None);
    assert!(fork_drop_preserved);
    assert!(owner_failed);
    assert!(tree_died_before_recovery);
    assert!(inherited_survived_owner);
    assert!(inherited_died);
}

#[cfg(target_os = "linux")]
fn spawn_containing_scope_owner(
    root: &std::path::Path,
    stable: &std::path::Path,
    journal: &std::path::Path,
) -> std::process::Child {
    let mut owner = Command::new(std::env::current_exe().unwrap());
    owner
        .args([
            "--exact",
            "containing_scope_guarded_owner_helper",
            "--nocapture",
        ])
        .env("QOL_PROCESS_SCOPE_GUARD_ROOT", root)
        .env("QOL_PROCESS_CGROUP_ROOT", stable)
        .env("QOL_PROCESS_CGROUP_JOURNAL_ROOT", journal);
    owner.spawn().unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn containing_scope_kill_recursively_cleans_the_guarded_tree() {
    let temp = tempfile::tempdir().unwrap();
    let journal = private_journal_root(temp.path(), "scope-journal");
    let stable = linux_cgroup_path(std::process::id());
    let unique = temp.path().file_name().unwrap().to_string_lossy();
    let scope = stable.join(format!("qol-scope-test-{unique}"));
    std::fs::create_dir(&scope).unwrap();
    let mut owner = spawn_containing_scope_owner(temp.path(), &stable, &journal);
    wait_for_path(&temp.path().join("owner"));
    std::fs::write(scope.join("cgroup.procs"), owner.id().to_string()).unwrap();
    let owner_was_moved = linux_cgroup_path(owner.id()) == scope;
    std::fs::write(temp.path().join("release"), "release").unwrap();
    wait_for_path(&temp.path().join("scope-ready"));
    let root = read_pid(&temp.path().join("scope-root"));
    let descendant = read_pid(&temp.path().join("scope-descendant"));
    let target = std::path::PathBuf::from(
        std::fs::read_to_string(temp.path().join("target-cgroup")).unwrap(),
    );
    let target_was_nested = target.parent() == Some(scope.as_path());

    std::fs::write(scope.join("cgroup.kill"), "1").unwrap();
    let owner_failed = !owner.wait().unwrap().success();
    let tree_died_before_recovery =
        wait_for_pids_to_exit(&[root, descendant], Duration::from_secs(3));
    recover_stale_guardian_journal(&journal, Some(&stable));
    std::fs::remove_dir(&scope).unwrap();

    assert!(owner_was_moved);
    assert!(target_was_nested);
    assert!(owner_failed);
    assert!(tree_died_before_recovery);
}

#[cfg(target_os = "linux")]
#[test]
fn delegated_root_override_helper() {
    if std::env::var_os("QOL_PROCESS_OVERRIDE_HELPER").is_none() {
        return;
    }
    qol_process::process_tree_containment_support().unwrap();
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(exit_command(0)).unwrap();
    let mut child = prepared.spawn().unwrap();
    assert!(child.wait().unwrap().success());
    let _proof = guard.terminate_and_wait(Duration::from_secs(1)).unwrap();
}

#[cfg(target_os = "linux")]
#[test]
fn validated_delegated_and_journal_root_overrides_support_containment() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let journal = temp.path().join("journal");
    std::fs::create_dir(&journal).unwrap();
    std::fs::set_permissions(&journal, std::fs::Permissions::from_mode(0o700)).unwrap();
    let cgroup = std::fs::read_to_string("/proc/self/cgroup")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .unwrap()
        .to_string();
    let cgroup = std::fs::canonicalize(
        std::path::Path::new("/sys/fs/cgroup").join(cgroup.trim_start_matches('/')),
    )
    .unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "delegated_root_override_helper", "--nocapture"])
        .env("QOL_PROCESS_OVERRIDE_HELPER", "1")
        .env("QOL_PROCESS_CGROUP_ROOT", cgroup)
        .env("QOL_PROCESS_CGROUP_JOURNAL_ROOT", &journal)
        .spawn()
        .unwrap()
        .wait()
        .unwrap();

    assert!(status.success());
    assert!(journal.is_dir());
}

#[cfg(target_os = "linux")]
#[test]
fn process_tree_escalates_and_verifies_for_a_term_resistant_group() {
    let temp = tempfile::tempdir().unwrap();
    let ready = temp.path().join("ready");
    let mut command =
        isolated_command("trap '' TERM; printf ready > \"$QOL_PROCESS_TREE_READY\"; exec sleep 30");
    command.env("QOL_PROCESS_TREE_READY", &ready);
    let guard = owned_process_tree();
    let prepared = guard.prepare_command(command).unwrap();
    let child = prepared.spawn().unwrap();
    let pid = child.id();

    wait_for_path(&ready);
    let waiter = std::thread::spawn(move || {
        let mut child = child;
        child.wait().unwrap()
    });

    let _proof = guard.terminate_and_wait(Duration::from_secs(1)).unwrap();
    let status = waiter.join().unwrap();

    assert!(!status.success());
    assert!(!qol_process::is_group_alive(pid));
}
