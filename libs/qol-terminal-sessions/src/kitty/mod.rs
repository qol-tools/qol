mod parse;
mod socket;

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use parse::{parse_ls, KittyLs};

use crate::{
    BackendId, DeliveryMode, ScreenReader, SessionBinding, SessionFacts, SessionFocus,
    SessionInventory, TerminalBackend, TerminalError, TerminalSnapshot, TextInput,
};

const BACKEND_ID: &str = "kitty";

pub fn backend_id() -> &'static BackendId {
    static ID: std::sync::OnceLock<BackendId> = std::sync::OnceLock::new();
    ID.get_or_init(|| BackendId::new(BACKEND_ID).expect("Kitty backend id is valid"))
}

pub struct KittyBackend {
    runner: Arc<dyn CommandRunner>,
}

impl KittyBackend {
    fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
        Self { runner }
    }

    fn run(
        &self,
        operation: &'static str,
        args: &[String],
        stdin: Option<&str>,
    ) -> Result<String, TerminalError> {
        let output =
            self.runner
                .run(args, stdin)
                .map_err(|source| TerminalError::BackendUnavailable {
                    backend: backend_id().clone(),
                    source,
                })?;
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} operation={} success={} code={:?} stdout_len={}",
            backend_id(),
            operation,
            output.success,
            output.code,
            output.stdout.len()
        );
        if !output.success {
            return Err(TerminalError::CommandFailed {
                backend: backend_id().clone(),
                operation,
                code: output.code,
                stderr: output.stderr.trim().to_owned(),
            });
        }
        Ok(output.stdout)
    }

    fn sessions(&self) -> Result<Vec<SessionFacts>, TerminalError> {
        let body = self.run("discover sessions", &strings(["@", "ls"]), None)?;
        Ok(parse_ls(&body, backend_id())?.sessions(backend_id()))
    }

    fn ensure_capability(
        &self,
        target: &SessionBinding,
        capability: crate::SessionCapabilities,
        label: &'static str,
    ) -> Result<(), TerminalError> {
        let session = self
            .sessions()?
            .into_iter()
            .find(|session| session.id == *target.session_id())
            .ok_or_else(|| TerminalError::TargetMissing(target.clone()))?;
        if session.root_pid != target.root_pid() {
            return Err(TerminalError::TargetChanged {
                target: session.id,
                expected_root_pid: target.root_pid(),
                actual_root_pid: session.root_pid,
            });
        }
        if session.capabilities.contains(capability) {
            return Ok(());
        }
        Err(TerminalError::Unsupported {
            target: target.session_id().clone(),
            capability: label,
        })
    }

    fn read_screen_command(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.run(
            "read screen",
            &strings([
                "@",
                "get-text",
                "--match",
                &matcher(target),
                "--extent",
                "screen",
            ]),
            None,
        )
    }
}

impl Default for KittyBackend {
    fn default() -> Self {
        Self::with_runner(Arc::new(SystemCommandRunner::default()))
    }
}

impl TerminalBackend for KittyBackend {
    fn id(&self) -> &BackendId {
        backend_id()
    }

    fn read_screen_from_snapshot(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError> {
        snapshot.validate_screen_target(target)?;
        self.read_screen_command(target)
    }
}

impl SessionInventory for KittyBackend {
    fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
        self.sessions()
    }
}

impl ScreenReader for KittyBackend {
    fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.ensure_capability(
            target,
            crate::SessionCapabilities::SCREEN_READING,
            "screen reading",
        )?;
        self.read_screen_command(target)
    }
}

impl SessionFocus for KittyBackend {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        self.ensure_capability(target, crate::SessionCapabilities::FOCUS, "focus")?;
        self.run(
            "focus session",
            &strings(["@", "focus-window", "--match", &matcher(target)]),
            None,
        )
        .map(drop)
    }
}

impl TextInput for KittyBackend {
    fn send_text(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError> {
        self.ensure_capability(target, crate::SessionCapabilities::TEXT_INPUT, "text input")?;
        if text.is_empty() {
            return Ok(());
        }
        if mode == DeliveryMode::Submit {
            self.run(
                "insert text",
                &strings([
                    "@",
                    "send-text",
                    "--match",
                    &matcher(target),
                    "--stdin",
                    "--bracketed-paste=auto",
                ]),
                Some(text),
            )?;
            self.ensure_capability(target, crate::SessionCapabilities::TEXT_INPUT, "text input")?;
            return self
                .run(
                    "submit text",
                    &strings([
                        "@",
                        "send-text",
                        "--match",
                        &matcher(target),
                        "--stdin",
                        "--bracketed-paste=disable",
                    ]),
                    Some("\r"),
                )
                .map(drop);
        }
        self.run(
            "insert text",
            &strings([
                "@",
                "send-text",
                "--match",
                &matcher(target),
                "--stdin",
                "--bracketed-paste=auto",
            ]),
            Some(text),
        )
        .map(drop)
    }
}

fn matcher(target: &SessionBinding) -> String {
    format!("id:{}", target.session_id().native())
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_owned).collect()
}

