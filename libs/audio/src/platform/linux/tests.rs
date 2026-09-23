use std::ffi::CString;
use std::io::{BufReader, ErrorKind, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use mio::net::UnixStream as MioUnixStream;
use pulseaudio::protocol::{self, Command, ProtocolError, PulseError};

use crate::control::ProfileAvailability;
use crate::devices::{Direction, State};

use super::connection::Connection;
use super::environment::{self, SelectionInputs};
use super::{control, default_output, devices};

const DEADLINE: Duration = Duration::from_secs(2);
const TEST_COOKIE: [u8; environment::COOKIE_LENGTH] = [0x5a; environment::COOKIE_LENGTH];

type StepReply = Box<dyn FnOnce(&mut UnixStream, u32, u16) -> Result<(), ProtocolError> + Send>;

enum Step {
    Typed(StepReply),
    Ack,
    Error(PulseError),
    Silent,
    Disconnect,
}

fn typed<R: protocol::CommandReply + Send + 'static>(value: R) -> Step {
    Step::Typed(Box::new(move |stream, seq, version| {
        protocol::write_reply_message(stream, seq, &value, version)
    }))
}

fn trickle_typed<R: protocol::CommandReply + Send + 'static>(
    value: R,
    fragments: usize,
    delay: Duration,
    tail_delay: Duration,
) -> Step {
    Step::Typed(Box::new(move |stream, seq, version| {
        let mut bytes = Vec::new();
        protocol::encode_reply_message(&mut bytes, seq, &value, version)?;
        let chunk = bytes.len().div_ceil(fragments + 1);
        let mut offset = 0;
        for _ in 0..fragments {
            let end = (offset + chunk).min(bytes.len());
            if end <= offset {
                break;
            }
            stream
                .write_all(&bytes[offset..end])
                .map_err(ProtocolError::Io)?;
            offset = end;
            thread::sleep(delay);
        }
        thread::sleep(tail_delay);
        stream
            .write_all(&bytes[offset..])
            .map_err(ProtocolError::Io)?;
        Ok(())
    }))
}

struct Fixture {
    commands: Arc<Mutex<Vec<Command>>>,
    cookie_matches: Arc<Mutex<Option<bool>>>,
    client: Arc<Mutex<Option<UnixStream>>>,
}

struct ClientGuard {
    client: Arc<Mutex<Option<UnixStream>>>,
}

