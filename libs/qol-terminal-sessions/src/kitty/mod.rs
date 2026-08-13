mod endpoint;
mod parse;
mod socket;
mod spawn;

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use parse::{parse_ls, KittyLs};

#[cfg(test)]
use endpoint::LegacyEndpointSource;
use endpoint::{Endpoint, EndpointSource, SystemEndpointSource};

use crate::{
    BackendId, DeliveryMode, ScreenReader, SessionBinding, SessionCloser, SessionFacts,
    SessionFocus, SessionId, SessionInventory, SessionSpawner, SpawnIdentity, SpawnRequest,
    SpawnSurface, TerminalBackend, TerminalError, TerminalSnapshot, TextInput,
};

const BACKEND_ID: &str = "kitty";

pub fn backend_id() -> &'static BackendId {
    static ID: std::sync::OnceLock<BackendId> = std::sync::OnceLock::new();
    ID.get_or_init(|| BackendId::new(BACKEND_ID).expect("Kitty backend id is valid"))
}

pub struct KittyBackend {
    runner: Arc<dyn CommandRunner>,
    endpoints: Arc<dyn EndpointSource>,
}

impl KittyBackend {
    #[cfg(test)]
    fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            runner,
            endpoints: Arc::new(LegacyEndpointSource),
        }
    }

    #[cfg(test)]
    fn with_parts(runner: Arc<dyn CommandRunner>, endpoints: Arc<dyn EndpointSource>) -> Self {
        Self { runner, endpoints }
    }

    fn run_at(
        &self,
        endpoint: &Endpoint,
        operation: &'static str,
        args: &[String],
        stdin: Option<&str>,
    ) -> Result<String, TerminalError> {
        let output = self.runner.run(endpoint, args, stdin).map_err(|source| {
            TerminalError::BackendUnavailable {
                backend: backend_id().clone(),
                source,
            }
        })?;
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} instance={} operation={} success={} code={:?} stdout_len={}",
            backend_id(),
            endpoint.instance().unwrap_or("legacy"),
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

    fn sessions_at(&self, endpoint: &Endpoint) -> Result<Vec<SessionFacts>, TerminalError> {
        let body = self.run_at(endpoint, "discover sessions", &strings(["@", "ls"]), None)?;
        Ok(parse_ls(&body, backend_id())?.sessions_for(backend_id(), endpoint.instance()))
    }

    fn sessions(&self) -> Result<Vec<SessionFacts>, TerminalError> {
        let endpoints = self.endpoints.endpoints();
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} operation=discover_instances count={}",
            backend_id(),
            endpoints.len()
        );
        let mut sessions = Vec::new();
        let mut first_error = None;
        let mut reached = false;
        for endpoint in endpoints {
            match self.sessions_at(&endpoint) {
                Ok(mut discovered) => {
                    reached = true;
                    sessions.append(&mut discovered);
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        if !reached {
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        Ok(sessions)
    }

    fn route_target(&self, target: &SessionBinding) -> Result<(Endpoint, u64), TerminalError> {
        let native = target.session_id().native();
        if let Some((instance, window_id)) = split_native_id(native) {
            let endpoint = self
                .endpoints
                .endpoints()
                .into_iter()
                .find(|endpoint| endpoint.instance() == Some(instance))
                .ok_or_else(|| TerminalError::TargetMissing(target.clone()))?;
            qol_runtime::probe!(
                "TERMINAL_SESSIONS",
                "backend={} operation=route instance={} outcome=resolved",
                backend_id(),
                instance
            );
            return Ok((endpoint, window_id));
        }
        let window_id = native
            .parse()
            .map_err(|_| TerminalError::TargetMissing(target.clone()))?;
        let endpoint = self.endpoints.current();
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} operation=route instance={} outcome=legacy",
            backend_id(),
            endpoint.instance().unwrap_or("legacy")
        );
        Ok((endpoint, window_id))
    }

    fn ensure_capability(
        &self,
        target: &SessionBinding,
        capability: crate::SessionCapabilities,
        label: &'static str,
    ) -> Result<(), TerminalError> {
        let (endpoint, _) = self.route_target(target)?;
        let session = self
            .sessions_at(&endpoint)?
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
        let (endpoint, window_id) = self.route_target(target)?;
        self.run_at(
            &endpoint,
            "read screen",
            &strings([
                "@",
                "get-text",
                "--match",
                &matcher(window_id),
                "--extent",
                "screen",
            ]),
            None,
        )
    }
}

