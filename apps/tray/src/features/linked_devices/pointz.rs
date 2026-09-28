use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use qol_peers::admin::{Error, PageCursor, PointzRequest, PointzStatus, Response};
use qol_peers::pointz::{PointzPairing, PointzPlugin, PointzTransport};
use qol_peers::service::pointz::{
    LegacyPointz, PointzAdapter, PointzOptions, PointzSink, COMMAND_PORT, DISCOVERY_PORT,
};
use qol_peers::service::PeerAuthority;
use qol_plugin_api::manifest::PeerTrust;
use qol_plugin_daemon::daemon::{DaemonConfig, SocketSource};
use qol_runtime::protocol::DaemonResponse;
use tokio::runtime::Handle;

use super::{ActiveAuthority, Host, PeerHostHandle, State};
use crate::plugins::PluginManager;

const POINTZ_UID: &str = "9cb88d65-1d43-4fde-95b6-105761f0a14b";
const LEGACY_SEED: &str = "pairing-secret";
const LEGACY_DEVICES: &str = "devices.json";
const MIGRATED_SUFFIX: &str = "migrated";
const DELIVERY_TIMEOUT: Duration = Duration::from_millis(250);
const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);

const CUTOVER_UNKNOWN: u8 = 0;
const CUTOVER_LEGACY: u8 = 1;
const CUTOVER_CORE: u8 = 2;
static CUTOVER: AtomicU8 = AtomicU8::new(CUTOVER_UNKNOWN);

pub(crate) fn legacy_pointz_allowed() -> bool {
    CUTOVER.load(Ordering::SeqCst) == CUTOVER_LEGACY
}

pub(crate) fn is_legacy_pointz(manifest: &qol_plugin_api::manifest::PluginManifest) -> bool {
    manifest
        .plugin
        .uid
        .as_ref()
        .is_some_and(|uid| uid.as_str() == POINTZ_UID)
        && manifest.capabilities.peer_trust != Some(PeerTrust::PointzV1)
}

#[derive(Clone)]
pub(crate) struct PointzHostOptions {
    pub bind: Ipv4Addr,
    pub discovery_port: u16,
    pub command_port: u16,
    pub data_root: Option<PathBuf>,
}

impl Default for PointzHostOptions {
    fn default() -> Self {
        Self {
            bind: Ipv4Addr::UNSPECIFIED,
            discovery_port: DISCOVERY_PORT,
            command_port: COMMAND_PORT,
            data_root: qol_config::data_dir(),
        }
    }
}

pub(super) struct PointzLauncher {
    runtime: Handle,
    options: PointzHostOptions,
}

#[derive(Default)]
pub(super) struct PointzSlot {
    running: Option<(Target, PointzAdapter)>,
    failure: Option<PointzTransport>,
}

#[derive(Clone, PartialEq, Eq)]
struct Target {
    socket: Option<PathBuf>,
    data: Option<PathBuf>,
}

enum Installed {
    Absent,
    Legacy,
    Compatible(Target),
}

impl Installed {
    fn plugin(&self) -> PointzPlugin {
        match self {
            Self::Absent => PointzPlugin::Absent,
            Self::Legacy => PointzPlugin::Legacy,
            Self::Compatible(_) => PointzPlugin::Compatible,
        }
    }
}

struct DaemonSink(Option<PathBuf>);

impl PointzSink for DaemonSink {
    fn deliver(&self, command: serde_json::Value) -> bool {
        let Some(socket) = &self.0 else {
            return false;
        };
        let config = DaemonConfig {
            socket: SocketSource::Path(socket.clone()),
            support_replace_existing: false,
        };
        matches!(
            qol_plugin_daemon::daemon::send_request(&config, "input", command, DELIVERY_TIMEOUT),
            Ok(DaemonResponse::Handled { .. })
        )
    }
}