trait CommandRunner: Send + Sync {
    fn run(&self, args: &[String], stdin: Option<&str>) -> std::io::Result<CommandOutput>;
}

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

struct SystemCommandRunner {
    program: String,
    timeout: Duration,
}

impl Default for SystemCommandRunner {
    fn default() -> Self {
        Self {
            program: "kitten".to_owned(),
            timeout: COMMAND_TIMEOUT,
        }
    }
}

impl CommandRunner for SystemCommandRunner {
    fn run(&self, args: &[String], stdin: Option<&str>) -> std::io::Result<CommandOutput> {
        if let Some(output) = socket::try_run(args, stdin) {
            return Ok(output);
        }
        self.spawn_kitten(args, stdin)
    }
}

impl SystemCommandRunner {
    fn spawn_kitten(&self, args: &[String], stdin: Option<&str>) -> std::io::Result<CommandOutput> {
        let mut command = Command::new(&self.program);
        command.args(args);
        if stdin.is_some() {
            command.stdin(Stdio::piped());
        }
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command.spawn()?;
        if let Some(input) = stdin {
            if let Some(mut child_stdin) = child.stdin.take() {
                let payload = input.to_owned();
                std::thread::spawn(move || {
                    let _ = child_stdin.write_all(payload.as_bytes());
                });
            }
        }
        wait_with_timeout(&mut child, self.timeout)
    }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> std::io::Result<CommandOutput> {
    let stdout_reader = child
        .stdout
        .take()
        .map(|mut pipe| std::thread::spawn(move || drain(&mut pipe)));
    let stderr_reader = child
        .stderr
        .take()
        .map(|mut pipe| std::thread::spawn(move || drain(&mut pipe)));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("child did not finish within {}s", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = join_drain(stdout_reader);
    let stderr = join_drain(stderr_reader);
    Ok(CommandOutput {
        success: status.success(),
        code: status.code(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn drain(pipe: &mut impl Read) -> Vec<u8> {
    let mut buffer = Vec::new();
    let _ = pipe.read_to_end(&mut buffer);
    buffer
}

fn join_drain(reader: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default()
}

#[derive(Debug)]
struct CommandOutput {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::{
        wait_with_timeout, CommandOutput, CommandRunner, KittyBackend, SystemCommandRunner,
    };
    use crate::{
        kitty::backend_id, DeliveryMode, SessionBinding, SessionCapabilities, SessionFacts,
        SessionId, TerminalBackend, TerminalError, TerminalSessionService, TerminalSnapshot,
        TextInput,
    };

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<(Vec<String>, Option<String>)>>,
        outputs: Mutex<VecDeque<CommandOutput>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<CommandOutput>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                outputs: Mutex::new(outputs.into()),
            })
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, args: &[String], stdin: Option<&str>) -> std::io::Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((args.to_vec(), stdin.map(str::to_owned)));
            Ok(self.outputs.lock().unwrap().pop_front().unwrap())
        }
    }

    #[test]
    fn submit_pastes_text_then_sends_carriage_return() {
        let runner = FakeRunner::with_outputs(vec![
            success(ls(42, 900)),
            success(String::new()),
            success(ls(42, 900)),
            success(String::new()),
        ]);
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);

        backend
            .send_text(&target, "cargo test", DeliveryMode::Submit)
            .unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].0, ["@", "ls"]);
        assert_eq!(
            calls[1].0,
            [
                "@",
                "send-text",
                "--match",
                "id:42",
                "--stdin",
                "--bracketed-paste=auto"
            ]
        );
        assert_eq!(calls[1].1.as_deref(), Some("cargo test"));
        assert_eq!(calls[2].0, ["@", "ls"]);
        assert_eq!(
            calls[3].0,
            [
                "@",
                "send-text",
                "--match",
                "id:42",
                "--stdin",
                "--bracketed-paste=disable"
            ]
        );
        assert_eq!(calls[3].1.as_deref(), Some("\r"));
    }

    #[test]
    fn submit_blocks_when_target_changes_between_writes() {
        let runner = FakeRunner::with_outputs(vec![
            success(ls(42, 900)),
            success(String::new()),
            success(ls(42, 901)),
        ]);
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);

        let error = backend
            .send_text(&target, "cargo test", DeliveryMode::Submit)
            .unwrap_err();

        assert!(error.to_string().contains("changed process"));
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2].0, ["@", "ls"]);
        assert_eq!(calls[1].1.as_deref(), Some("cargo test"));
    }

    #[test]
    fn insert_stays_bracketed_without_submit() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 900)), success(String::new())]);
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);

        backend
            .send_text(&target, "cargo test", DeliveryMode::Insert)
            .unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[1].0,
            [
                "@",
                "send-text",
                "--match",
                "id:42",
                "--stdin",
                "--bracketed-paste=auto"
            ]
        );
        assert_eq!(calls[1].1.as_deref(), Some("cargo test"));
    }

    #[test]
    fn changed_process_blocks_delivery() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 901))]);
        let backend = KittyBackend::with_runner(runner.clone());

        let error = backend
            .send_text(&binding(42, 900), "hello", DeliveryMode::Insert)
            .unwrap_err();

        assert!(error.to_string().contains("changed process"));
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn snapshot_screen_reads_validate_binding_before_command() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 901))]);
        let backend = Arc::new(KittyBackend::with_runner(runner.clone()));
        let service =
            TerminalSessionService::from_backends([backend.clone() as Arc<dyn TerminalBackend>])
                .unwrap();
        let snapshot = service.snapshot().unwrap();

        let error = backend
            .read_screen_from_snapshot(&snapshot, &binding(42, 900))
            .unwrap_err();

        assert!(matches!(error, TerminalError::TargetChanged { .. }));
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn snapshot_screen_reads_validate_capability_before_command() {
        let runner = FakeRunner::with_outputs(Vec::new());
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);
        let snapshot = TerminalSnapshot::new(vec![SessionFacts {
            id: target.session_id().clone(),
            root_pid: target.root_pid(),
            cwd: "/a".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: false,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::NONE,
        }]);

        let error = backend
            .read_screen_from_snapshot(&snapshot, &target)
            .unwrap_err();

        assert!(matches!(error, TerminalError::Unsupported { .. }));
        assert!(runner.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn snapshot_reuses_discovery_and_unchanged_screen_reads() {
        let runner =
            FakeRunner::with_outputs(vec![success(ls(42, 900)), success("screen".to_owned())]);
        let backend = KittyBackend::with_runner(runner.clone());
        let service =
            TerminalSessionService::from_backends([Arc::new(backend) as Arc<dyn TerminalBackend>])
                .unwrap();
        let snapshot = service.snapshot().unwrap();
        let target = binding(42, 900);

        assert_eq!(
            service.read_screen_from(&snapshot, &target).unwrap(),
            "screen"
        );
        assert_eq!(
            service.read_screen_from(&snapshot, &target).unwrap(),
            "screen"
        );

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, ["@", "ls"]);
        assert_eq!(
            calls[1].0,
            ["@", "get-text", "--match", "id:42", "--extent", "screen"]
        );
    }

    fn binding(id: u64, root_pid: i32) -> SessionBinding {
        SessionBinding::new(
            SessionId::new(backend_id().clone(), id.to_string()).unwrap(),
            root_pid,
        )
        .unwrap()
    }

    fn ls(id: u64, root_pid: i32) -> String {
        format!(
            r#"[{{"tabs":[{{"windows":[{{"id":{id},"title":"Terminal","cwd":"/a","pid":{root_pid}}}]}}]}}]"#
        )
    }

    fn success(stdout: String) -> CommandOutput {
        CommandOutput {
            success: true,
            code: Some(0),
            stdout,
            stderr: String::new(),
        }
    }

    #[test]
    fn wait_with_timeout_kills_a_hung_child() {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("sleep 60")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        let error = wait_with_timeout(&mut child, Duration::from_millis(300)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn wait_with_timeout_collects_large_output_without_deadlock() {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("head -c 200000 /dev/zero | tr '\\0' x")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = wait_with_timeout(&mut child, Duration::from_secs(10)).unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.len(), 200_000);
    }

    #[test]
    fn system_runner_times_out_while_stdin_is_unread() {
        let runner = SystemCommandRunner {
            program: "sh".to_owned(),
            timeout: Duration::from_millis(300),
        };
        let started = std::time::Instant::now();
        let error = runner
            .run(
                &["-c".to_owned(), "sleep 60".to_owned()],
                Some(&"x".repeat(100_000)),
            )
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
