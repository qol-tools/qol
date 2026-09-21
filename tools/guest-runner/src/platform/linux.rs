use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_dev_guest::{
    read_frame, write_frame, CommandSpec, GuestHello, GuestMessage, GuestRequest, GuestResponse,
    GuestSession, ImageIdentity, ProcessOutcome, ProcessState, RequestAction, ResponseResult,
    PROTOCOL_VERSION,
};
use qol_headless::DoctorCheckResult;

use super::GuestRunnerPlatform;
use crate::cli::RunOptions;

const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_OUTPUT_BYTES: u64 = 1024 * 1024;
const RECONNECT_DELAY: Duration = Duration::from_millis(250);

pub(super) struct Platform;

impl GuestRunnerPlatform for Platform {
    fn run(&self, options: RunOptions) -> Result<()> {
        let image = read_identity(&options.identity_path)?;
        let run_id = read_run_id(&options.run_id_path)?;
        let session = current_session();
        let hello = GuestHello {
            protocol_version: PROTOCOL_VERSION,
            run_id,
            image,
            session,
            runner_pid: std::process::id(),
        };
        hello
            .validate_for(&hello.image.environment_id)
            .context("guest runner must start inside the prepared graphical session")?;

        loop {
            match serve_device(&options.device_path, &hello) {
                Ok(()) => {}
                Err(error) => eprintln!("qol-guest-runner: {error:#}"),
            }
            thread::sleep(RECONNECT_DELAY);
        }
    }

    fn platform_check(&self) -> DoctorCheckResult {
        DoctorCheckResult::ok(
            "platform_supported",
            "Linux guest-control runtime is supported",
        )
    }

    fn runtime_paths_check(&self) -> DoctorCheckResult {
        let options = RunOptions::default();
        let missing = [
            ("device", options.device_path),
            ("identity", options.identity_path),
            ("run id", options.run_id_path),
        ]
        .into_iter()
        .filter_map(|(label, path)| {
            (!path.exists()).then_some(format!("{label}: {}", path.display()))
        })
        .collect::<Vec<_>>();
        if missing.is_empty() {
            return DoctorCheckResult::ok(
                "runtime_paths",
                "guest-control device and identity files are available",
            );
        }
        DoctorCheckResult::warn(
            "runtime_paths",
            format!(
                "prepared guest runtime paths are missing: {}",
                missing.join(", ")
            ),
        )
        .with_fix("run qol-guest-runner inside a prepared qol development guest")
    }
}

fn read_identity(path: &Path) -> Result<ImageIdentity> {
    let content = fs::read(path)
        .with_context(|| format!("failed to read guest identity {}", path.display()))?;
    let identity: ImageIdentity = serde_json::from_slice(&content)
        .with_context(|| format!("invalid guest identity {}", path.display()))?;
    identity.validate()?;
    Ok(identity)
}

fn read_run_id(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read guest run identity {}", path.display()))?;
    let run_id = String::from_utf8_lossy(&bytes)
        .trim_matches(|character: char| character == '\0' || character.is_ascii_whitespace())
        .to_string();
    let valid = !run_id.is_empty()
        && run_id.len() <= 64
        && run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if !valid {
        bail!("guest run identity is not a safe nonempty run id");
    }
    Ok(run_id)
}

fn current_session() -> GuestSession {
    GuestSession {
        user: env::var("USER").unwrap_or_default(),
        desktop: env::var("XDG_CURRENT_DESKTOP")
            .ok()
            .and_then(|value| normalize_desktop(&value)),
        session_type: env::var("XDG_SESSION_TYPE")
            .ok()
            .map(|value| value.to_ascii_lowercase()),
        display: env::var("DISPLAY").ok().filter(|value| !value.is_empty()),
        runtime_dir: env::var("XDG_RUNTIME_DIR")
            .ok()
            .filter(|value| !value.is_empty()),
        dbus_session: env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some(),
    }
}

fn normalize_desktop(raw: &str) -> Option<String> {
    raw.split(':')
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(|value| {
            value
                .strip_prefix("X-")
                .unwrap_or(value)
                .to_ascii_lowercase()
        })
}

fn serve_device(path: &Path, hello: &GuestHello) -> Result<()> {
    let device = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open guest-control device {}", path.display()))?;
    let writer = device
        .try_clone()
        .context("failed to clone guest-control device")?;
    serve_connection(BufReader::new(device), writer, hello)
}