impl PeerHostHandle {
    pub(super) fn attach_pointz(&self, runtime: Handle, options: PointzHostOptions) {
        let mut closing = self.closing.subscribe();
        let handle = self.clone();
        runtime.spawn(async move {
            loop {
                tokio::select! {
                    _ = closing.changed() => return,
                    _ = tokio::time::sleep(RECONCILE_INTERVAL) => {}
                }
                let owner = handle.clone();
                let _ = tokio::task::spawn_blocking(move || owner.reconcile_pointz()).await;
            }
        });
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pointz = Some(PointzLauncher { runtime, options });
    }

    pub(super) fn reconcile_pointz(&self) {
        let installed = self.installed_pointz();
        let mut host = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        host.reconcile_pointz(installed);
    }

    fn installed_pointz(&self) -> Installed {
        let data_root = self
            .inner
            .lock()
            .ok()
            .and_then(|host| host.pointz.as_ref()?.options.data_root.clone());
        let Ok(plugins) = self.plugins.lock() else {
            return Installed::Absent;
        };
        installed(&plugins, data_root.as_deref())
    }

    pub(super) fn pointz_request(&self, request: PointzRequest) -> Result<Response, Error> {
        let installed = self.installed_pointz();
        let mut host = self.inner.lock().map_err(|_| Error::HostUnavailable)?;
        match request {
            PointzRequest::Status {} => Ok(Response::PointzStatus {
                status: host.pointz_status(&installed)?,
            }),
            PointzRequest::Devices { cursor } => host.pointz_devices(cursor),
            PointzRequest::BeginPairing {} => {
                let Installed::Compatible(target) = &installed else {
                    return Err(Error::PointzUnavailable);
                };
                host.ensure_pointz_authority(target)?;
                host.reconcile_pointz(installed);
                let active = host.authority()?;
                let Some((_, adapter)) = &active.pointz.running else {
                    return Err(Error::PointzUnavailable);
                };
                let pairing = adapter.begin_pairing().map_err(Error::from)?;
                qol_runtime::probe!("TRAY_PEERS", "event=pointz_pairing outcome=open");
                Ok(Response::PointzPairing { pairing })
            }
            PointzRequest::CancelPairing {} => {
                if let Ok(active) = host.authority() {
                    if let Some((_, adapter)) = &active.pointz.running {
                        adapter.cancel_pairing();
                    }
                }
                qol_runtime::probe!("TRAY_PEERS", "event=pointz_pairing outcome=cancelled");
                Ok(Response::PointzPairing {
                    pairing: PointzPairing::CLOSED,
                })
            }
            PointzRequest::Remove {
                expected,
                device_id,
            } => {
                let active = host.authority()?;
                let revision = active
                    .check_expected(expected)?
                    .remove_pointz_device(expected.revision, device_id)?;
                qol_runtime::probe!(
                    "TRAY_PEERS",
                    "event=pointz_removed device={} revision={}",
                    device_id,
                    revision
                );
                Ok(Response::Changed {
                    authority_id: active.authority.projection()?.peer_id,
                    activation_id: active.activation_id,
                    revision,
                })
            }
        }
    }
}

impl Host {
    fn reconcile_pointz(&mut self, installed: Installed) {
        let Some(launcher) = &self.pointz else {
            return;
        };
        let runtime = launcher.runtime.clone();
        let options = launcher.options.clone();
        publish_cutover(&self.state);
        if matches!(self.state, State::Inactive) {
            if let Installed::Compatible(target) = &installed {
                if legacy_files_exist(target) {
                    let _ = self.ensure_pointz_authority(target);
                }
            }
        }
        let State::Active(active) = &mut self.state else {
            return;
        };
        let Installed::Compatible(target) = installed else {
            stop_adapter(active);
            return;
        };
        if active.authority.pointz().ok().flatten().is_none() && legacy_files_exist(&target) {
            import_legacy(&active.authority, &target);
        }
        publish_cutover(&self.state);
        let State::Active(active) = &mut self.state else {
            return;
        };
        if active.authority.pointz().ok().flatten().is_none() {
            stop_adapter(active);
            return;
        }
        if active
            .pointz
            .running
            .as_ref()
            .is_some_and(|(running, _)| *running == target)
        {
            return;
        }
        stop_adapter(active);
        let sink = Arc::new(DaemonSink(target.socket.clone()));
        let adapter_options = PointzOptions {
            bind: options.bind,
            discovery_port: options.discovery_port,
            command_port: options.command_port,
            hostname: hostname(),
        };
        match PointzAdapter::start(&runtime, &active.authority, adapter_options, sink) {
            Ok(adapter) => {
                qol_runtime::probe!("TRAY_PEERS", "event=pointz_adapter outcome=started");
                active.pointz.failure = None;
                active.pointz.running = Some((target, adapter));
            }
            Err(failure) => {
                if active.pointz.failure != Some(failure) {
                    qol_runtime::probe!(
                        "TRAY_PEERS",
                        "event=pointz_adapter outcome=failed failure={:?}",
                        failure
                    );
                }
                active.pointz.failure = Some(failure);
            }
        }
    }