impl Default for KittyBackend {
    fn default() -> Self {
        Self {
            runner: Arc::new(SystemCommandRunner::default()),
            endpoints: Arc::new(SystemEndpointSource),
        }
    }
}

impl TerminalBackend for KittyBackend {
    fn id(&self) -> &BackendId {
        backend_id()
    }

    fn spawner(&self) -> Option<&dyn SessionSpawner> {
        Some(self)
    }

    fn closer(&self) -> Option<&dyn SessionCloser> {
        Some(self)
    }

    fn read_screen_from_snapshot(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError> {
        snapshot.validate_screen_target(target)?;
        self.read_screen_command(target)
    }

    fn current_session_id(&self) -> Option<SessionId> {
        current_session_id(
            std::env::var("KITTY_WINDOW_ID").ok().as_deref(),
            &self.endpoints.current(),
        )
    }
}

fn current_session_id(native: Option<&str>, endpoint: &Endpoint) -> Option<SessionId> {
    let window_id = native?.parse().ok()?;
    SessionId::new(backend_id().clone(), endpoint.native_id(window_id)).ok()
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

    fn read_screen_relaxed(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.read_screen_command(target)
    }
}

impl SessionFocus for KittyBackend {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        self.ensure_capability(target, crate::SessionCapabilities::FOCUS, "focus")?;
        let (endpoint, window_id) = self.route_target(target)?;
        self.run_at(
            &endpoint,
            "focus session",
            &strings(["@", "focus-window", "--match", &matcher(window_id)]),
            None,
        )
        .map(drop)
    }
}

impl SessionCloser for KittyBackend {
    fn close(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        self.ensure_capability(target, crate::SessionCapabilities::NONE, "close")?;
        let (endpoint, window_id) = self.route_target(target)?;
        self.run_at(
            &endpoint,
            "close session",
            &strings(["@", "close-window", "--match", &matcher(window_id)]),
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
        let (endpoint, window_id) = self.route_target(target)?;
        if mode == DeliveryMode::Submit {
            self.run_at(
                &endpoint,
                "insert text",
                &strings([
                    "@",
                    "send-text",
                    "--match",
                    &matcher(window_id),
                    "--stdin",
                    "--bracketed-paste=auto",
                ]),
                Some(text),
            )?;
            self.ensure_capability(target, crate::SessionCapabilities::TEXT_INPUT, "text input")?;
            let (endpoint, window_id) = self.route_target(target)?;
            return self
                .run_at(
                    &endpoint,
                    "submit text",
                    &strings([
                        "@",
                        "send-text",
                        "--match",
                        &matcher(window_id),
                        "--stdin",
                        "--bracketed-paste=disable",
                    ]),
                    Some("\r"),
                )
                .map(drop);
        }
        self.run_at(
            &endpoint,
            "insert text",
            &strings([
                "@",
                "send-text",
                "--match",
                &matcher(window_id),
                "--stdin",
                "--bracketed-paste=auto",
            ]),
            Some(text),
        )
        .map(drop)
    }

    fn send_key(&self, target: &SessionBinding, key: &str) -> Result<(), TerminalError> {
        self.ensure_capability(target, crate::SessionCapabilities::TEXT_INPUT, "text input")?;
        let (endpoint, window_id) = self.route_target(target)?;
        self.run_at(
            &endpoint,
            "send key",
            &strings(["@", "send-key", "--match", &matcher(window_id), key]),
            None,
        )
        .map(drop)
    }
}

impl SessionSpawner for KittyBackend {
    fn supports(&self, _surface: SpawnSurface) -> bool {
        true
    }

