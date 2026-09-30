mod console;
mod devices;
pub(crate) mod install;
mod modifier_mapping;
mod peer_signature;
pub(crate) mod protocol;
pub(crate) mod report;
mod server;
pub(crate) mod watchdog;

use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};

use crate::platform::macos::virtual_hid::client::request::{Request, Status};
use crate::platform::macos::virtual_hid::client::{ClientEvent, Connection};
use crate::platform::macos::virtual_hid::PQRS_SOCKET;
use protocol::{
    HelperStatus, ToDaemon, PAGE_APPLE_VENDOR_TOP_CASE, PAGE_KEYBOARD, PROTOCOL_VERSION,
};
use report::ReportState;
use watchdog::{Action, Watchdog};

const RECONNECT_INTERVAL: Duration = Duration::from_secs(1);
const COUNTRY_CODE_WAIT: Duration = Duration::from_secs(2);

extern "C" {
    fn geteuid() -> u32;
}

pub(crate) fn is_root() -> bool {
    unsafe { geteuid() == 0 }
}

pub(crate) fn run() -> Result<()> {
    if !is_root() {
        bail!(
            "hid-helper must run as root; install it with `sudo qol-keyremap install-hid-helper`"
        );
    }
    devices::request_input_monitoring();
    let shared = Arc::new(Shared::default());
    let server = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("keyremap-helper-server".into())
        .spawn(move || server::serve(&server))?;
    let supervisor = Arc::clone(&shared);
    std::thread::Builder::new()
        .name("keyremap-helper-vhid".into())
        .spawn(move || supervise_virtual_keyboard(&supervisor))?;
    log::info!("keyboard helper started, protocol {PROTOCOL_VERSION}");
    devices::run(shared)
}

pub(crate) fn query_status() -> crate::platform::Probe<crate::platform::HelperState> {
    use std::io::{BufRead, BufReader, ErrorKind};

    use crate::platform::{HelperReport, HelperState, Probe};

    let stream = match UnixStream::connect(protocol::SOCKET_PATH) {
        Ok(stream) => stream,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            return Probe::Known(if Path::new(install::HELPER_PLIST).exists() {
                HelperState::NotRunning
            } else {
                HelperState::NotInstalled
            });
        }
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            return Probe::Unknown("the helper socket belongs to another user".to_string());
        }
        Err(error) => return Probe::Unknown(format!("could not reach the helper: {error}")),
    };
    let exchange = || -> anyhow::Result<ToDaemon> {
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        let mut writer = stream.try_clone()?;
        protocol::write_message(
            &mut writer,
            &protocol::ToHelper::Hello {
                protocol: PROTOCOL_VERSION,
                role: protocol::Role::Status,
            },
        )?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line)?;
        Ok(protocol::parse_message(&line)?)
    };
    match exchange() {
        Ok(ToDaemon::Status(status)) => Probe::Known(HelperState::Running(HelperReport {
            virtual_keyboard_ready: status.virtual_keyboard_ready,
            input_monitoring: status.input_monitoring,
            seized: status.seized,
            conflicts: status.conflicts,
        })),
        Ok(ToDaemon::Refused { protocol, .. }) => Probe::Known(HelperState::VersionMismatch {
            helper: protocol,
            expected: PROTOCOL_VERSION,
        }),
        Ok(other) => Probe::Unknown(format!("the helper answered {other:?}")),
        Err(error) => Probe::Unknown(format!("the helper did not answer: {error:#}")),
    }
}

#[derive(Default)]
pub(super) struct Shared {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    watchdog: Watchdog,
    reports: ReportState,
    vhid: Option<Connection>,
    session: Option<(u64, UnixStream)>,
    caps_light: Option<bool>,
    keyboard_ready: bool,
    input_monitoring: bool,
    country_code: Option<u64>,
    seized: Vec<String>,
    conflicts: Vec<String>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn open_session(&self, generation: u64, writer: UnixStream) {
        let mut state = self.lock();
        if let Some((_, previous)) = state.session.take() {
            let _ = previous.shutdown(Shutdown::Both);
        }
        state.watchdog.session_opened(generation);
        state.session = Some((generation, writer));
    }

    fn close_session(&self, generation: u64) {
        let mut state = self.lock();
        state.watchdog.session_closed(generation);
        if state
            .session
            .as_ref()
            .is_some_and(|(current, _)| *current == generation)
        {
            state.session = None;
        }
    }

    fn drop_session(&self) {
        let mut state = self.lock();
        if let Some((generation, stream)) = state.session.take() {
            let _ = stream.shutdown(Shutdown::Both);
            state.watchdog.session_closed(generation);
        }
    }

    fn heartbeat(&self, generation: u64) {
        self.lock().watchdog.heartbeat(generation, Instant::now());
    }

    fn emit(&self, generation: u64, page: u16, usage: u16, pressed: bool) {
        let mut state = self.lock();
        if state.watchdog.accepts_emit(generation) {
            state.post(page, usage, pressed);
        }
    }

    fn set_caps_light(&self, generation: u64, on: bool) {
        let mut state = self.lock();
        if state.watchdog.is_current(generation) {
            state.caps_light = Some(on);
        }
    }

    fn set_input_monitoring(&self, granted: bool) {
        let mut state = self.lock();
        state.input_monitoring = granted;
        state.watchdog.set_input_monitoring(granted);
    }

