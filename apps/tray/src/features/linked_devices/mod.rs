mod administration;
pub(crate) mod enrollment;
mod nearby;
mod network;
mod operations;
mod platform;
mod pointz;
mod projection;
pub(crate) mod settings;

pub(crate) use pointz::{is_legacy_pointz, legacy_pointz_allowed};

#[cfg(test)]
pub(crate) mod tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use qol_peers::admin::{
    ActivationId, Error, ExpectedAuthority, Lifecycle, Request, Response, Status,
};
use qol_peers::network::NetworkFailure;
use qol_peers::service::network::{NetworkControl, NetworkOptions};
use qol_peers::service::PeerAuthority;
use qol_peers::AuthorityError;
use tokio::sync::{broadcast, watch};

use crate::plugins::PluginManager;

pub struct LinkedDevices {
    handle: PeerHostHandle,
    shutdown_task: Option<tokio::task::JoinHandle<Result<(), Error>>>,
}

#[derive(Clone)]
pub struct PeerHostHandle {
    inner: Arc<Mutex<Host>>,
    plugins: Arc<Mutex<PluginManager>>,
    closing: watch::Sender<bool>,
    completion: watch::Sender<u64>,
}

struct Host {
    root: Result<PathBuf, Error>,
    state: State,
    activation: fn() -> Result<ActivationId, Error>,
    defaults: Defaults,
    network: Option<network::NetworkLauncher>,
    pointz: Option<pointz::PointzLauncher>,
    driver: Option<tokio::task::JoinHandle<()>>,
    observer: Option<tokio::task::JoinHandle<Result<(), Error>>>,
    shutting_down: bool,
    completion: watch::Sender<u64>,
}

struct Defaults {
    resident: fn() -> bool,
    name: fn() -> String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            resident: || qol_host_fixes::residency::HostResidency::current().is_resident(),
            name: device_name,
        }
    }
}

struct ActiveAuthority {
    authority: PeerAuthority,
    activation_id: ActivationId,
    network: Option<NetworkControl>,
    network_complete: bool,
    network_failure: Option<NetworkFailure>,
    cleanup_failed: bool,
    pointz: pointz::PointzSlot,
}

enum State {
    Inactive,
    Standby,
    Active(ActiveAuthority),
    Stopping(ActiveAuthority),
    Unavailable(Error),
    Shutdown,
}

impl LinkedDevices {
    pub async fn start(
        plugins: Arc<Mutex<PluginManager>>,
        shutdown: broadcast::Receiver<()>,
    ) -> Self {
        let root = crate::paths::base_data_dir()
            .map(|base| base.join("peers"))
            .map_err(|_| Error::RootUnavailable);
        let owner = Self::start_at(
            root,
            plugins,
            crate::dev_generation::is_shadow(),
            shutdown,
            Some(NetworkOptions::default()),
        )
        .await;
        owner.handle.attach_pointz(
            tokio::runtime::Handle::current(),
            pointz::PointzHostOptions::default(),
        );
        let reconciling = owner.handle.clone();
        let _ = tokio::task::spawn_blocking(move || reconciling.reconcile_pointz()).await;
        owner
    }

    async fn start_at(
        root: Result<PathBuf, Error>,
        plugins: Arc<Mutex<PluginManager>>,
        shadow: bool,
        mut shutdown: broadcast::Receiver<()>,
        options: Option<NetworkOptions>,
    ) -> Self {
        let handle = PeerHostHandle::new(root, plugins, shadow);
        if let Some(options) = options {
            handle.attach_network(tokio::runtime::Handle::current(), options);
        }
        let closing = handle.clone();
        let mut dropped = handle.closing.subscribe();
        let shutdown_task = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown.recv() => {},
                _ = dropped.changed() => {},
            }
            closing.shutdown_and_wait().await
        });
        let owner = Self {
            handle,
            shutdown_task: Some(shutdown_task),
        };
        let starting = owner.handle.clone();
        if tokio::task::spawn_blocking(move || starting.initialize())
            .await
            .is_err()
        {
            owner.handle.fail_initialization();
        }
        owner
    }

    pub async fn closed(mut self) -> Result<(), Error> {
        if let Some(task) = self.shutdown_task.as_mut() {
            let result = task.await.map_err(|_| Error::HostUnavailable)?;
            self.shutdown_task.take();
            return result;
        }
        self.handle.shutdown_and_wait().await
    }

    pub fn handle(&self) -> PeerHostHandle {
        self.handle.clone()
    }
}

impl Drop for LinkedDevices {
    fn drop(&mut self) {
        self.handle.shutdown();
        if let Some(task) = self.shutdown_task.take() {
            self.handle
                .inner
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .observer = Some(task);
        }
    }
}