    fn spawn(&self, request: &SpawnRequest) -> Result<SessionId, TerminalError> {
        self.spawn_at(
            request,
            current_window_id(),
            std::env::var("PATH").ok().as_deref(),
        )
    }
}

impl KittyBackend {
    fn spawn_at(
        &self,
        request: &SpawnRequest,
        anchor_window_id: Option<u64>,
        path: Option<&str>,
    ) -> Result<SessionId, TerminalError> {
        let endpoint = self.endpoints.current();
        let argv = spawn::launch_argv(request, path, anchor_window_id)?;
        let stdout = self.run_at(&endpoint, "spawn session", &argv, None)?;
        let window_id =
            spawn::parse_spawned_window_id(&stdout).ok_or_else(|| TerminalError::SpawnFailed {
                backend: backend_id().clone(),
                message: format!("kitten returned an invalid window id `{}`", stdout.trim()),
            })?;
        self.verify_spawned_identity(&endpoint, window_id, &request.identity)?;
        let session_id = SessionId::new(backend_id().clone(), endpoint.native_id(window_id))
            .expect("Kitty endpoint and window ids are valid terminal session identities");
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "backend={} instance={} operation=spawn surface={} window_id={} outcome=ok",
            backend_id(),
            endpoint.instance().unwrap_or("legacy"),
            spawn::surface_tag(request.identity.surface),
            window_id
        );
        Ok(session_id)
    }

    fn verify_spawned_identity(
        &self,
        endpoint: &Endpoint,
        window_id: u64,
        expected: &SpawnIdentity,
    ) -> Result<(), TerminalError> {
        let target = SessionId::new(backend_id().clone(), endpoint.native_id(window_id))
            .expect("Kitty endpoint and window ids are valid terminal session identities");
        let sessions = self.sessions_at(endpoint)?;
        let live = sessions
            .into_iter()
            .find(|session| session.id == target)
            .ok_or_else(|| TerminalError::SpawnFailed {
                backend: backend_id().clone(),
                message: format!("spawned window {window_id} is missing from discovery"),
            })?;
        if live.spawn_identity.as_ref() != Some(expected) {
            return Err(TerminalError::SpawnFailed {
                backend: backend_id().clone(),
                message: format!(
                    "spawned window {window_id} carries identity {:?} instead of {expected:?}",
                    live.spawn_identity.as_ref()
                ),
            });
        }
        Ok(())
    }
}

fn current_window_id() -> Option<u64> {
    std::env::var("KITTY_WINDOW_ID")
        .ok()
        .and_then(|value| value.trim().parse().ok())
}

fn split_native_id(native: &str) -> Option<(&str, u64)> {
    let (instance, window_id) = native.rsplit_once('.')?;
    let window_id = window_id.parse().ok()?;
    (!instance.is_empty()).then_some((instance, window_id))
}

fn matcher(window_id: u64) -> String {
    format!("id:{window_id}")
}

fn strings<const N: usize>(values: [&str; N]) -> Vec<String> {
    values.into_iter().map(str::to_owned).collect()
}

trait CommandRunner: Send + Sync {
    fn run(
        &self,
        endpoint: &Endpoint,
        args: &[String],
        stdin: Option<&str>,
    ) -> std::io::Result<CommandOutput>;
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
    fn run(
        &self,
        endpoint: &Endpoint,
        args: &[String],
        stdin: Option<&str>,
    ) -> std::io::Result<CommandOutput> {
        if let Some(path) = endpoint.socket_path() {
            if let Some(output) = socket::try_run(path, args, stdin) {
                return Ok(output);
            }
        }
        self.spawn_kitten(endpoint, args, stdin)
    }
}

