use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const VIRTIO_PORT_NAME: &str = "org.qol-tools.guest-control";
pub const DEFAULT_DEVICE_PATH: &str = "/dev/virtio-ports/org.qol-tools.guest-control";
pub const DEFAULT_IDENTITY_PATH: &str = "/etc/qol-dev-image.json";
pub const DEFAULT_RUN_ID_PATH: &str = "/sys/firmware/qemu_fw_cfg/by_name/opt/qol/run-id/raw";
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageIdentity {
    pub schema: u32,
    pub environment_id: String,
    pub revision: String,
    pub desktop: String,
    pub display_protocol: String,
    pub user: String,
}

impl ImageIdentity {
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 {
            bail!("unsupported guest image identity schema {}", self.schema);
        }
        for (field, value) in [
            ("environment_id", self.environment_id.as_str()),
            ("revision", self.revision.as_str()),
            ("desktop", self.desktop.as_str()),
            ("display_protocol", self.display_protocol.as_str()),
            ("user", self.user.as_str()),
        ] {
            if value.trim().is_empty() {
                bail!("guest image identity field `{field}` must not be empty");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuestSession {
    pub user: String,
    pub desktop: Option<String>,
    pub session_type: Option<String>,
    pub display: Option<String>,
    pub runtime_dir: Option<String>,
    pub dbus_session: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuestHello {
    pub protocol_version: u32,
    pub run_id: String,
    pub image: ImageIdentity,
    pub session: GuestSession,
    pub runner_pid: u32,
}

impl GuestHello {
    pub fn validate_for(&self, expected_environment_id: &str) -> Result<()> {
        if self.protocol_version != PROTOCOL_VERSION {
            bail!(
                "guest protocol mismatch: host {}, guest {}",
                PROTOCOL_VERSION,
                self.protocol_version
            );
        }
        self.image.validate()?;
        if self.run_id.is_empty()
            || self.run_id.len() > 64
            || !self
                .run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            bail!("guest run identity is not a safe nonempty run id");
        }
        if self.image.environment_id != expected_environment_id {
            bail!(
                "guest image identity mismatch: expected `{expected_environment_id}`, got `{}`",
                self.image.environment_id
            );
        }
        if self.session.user != self.image.user {
            bail!(
                "guest session user mismatch: image expects `{}`, runner is `{}`",
                self.image.user,
                self.session.user
            );
        }
        if self.session.desktop.as_deref() != Some(self.image.desktop.as_str()) {
            bail!(
                "guest desktop mismatch: image expects `{}`, session reported `{}`",
                self.image.desktop,
                self.session.desktop.as_deref().unwrap_or("unknown")
            );
        }
        if self.session.session_type.as_deref() != Some(self.image.display_protocol.as_str()) {
            bail!(
                "guest display protocol mismatch: image expects `{}`, session reported `{}`",
                self.image.display_protocol,
                self.session.session_type.as_deref().unwrap_or("unknown")
            );
        }
        if self.session.display.is_none() || !self.session.dbus_session {
            bail!("guest runner is not attached to a graphical desktop session");
        }
        Ok(())
    }

    pub fn validate_identity(
        &self,
        expected_environment_id: &str,
        expected_image_revision: &str,
        expected_run_id: &str,
    ) -> Result<()> {
        self.validate_for(expected_environment_id)?;
        if self.image.revision != expected_image_revision {
            bail!(
                "guest image revision mismatch: expected `{expected_image_revision}`, got `{}`",
                self.image.revision
            );
        }
        if self.run_id != expected_run_id {
            bail!(
                "guest run identity mismatch: expected `{expected_run_id}`, got `{}`",
                self.run_id
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl CommandSpec {
    pub fn validate(&self) -> Result<()> {
        if !self.program.starts_with('/') {
            bail!("guest command program must be an absolute path");
        }
        if self.program.contains('\0')
            || self.args.iter().any(|value| value.contains('\0'))
            || self.cwd.as_ref().is_some_and(|value| value.contains('\0'))
        {
            bail!("guest command contains a NUL byte");
        }
        for (name, value) in &self.env {
            if !valid_environment_name(name) || value.contains('\0') {
                bail!("invalid guest command environment entry `{name}`");
            }
        }
        Ok(())
    }
}

fn valid_environment_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && characters.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RequestAction {
    Ping,
    Exec {
        command: CommandSpec,
        timeout_ms: u64,
    },
    Spawn {
        command: CommandSpec,
    },
    Wait {
        process_id: u64,
        timeout_ms: u64,
    },
    Terminate {
        process_id: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GuestRequest {
    pub request_id: u64,
    #[serde(flatten)]
    pub action: RequestAction,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessState {
    Running,
    Exited,
    TimedOut,
    Terminated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessOutcome {
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ResponseResult {
    Pong,
    Spawned { process_id: u64, guest_pid: u32 },
    Process { outcome: ProcessOutcome },
    Error { message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct GuestResponse {
    pub request_id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum GuestMessage {
    Hello { hello: Box<GuestHello> },
    Response { response: GuestResponse },
}

pub fn write_frame(mut writer: impl Write, value: &impl Serialize) -> Result<()> {
    let encoded = serde_json::to_vec(value).context("failed to encode guest-control frame")?;
    if encoded.len() > MAX_FRAME_BYTES {
        bail!(
            "guest-control frame is {} bytes; maximum is {MAX_FRAME_BYTES}",
            encoded.len()
        );
    }
    writer
        .write_all(&encoded)
        .context("failed to write guest-control frame")?;
    writer
        .write_all(b"\n")
        .context("failed to terminate guest-control frame")?;
    writer
        .flush()
        .context("failed to flush guest-control frame")
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl BufRead) -> Result<T> {
    let mut encoded = Vec::new();
    let read = reader
        .take((MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut encoded)
        .context("failed to read guest-control frame")?;
    if read == 0 {
        bail!("guest-control connection closed");
    }
    if encoded.last() != Some(&b'\n') {
        bail!("guest-control frame exceeds {MAX_FRAME_BYTES} bytes");
    }
    encoded.pop();
    serde_json::from_slice(&encoded).context("invalid guest-control frame")
}

pub struct GuestControlClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    hello: GuestHello,
    next_request_id: u64,
}

impl GuestControlClient {
    pub fn connect_cancellable(
        address: SocketAddr,
        connect_timeout: Duration,
        hello_timeout: Duration,
        expected_environment_id: &str,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Self> {
        let deadline = Instant::now() + connect_timeout;
        let stream = loop {
            if cancelled() {
                bail!("guest-control connection cancelled");
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            let attempt_timeout = remaining
                .min(Duration::from_millis(250))
                .max(Duration::from_millis(1));
            match TcpStream::connect_timeout(&address, attempt_timeout) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25));
                    let _ = error;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to connect to guest control at {address}")
                    })
                }
            }
        };
        let mut stream = stream;
        let message = read_initial_message_cancellable(&mut stream, hello_timeout, &mut cancelled)?;
        stream
            .set_write_timeout(Some(connect_timeout))
            .context("failed to set guest-control write timeout")?;
        let writer = stream
            .try_clone()
            .context("failed to clone guest-control stream")?;
        let reader = BufReader::new(stream);
        let GuestMessage::Hello { hello } = message else {
            bail!("guest-control connection did not begin with a hello frame");
        };
        hello.validate_for(expected_environment_id)?;
        Ok(Self {
            reader,
            writer,
            hello: *hello,
            next_request_id: 1,
        })
    }

    pub fn connect_verified_identity(
        address: SocketAddr,
        connect_timeout: Duration,
        hello_timeout: Duration,
        expected_environment_id: &str,
        expected_image_revision: &str,
        expected_run_id: &str,
    ) -> Result<Self> {
        Self::connect_verified_identity_cancellable(
            address,
            connect_timeout,
            hello_timeout,
            expected_environment_id,
            expected_image_revision,
            expected_run_id,
            || false,
        )
    }

    pub fn connect_verified_identity_cancellable(
        address: SocketAddr,
        connect_timeout: Duration,
        hello_timeout: Duration,
        expected_environment_id: &str,
        expected_image_revision: &str,
        expected_run_id: &str,
        cancelled: impl FnMut() -> bool,
    ) -> Result<Self> {
        let client = Self::connect_cancellable(
            address,
            connect_timeout,
            hello_timeout,
            expected_environment_id,
            cancelled,
        )?;
        client.hello.validate_identity(
            expected_environment_id,
            expected_image_revision,
            expected_run_id,
        )?;
        Ok(client)
    }

    pub fn hello(&self) -> &GuestHello {
        &self.hello
    }

    pub fn request(&mut self, action: RequestAction, timeout: Duration) -> Result<ResponseResult> {
        self.request_cancellable(action, timeout, || false)
    }

    pub fn request_cancellable(
        &mut self,
        action: RequestAction,
        timeout: Duration,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<ResponseResult> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .context("guest-control request id overflow")?;
        write_frame(&mut self.writer, &GuestRequest { request_id, action })?;
        let message: GuestMessage =
            read_frame_cancellable(&mut self.reader, timeout, &mut cancelled)?;
        let GuestMessage::Response { response } = message else {
            bail!("guest sent a duplicate hello frame");
        };
        if response.request_id != request_id {
            bail!(
                "guest-control response id mismatch: expected {request_id}, got {}",
                response.request_id
            );
        }
        match response.result {
            ResponseResult::Error { message } => bail!("guest command failed: {message}"),
            result => Ok(result),
        }
    }
}

fn read_frame_cancellable<T: DeserializeOwned>(
    reader: &mut BufReader<TcpStream>,
    timeout: Duration,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<T> {
    let deadline = Instant::now() + timeout;
    let mut encoded = Vec::new();
    loop {
        if cancelled() {
            bail!("guest-control request cancelled");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("guest-control response timed out");
        }
        reader
            .get_ref()
            .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
            .context("failed to set guest-control response timeout")?;
        match reader.read_until(b'\n', &mut encoded) {
            Ok(0) => bail!("guest-control connection closed"),
            Ok(_) if encoded.last() == Some(&b'\n') => {
                encoded.pop();
                return serde_json::from_slice(&encoded).context("invalid guest-control frame");
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error).context("failed to read guest-control response"),
        }
        if encoded.len() > MAX_FRAME_BYTES {
            bail!("guest-control frame exceeds {MAX_FRAME_BYTES} bytes");
        }
    }
}

fn read_initial_message_cancellable(
    stream: &mut TcpStream,
    timeout: Duration,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<GuestMessage> {
    let deadline = Instant::now() + timeout;
    let mut encoded = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if cancelled() {
            bail!("guest-control connection cancelled while waiting for hello");
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("guest-control hello timed out");
        }
        stream
            .set_read_timeout(Some(remaining.min(Duration::from_millis(100))))
            .context("failed to set guest-control hello timeout")?;
        match stream.read(&mut chunk) {
            Ok(0) => bail!("guest-control connection closed"),
            Ok(read) => {
                encoded.extend_from_slice(&chunk[..read]);
                if encoded.len() > MAX_FRAME_BYTES + 1 {
                    bail!("guest-control frame exceeds {MAX_FRAME_BYTES} bytes");
                }
                let Some(newline) = encoded.iter().position(|byte| *byte == b'\n') else {
                    continue;
                };
                if encoded[newline + 1..]
                    .iter()
                    .any(|byte| !byte.is_ascii_whitespace())
                {
                    bail!("guest sent data after its initial hello frame");
                }
                return serde_json::from_slice(&encoded[..newline])
                    .context("invalid guest-control frame");
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error).context("failed to read guest-control hello"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    fn identity() -> ImageIdentity {
        ImageIdentity {
            schema: 1,
            environment_id: "linux/mint-cinnamon".to_string(),
            revision: "fixture-1".to_string(),
            desktop: "cinnamon".to_string(),
            display_protocol: "x11".to_string(),
            user: "qol".to_string(),
        }
    }

    fn hello() -> GuestHello {
        GuestHello {
            protocol_version: PROTOCOL_VERSION,
            run_id: "mint-lane-1".to_string(),
            image: identity(),
            session: GuestSession {
                user: "qol".to_string(),
                desktop: Some("cinnamon".to_string()),
                session_type: Some("x11".to_string()),
                display: Some(":0".to_string()),
                runtime_dir: Some("/run/user/1000".to_string()),
                dbus_session: true,
            },
            runner_pid: 123,
        }
    }

    #[test]
    fn image_identity_rejects_empty_and_unknown_contracts() {
        let mut value = identity();
        value.revision.clear();
        assert_eq!(
            value.validate().unwrap_err().to_string(),
            "guest image identity field `revision` must not be empty"
        );
        value = identity();
        value.schema = 2;
        assert_eq!(
            value.validate().unwrap_err().to_string(),
            "unsupported guest image identity schema 2"
        );
    }

    #[test]
    fn hello_requires_the_expected_graphical_session() {
        hello().validate_for("linux/mint-cinnamon").unwrap();
        hello()
            .validate_identity("linux/mint-cinnamon", "fixture-1", "mint-lane-1")
            .unwrap();
        let mut wrong = hello();
        wrong.session.display = None;
        assert_eq!(
            wrong
                .validate_for("linux/mint-cinnamon")
                .unwrap_err()
                .to_string(),
            "guest runner is not attached to a graphical desktop session"
        );
        let error = hello().validate_for("linux/debian-nocloud").unwrap_err();
        assert!(error.to_string().contains("guest image identity mismatch"));
        let error = hello()
            .validate_identity("linux/mint-cinnamon", "fixture-2", "mint-lane-1")
            .unwrap_err();
        assert!(error.to_string().contains("revision mismatch"));
        let error = hello()
            .validate_identity("linux/mint-cinnamon", "fixture-1", "mint-lane-2")
            .unwrap_err();
        assert!(error.to_string().contains("run identity mismatch"));
    }

    #[test]
    fn command_specs_are_argv_typed_and_require_absolute_programs() {
        let valid = CommandSpec {
            program: "/opt/qol/payload/qol-shot".to_string(),
            args: vec!["doctor".to_string()],
            cwd: Some("/tmp".to_string()),
            env: BTreeMap::from([("QOL_RUN_ID".to_string(), "lane-1".to_string())]),
        };
        valid.validate().unwrap();
        let mut invalid = valid;
        invalid.program = "sh".to_string();
        assert_eq!(
            invalid.validate().unwrap_err().to_string(),
            "guest command program must be an absolute path"
        );
    }

    #[test]
    fn frames_round_trip_and_are_newline_delimited() {
        let message = GuestMessage::Hello {
            hello: Box::new(hello()),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &message).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        let decoded: GuestMessage = read_frame(&mut bytes.as_slice()).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn oversized_frames_fail_closed() {
        let bytes = vec![b'x'; MAX_FRAME_BYTES + 1];
        let error = read_frame::<GuestMessage>(&mut bytes.as_slice()).unwrap_err();
        assert!(error.to_string().contains("exceeds"));
    }

    #[test]
    fn client_verifies_hello_and_correlates_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            write_frame(
                &mut stream,
                &GuestMessage::Hello {
                    hello: Box::new(hello()),
                },
            )
            .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let request: GuestRequest = read_frame(&mut reader).unwrap();
            assert_eq!(request.request_id, 1);
            assert_eq!(request.action, RequestAction::Ping);
            write_frame(
                &mut stream,
                &GuestMessage::Response {
                    response: GuestResponse {
                        request_id: request.request_id,
                        result: ResponseResult::Pong,
                    },
                },
            )
            .unwrap();
        });
        let mut client = GuestControlClient::connect_verified_identity(
            address,
            Duration::from_secs(1),
            Duration::from_secs(1),
            "linux/mint-cinnamon",
            "fixture-1",
            "mint-lane-1",
        )
        .unwrap();
        assert_eq!(client.hello().image, identity());
        assert_eq!(
            client
                .request(RequestAction::Ping, Duration::from_secs(1))
                .unwrap(),
            ResponseResult::Pong
        );
        server.join().unwrap();
    }

    #[test]
    fn client_waits_for_guest_hello_after_the_transport_is_available() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(75));
            write_frame(
                &mut stream,
                &GuestMessage::Hello {
                    hello: Box::new(hello()),
                },
            )
            .unwrap();
        });

        let client = GuestControlClient::connect_verified_identity(
            address,
            Duration::from_millis(25),
            Duration::from_millis(500),
            "linux/mint-cinnamon",
            "fixture-1",
            "mint-lane-1",
        )
        .unwrap();
        assert_eq!(client.hello().runner_pid, 123);
        server.join().unwrap();
    }

    #[test]
    fn cancellable_client_does_not_wait_for_a_stalled_guest_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let mut checks = 0;
        let error = GuestControlClient::connect_verified_identity_cancellable(
            address,
            Duration::from_secs(1),
            Duration::from_secs(5),
            "linux/mint-cinnamon",
            "fixture-1",
            "mint-lane-1",
            || {
                checks += 1;
                checks >= 3
            },
        )
        .err()
        .unwrap();
        assert!(error.to_string().contains("cancelled"));
        server.join().unwrap();
    }

    #[test]
    fn cancellable_request_does_not_wait_for_a_stalled_guest_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            write_frame(
                &mut stream,
                &GuestMessage::Hello {
                    hello: Box::new(hello()),
                },
            )
            .unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _: GuestRequest = read_frame(&mut reader).unwrap();
            thread::sleep(Duration::from_millis(250));
        });
        let mut client = GuestControlClient::connect_verified_identity(
            address,
            Duration::from_secs(1),
            Duration::from_secs(1),
            "linux/mint-cinnamon",
            "fixture-1",
            "mint-lane-1",
        )
        .unwrap();
        let mut checks = 0;
        let error = client
            .request_cancellable(RequestAction::Ping, Duration::from_secs(5), || {
                checks += 1;
                checks >= 3
            })
            .err()
            .unwrap();
        assert!(error.to_string().contains("cancelled"));
        server.join().unwrap();
    }
}