    fn ensure_pointz_authority(&mut self, target: &Target) -> Result<(), Error> {
        if matches!(self.state, State::Inactive) {
            let root = self.root.clone()?;
            let name = hostname();
            self.begin_persistent(&root, name)?;
            qol_runtime::probe!("TRAY_PEERS", "event=pointz_authority outcome=created");
        }
        let active = self.authority()?;
        if active.authority.pointz()?.is_some() {
            return Ok(());
        }
        if legacy_files_exist(target) {
            import_legacy(&active.authority, target);
        }
        active.authority.initialize_pointz()?;
        Ok(())
    }

    fn begin_persistent(&mut self, root: &Path, name: String) -> Result<(), Error> {
        let activation_id = (self.activation)()?;
        let authority = PeerAuthority::create_persistent(root, name, SystemTime::now())?;
        self.activate(ActiveAuthority::new(authority, activation_id));
        Ok(())
    }

    fn pointz_status(&self, installed: &Installed) -> Result<PointzStatus, Error> {
        let (authority, projection, pairing, transport) = match &self.state {
            State::Active(active) => {
                let expected = super::projection::summary(
                    active.authority.projection()?,
                    active.activation_id,
                )
                .expected();
                let (pairing, transport) = match (&active.pointz.running, active.pointz.failure) {
                    (Some((_, adapter)), _) => (adapter.pairing(), adapter.transport()),
                    (None, Some(failure)) => (PointzPairing::CLOSED, failure),
                    (None, None) => (PointzPairing::CLOSED, PointzTransport::Stopped {}),
                };
                (
                    Some(expected),
                    active.authority.pointz()?,
                    pairing,
                    transport,
                )
            }
            _ => (
                None,
                None,
                PointzPairing::CLOSED,
                PointzTransport::Stopped {},
            ),
        };
        Ok(PointzStatus {
            plugin: installed.plugin(),
            authority,
            migration: projection.as_ref().map(|projection| projection.import),
            server_id: projection
                .as_ref()
                .map(|projection| projection.server_id.clone()),
            device_count: projection
                .as_ref()
                .map_or(0, |projection| projection.devices.len() as u32),
            pairing,
            transport,
        })
    }

    fn pointz_devices(&self, cursor: PageCursor) -> Result<Response, Error> {
        let active = self.authority()?;
        let devices = active
            .authority
            .pointz()?
            .map(|projection| projection.devices)
            .unwrap_or_default();
        super::projection::pointz_devices(
            active.authority.projection()?,
            active.activation_id,
            devices,
            cursor,
        )
    }
}

pub(super) fn stop_adapter(active: &mut ActiveAuthority) {
    if active.pointz.running.take().is_some() {
        qol_runtime::probe!("TRAY_PEERS", "event=pointz_adapter outcome=stopped");
    }
}

fn publish_cutover(state: &State) {
    let cutover = match state {
        State::Inactive => CUTOVER_LEGACY,
        State::Active(active) => match active.authority.pointz() {
            Ok(Some(_)) => CUTOVER_CORE,
            Ok(None) => CUTOVER_LEGACY,
            Err(_) => CUTOVER_UNKNOWN,
        },
        State::Standby | State::Stopping(_) | State::Unavailable(_) | State::Shutdown => {
            CUTOVER_UNKNOWN
        }
    };
    CUTOVER.store(cutover, Ordering::SeqCst);
}