fn serve_connection(
    mut reader: impl std::io::BufRead,
    mut writer: impl std::io::Write,
    hello: &GuestHello,
) -> Result<()> {
    write_frame(
        &mut writer,
        &GuestMessage::Hello {
            hello: Box::new(hello.clone()),
        },
    )?;
    let mut processes = ProcessManager::default();
    loop {
        let request: GuestRequest = read_frame(&mut reader)?;
        let result =
            processes
                .handle(request.action)
                .unwrap_or_else(|error| ResponseResult::Error {
                    message: format!("{error:#}"),
                });
        write_frame(
            &mut writer,
            &GuestMessage::Response {
                response: GuestResponse {
                    request_id: request.request_id,
                    result,
                },
            },
        )?;
    }
}

struct ProcessManager {
    next_id: u64,
    output_root: PathBuf,
    processes: BTreeMap<u64, ManagedProcess>,
}

static NEXT_MANAGER_ID: AtomicU64 = AtomicU64::new(1);

impl Default for ProcessManager {
    fn default() -> Self {
        let manager_id = NEXT_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        Self {
            next_id: 0,
            output_root: PathBuf::from("/tmp/qol-guest-runner")
                .join(std::process::id().to_string())
                .join(manager_id.to_string()),
            processes: BTreeMap::new(),
        }
    }
}

struct ManagedProcess {
    child: Child,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
}

impl ProcessManager {
    fn handle(&mut self, action: RequestAction) -> Result<ResponseResult> {
        match action {
            RequestAction::Ping => Ok(ResponseResult::Pong),
            RequestAction::Exec {
                command,
                timeout_ms,
            } => {
                let process_id = self.spawn(command)?.0;
                let timeout = command_timeout(timeout_ms)?;
                let outcome = self.wait(process_id, timeout, true)?;
                Ok(ResponseResult::Process { outcome })
            }
            RequestAction::Spawn { command } => {
                let (process_id, guest_pid) = self.spawn(command)?;
                Ok(ResponseResult::Spawned {
                    process_id,
                    guest_pid,
                })
            }
            RequestAction::Wait {
                process_id,
                timeout_ms,
            } => {
                let timeout = command_timeout(timeout_ms)?;
                let outcome = self.wait(process_id, timeout, false)?;
                Ok(ResponseResult::Process { outcome })
            }
            RequestAction::Terminate { process_id } => {
                let outcome = self.terminate(process_id)?;
                Ok(ResponseResult::Process { outcome })
            }
        }
    }

