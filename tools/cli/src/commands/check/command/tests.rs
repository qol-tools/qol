use super::*;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
struct FakeCancellation {
    cancelled: Arc<AtomicBool>,
    escalated: Arc<AtomicBool>,
}

impl CancellationState for FakeCancellation {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn escalation_requested(&self) -> bool {
        self.escalated.load(Ordering::Acquire)
    }
}

#[test]
fn required_containment_rejects_unavailable_ownership() {
    let error = CommandOwner::from_attempt(
        Containment::Required,
        Err(anyhow::anyhow!("unsupported containment")),
    )
    .err()
    .unwrap();

    assert!(error
        .to_string()
        .contains("verified process-tree containment is required"));
}

#[test]
fn preferred_containment_retains_the_worktree_fallback() {
    assert!(matches!(
        CommandOwner::from_attempt(
            Containment::Preferred,
            Err(anyhow::anyhow!("unsupported containment")),
        )
        .unwrap(),
        CommandOwner::Fallback { .. }
    ));
}

#[cfg(unix)]
#[test]
fn cancellation_escalates_and_reaps_the_owned_group() {
    let root = tempfile::tempdir().unwrap();
    let leader = root.path().join("leader");
    let descendant = root.path().join("descendant");
    let cancellation = FakeCancellation::default();
    let trigger = cancellation.clone();
    let leader_for_trigger = leader.clone();
    let trigger_thread = thread::spawn(move || {
        wait_for_path(&leader_for_trigger);
        trigger.cancelled.store(true, Ordering::Release);
        thread::sleep(Duration::from_millis(50));
        trigger.escalated.store(true, Ordering::Release);
    });
    let mut command = stubborn_group_command(&leader, &descendant);

    let output = run(&mut command, &cancellation, Containment::Preferred, false);
    let error = output.result.unwrap_err();

    trigger_thread.join().unwrap();
    assert!(
        error.to_string().contains("cancelled"),
        "unexpected error: {error:#}"
    );
    let leader = read_pid(&leader);
    let descendant = read_pid(&descendant);
    assert!(!qol_process::is_group_alive(leader));
    assert!(!qol_process::is_pid_alive(descendant));
}

#[cfg(unix)]
#[test]
fn signal_cancellation_runner_helper() {
    let Some(root) = std::env::var_os("QOL_CHECK_CANCELLATION_TEST_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let token = CancellationToken::install().unwrap();
    let mut command = stubborn_group_command(&root.join("leader"), &root.join("descendant"));
    let output = run(&mut command, &token, Containment::Preferred, false);
    let exit = output.leader;
    let result = output.result;
    fs::write(root.join("terminal"), format!("{exit:?} {result:?}")).unwrap();
}

#[cfg(unix)]
#[test]
fn sigterm_reaches_the_runner_and_allows_terminal_finalization() {
    let root = tempfile::tempdir().unwrap();
    let mut helper = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::check::command::tests::signal_cancellation_runner_helper",
        ])
        .env("QOL_CHECK_CANCELLATION_TEST_ROOT", root.path())
        .spawn()
        .unwrap();
    wait_for_path(&root.path().join("leader"));
    qol_process::signal_term_pid(helper.id()).unwrap();
    thread::sleep(Duration::from_millis(50));
    qol_process::signal_term_pid(helper.id()).unwrap();
    let status = helper.wait().unwrap();

    assert!(status.success());
    let terminal = fs::read_to_string(root.path().join("terminal")).unwrap();
    assert!(
        terminal.contains("cancelled"),
        "unexpected terminal result: {terminal}"
    );
    assert!(!qol_process::is_group_alive(read_pid(
        &root.path().join("leader")
    )));
    assert!(!qol_process::is_pid_alive(read_pid(
        &root.path().join("descendant")
    )));
}