impl PeerHostHandle {
    fn new(root: Result<PathBuf, Error>, plugins: Arc<Mutex<PluginManager>>, shadow: bool) -> Self {
        let state = if shadow {
            State::Standby
        } else {
            State::Inactive
        };
        let completion = watch::channel(0).0;
        Self {
            closing: watch::channel(false).0,
            completion: completion.clone(),
            inner: Arc::new(Mutex::new(Host {
                root,
                state,
                activation: activation_id,
                defaults: Defaults::default(),
                network: None,
                pointz: None,
                driver: None,
                observer: None,
                shutting_down: false,
                completion,
            })),
            plugins,
        }
    }

    fn initialize(&self) {
        let mut host = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(host.state, State::Inactive) {
            host.open_existing();
        }
        trace_lifecycle("initialize", &host.state);
    }

    fn fail_initialization(&self) {
        let mut host = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(host.state, State::Inactive | State::Unavailable(_)) {
            host.state = State::Unavailable(Error::HostUnavailable);
        }
        trace_lifecycle("initialize_failed", &host.state);
    }

    pub fn promote(&self) {
        let mut host = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if !matches!(host.state, State::Standby) {
            return;
        }
        host.open_existing();
        trace_lifecycle("promote", &host.state);
    }

    pub fn shutdown(&self) {
        let mut host = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(host.state, State::Shutdown) {
            return;
        }
        host.shutting_down = true;
        host.begin_stop();
        self.closing.send_replace(true);
        self.completion
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        trace_lifecycle("shutdown", &host.state);
    }

    pub fn request(&self, request: Request) -> Response {
        let operation = administration::operation_name(&request);
        if let Request::Enrollment { request } = &request {
            enrollment::trace_requested(request);
        }
        if let Request::SetGrants { peer_id, .. }
        | Request::Revoke { peer_id, .. }
        | Request::Grants { peer_id, .. } = &request
        {
            qol_runtime::probe!(
                "TRAY_PEERS",
                "event=admin_requested operation={} peer={}",
                operation,
                peer_id
            );
        }
        let result = self.dispatch(request);
        administration::trace_outcome(operation, &result);
        result.unwrap_or_else(|error| Response::Error { error })
    }
}

impl Host {
    fn open_existing(&mut self) {
        let result = self.root.as_ref().map_err(|error| *error).and_then(|root| {
            match std::fs::symlink_metadata(root) {
                Ok(_) => {
                    let activation_id = (self.activation)()?;
                    let authority = PeerAuthority::open_persistent(root, SystemTime::now())?;
                    Ok(Some(ActiveAuthority::new(authority, activation_id)))
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(_) => Err(AuthorityError::Storage.into()),
            }
        });
        match result {
            Ok(Some(authority)) => self.activate(authority),
            Ok(None) => self.state = State::Inactive,
            Err(error) => self.state = State::Unavailable(error),
        }
    }

    fn authority(&self) -> Result<&ActiveAuthority, Error> {
        match &self.state {
            State::Active(authority) => Ok(authority),
            State::Stopping(_) => Err(Error::Stopping),
            State::Inactive => Err(Error::Inactive),
            State::Standby => Err(Error::Standby),
            State::Unavailable(error) => Err(*error),
            State::Shutdown => Err(Error::Shutdown),
        }
    }

    fn status(&self) -> Result<Status, Error> {
        let authority = match &self.state {
            State::Active(active) | State::Stopping(active) => Some(projection::summary(
                active.authority.projection()?,
                active.activation_id,
            )),
            State::Inactive | State::Standby | State::Unavailable(_) | State::Shutdown => None,
        };
        Ok(Status {
            lifecycle: lifecycle(&self.state),
            authority,
        })
    }
}

impl ActiveAuthority {
    fn new(authority: PeerAuthority, activation_id: ActivationId) -> Self {
        Self {
            authority,
            activation_id,
            network: None,
            network_complete: true,
            network_failure: None,
            cleanup_failed: false,
            pointz: pointz::PointzSlot::default(),
        }
    }

    fn check_expected(&self, expected: ExpectedAuthority) -> Result<&PeerAuthority, Error> {
        if self.activation_id != expected.activation_id
            || self.authority.projection()?.peer_id != expected.authority_id
        {
            return Err(Error::StaleAuthority);
        }
        Ok(&self.authority)
    }
}

fn device_name() -> String {
    platform::device_name().map_or_else(pointz::hostname, |name| pointz::readable(&name))
}

fn activation_id() -> Result<ActivationId, Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).map_err(|_| Error::ActivationUnavailable)?;
    Ok(ActivationId::from_bytes(bytes))
}

fn lifecycle(state: &State) -> Lifecycle {
    match state {
        State::Inactive => Lifecycle::Inactive,
        State::Standby => Lifecycle::Standby,
        State::Active(_) => Lifecycle::Active,
        State::Stopping(_) => Lifecycle::Stopping,
        State::Unavailable(error) => Lifecycle::Unavailable { error: *error },
        State::Shutdown => Lifecycle::Shutdown,
    }
}

fn trace_lifecycle(event: &str, state: &State) {
    qol_runtime::probe!(
        "TRAY_PEERS",
        "event={} lifecycle={:?}",
        event,
        lifecycle(state)
    );
}