impl ClientGuard {
    fn track(client: Arc<Mutex<Option<UnixStream>>>, stream: &UnixStream) -> Self {
        if let Ok(clone) = stream.try_clone() {
            *lock(&client) = Some(clone);
        }
        Self { client }
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let tracked = lock(&self.client).take();
        if let Some(stream) = tracked {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

struct FakeServer {
    path: PathBuf,
    commands: Arc<Mutex<Vec<Command>>>,
    cookie_matches: Arc<Mutex<Option<bool>>>,
    client: Arc<Mutex<Option<UnixStream>>>,
    finished: Receiver<()>,
    handle: Option<JoinHandle<()>>,
    _directory: tempfile::TempDir,
}

impl FakeServer {
    fn start(steps: Vec<Step>) -> Self {
        Self::launch(None, steps, protocol::MAX_VERSION)
    }

    fn start_with_version(steps: Vec<Step>, version: u16) -> Self {
        Self::launch(None, steps, version)
    }

    fn reject_auth(error: PulseError) -> Self {
        Self::launch(Some(error), Vec::new(), protocol::MAX_VERSION)
    }

    fn launch(auth_error: Option<PulseError>, steps: Vec<Step>, version: u16) -> Self {
        let directory = tempfile::TempDir::new().expect("fixture directory");
        let path = directory.path().join("native");
        let listener = UnixListener::bind(&path).expect("fixture socket");
        let commands = Arc::new(Mutex::new(Vec::new()));
        let cookie_matches = Arc::new(Mutex::new(None));
        let client = Arc::new(Mutex::new(None));
        let (finished_tx, finished) = mpsc::channel();
        let fixture = Fixture {
            commands: Arc::clone(&commands),
            cookie_matches: Arc::clone(&cookie_matches),
            client: Arc::clone(&client),
        };
        let handle = thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve(stream, auth_error, steps, version, fixture);
            }
            let _ = finished_tx.send(());
        });
        Self {
            path,
            commands,
            cookie_matches,
            client,
            finished,
            handle: Some(handle),
            _directory: directory,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn commands(&self) -> Vec<Command> {
        lock(&self.commands).clone()
    }

    fn auth_cookie_matches(&self) -> Option<bool> {
        *lock(&self.cookie_matches)
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        let client = lock(&self.client).take();
        if let Some(client) = client {
            let _ = client.shutdown(std::net::Shutdown::Both);
        }
        let _ = UnixStream::connect(&self.path);
        let finished = self.finished.recv_timeout(Duration::from_secs(2)).is_ok();
        let Some(handle) = self.handle.take() else {
            return;
        };
        if finished {
            let _ = handle.join();
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn serve(
    stream: UnixStream,
    auth_error: Option<PulseError>,
    steps: Vec<Step>,
    version: u16,
    fixture: Fixture,
) {
    let _guard = ClientGuard::track(Arc::clone(&fixture.client), &stream);
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let Ok((auth_seq, Command::Auth(params))) =
        protocol::read_command_message(&mut reader, protocol::MAX_VERSION)
    else {
        return;
    };
    *lock(&fixture.cookie_matches) = Some(params.cookie.as_slice() == TEST_COOKIE.as_slice());
    let version = params.version.min(protocol::MAX_VERSION).min(version);

    if let Some(error) = auth_error {
        let _ = protocol::write_error(&mut writer, auth_seq, &error);
        return;
    }

    let auth_reply = protocol::AuthReply {
        version,
        use_shm: false,
        use_memfd: false,
    };
    if protocol::write_reply_message(&mut writer, auth_seq, &auth_reply, protocol::MAX_VERSION)
        .is_err()
    {
        return;
    }

    let Ok((name_seq, Command::SetClientName(_))) =
        protocol::read_command_message(&mut reader, version)
    else {
        return;
    };
    let name_reply = protocol::SetClientNameReply { client_id: 1 };
    if protocol::write_reply_message(&mut writer, name_seq, &name_reply, version).is_err() {
        return;
    }

    for step in steps {
        match step {
            Step::Disconnect => return,
            step => {
                let Ok((seq, command)) = protocol::read_command_message(&mut reader, version)
                else {
                    return;
                };
                lock(&fixture.commands).push(command);
                let outcome = match step {
                    Step::Ack => protocol::write_ack_message(&mut writer, seq),
                    Step::Error(error) => protocol::write_error(&mut writer, seq, &error),
                    Step::Typed(reply) => reply(&mut writer, seq, version),
                    Step::Silent => Ok(()),
                    Step::Disconnect => return,
                };
                if outcome.is_err() {
                    return;
                }
            }
        }
    }

    while protocol::read_command_message(&mut reader, version).is_ok() {}
}

fn connect_with_deadline(server: &FakeServer, deadline: Duration) -> Connection {
    Connection::connect_at(server.path(), deadline, &TEST_COOKIE).expect("fixture connection")
}

fn connect(server: &FakeServer) -> Connection {
    connect_with_deadline(server, DEADLINE)
}

fn small_backlog_listener(path: &Path) -> UnixListener {
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)
        .expect("fixture socket");
    let address = socket2::SockAddr::unix(path).expect("fixture address");
    socket.bind(&address).expect("fixture bind");
    socket.listen(1).expect("fixture listen");
    UnixListener::from(socket)
}

fn port(name: &str) -> protocol::port_info::PortInfo {
    protocol::port_info::PortInfo {
        name: CString::new(name).expect("port name"),
        port_type: protocol::port_info::PortType::Unknown,
        description: None,
        dir: protocol::port_info::PortDirection::Input,
        priority: 0,
        available: protocol::port_info::PortAvailable::Unknown,
        availability_group: None,
    }
}

fn sink(
    index: u32,
    name: &str,
    description: Option<&str>,
    state: protocol::SinkState,
    active_port: usize,
) -> protocol::SinkInfo {
    protocol::SinkInfo {
        index,
        name: CString::new(name).expect("sink name"),
        description: description.map(|text| CString::new(text).expect("sink description")),
        state,
        active_port,
        ports: vec![port("analog-stereo"), port("hdmi-stereo")],
        ..protocol::SinkInfo::default()
    }
}

fn source(
    index: u32,
    name: &str,
    description: Option<&str>,
    state: protocol::SourceState,
    monitor_of_sink: Option<u32>,
) -> protocol::SourceInfo {
    let mut input = port("analog-input");
    input.port_type = protocol::port_info::PortType::Mic;
    input.availability_group = Some(CString::new("input-group").expect("port group"));
    protocol::SourceInfo {
        index,
        name: CString::new(name).expect("source name"),
        description: description.map(|text| CString::new(text).expect("source description")),
        state,
        monitor_of_sink_index: monitor_of_sink,
        ports: vec![input],
        ..protocol::SourceInfo::default()
    }
}

fn profile(name: &str, available: u32) -> protocol::CardProfileInfo {
    protocol::CardProfileInfo {
        name: CString::new(name).expect("profile"),
        description: None,
        priority: 0,
        available,
        num_sinks: 0,
        num_sources: 0,
    }
}

fn card_record(profiles: Vec<protocol::CardProfileInfo>) -> protocol::CardInfo {
    let mut props = protocol::Props::new();
    props.set(protocol::Prop::DeviceDescription, c"Built-in Audio");
    protocol::CardInfo {
        index: 1,
        name: CString::new("bluez_card.74_68_59_7F_5F_E9").expect("card name"),
        props,
        owner_module_index: None,
        driver: Some(CString::new("module-bluez5-device").expect("driver")),
        ports: Vec::new(),
        profiles,
        active_profile: Some(CString::new("a2dp-sink").expect("active profile")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceReply(Vec<protocol::SourceInfo>);

impl protocol::TagStructWrite for SourceReply {
    fn write(
        &self,
        writer: &mut protocol::TagStructWriter<'_>,
        version: u16,
    ) -> Result<(), ProtocolError> {
        for source in &self.0 {
            write_source_record(writer, source, version)?;
        }
        Ok(())
    }
}

impl protocol::TagStructRead for SourceReply {
    fn read(
        reader: &mut protocol::TagStructReader<'_>,
        version: u16,
    ) -> Result<Self, ProtocolError> {
        let sources =
            <Vec<protocol::SourceInfo> as protocol::TagStructRead>::read(reader, version)?;
        Ok(Self(sources))
    }
}

impl protocol::CommandReply for SourceReply {}

fn write_source_record(
    writer: &mut protocol::TagStructWriter<'_>,
    source: &protocol::SourceInfo,
    version: u16,
) -> Result<(), ProtocolError> {
    writer.write_index(Some(source.index))?;
    writer.write_string(Some(&source.name))?;
    writer.write_string(source.description.as_ref())?;
    writer.write(source.sample_spec)?;
    writer.write(source.channel_map)?;
    writer.write_index(source.owner_module_index)?;
    writer.write(source.cvolume)?;
    writer.write_bool(source.muted)?;
    writer.write_index(source.monitor_of_sink_index)?;
    writer.write_string(source.monitor_of_sink_name.as_ref())?;
    writer.write_usec(source.actual_latency)?;
    writer.write_string(source.driver.as_ref())?;
    writer.write_u32(source.flags.bits())?;
    if version >= 13 {
        writer.write(&source.props)?;
        writer.write_usec(source.configured_latency)?;
    }
    if version >= 15 {
        writer.write(source.base_volume)?;
        writer.write_u32(source.state as u32)?;
        writer.write_u32(source.volume_steps.unwrap_or(0))?;
        writer.write_index(source.card_index)?;
    }
    if version >= 16 {
        writer.write_u32(source.ports.len() as u32)?;
        for port in &source.ports {
            writer.write_string(Some(&port.name))?;
            writer.write_string(port.description.as_ref())?;
            writer.write_u32(port.priority)?;
            if version >= 24 {
                writer.write_u32(port.available as u32)?;
            }
            if version >= 34 {
                writer.write_string(port.availability_group.as_ref())?;
                writer.write_u32(port.port_type as u32)?;
            }
        }
        let active_port = if source.active_port < source.ports.len() {
            Some(&source.ports[source.active_port].name)
        } else {
            None
        };
        writer.write_string(active_port)?;
    }
    if version >= 21 {
        writer.write_u8(source.formats.len() as u8)?;
        for format in &source.formats {
            writer.write(format)?;
        }
    }
    Ok(())
}

fn selection(directory: &tempfile::TempDir) -> SelectionInputs {
    SelectionInputs {
        client_config: None,
        config_path: None,
        home: Some(directory.path().to_path_buf()),
        xdg_config_home: Some(directory.path().join("xdg-config")),
        server: None,
        runtime_path: Some(directory.path().join("runtime")),
        xdg_runtime: Some(directory.path().join("xdg-runtime")),
        cookie: None,
        system_config: directory.path().join("system-client.conf"),
    }
}

fn unix_socket(path: &Path) -> String {
    format!("unix:{}", path.display())
}

fn write_socket(path: &Path) {
    std::fs::write(path, b"").expect("socket file");
}

fn write_cookie(path: &Path, byte: u8) {
    std::fs::write(path, [byte; environment::COOKIE_LENGTH]).expect("cookie file");
}

#[test]
fn an_unused_fixture_shuts_down_within_the_bound() {
    let server = FakeServer::start(Vec::new());
    let started = Instant::now();
    drop(server);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "teardown took {:?}",
        started.elapsed()
    );
}

#[test]
fn remote_endpoints_are_rejected() {
    let endpoint = "tcp:audio.example:4713".to_owned();
    match environment::resolve_socket(Some(endpoint.clone()), None, None) {
        Err(crate::AudioError::Operation(reason)) => {
            assert!(reason.contains(endpoint.as_str()), "reason: {reason}");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn malformed_or_relative_server_specifications_are_rejected() {
    for endpoint in [
        "tcp:audio.example:4713",
        "unix:relative/native",
        "relative/native",
        "{guid}",
    ] {
        match environment::resolve_socket(Some(endpoint.to_owned()), None, None) {
            Err(crate::AudioError::Operation(reason)) => {
                assert!(reason.contains(endpoint), "endpoint {endpoint}: {reason}");
            }
            other => panic!("endpoint {endpoint}: unexpected result: {other:?}"),
        }
    }
}

#[test]
fn socket_resolution_uses_runtime_and_xdg_locations() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let runtime = directory.path().join("runtime");
    let xdg = directory.path().join("xdg");
    let runtime_socket = runtime.join("native");
    let xdg_socket = xdg.join("pulse/native");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    std::fs::create_dir_all(xdg_socket.parent().expect("xdg parent")).expect("xdg dir");
    std::fs::write(&runtime_socket, b"").expect("runtime socket");
    std::fs::write(&xdg_socket, b"").expect("xdg socket");

    let resolved = environment::resolve_socket(None, Some(runtime.clone()), Some(xdg.clone()))
        .expect("runtime socket wins");
    assert_eq!(resolved, runtime_socket);

    std::fs::remove_file(&runtime_socket).expect("remove runtime socket");
    let resolved = environment::resolve_socket(None, None, Some(xdg)).expect("xdg socket wins");
    assert_eq!(resolved, xdg_socket);

    let explicit = directory.path().join("explicit");
    std::fs::write(&explicit, b"").expect("explicit socket");
    let server = format!("unix:{}", explicit.display());
    let resolved =
        environment::resolve_socket(Some(server), Some(runtime), None).expect("server wins");
    assert_eq!(resolved, explicit);
}

#[test]
fn an_empty_server_override_is_treated_as_unset() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let runtime = directory.path().join("runtime");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    std::fs::write(runtime.join("native"), b"").expect("runtime socket");
    let resolved = environment::resolve_socket(Some(String::new()), Some(runtime), None)
        .expect("runtime socket");
    assert_eq!(resolved, directory.path().join("runtime/native"));
}

#[test]
fn explicit_server_override_does_not_fall_back() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let runtime = directory.path().join("runtime");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    std::fs::write(runtime.join("native"), b"").expect("runtime socket");
    let missing = directory.path().join("missing");
    let server = format!("unix:{}", missing.display());
    match environment::resolve_socket(Some(server), Some(runtime), None) {
        Err(crate::AudioError::ServerUnavailable(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn explicit_runtime_override_does_not_fall_back() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let runtime = directory.path().join("runtime");
    let xdg = directory.path().join("xdg");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    let xdg_socket = xdg.join("pulse/native");
    std::fs::create_dir_all(xdg_socket.parent().expect("xdg parent")).expect("xdg dir");
    std::fs::write(&xdg_socket, b"").expect("xdg socket");
    match environment::resolve_socket(None, Some(runtime), Some(xdg)) {
        Err(crate::AudioError::ServerUnavailable(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn cookie_resolution_prefers_explicit_over_config() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let explicit = directory.path().join("explicit-cookie");
    let config_home = directory.path().join("config");
    let config_cookie = config_home.join("pulse/cookie");
    std::fs::create_dir_all(config_cookie.parent().expect("config parent")).expect("config dir");
    write_cookie(&explicit, 0x11);
    write_cookie(&config_cookie, 0x22);
    let cookie = environment::resolve_cookie(Some(explicit), Some(config_home), None)
        .expect("explicit cookie");
    assert_eq!(cookie, [0x11u8; environment::COOKIE_LENGTH]);
}

#[test]
fn relative_explicit_cookie_resolves_under_config_home() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let config_home = directory.path().join("config");
    let config_cookie = config_home.join("pulse/quirky-cookie");
    std::fs::create_dir_all(config_cookie.parent().expect("config parent")).expect("config dir");
    write_cookie(&config_cookie, 0x77);
    let cookie = environment::resolve_cookie(
        Some(PathBuf::from("quirky-cookie")),
        Some(config_home),
        None,
    )
    .expect("relative cookie");
    assert_eq!(cookie, [0x77u8; environment::COOKIE_LENGTH]);
}

#[test]
fn cookie_resolution_uses_config_home_then_home() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let home = directory.path().to_path_buf();
    let config_home = directory.path().join("config");
    let config_cookie = config_home.join("pulse/cookie");
    std::fs::create_dir_all(config_cookie.parent().expect("config parent")).expect("config dir");
    write_cookie(&config_cookie, 0x33);
    let cookie = environment::resolve_cookie(None, Some(config_home.clone()), Some(home.clone()))
        .expect("config cookie");
    assert_eq!(cookie, [0x33u8; environment::COOKIE_LENGTH]);

    std::fs::remove_file(&config_cookie).expect("remove config cookie");
    write_cookie(&home.join(".pulse-cookie"), 0x44);
    let cookie =
        environment::resolve_cookie(None, Some(config_home), Some(home)).expect("legacy cookie");
    assert_eq!(cookie, [0x44u8; environment::COOKIE_LENGTH]);
}

#[test]
fn an_absent_cookie_becomes_a_zero_frame() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let cookie = environment::resolve_cookie(
        None,
        Some(directory.path().join("config")),
        Some(directory.path().to_path_buf()),
    )
    .expect("absent cookie");
    assert_eq!(cookie, [0u8; environment::COOKIE_LENGTH]);
}

#[test]
fn explicit_cookie_failures_are_authentication_errors() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let cases = [
        directory.path().join("missing"),
        directory.path().join("short"),
        PathBuf::from("relative-cookie"),
    ];
    std::fs::write(&cases[1], [0x55u8; 16]).expect("short cookie");
    for case in cases {
        match environment::resolve_cookie(Some(case.clone()), None, None) {
            Err(crate::AudioError::Authentication(_)) => {}
            other => panic!("case {case:?}: unexpected result: {other:?}"),
        }
    }
}

#[test]
fn a_long_explicit_cookie_is_truncated_to_the_frame() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let long = directory.path().join("long-cookie");
    std::fs::write(&long, [0x66u8; environment::COOKIE_LENGTH + 64]).expect("long cookie");
    let cookie = environment::resolve_cookie(Some(long), None, None).expect("long cookie");
    assert_eq!(cookie, [0x66u8; environment::COOKIE_LENGTH]);
}

#[test]
fn non_regular_cookie_input_is_rejected() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let cookie_directory = directory.path().join("cookie-directory");
    std::fs::create_dir_all(&cookie_directory).expect("cookie directory");
    match environment::resolve_cookie(Some(cookie_directory), None, None) {
        Err(crate::AudioError::Authentication(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn client_config_selects_server_and_cookie() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let runtime = inputs.runtime_path.clone().expect("runtime path");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    write_socket(&runtime.join("native"));
    let socket = directory.path().join("configured-native");
    write_socket(&socket);
    let cookie = directory.path().join("configured-cookie");
    write_cookie(&cookie, 0x21);
    let config = directory.path().join("client.conf");
    std::fs::write(
        &config,
        format!(
            "# managed client configuration\ndefault-server = {}\ncookie-file = {}\n",
            unix_socket(&socket),
            cookie.display()
        ),
    )
    .expect("client config");
    inputs.client_config = Some(config);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, socket);
    assert_eq!(settings.cookie, [0x21u8; environment::COOKIE_LENGTH]);
}

#[test]
fn environment_overrides_client_config() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let config_socket = directory.path().join("config-native");
    let env_socket = directory.path().join("env-native");
    write_socket(&config_socket);
    write_socket(&env_socket);
    let config_cookie = directory.path().join("config-cookie");
    let env_cookie = directory.path().join("env-cookie");
    write_cookie(&config_cookie, 0x22);
    write_cookie(&env_cookie, 0x23);
    let config = directory.path().join("client.conf");
    std::fs::write(
        &config,
        format!(
            "default-server = {}\ncookie-file = {}\n",
            unix_socket(&config_socket),
            config_cookie.display()
        ),
    )
    .expect("client config");
    inputs.client_config = Some(config);
    inputs.server = Some(unix_socket(&env_socket));
    inputs.cookie = Some(env_cookie);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, env_socket);
    assert_eq!(settings.cookie, [0x23u8; environment::COOKIE_LENGTH]);
}

#[test]
fn missing_explicit_client_config_is_an_error() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    inputs.client_config = Some(directory.path().join("missing-client.conf"));
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn malformed_client_config_is_an_error() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let config = directory.path().join("client.conf");
    std::fs::write(&config, "default-server unix:/missing/equals\n").expect("client config");
    inputs.client_config = Some(config);
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn oversized_client_config_is_an_error() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let config = directory.path().join("client.conf");
    let padding = "#\n".repeat(40_000);
    std::fs::write(&config, padding).expect("client config");
    inputs.client_config = Some(config);
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn unsupported_auto_connect_directive_is_rejected() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let runtime = inputs.runtime_path.clone().expect("runtime path");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    write_socket(&runtime.join("native"));

    let rejected = directory.path().join("auto-connect.conf");
    std::fs::write(&rejected, "auto-connect-display = yes\n").expect("client config");
    inputs.client_config = Some(rejected);
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }

    let accepted = directory.path().join("auto-connect-disabled.conf");
    std::fs::write(&accepted, "auto-connect-display = no\n").expect("client config");
    inputs.client_config = Some(accepted);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, runtime.join("native"));
}

#[test]
fn client_config_includes_resolve_relative_paths() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let socket = directory.path().join("included-native");
    write_socket(&socket);
    let included = directory.path().join("extra.conf");
    std::fs::write(
        &included,
        format!("default-server = {}\n", unix_socket(&socket)),
    )
    .expect("included config");
    let config = directory.path().join("client.conf");
    std::fs::write(&config, ".include extra.conf\n").expect("client config");
    inputs.client_config = Some(config);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, socket);
}

#[test]
fn client_config_include_cycle_is_rejected() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let first = directory.path().join("first.conf");
    let second = directory.path().join("second.conf");
    std::fs::write(&first, ".include second.conf\n").expect("first config");
    std::fs::write(&second, ".include first.conf\n").expect("second config");
    inputs.client_config = Some(first);
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn client_config_include_depth_is_bounded() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    for index in 0..12 {
        let path = directory.path().join(format!("layer-{index}.conf"));
        let body = if index == 11 {
            "# leaf\n".to_owned()
        } else {
            format!(".include layer-{}.conf\n", index + 1)
        };
        std::fs::write(&path, body).expect("layer config");
    }
    inputs.client_config = Some(directory.path().join("layer-0.conf"));
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn client_config_drop_ins_apply_in_order() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let main_socket = directory.path().join("main-native");
    let first_socket = directory.path().join("first-native");
    let second_socket = directory.path().join("second-native");
    let ignored_socket = directory.path().join("ignored-native");
    let included_socket = directory.path().join("included-native");
    for socket in [
        &main_socket,
        &first_socket,
        &second_socket,
        &ignored_socket,
        &included_socket,
    ] {
        write_socket(socket);
    }
    let config = directory.path().join("client.conf");
    std::fs::write(
        &config,
        format!(
            "default-server = {}\n.include included.conf\n",
            unix_socket(&main_socket)
        ),
    )
    .expect("client config");
    std::fs::write(
        directory.path().join("included.conf"),
        format!("default-server = {}\n", unix_socket(&included_socket)),
    )
    .expect("included config");
    let drop_dir = directory.path().join("client.conf.d");
    std::fs::create_dir_all(&drop_dir).expect("drop dir");
    std::fs::write(
        drop_dir.join("10-first.conf"),
        format!("default-server = {}\n", unix_socket(&first_socket)),
    )
    .expect("first drop-in");
    std::fs::write(
        drop_dir.join("05-ignored.txt"),
        format!("default-server = {}\n", unix_socket(&ignored_socket)),
    )
    .expect("ignored drop-in");
    std::fs::write(
        drop_dir.join("20-second.conf"),
        format!("default-server = {}\n", unix_socket(&second_socket)),
    )
    .expect("second drop-in");
    let included_drop_dir = directory.path().join("included.conf.d");
    std::fs::create_dir_all(&included_drop_dir).expect("included drop dir");
    std::fs::write(
        included_drop_dir.join("99-late.conf"),
        format!("default-server = {}\n", unix_socket(&ignored_socket)),
    )
    .expect("included drop-in");
    inputs.client_config = Some(config);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, second_socket);
}