#[cfg(target_os = "linux")]
#[test]
fn escaped_session_descendant_helper() {
    let Some(marker) = std::env::var_os("QOL_CHECK_ESCAPED_SESSION_MARKER") else {
        return;
    };
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "trap '' TERM; echo $$ > \"$1\"; exec sleep 30",
            "qol-check-escaped",
        ])
        .arg(marker);
    qol_process::isolate_owned_session(&mut command).unwrap();
    let mut child = command.spawn().unwrap();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn required_containment_reaps_a_descendant_that_escapes_its_session() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("escaped");
    let cancellation = FakeCancellation::default();
    let trigger = cancellation.clone();
    let marker_for_trigger = marker.clone();
    let trigger_thread = thread::spawn(move || {
        wait_for_path(&marker_for_trigger);
        trigger.cancelled.store(true, Ordering::Release);
        trigger.escalated.store(true, Ordering::Release);
    });
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "commands::check::command::tests::escaped_session_descendant_helper",
        ])
        .env("QOL_CHECK_ESCAPED_SESSION_MARKER", &marker);

    let output = run(&mut command, &cancellation, Containment::Required, false);
    let error = output.result.unwrap_err();

    trigger_thread.join().unwrap();
    assert!(
        error.to_string().contains("cancelled"),
        "unexpected error: {error:#}"
    );
    assert!(!qol_process::is_pid_alive(read_pid(&marker)));
}

#[cfg(unix)]
#[test]
fn successful_steps_report_a_zero_exit_code() {
    let cancellation = FakeCancellation::default();
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);

    let output = run(&mut command, &cancellation, Containment::Preferred, false);

    assert_eq!(output.leader.and_then(|status| status.code()), Some(0));
    assert!(output.result.is_ok());
}

#[cfg(unix)]
#[test]
fn failed_steps_report_their_exit_code() {
    let cancellation = FakeCancellation::default();
    let mut command = Command::new("sh");
    command.args(["-c", "exit 7"]);

    let output = run(&mut command, &cancellation, Containment::Preferred, false);

    assert_eq!(output.leader.and_then(|status| status.code()), Some(7));
    assert!(output.result.is_err());
}

#[cfg(unix)]
#[test]
fn captured_run_returns_stdout_and_exit_status() {
    let cancellation = FakeCancellation::default();
    let mut command = Command::new("sh");
    command.args(["-c", "cat"]);

    let output = run_captured(
        &mut command,
        b"formatted source",
        &cancellation,
        Containment::Preferred,
    );

    output.command.result.unwrap();
    assert!(output.command.leader.unwrap().success());
    assert_eq!(output.stdout, b"formatted source".as_slice());
}

#[cfg(unix)]
#[test]
fn captured_run_reports_a_nonzero_exit_and_stderr() {
    let cancellation = FakeCancellation::default();
    let mut command = Command::new("sh");
    command.args(["-c", "printf problem >&2; exit 3"]);

    let output = run_captured(&mut command, b"", &cancellation, Containment::Preferred);

    assert!(output.command.result.is_err());
    assert_eq!(output.command.leader.unwrap().code(), Some(3));
    assert_eq!(output.stderr, b"problem".as_slice());
}

#[cfg(unix)]
#[test]
fn captured_run_rejects_a_cancelled_token_before_spawn() {
    let cancellation = FakeCancellation::default();
    cancellation.cancelled.store(true, Ordering::Release);
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);

    let output = run_captured(&mut command, b"", &cancellation, Containment::Preferred);

    assert!(output
        .command
        .result
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
}

#[cfg(unix)]
#[test]
fn captured_run_cancellation_reaps_the_owned_group() {
    let root = tempfile::tempdir().unwrap();
    let leader = root.path().join("leader");
    let descendant = root.path().join("descendant");
    let cancellation = FakeCancellation::default();
    let trigger = cancellation.clone();
    let leader_for_trigger = leader.clone();
    let trigger_thread = thread::spawn(move || {
        wait_for_path(&leader_for_trigger);
        trigger.cancelled.store(true, Ordering::Release);
        thread::sleep(Duration::from_millis(50));
        trigger.escalated.store(true, Ordering::Release);
    });
    let mut command = stubborn_group_command(&leader, &descendant);

    let output = run_captured(&mut command, b"", &cancellation, Containment::Preferred);
    trigger_thread.join().unwrap();

    assert!(output
        .command
        .result
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    assert!(output.command.leader.is_some());
    assert!(!qol_process::is_group_alive(read_pid(&leader)));
    assert!(!qol_process::is_pid_alive(read_pid(&descendant)));
}

#[cfg(unix)]
fn stubborn_group_command(leader: &Path, descendant: &Path) -> Command {
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "trap '' TERM; echo $$ > \"$1\"; sleep 30 & echo $! > \"$2\"; wait",
            "qol-check-test",
        ])
        .arg(leader)
        .arg(descendant);
    command
}