    fn spawn(&mut self, command: CommandSpec) -> Result<(u64, u32)> {
        command.validate()?;
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("guest process id overflow")?;
        let process_id = self.next_id;
        let output_dir = self.output_root.join(process_id.to_string());
        fs::create_dir_all(&output_dir)
            .with_context(|| format!("failed to create {}", output_dir.display()))?;
        let stdout_path = output_dir.join("stdout");
        let stderr_path = output_dir.join("stderr");
        let stdout = File::create(&stdout_path)
            .with_context(|| format!("failed to create {}", stdout_path.display()))?;
        let stderr = File::create(&stderr_path)
            .with_context(|| format!("failed to create {}", stderr_path.display()))?;
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .envs(&command.env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        if let Some(cwd) = &command.cwd {
            process.current_dir(cwd);
        }
        let child = process
            .spawn()
            .with_context(|| format!("failed to spawn guest command {}", command.program))?;
        let guest_pid = child.id();
        self.processes.insert(
            process_id,
            ManagedProcess {
                child,
                stdout_path,
                stderr_path,
            },
        );
        Ok((process_id, guest_pid))
    }

    fn wait(
        &mut self,
        process_id: u64,
        timeout: Duration,
        terminate_on_timeout: bool,
    ) -> Result<ProcessOutcome> {
        let deadline = Instant::now() + timeout;
        loop {
            let process = self
                .processes
                .get_mut(&process_id)
                .with_context(|| format!("unknown guest process {process_id}"))?;
            if let Some(status) = process
                .child
                .try_wait()
                .with_context(|| format!("failed to poll guest process {process_id}"))?
            {
                return self.finish(process_id, ProcessState::Exited, status);
            }
            if Instant::now() >= deadline {
                if !terminate_on_timeout {
                    return Ok(ProcessOutcome {
                        state: ProcessState::Running,
                        exit_code: None,
                        stdout: String::new(),
                        stderr: String::new(),
                    });
                }
                process
                    .child
                    .kill()
                    .with_context(|| format!("failed to terminate guest process {process_id}"))?;
                let status = process
                    .child
                    .wait()
                    .with_context(|| format!("failed to reap guest process {process_id}"))?;
                return self.finish(process_id, ProcessState::TimedOut, status);
            }
            thread::sleep(PROCESS_POLL_INTERVAL);
        }
    }

    fn terminate(&mut self, process_id: u64) -> Result<ProcessOutcome> {
        let process = self
            .processes
            .get_mut(&process_id)
            .with_context(|| format!("unknown guest process {process_id}"))?;
        let status = match process.child.try_wait()? {
            Some(status) => status,
            None => {
                process.child.kill()?;
                process.child.wait()?
            }
        };
        self.finish(process_id, ProcessState::Terminated, status)
    }

    fn finish(
        &mut self,
        process_id: u64,
        state: ProcessState,
        status: ExitStatus,
    ) -> Result<ProcessOutcome> {
        let process = self
            .processes
            .remove(&process_id)
            .with_context(|| format!("unknown guest process {process_id}"))?;
        let stdout = read_bounded(&process.stdout_path)?;
        let stderr = read_bounded(&process.stderr_path)?;
        if let Some(output_dir) = process.stdout_path.parent() {
            let _ = fs::remove_dir_all(output_dir);
        }
        Ok(ProcessOutcome {
            state,
            exit_code: status.code(),
            stdout,
            stderr,
        })
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        for process in self.processes.values_mut() {
            let _ = process.child.kill();
            let _ = process.child.wait();
            if let Some(output_dir) = process.stdout_path.parent() {
                let _ = fs::remove_dir_all(output_dir);
            }
        }
    }
}

fn command_timeout(timeout_ms: u64) -> Result<Duration> {
    let timeout = Duration::from_millis(timeout_ms);
    if timeout > MAX_COMMAND_TIMEOUT {
        bail!(
            "guest command timeout exceeds {} seconds",
            MAX_COMMAND_TIMEOUT.as_secs()
        );
    }
    Ok(timeout)
}

fn read_bounded(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("failed to read {}", path.display()))?
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.len() as u64 > MAX_OUTPUT_BYTES {
        bytes.truncate(MAX_OUTPUT_BYTES as usize);
        bytes.extend_from_slice(b"\n[output truncated by qol-guest-runner]\n");
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(program: &str, args: &[&str]) -> CommandSpec {
        CommandSpec {
            program: program.to_string(),
            args: args.iter().map(|value| (*value).to_string()).collect(),
            cwd: None,
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn normalizes_cinnamon_session_names() {
        assert_eq!(
            normalize_desktop("X-Cinnamon"),
            Some("cinnamon".to_string())
        );
        assert_eq!(
            normalize_desktop("Cinnamon:GNOME"),
            Some("cinnamon".to_string())
        );
        assert_eq!(normalize_desktop(""), None);
    }

    #[test]
    fn exec_returns_bounded_typed_output() {
        let mut manager = ProcessManager::default();
        let result = manager
            .handle(RequestAction::Exec {
                command: command("/bin/sh", &["-c", "printf hello; printf err >&2; exit 7"]),
                timeout_ms: 1_000,
            })
            .unwrap();
        let ResponseResult::Process { outcome } = result else {
            panic!("expected process outcome");
        };
        assert_eq!(outcome.state, ProcessState::Exited);
        assert_eq!(outcome.exit_code, Some(7));
        assert_eq!(outcome.stdout, "hello");
        assert_eq!(outcome.stderr, "err");
    }

    #[test]
    fn spawned_process_can_be_polled_then_terminated() {
        let mut manager = ProcessManager::default();
        let ResponseResult::Spawned { process_id, .. } = manager
            .handle(RequestAction::Spawn {
                command: command("/bin/sh", &["-c", "sleep 30"]),
            })
            .unwrap()
        else {
            panic!("expected spawned process");
        };
        let ResponseResult::Process { outcome } = manager
            .handle(RequestAction::Wait {
                process_id,
                timeout_ms: 0,
            })
            .unwrap()
        else {
            panic!("expected running process");
        };
        assert_eq!(outcome.state, ProcessState::Running);
        let ResponseResult::Process { outcome } = manager
            .handle(RequestAction::Terminate { process_id })
            .unwrap()
        else {
            panic!("expected terminated process");
        };
        assert_eq!(outcome.state, ProcessState::Terminated);
    }

    #[test]
    fn reads_and_validates_image_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("identity.json");
        fs::write(
            &path,
            r#"{"schema":1,"environment_id":"linux/mint-cinnamon","revision":"fixture","desktop":"cinnamon","display_protocol":"x11","user":"qol"}"#,
        )
        .unwrap();
        assert_eq!(
            read_identity(&path).unwrap().environment_id,
            "linux/mint-cinnamon"
        );
    }

    #[test]
    fn reads_fw_cfg_run_identity_and_trims_transport_padding() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("run-id");
        fs::write(&path, b"mint-lane-1\0\n").unwrap();
        assert_eq!(read_run_id(&path).unwrap(), "mint-lane-1");
        fs::write(&path, b"../host").unwrap();
        assert!(read_run_id(&path).is_err());
    }
}