impl SystemCommandRunner {
    fn spawn_kitten(
        &self,
        endpoint: &Endpoint,
        args: &[String],
        stdin: Option<&str>,
    ) -> std::io::Result<CommandOutput> {
        let mut command = Command::new(&self.program);
        command.args(routed_args(endpoint, args));
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

fn routed_args(endpoint: &Endpoint, args: &[String]) -> Vec<String> {
    let Some(listen_on) = endpoint.listen_on() else {
        return args.to_vec();
    };
    let Some((marker, rest)) = args.split_first() else {
        return Vec::new();
    };
    if marker != "@" {
        return args.to_vec();
    }
    let mut routed = Vec::with_capacity(args.len() + 2);
    routed.extend(strings(["@", "--to", listen_on]));
    routed.extend_from_slice(rest);
    routed
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

    use super::endpoint::{Endpoint, EndpointSource};
    use super::{
        current_session_id, routed_args, strings, wait_with_timeout, CommandOutput, CommandRunner,
        KittyBackend, SystemCommandRunner,
    };
    use crate::cli::{CliLaunchProgram, CliToolId};
    use crate::{
        kitty::backend_id, DeliveryMode, ScreenReader, SessionBinding, SessionCapabilities,
        SessionCloser, SessionFacts, SessionFocus, SessionId, SessionInventory, SpawnIdentity,
        SpawnKey, SpawnRequest, SpawnSurface, TerminalBackend, TerminalError,
        TerminalSessionService, TerminalSnapshot, TextInput,
    };

    type RecordedCall = (Option<String>, Vec<String>, Option<String>);

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<RecordedCall>>,
        outputs: Mutex<VecDeque<std::io::Result<CommandOutput>>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: Vec<CommandOutput>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                outputs: Mutex::new(outputs.into_iter().map(Ok).collect()),
            })
        }

        fn with_results(outputs: Vec<std::io::Result<CommandOutput>>) -> Arc<Self> {
            Arc::new(Self {
                calls: Mutex::new(Vec::new()),
                outputs: Mutex::new(outputs.into()),
            })
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            endpoint: &Endpoint,
            args: &[String],
            stdin: Option<&str>,
        ) -> std::io::Result<CommandOutput> {
            self.calls.lock().unwrap().push((
                endpoint.instance().map(str::to_owned),
                args.to_vec(),
                stdin.map(str::to_owned),
            ));
            self.outputs.lock().unwrap().pop_front().unwrap()
        }
    }

    struct FixedEndpointSource {
        endpoints: Vec<Endpoint>,
        current: Endpoint,
    }

    impl FixedEndpointSource {
        fn new(endpoints: Vec<Endpoint>) -> Arc<Self> {
            Arc::new(Self {
                current: endpoints[0].clone(),
                endpoints,
            })
        }
    }

    impl EndpointSource for FixedEndpointSource {
        fn endpoints(&self) -> Vec<Endpoint> {
            self.endpoints.clone()
        }

        fn current(&self) -> Endpoint {
            self.current.clone()
        }
    }

    #[test]
    fn current_session_identity_uses_the_backend_contract() {
        let endpoint = Endpoint::legacy();
        assert_eq!(
            current_session_id(Some("42"), &endpoint).unwrap(),
            SessionId::new(backend_id().clone(), "42").unwrap()
        );
        assert!(current_session_id(None, &endpoint).is_none());
        assert!(current_session_id(Some("bad:id"), &endpoint).is_none());
    }

    #[test]
    fn current_session_identity_includes_the_terminal_instance() {
        let endpoint = Endpoint::fixture("k1_2", "unix:/tmp/kitty-1");

        assert_eq!(
            current_session_id(Some("42"), &endpoint).unwrap(),
            SessionId::new(backend_id().clone(), "k1_2.42").unwrap()
        );
    }

    #[test]
    fn discovery_keeps_equal_window_ids_from_distinct_instances() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 900)), success(ls(42, 901))]);
        let endpoints = FixedEndpointSource::new(vec![
            Endpoint::fixture("k1_1", "unix:/tmp/kitty-1"),
            Endpoint::fixture("k1_2", "unix:/tmp/kitty-2"),
        ]);
        let backend = KittyBackend::with_parts(runner.clone(), endpoints);

        let sessions = backend.discover().unwrap();

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id.native(), "k1_1.42");
        assert_eq!(sessions[1].id.native(), "k1_2.42");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls[0].0.as_deref(), Some("k1_1"));
        assert_eq!(calls[1].0.as_deref(), Some("k1_2"));
    }

    #[test]
    fn unreachable_sibling_does_not_hide_reachable_sessions() {
        let runner = FakeRunner::with_results(vec![
            Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                "stale",
            )),
            Ok(success(ls(42, 901))),
        ]);
        let endpoints = FixedEndpointSource::new(vec![
            Endpoint::fixture("k1_1", "unix:/tmp/kitty-1"),
            Endpoint::fixture("k1_2", "unix:/tmp/kitty-2"),
        ]);
        let backend = KittyBackend::with_parts(runner, endpoints);

        let sessions = backend.discover().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.native(), "k1_2.42");
    }

    #[test]
    fn target_operations_route_to_the_encoded_instance() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 901)), success(String::new())]);
        let endpoints = FixedEndpointSource::new(vec![
            Endpoint::fixture("k1_1", "unix:/tmp/kitty-1"),
            Endpoint::fixture("k1_2", "unix:/tmp/kitty-2"),
        ]);
        let backend = KittyBackend::with_parts(runner.clone(), endpoints);
        let target = SessionBinding::new(
            SessionId::new(backend_id().clone(), "k1_2.42").unwrap(),
            901,
        )
        .unwrap();

        backend.focus(&target).unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls.iter().all(|call| call.0.as_deref() == Some("k1_2")));
        assert_eq!(calls[1].1, ["@", "focus-window", "--match", "id:42"]);
    }

    #[test]
    fn close_validates_the_target_then_closes_the_window() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 900)), success(String::new())]);
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);

        backend.close(&target).unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].1, ["@", "ls"]);
        assert_eq!(calls[1].1, ["@", "close-window", "--match", "id:42"]);

        let runner = FakeRunner::with_outputs(vec![success(ls(42, 901))]);
        let backend = KittyBackend::with_runner(runner.clone());
        let error = backend.close(&binding(42, 900)).unwrap_err();
        assert!(matches!(error, TerminalError::TargetChanged { .. }));
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn explicit_endpoints_add_kitten_to_routing_without_changing_the_command() {
        let endpoint = Endpoint::fixture("k1_2", "unix:/tmp/kitty-2");

        assert_eq!(
            routed_args(
                &endpoint,
                &strings(["@", "focus-window", "--match", "id:42"])
            ),
            [
                "@",
                "--to",
                "unix:/tmp/kitty-2",
                "focus-window",
                "--match",
                "id:42"
            ]
        );
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
        assert_eq!(calls[0].1, ["@", "ls"]);
        assert_eq!(
            calls[1].1,
            [
                "@",
                "send-text",
                "--match",
                "id:42",
                "--stdin",
                "--bracketed-paste=auto"
            ]
        );
        assert_eq!(calls[1].2.as_deref(), Some("cargo test"));
        assert_eq!(calls[2].1, ["@", "ls"]);
        assert_eq!(
            calls[3].1,
            [
                "@",
                "send-text",
                "--match",
                "id:42",
                "--stdin",
                "--bracketed-paste=disable"
            ]
        );
        assert_eq!(calls[3].2.as_deref(), Some("\r"));
    }

    #[test]
    fn send_key_delivers_a_key_event_to_the_window() {
        let runner = FakeRunner::with_outputs(vec![success(ls(42, 900)), success(String::new())]);
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);

        backend.send_key(&target, "ctrl+c").unwrap();

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].1, ["@", "send-key", "--match", "id:42", "ctrl+c"]);
        assert_eq!(calls[1].2, None);
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
        assert_eq!(calls[2].1, ["@", "ls"]);
        assert_eq!(calls[1].2.as_deref(), Some("cargo test"));
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
            calls[1].1,
            [
                "@",
                "send-text",
                "--match",
                "id:42",
                "--stdin",
                "--bracketed-paste=auto"
            ]
        );
        assert_eq!(calls[1].2.as_deref(), Some("cargo test"));
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
            spawn_identity: None,
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
        assert_eq!(calls[0].1, ["@", "ls"]);
        assert_eq!(
            calls[1].1,
            ["@", "get-text", "--match", "id:42", "--extent", "screen"]
        );
    }

    #[test]
    fn relaxed_screen_reads_skip_discovery_and_the_capability_recheck() {
        let runner = FakeRunner::with_outputs(vec![success("screen".to_owned())]);
        let backend = KittyBackend::with_runner(runner.clone());
        let target = binding(42, 900);

        assert_eq!(backend.read_screen_relaxed(&target).unwrap(), "screen");

        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].1,
            ["@", "get-text", "--match", "id:42", "--extent", "screen"]
        );
    }

    #[test]
    fn wait_cycle_emits_only_window_matching_get_text_commands() {
        let mut outputs = Vec::new();
        for _ in 0..9 {
            outputs.push(success("idle".to_owned()));
        }
        outputs.push(success(ls(42, 900)));
        for _ in 0..10 {
            outputs.push(success("idle".to_owned()));
        }
        outputs.push(success(ls(42, 900)));
        for _ in 0..6 {
            outputs.push(success("idle".to_owned()));
        }
        outputs.push(success("done\nQOL_BRIDGE_DONE_wait".to_owned()));
        outputs.push(success("done\nQOL_BRIDGE_DONE_wait".to_owned()));
        let runner = FakeRunner::with_outputs(outputs);
        let backend = Arc::new(KittyBackend::with_runner(runner.clone()));
        let terminals =
            TerminalSessionService::from_backends(vec![backend as Arc<dyn TerminalBackend>])
                .unwrap();
        let (rx, _stop) = ticker();

        let outcome = terminals
            .wait_for_completion(
                &binding(42, 900),
                "QOL_BRIDGE_DONE_wait",
                Duration::from_secs(60),
                rx,
                true,
                true,
                &|| None,
                Duration::from_secs(3600),
            )
            .unwrap();

        assert!(outcome.completed);
        assert_eq!(outcome.reads, 27);
        let calls = runner.calls.lock().unwrap();
        let get_text: Vec<&Vec<String>> = calls
            .iter()
            .map(|call| &call.1)
            .filter(|args| args.first().map(String::as_str) == Some("@"))
            .filter(|args| args.get(1).map(String::as_str) == Some("get-text"))
            .collect();
        assert_eq!(get_text.len(), 27);
        for args in get_text {
            assert_eq!(
                args,
                &["@", "get-text", "--match", "id:42", "--extent", "screen"]
            );
            assert_eq!(
                args.iter().filter(|arg| arg.as_str() == "--match").count(),
                1
            );
        }
        assert_eq!(calls.iter().filter(|call| call.1 == ["@", "ls"]).count(), 2);
    }

    struct StopSignal(Arc<std::sync::atomic::AtomicBool>);

    impl Drop for StopSignal {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn ticker() -> (std::sync::mpsc::Receiver<()>, StopSignal) {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tick_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !tick_stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = tx.try_send(());
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        (rx, StopSignal(stop))
    }

    fn binding(id: u64, root_pid: i32) -> SessionBinding {
        SessionBinding::new(
            SessionId::new(backend_id().clone(), id.to_string()).unwrap(),
            root_pid,
        )
        .unwrap()
    }

    fn spawn_request(surface: SpawnSurface) -> SpawnRequest {
        SpawnRequest {
            identity: SpawnIdentity {
                key: SpawnKey::new("voice-42").unwrap(),
                tool: CliToolId::new("codex").unwrap(),
                surface,
            },
            launch: CliLaunchProgram {
                program: "codex".to_owned(),
                args: vec!["--full-auto".to_owned()],
            },
            cwd: "/work/project".into(),
            title: None,
        }
    }

    fn ls_with_identity(id: u64, key: &str, tool: &str, surface: &str) -> String {
        format!(
            r#"[{{"id":1,"tabs":[{{"windows":[{{"id":{id},"title":"Spawned","cwd":"/work/project","pid":900,"user_vars":{{"qol_session_key":"{key}","qol_session_tool":"{tool}","qol_session_surface":"{surface}"}}}}]}}]}}]"#
        )
    }

    fn ls_without_identity(id: u64) -> String {
        format!(
            r#"[{{"id":1,"tabs":[{{"windows":[{{"id":{id},"title":"Spawned","cwd":"/work/project","pid":900}}]}}]}}]"#
        )
    }

    #[test]
    fn spawner_capability_supports_both_surfaces() {
        let backend = KittyBackend::with_runner(FakeRunner::with_outputs(Vec::new()));

        let spawner = backend.spawner().expect("kitty exposes a spawner");

        assert!(spawner.supports(SpawnSurface::Tab));
        assert!(spawner.supports(SpawnSurface::OsWindow));
    }

    #[test]
    fn spawn_launches_an_anchored_tab_into_the_current_endpoint() {
        let runner = FakeRunner::with_outputs(vec![
            success("77\n".to_owned()),
            success(ls_with_identity(77, "voice-42", "codex", "tab")),
        ]);
        let endpoints =
            FixedEndpointSource::new(vec![Endpoint::fixture("k1_2", "unix:/tmp/kitty-2")]);
        let backend = KittyBackend::with_parts(runner.clone(), endpoints);

        let session = backend
            .spawn_at(
                &spawn_request(SpawnSurface::Tab),
                Some(42),
                Some("/usr/bin:/bin"),
            )
            .unwrap();

        assert_eq!(session.backend().to_string(), "kitty");
        assert_eq!(session.native(), "k1_2.77");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0.as_deref(), Some("k1_2"));
        assert_eq!(
            calls[0].1,
            [
                "@",
                "launch",
                "--type",
                "tab",
                "--next-to",
                "id:42",
                "--dont-take-focus",
                "--env",
                "PATH=/usr/bin:/bin",
                "--cwd",
                "/work/project",
                "--var",
                "qol_session_key=voice-42",
                "--var",
                "qol_session_tool=codex",
                "--var",
                "qol_session_surface=tab",
                "--",
                "codex",
                "--full-auto",
            ]
        );
        assert_eq!(calls[1].1, ["@", "ls"]);
    }

    #[test]
    fn spawn_launches_an_os_window_without_an_anchor_or_path() {
        let runner = FakeRunner::with_outputs(vec![
            success("77\n".to_owned()),
            success(ls_with_identity(77, "voice-42", "codex", "os_window")),
        ]);
        let backend = KittyBackend::with_runner(runner.clone());

        let session = backend
            .spawn_at(&spawn_request(SpawnSurface::OsWindow), None, None)
            .unwrap();

        assert_eq!(session.native(), "77");
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, None);
        assert_eq!(
            calls[0].1,
            [
                "@",
                "launch",
                "--type",
                "os-window",
                "--dont-take-focus",
                "--cwd",
                "/work/project",
                "--var",
                "qol_session_key=voice-42",
                "--var",
                "qol_session_tool=codex",
                "--var",
                "qol_session_surface=os_window",
                "--",
                "codex",
                "--full-auto",
            ]
        );
    }

    #[test]
    fn spawn_fails_closed_on_unparseable_window_ids() {
        for stdout in [
            "",
            "   ",
            "garbage",
            "77 88",
            "-1",
            "0",
            "0\n",
            "77\n88",
            "99999999999999999999999999",
        ] {
            let runner = FakeRunner::with_outputs(vec![success(stdout.to_owned())]);
            let backend = KittyBackend::with_runner(runner.clone());

            let error = backend
                .spawn_at(&spawn_request(SpawnSurface::OsWindow), None, None)
                .unwrap_err();

            assert!(
                matches!(error, TerminalError::SpawnFailed { .. }),
                "stdout: {stdout:?}"
            );
            assert_eq!(runner.calls.lock().unwrap().len(), 1, "stdout: {stdout:?}");
        }
    }

    #[test]
    fn spawn_fails_when_the_spawned_window_is_missing_from_discovery() {
        let runner = FakeRunner::with_outputs(vec![
            success("77\n".to_owned()),
            success(ls_with_identity(78, "voice-42", "codex", "os_window")),
        ]);
        let backend = KittyBackend::with_runner(runner.clone());

        let error = backend
            .spawn_at(&spawn_request(SpawnSurface::OsWindow), None, None)
            .unwrap_err();

        assert!(error.to_string().contains("missing from discovery"));
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn spawn_fails_when_the_spawned_window_carries_a_different_identity() {
        let bodies = [
            ls_with_identity(77, "other-key", "codex", "tab"),
            ls_without_identity(77),
        ];
        for body in bodies {
            let runner = FakeRunner::with_outputs(vec![success("77\n".to_owned()), success(body)]);
            let backend = KittyBackend::with_runner(runner.clone());

            let error = backend
                .spawn_at(&spawn_request(SpawnSurface::OsWindow), None, None)
                .unwrap_err();

            assert!(matches!(error, TerminalError::SpawnFailed { .. }));
            assert!(error.to_string().contains("instead of"));
            assert_eq!(runner.calls.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn spawn_fails_closed_when_a_tab_has_no_current_window() {
        let runner = FakeRunner::with_outputs(Vec::new());
        let backend = KittyBackend::with_runner(runner.clone());

        let error = backend
            .spawn_at(&spawn_request(SpawnSurface::Tab), None, None)
            .unwrap_err();

        assert!(error.to_string().contains("current window"));
        assert!(runner.calls.lock().unwrap().is_empty());
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
            .arg("exec sleep 60")
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
                &Endpoint::legacy(),
                &["-c".to_owned(), "exec sleep 60".to_owned()],
                Some("x".repeat(100_000).as_str()),
            )
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