#[test]
fn user_config_selection_prefers_pulse_home_then_xdg_config() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let pulse_socket = directory.path().join("pulse-native");
    let xdg_socket = directory.path().join("xdg-native");
    let override_socket = directory.path().join("override-native");
    for socket in [&pulse_socket, &xdg_socket, &override_socket] {
        write_socket(socket);
    }
    let pulse_config = directory.path().join(".pulse/client.conf");
    let xdg_config = directory.path().join(".config/pulse/client.conf");
    for (path, socket) in [(&pulse_config, &pulse_socket), (&xdg_config, &xdg_socket)] {
        std::fs::create_dir_all(path.parent().expect("config parent")).expect("config dir");
        std::fs::write(path, format!("default-server = {}\n", unix_socket(socket)))
            .expect("user config");
    }
    let settings = environment::resolve(&inputs).expect("pulse home config");
    assert_eq!(settings.socket, pulse_socket);

    std::fs::remove_file(&pulse_config).expect("remove pulse home config");
    let settings = environment::resolve(&inputs).expect("xdg config");
    assert_eq!(settings.socket, xdg_socket);

    let config_path = directory.path().join("config-path");
    std::fs::create_dir_all(&config_path).expect("config path dir");
    std::fs::write(
        config_path.join("client.conf"),
        format!("default-server = {}\n", unix_socket(&override_socket)),
    )
    .expect("override config");
    inputs.config_path = Some(config_path);
    let settings = environment::resolve(&inputs).expect("config path config");
    assert_eq!(settings.socket, override_socket);
}