    fn status(&self) -> HelperStatus {
        let state = self.lock();
        HelperStatus {
            protocol: PROTOCOL_VERSION,
            virtual_keyboard_ready: state.keyboard_ready,
            input_monitoring: state.input_monitoring,
            seized: state.seized.clone(),
            conflicts: state.conflicts.clone(),
        }
    }

    fn tick(&self) -> (Action, Option<bool>) {
        let mut state = self.lock();
        let action = state.watchdog.tick(Instant::now());
        (action, state.caps_light.take())
    }

    fn device_arrived(&self, country_code: u64) {
        let mut state = self.lock();
        state.watchdog.device_arrived();
        state.country_code.get_or_insert(country_code);
    }

    fn publish_devices(&self, seized: Vec<String>, conflicts: Vec<String>) {
        let mut state = self.lock();
        state.seized = seized;
        state.conflicts = conflicts;
    }

    fn announce_seized(&self, active: bool) {
        let mut state = self.lock();
        if !active {
            for release in state.reports.clear() {
                state.send_vhid(&release);
            }
            state.send_vhid(&Request::KeyboardReset);
        }
        state.tell_session(&ToDaemon::Seized { active });
    }

    fn forward(&self, page: u16, usage: u16, pressed: bool, apple: bool) {
        let mut state = self.lock();
        match page {
            PAGE_KEYBOARD | PAGE_APPLE_VENDOR_TOP_CASE => state.tell_session(&ToDaemon::Key {
                usage_page: page,
                usage,
                pressed,
                apple,
            }),
            _ => state.post(page, usage, pressed),
        }
    }

    fn set_vhid(&self, connection: Option<Connection>) {
        let mut state = self.lock();
        if connection.is_none() {
            state.keyboard_ready = false;
            state.watchdog.set_keyboard_ready(false);
        }
        state.vhid = connection;
    }

    fn set_keyboard_ready(&self, ready: bool) {
        let mut state = self.lock();
        state.keyboard_ready = ready;
        state.watchdog.set_keyboard_ready(ready);
    }

    fn country_code(&self) -> Option<u64> {
        self.lock().country_code
    }
}

impl State {
    fn post(&mut self, page: u16, usage: u16, pressed: bool) {
        if let Some(report) = self.reports.apply(page, usage, pressed) {
            self.send_vhid(&report);
        }
    }

    fn send_vhid(&self, request: &Request) {
        if let Some(vhid) = &self.vhid {
            if let Err(error) = vhid.send(request) {
                log::warn!("posting to the virtual keyboard failed: {error}");
            }
        }
    }

    fn tell_session(&mut self, message: &ToDaemon) {
        let Some((generation, stream)) = self.session.as_mut() else {
            return;
        };
        let generation = *generation;
        if let Err(error) = protocol::write_message(stream, message) {
            log::warn!("dropping keyremap session {generation}: {error}");
            if let Some((_, stream)) = self.session.take() {
                let _ = stream.shutdown(Shutdown::Both);
            }
            self.watchdog.session_closed(generation);
        }
    }
}

fn supervise_virtual_keyboard(shared: &Shared) {
    let mut last_error = String::new();
    loop {
        let (sender, events) = mpsc::channel();
        match Connection::connect(Path::new(PQRS_SOCKET), sender) {
            Ok(connection) => {
                last_error.clear();
                let country_code = wait_for_country_code(shared);
                match connection.send(&Request::KeyboardInitialize { country_code }) {
                    Ok(()) => {
                        log::info!("creating the virtual keyboard, country code {country_code}");
                        shared.set_vhid(Some(connection));
                        watch_virtual_keyboard(shared, &events);
                        shared.set_vhid(None);
                    }
                    Err(error) => log::warn!("creating the virtual keyboard failed: {error}"),
                }
            }
            Err(error) => {
                let message = error.to_string();
                if message != last_error {
                    log::warn!("the pqrs daemon is not reachable: {message}");
                    last_error = message;
                }
            }
        }
        std::thread::sleep(RECONNECT_INTERVAL);
    }
}

fn wait_for_country_code(shared: &Shared) -> u64 {
    let deadline = Instant::now() + COUNTRY_CODE_WAIT;
    loop {
        if let Some(country_code) = shared.country_code() {
            return country_code;
        }
        if Instant::now() >= deadline {
            return 0;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn watch_virtual_keyboard(shared: &Shared, events: &Receiver<ClientEvent>) {
    for event in events.iter() {
        match event {
            ClientEvent::Status(Status::KeyboardReady(ready)) => {
                log::info!("virtual keyboard ready: {ready}");
                shared.set_keyboard_ready(ready);
            }
            ClientEvent::Status(Status::DriverActivated(active)) => {
                log::info!("virtual HID driver activated: {active}");
            }
            ClientEvent::Status(Status::DriverConnected(connected)) => {
                log::info!("virtual HID driver connected: {connected}");
            }
            ClientEvent::Status(Status::DriverVersionMismatched(mismatched)) => {
                if mismatched {
                    log::error!("the virtual HID driver does not match the pqrs daemon; reinstall the driver package");
                }
            }
            ClientEvent::Disconnected(reason) => {
                log::warn!("lost the pqrs daemon: {reason}");
                return;
            }
        }
    }
}