fn installed(plugins: &PluginManager, data_root: Option<&Path>) -> Installed {
    let Some(plugin) = plugins.plugins().find(|plugin| {
        plugin
            .manifest
            .plugin
            .uid
            .as_ref()
            .is_some_and(|uid| uid.as_str() == POINTZ_UID)
    }) else {
        return Installed::Absent;
    };
    if is_legacy_pointz(&plugin.manifest) {
        return Installed::Legacy;
    }
    Installed::Compatible(Target {
        socket: crate::plugins::action_executor::daemon_socket(plugin),
        data: data_root.map(|root| root.join("plugins").join(plugin.id.as_str())),
    })
}

fn legacy_files_exist(target: &Target) -> bool {
    target.data.as_ref().is_some_and(|data| {
        [LEGACY_SEED, LEGACY_DEVICES]
            .iter()
            .any(|name| std::fs::symlink_metadata(data.join(name)).is_ok())
    })
}

fn import_legacy(authority: &PeerAuthority, target: &Target) {
    let Some(data) = &target.data else {
        return;
    };
    let seed = read_regular(&data.join(LEGACY_SEED));
    let devices = read_regular(&data.join(LEGACY_DEVICES));
    let legacy = match LegacyPointz::decode(seed.as_deref(), devices.as_deref()) {
        Ok(legacy) => legacy,
        Err(error) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=pointz_import outcome=failed error={:?}",
                error
            );
            return;
        }
    };
    let import = legacy.import();
    match authority.import_pointz(legacy) {
        Ok(revision) => {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=pointz_import outcome=imported revision={} imported={} dropped={} seed_replaced={} devices_unreadable={}",
                revision,
                import.imported,
                import.dropped,
                import.seed_replaced,
                import.devices_unreadable
            );
            retire_legacy(data);
        }
        Err(error) => qol_runtime::probe!(
            "TRAY_PEERS",
            "event=pointz_import outcome=failed error={:?}",
            error
        ),
    }
}

fn read_regular(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.file_type().is_file() || metadata.len() > 1024 * 1024 {
        return None;
    }
    std::fs::read(path).ok()
}

fn retire_legacy(data: &Path) {
    for name in [LEGACY_SEED, LEGACY_DEVICES] {
        let path = data.join(name);
        if std::fs::symlink_metadata(&path).is_err() {
            continue;
        }
        if let Err(error) = std::fs::rename(&path, data.join(format!("{name}.{MIGRATED_SUFFIX}"))) {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=pointz_retire outcome=failed file={} kind={:?}",
                name,
                error.kind()
            );
        }
    }
}

pub(crate) fn hostname() -> String {
    readable(&gethostname::gethostname().to_string_lossy())
}

pub(super) fn readable(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect();
    match name.trim() {
        "" => "This device".into(),
        trimmed => trimmed.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    use super::*;

    #[test]
    fn each_verified_command_reaches_the_daemon_as_one_input_request() {
        let temporary = tempfile::tempdir().unwrap();
        let socket = temporary.path().join("pointz.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let daemon = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            let reply = serde_json::to_string(&DaemonResponse::Handled { data: None }).unwrap();
            (&stream)
                .write_all(format!("{reply}\n").as_bytes())
                .unwrap();
            serde_json::from_str::<serde_json::Value>(&line).unwrap()
        });

        let command = serde_json::json!({"type": "MouseClick", "button": 1});
        assert!(DaemonSink(Some(socket.clone())).deliver(command.clone()));

        let request = daemon.join().unwrap();
        assert_eq!(request["action"], "input");
        assert_eq!(request["input"], command);
        assert!(!DaemonSink(None).deliver(command.clone()));
        assert!(!DaemonSink(Some(temporary.path().join("missing.sock"))).deliver(command));
    }
}