#[test]
fn config_path_suppresses_user_config_candidates() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let home_socket = directory.path().join("home-native");
    let system_socket = directory.path().join("system-native");
    write_socket(&home_socket);
    write_socket(&system_socket);
    let home_config = directory.path().join(".pulse/client.conf");
    std::fs::create_dir_all(home_config.parent().expect("config parent")).expect("config dir");
    std::fs::write(
        &home_config,
        format!("default-server = {}\n", unix_socket(&home_socket)),
    )
    .expect("home config");
    let system_config = directory.path().join("system-client.conf");
    std::fs::write(
        &system_config,
        format!("default-server = {}\n", unix_socket(&system_socket)),
    )
    .expect("system config");
    inputs.system_config = system_config;
    let empty_config_path = directory.path().join("empty-config-path");
    std::fs::create_dir_all(&empty_config_path).expect("config path dir");
    inputs.config_path = Some(empty_config_path);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, system_socket);
}

#[test]
fn system_config_is_used_when_user_configs_absent() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let socket = directory.path().join("system-native");
    write_socket(&socket);
    let system_config = directory.path().join("system-client.conf");
    std::fs::write(
        &system_config,
        format!("default-server = {}\n", unix_socket(&socket)),
    )
    .expect("system config");
    inputs.system_config = system_config;
    let settings = environment::resolve(&inputs).expect("system config");
    assert_eq!(settings.socket, socket);
}