#[cfg(unix)]
fn wait_for_path(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(path.exists(), "timed out waiting for {}", path.display());
}

#[cfg(unix)]
fn read_pid(path: &Path) -> u32 {
    fs::read_to_string(path).unwrap().trim().parse().unwrap()
}

#[cfg(target_os = "linux")]
#[test]
#[allow(clippy::zombie_processes)]
fn residual_evidence_child_helper() {
    use std::os::unix::net::UnixStream;
    let Some(root) = std::env::var_os("QOL_CHECK_RESIDUAL_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let member = std::env::var_os("QOL_CHECK_RESIDUAL_MEMBER").is_some();
    if !member {
        let _child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::check::command::tests::residual_evidence_child_helper",
            ])
            .env("QOL_CHECK_RESIDUAL_MEMBER", "1")
            .spawn()
            .unwrap();
    }
    let endpoint = if member { "member.sock" } else { "leader.sock" };
    let mut socket = UnixStream::connect(root.join(endpoint)).unwrap();
    socket.write_all(&std::process::id().to_le_bytes()).unwrap();
    socket.read_exact(&mut [0_u8]).unwrap();
    let code = if member {
        0
    } else {
        std::env::var("QOL_CHECK_RESIDUAL_EXIT")
            .unwrap()
            .parse()
            .unwrap()
    };
    std::process::exit(code);
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
enum ResidualCase {
    Observed,
    Disappearing,
    Unavailable,
    Truncated,
    Cancelled,
}

#[cfg(target_os = "linux")]
#[test]
fn residual_evidence_preserves_status_verdict_and_cleanup() {
    use qol_process::Observation;
    use std::os::unix::net::UnixListener;
    for (code, case) in [
        (0, ResidualCase::Observed),
        (7, ResidualCase::Observed),
        (0, ResidualCase::Disappearing),
        (7, ResidualCase::Unavailable),
        (0, ResidualCase::Truncated),
        (7, ResidualCase::Cancelled),
    ] {
        let mut output = CommandResult::new(Containment::Required);
        let owner = output
            .acquire()
            .expect("Required containment prerequisite must be available");
        let root = tempfile::tempdir().unwrap();
        let member_listener = UnixListener::bind(root.path().join("member.sock")).unwrap();
        let leader_listener = UnixListener::bind(root.path().join("leader.sock")).unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "commands::check::command::tests::residual_evidence_child_helper",
            ])
            .env("QOL_CHECK_RESIDUAL_FIXTURE", root.path())
            .env("QOL_CHECK_RESIDUAL_EXIT", code.to_string())
            .env_remove("QOL_CHECK_RESIDUAL_MEMBER");
        let mut child = owner.spawn(&mut command).unwrap();
        let mut member = accept_residual_fixture(&member_listener);
        let mut member_pid = [0; 4];
        member.read_exact(&mut member_pid).unwrap();
        let member_pid = u32::from_le_bytes(member_pid);
        let mut leader = accept_residual_fixture(&leader_listener);
        leader.read_exact(&mut [0; 4]).unwrap();
        leader.write_all(&[1]).unwrap();
        assert_eq!(child.wait().unwrap().code(), Some(code));
        let cancellation = FakeCancellation::default();
        if matches!(case, ResidualCase::Cancelled) {
            cancellation.cancelled.store(true, Ordering::Release);
            cancellation.escalated.store(true, Ordering::Release);
        }
        let leader_pid = child.id();
        let mut observations = 0;
        let outcome =
            wait_with_observation(&mut child, &owner, &cancellation, &mut output, |owner| {
                observations += 1;
                let CommandOwner::Tree(tree) = owner else {
                    panic!("Required selected fallback")
                };
                match case {
                    ResidualCase::Observed => tree.observe_residual(),
                    ResidualCase::Disappearing => {
                        member.write_all(&[1]).unwrap();
                        let deadline = Instant::now() + Duration::from_secs(3);
                        while owner.is_alive(leader_pid).unwrap() {
                            assert!(
                                Instant::now() < deadline,
                                "member did not exit after release"
                            );
                            thread::yield_now();
                        }
                        tree.observe_residual()
                    }
                    ResidualCase::Unavailable | ResidualCase::Truncated => {
                        let mut observation = tree.observe_residual();
                        observation.incomplete = true;
                        observation.truncated = matches!(case, ResidualCase::Truncated);
                        observation.root_populated_after =
                            if matches!(case, ResidualCase::Unavailable) {
                                Observation::Unavailable
                            } else {
                                Observation::Truncated
                            };
                        observation
                    }
                    ResidualCase::Cancelled => {
                        panic!("cancellation must not collect residual members")
                    }
                }
            });
        let outcome = recover_wait_failure(&mut child, &owner, outcome, &mut output);
        output.result = outcome.map(drop);
        assert!(output.result.is_err(), "{code} {case:?}");
        assert_eq!(output.leader.unwrap().code(), Some(code), "{case:?}");
        assert!(!owner.is_alive(child.id()).unwrap(), "{case:?}");
        assert!(qol_process::is_pid_gone(member_pid), "{case:?}");
        assert!(output.lifecycle.recovery_seal.as_ref().unwrap().is_ok());
        let evidence = super::super::report::command_evidence(&output);
        assert_eq!(evidence["leader"]["code"], code);
        assert_eq!(evidence["containment"]["acquired"], "verified_tree");
        assert_eq!(evidence["containment"]["backend"], "linux_cgroup_v2");
        let cancelled = matches!(case, ResidualCase::Cancelled);
        assert_eq!(observations, usize::from(!cancelled));
        assert_eq!(
            evidence["shutdown_reason"],
            if cancelled {
                "cancelled"
            } else {
                "residual_tree"
            }
        );
        if !cancelled {
            assert_eq!(evidence["first_post_leader_liveness"]["value"], true);
        }
        if matches!(case, ResidualCase::Disappearing) {
            let observation = output.lifecycle.observation.as_ref().unwrap();
            match &observation.root_populated_before {
                Observation::Value(populated) => assert!(!*populated),
                Observation::Vanished
                | Observation::Unavailable
                | Observation::Reused
                | Observation::Truncated
                | Observation::BudgetExceeded
                | Observation::Unsupported
                | Observation::NotCaptured => assert!(observation.incomplete),
            }
        }
    }
}