#[test]
fn config_cookie_file_relative_resolves_under_config_home() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let runtime = inputs.runtime_path.clone().expect("runtime path");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    write_socket(&runtime.join("native"));
    let config_home = inputs.xdg_config_home.clone().expect("config home");
    let cookie = config_home.join("pulse/shared-cookie");
    std::fs::create_dir_all(cookie.parent().expect("cookie parent")).expect("config home");
    write_cookie(&cookie, 0x31);
    let config = directory.path().join("client.conf");
    std::fs::write(&config, "cookie-file = shared-cookie\n").expect("client config");
    inputs.client_config = Some(config);
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.cookie, [0x31u8; environment::COOKIE_LENGTH]);
}

#[test]
fn config_default_server_rejects_remote_endpoints() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let mut inputs = selection(&directory);
    let runtime = inputs.runtime_path.clone().expect("runtime path");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    write_socket(&runtime.join("native"));
    let config = directory.path().join("client.conf");
    std::fs::write(&config, "default-server = tcp:audio.example:4713\n").expect("client config");
    inputs.client_config = Some(config);
    match environment::resolve(&inputs) {
        Err(crate::AudioError::Operation(reason)) => {
            assert!(
                reason.contains("tcp:audio.example:4713"),
                "reason: {reason}"
            );
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn absent_config_defaults_leave_runtime_resolution() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let inputs = selection(&directory);
    let runtime = inputs.runtime_path.clone().expect("runtime path");
    std::fs::create_dir_all(&runtime).expect("runtime dir");
    write_socket(&runtime.join("native"));
    let settings = environment::resolve(&inputs).expect("settings");
    assert_eq!(settings.socket, runtime.join("native"));
    assert_eq!(settings.cookie, [0u8; environment::COOKIE_LENGTH]);
}

#[test]
fn missing_local_endpoints_are_server_failures() {
    match environment::resolve_socket(None, None, None) {
        Err(crate::AudioError::ServerUnavailable(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    match Connection::connect_at(
        Path::new("/nonexistent/qol-audio-native"),
        DEADLINE,
        &TEST_COOKIE,
    ) {
        Err(crate::AudioError::ServerUnavailable(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn native_auth_sends_the_injected_cookie() {
    let server = FakeServer::start(vec![typed(Vec::<protocol::SinkInfo>::new())]);
    let mut connection = connect(&server);
    let listed = devices::list_devices(&mut connection, Direction::Output).expect("sinks");
    assert!(listed.is_empty());
    assert_eq!(server.auth_cookie_matches(), Some(true));
}

#[test]
fn auth_rejection_is_reported_without_credentials() {
    let server = FakeServer::reject_auth(PulseError::AuthKey);
    match Connection::connect_at(server.path(), DEADLINE, &TEST_COOKIE) {
        Err(crate::AudioError::Authentication(reason)) => {
            assert!(reason.contains("key"), "reason: {reason}");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn silent_server_times_out() {
    let server = FakeServer::start(vec![Step::Silent]);
    let mut connection = connect_with_deadline(&server, Duration::from_millis(400));
    let started = Instant::now();
    match connection.request::<Vec<protocol::SinkInfo>>(&Command::GetSinkInfoList) {
        Err(crate::AudioError::Timeout) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn request_deadline_covers_a_trickle() {
    let record = vec![sink(
        2,
        "alsa_output.pci",
        None,
        protocol::SinkState::Running,
        0,
    )];
    let server = FakeServer::start(vec![trickle_typed(
        record,
        9,
        Duration::from_millis(60),
        Duration::from_millis(60),
    )]);
    let mut connection = connect_with_deadline(&server, Duration::from_millis(400));
    let started = Instant::now();
    match connection.request::<Vec<protocol::SinkInfo>>(&Command::GetSinkInfoList) {
        Err(crate::AudioError::Timeout) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "the request waited for the whole trickle"
    );
}

#[test]
fn connect_deadline_covers_a_full_backlog() {
    let directory = tempfile::TempDir::new().expect("fixture directory");
    let path = directory.path().join("native");
    let listener = small_backlog_listener(&path);
    let mut _held = Vec::new();
    let mut saturated = false;
    for _ in 0..64 {
        match MioUnixStream::connect(&path) {
            Ok(stream) => _held.push(stream),
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                saturated = true;
                break;
            }
            Err(error) => panic!("unexpected connect error: {error}"),
        }
    }
    assert!(saturated, "the fixture never saturated its listen backlog");
    let started = Instant::now();
    match Connection::connect_at(&path, Duration::from_millis(400), &TEST_COOKIE) {
        Err(crate::AudioError::Timeout) => {}
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(listener);
}

#[test]
fn disconnect_is_reported_as_server_unavailable() {
    let server = FakeServer::start(vec![Step::Disconnect]);
    let mut connection = connect(&server);
    match connection.request::<Vec<protocol::SinkInfo>>(&Command::GetSinkInfoList) {
        Err(crate::AudioError::ServerUnavailable(_)) => {}
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn sinks_map_description_properties_state_and_active_port() {
    let mut record = sink(2, "alsa_output.pci", None, protocol::SinkState::Running, 1);
    record
        .props
        .set(protocol::Prop::DeviceFormFactor, c"headset");
    let server = FakeServer::start(vec![typed(vec![record])]);
    let mut connection = connect(&server);
    let listed = devices::list_devices(&mut connection, Direction::Output).expect("sinks");
    assert_eq!(listed.len(), 1, "listed: {listed:?}");
    let device = &listed[0];
    assert_eq!(device.index, 2);
    assert_eq!(device.name, "alsa_output.pci");
    assert_eq!(device.description, "alsa_output.pci");
    assert_eq!(device.state, State::Running);
    assert_eq!(device.active_port.as_deref(), Some("hdmi-stereo"));
    assert_eq!(device.monitor_of_sink, None);
    assert_eq!(
        device
            .properties
            .get("device.form_factor")
            .map(String::as_str),
        Some("headset")
    );
    assert_eq!(server.commands().last(), Some(&Command::GetSinkInfoList));
}

#[test]
fn sources_keep_monitors_distinct_from_microphones() {
    let records = vec![
        source(
            4,
            "alsa_output.pci.monitor",
            Some("Monitor of Speakers"),
            protocol::SourceState::Idle,
            Some(3),
        ),
        source(
            5,
            "alsa_input.usb",
            Some("USB Mic"),
            protocol::SourceState::Suspended,
            None,
        ),
        source(
            6,
            "alsa_input.sentinel",
            None,
            protocol::SourceState::Running,
            Some(u32::MAX),
        ),
    ];
    let server = FakeServer::start(vec![typed(SourceReply(records))]);
    let mut connection = connect(&server);
    let listed = devices::list_devices(&mut connection, Direction::Input).expect("sources");
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[0].monitor_of_sink, Some(3));
    assert_eq!(listed[0].description, "Monitor of Speakers");
    assert_eq!(listed[0].state, State::Idle);
    assert_eq!(listed[1].monitor_of_sink, None);
    assert_eq!(listed[1].description, "USB Mic");
    assert_eq!(listed[1].state, State::Suspended);
    assert_eq!(listed[2].monitor_of_sink, None);
    assert_eq!(listed[2].description, "alsa_input.sentinel");
    assert_eq!(listed[2].active_port.as_deref(), Some("analog-input"));
    assert_eq!(server.commands().last(), Some(&Command::GetSourceInfoList));
}

#[test]
fn empty_inventory_is_a_valid_reply() {
    let server = FakeServer::start(vec![typed(Vec::<protocol::SinkInfo>::new())]);
    let mut connection = connect(&server);
    let listed = devices::list_devices(&mut connection, Direction::Output).expect("sinks");
    assert!(listed.is_empty());
}

#[test]
fn effective_default_reports_presence_and_absence_per_direction() {
    let populated = || protocol::ServerInfo {
        default_sink_name: Some(CString::new("alsa_output.pci").expect("default sink")),
        default_source_name: Some(CString::new("alsa_input.usb").expect("default source")),
        ..protocol::ServerInfo::default()
    };
    let server = FakeServer::start(vec![
        typed(populated()),
        typed(populated()),
        typed(protocol::ServerInfo::default()),
    ]);
    let mut connection = connect(&server);
    let output = default_output::effective_default(&mut connection, Direction::Output)
        .expect("output default");
    assert_eq!(output.as_deref(), Some("alsa_output.pci"));
    let input = default_output::effective_default(&mut connection, Direction::Input)
        .expect("input default");
    assert_eq!(input.as_deref(), Some("alsa_input.usb"));
    let absent = default_output::effective_default(&mut connection, Direction::Output)
        .expect("absent default");
    assert_eq!(absent, None);
}

#[test]
fn cards_report_boolean_profile_availability() {
    let detailed = protocol::CardProfileInfo {
        description: Some(CString::new("High Fidelity Playback").expect("description")),
        priority: 10,
        num_sinks: 1,
        ..profile("a2dp-sink", 1)
    };
    let record = card_record(vec![detailed, profile("off", 0), profile("odd", 7)]);
    let server = FakeServer::start(vec![typed(vec![record])]);
    let mut connection = connect(&server);
    let cards = control::list_cards(&mut connection).expect("cards");
    assert_eq!(cards.len(), 1);
    let card = &cards[0];
    assert_eq!(card.index, 1);
    assert_eq!(card.description, "Built-in Audio");
    assert_eq!(card.driver.as_deref(), Some("module-bluez5-device"));
    assert_eq!(card.active_profile.as_deref(), Some("a2dp-sink"));
    assert_eq!(
        card.profiles[0].availability,
        ProfileAvailability::Available
    );
    assert_eq!(card.profiles[0].description, "High Fidelity Playback");
    assert_eq!(
        card.profiles[1].availability,
        ProfileAvailability::Unavailable
    );
    assert_eq!(
        card.profiles[2].availability,
        ProfileAvailability::Available
    );
    assert_eq!(server.commands().last(), Some(&Command::GetCardInfoList));
}

#[test]
fn cards_report_unknown_availability_below_protocol_29() {
    let record = card_record(vec![profile("a2dp-sink", 0)]);
    let server = FakeServer::start_with_version(vec![typed(vec![record])], 28);
    let mut connection = connect(&server);
    let cards = control::list_cards(&mut connection).expect("cards");
    assert_eq!(
        cards[0].profiles[0].availability,
        ProfileAvailability::Unknown
    );
}

#[test]
fn set_card_profile_carries_the_requested_profile() {
    let server = FakeServer::start(vec![Step::Ack]);
    let mut connection = connect(&server);
    control::set_card_profile(&mut connection, "bluez_card.74_68_59_7F_5F_E9", "a2dp-sink")
        .expect("set profile");
    let expected = Command::SetCardProfile(protocol::SetCardProfileParams {
        card_index: None,
        card_name: Some(CString::new("bluez_card.74_68_59_7F_5F_E9").expect("card name")),
        profile_name: CString::new("a2dp-sink").expect("profile name"),
    });
    assert_eq!(server.commands().last(), Some(&expected));
}

#[test]
fn server_errors_keep_the_server_message() {
    let server = FakeServer::start(vec![Step::Error(PulseError::NoEntity)]);
    let mut connection = connect(&server);
    match control::set_card_profile(&mut connection, "missing", "a2dp-sink") {
        Err(crate::AudioError::Operation(reason)) => {
            assert!(reason.contains("No such entity"), "reason: {reason}");
        }
        other => panic!("unexpected result: {other:?}"),
    }
}

#[test]
fn source_outputs_report_source_and_corked() {
    let record = protocol::SourceOutputInfo {
        index: 11,
        name: CString::new("output").expect("output name"),
        source_index: 5,
        client_index: None,
        corked: false,
        ..protocol::SourceOutputInfo::default()
    };
    let server = FakeServer::start(vec![typed(vec![record])]);
    let mut connection = connect(&server);
    let outputs = control::list_source_outputs(&mut connection).expect("source outputs");
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].index, 11);
    assert_eq!(outputs[0].source_index, 5);
    assert_eq!(outputs[0].client_index, None);
    assert!(!outputs[0].corked);
    assert_eq!(
        server.commands().last(),
        Some(&Command::GetSourceOutputInfoList)
    );
}

#[test]
fn suspend_sink_and_default_sink_acknowledge() {
    let server = FakeServer::start(vec![Step::Ack, Step::Ack]);
    let mut connection = connect(&server);
    control::suspend_sink(&mut connection, "bluez_output.74_68_59_7F_5F_E9.1", true)
        .expect("suspend sink");
    control::set_default_sink(&mut connection, "bluez_output.74_68_59_7F_5F_E9.1")
        .expect("default sink");
    let commands = server.commands();
    assert_eq!(commands.len(), 2, "commands: {commands:?}");
    assert_eq!(
        commands[0],
        Command::SuspendSink(protocol::SuspendParams {
            device_index: None,
            device_name: Some(CString::new("bluez_output.74_68_59_7F_5F_E9.1").expect("sink name")),
            suspend: true,
        })
    );
    assert_eq!(
        commands[1],
        Command::SetDefaultSink(
            CString::new("bluez_output.74_68_59_7F_5F_E9.1").expect("sink name")
        )
    );
}

#[test]
fn server_facts_report_incarnation_and_defaults() {
    let record = protocol::ServerInfo {
        server_name: Some(CString::new("PulseAudio (on PipeWire 1.0.5)").expect("server name")),
        server_version: Some(CString::new("15.0.0").expect("server version")),
        cookie: 0xdead_beef,
        default_sink_name: Some(CString::new("alsa_output.pci").expect("default sink")),
        default_source_name: None,
        ..protocol::ServerInfo::default()
    };
    let server = FakeServer::start(vec![typed(record)]);
    let mut connection = connect(&server);
    let facts = control::server_facts(&mut connection).expect("server facts");
    assert_eq!(
        facts.name.as_deref(),
        Some("PulseAudio (on PipeWire 1.0.5)")
    );
    assert_eq!(facts.version.as_deref(), Some("15.0.0"));
    assert_eq!(facts.incarnation, 0xdead_beef);
    assert_eq!(facts.default_sink.as_deref(), Some("alsa_output.pci"));
    assert_eq!(facts.default_source, None);
}