#[test]
fn fallback_acquisition_is_sanitized_and_reported_without_membership_scan() {
    for kind in [io::ErrorKind::Unsupported, io::ErrorKind::PermissionDenied] {
        let owner = CommandOwner::from_attempt(
            Containment::Preferred,
            Err(io::Error::new(kind, "/private/fixture must never enter evidence").into()),
        )
        .unwrap();
        let CommandOwner::Fallback {
            acquisition_failure,
        } = &owner
        else {
            panic!("failed acquisition must use fallback");
        };
        assert_eq!(acquisition_failure, &format!("{kind:?}"));
        let mut output = CommandResult::new(Containment::Preferred);
        output.lifecycle.backend = platform::FALLBACK_BACKEND;
        output.lifecycle.acquisition_failure = Some(acquisition_failure.clone());
        output.lifecycle.membership_observation_supported = Some(false);
        output.lifecycle.observation = Some(owner.observe_residual());
        let mut command = Command::new("sh");
        command.args(["-c", "exit 7"]);
        let mut child = owner.spawn(&mut command).unwrap();
        let outcome = wait_for_exit(
            &mut child,
            &owner,
            &FakeCancellation::default(),
            &mut output,
        );
        let outcome = recover_wait_failure(&mut child, &owner, outcome, &mut output);
        output.result = outcome.and_then(|status| {
            if !status.success() {
                bail!("command failed with {status}");
            }
            Ok(())
        });
        assert_eq!(output.leader.unwrap().code(), Some(7));
        assert!(output.result.is_err());
        let evidence = super::super::report::command_evidence(&output);
        assert_eq!(evidence["containment"]["acquired"], "fallback");
        assert_eq!(evidence["containment"]["backend"], "unix_process_group");
        assert_eq!(evidence["residual_observation"]["support"], "unsupported");
        assert!(!evidence.to_string().contains("private"));
    }
}

#[test]
fn signal_status_remains_distinct_from_unobserved_status() {
    let mut command = Command::new("sh");
    command.args(["-c", "kill -TERM $$"]);
    let output = run(
        &mut command,
        &FakeCancellation::default(),
        Containment::Preferred,
        false,
    );
    assert!(output.result.is_err());
    let evidence = super::super::report::command_evidence(&output);
    assert_eq!(evidence["leader"]["kind"], "signal");
    assert_eq!(evidence["leader"]["signal"], 15);
    let cancellation = FakeCancellation::default();
    cancellation.cancelled.store(true, Ordering::Release);
    let output = run(
        &mut Command::new("unused"),
        &cancellation,
        Containment::Required,
        false,
    );
    let evidence = super::super::report::command_evidence(&output);
    assert_eq!(evidence["leader"]["kind"], "not_observed");
    assert_eq!(evidence["containment"]["acquired"], "not_acquired");
}

#[cfg(target_os = "linux")]
#[test]
fn recovery_failure_retains_observed_status_and_original_error() {
    for code in [0, 7] {
        let mut output = CommandResult::new(Containment::Required);
        let owner = output
            .acquire()
            .expect("Required containment prerequisite must be available");
        let mut command = Command::new("sh");
        command.args(["-c", &format!("exit {code}")]);
        qol_process::isolate_owned_command(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        output.leader = Some(child.wait().unwrap());
        let failure = Err(anyhow::anyhow!("original wait failure"));
        output.result = recover_wait_failure(&mut child, &owner, failure, &mut output).map(drop);
        assert_eq!(output.leader.unwrap().code(), Some(code));
        let error = format!("{:#}", output.result.as_ref().unwrap_err());
        assert!(error.contains("original wait failure"));
        assert!(error.contains("no assigned process"));
        assert!(output.lifecycle.recovery_force.as_ref().unwrap().is_ok());
        assert!(output.lifecycle.recovery_reap.as_ref().unwrap().is_ok());
        assert!(output.lifecycle.recovery_seal.as_ref().unwrap().is_err());
        let evidence = super::super::report::command_evidence(&output);
        assert_eq!(evidence["leader"]["code"], code);
        assert_eq!(evidence["cleanup"]["recovery_seal"]["status"], "failed");
    }
}

#[cfg(target_os = "linux")]
fn accept_residual_fixture(
    listener: &std::os::unix::net::UnixListener,
) -> std::os::unix::net::UnixStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                return stream;
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "fixture never became ready");
                thread::yield_now();
            }
            Err(error) => panic!("fixture accept failed: {error}"),
        }
    }
}

#[test]
fn residual_deadline_starts_before_observation() {
    let owner = CommandOwner::from_attempt(
        Containment::Preferred,
        Err(io::Error::from(io::ErrorKind::Unsupported).into()),
    )
    .unwrap();
    let mut command = Command::new("sh");
    command.args(["-c", "read value"]).stdin(Stdio::piped());
    let mut child = owner.spawn(&mut command).unwrap();
    let mut output = CommandResult::new(Containment::Preferred);
    let mut observed_at = None;
    let mut observe = Some(|owner: &CommandOwner| {
        observed_at = Some(Instant::now());
        owner.observe_residual()
    });
    let shutdown = begin_shutdown(
        &owner,
        child.id(),
        ShutdownReason::ResidualGroup,
        &mut output,
        &mut observe,
    )
    .unwrap();
    assert!(shutdown.deadline <= observed_at.unwrap() + TERMINATION_GRACE);
    force_stop(&mut child, &owner, &mut output).unwrap();
}

#[test]
fn recovery_accepts_a_group_whose_leader_exited_before_the_kill() {
    let owner = CommandOwner::from_attempt(
        Containment::Preferred,
        Err(io::Error::from(io::ErrorKind::Unsupported).into()),
    )
    .unwrap();
    let mut command = Command::new("sh");
    command.args(["-c", "exit 0"]);
    let mut child = owner.spawn(&mut command).unwrap();
    thread::sleep(Duration::from_millis(200));
    let mut output = CommandResult::new(Containment::Preferred);

    force_stop(&mut child, &owner, &mut output).unwrap();
    assert!(output.leader.is_some_and(|status| status.success()));
}
