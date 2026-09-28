use crate::plugins::action_transport::{self, DaemonActionDispatch};
use crate::plugins::manager::reload_delivery::DaemonInstance;
use crate::plugins::operation_catalog::{CatalogCapture, PluginDeclaration, SelectedOperation};
use crate::plugins::{PluginId, PluginManager};
use qol_peers::operations::{Failure, OperationBody, Outcome};
use qol_plugin_api::operations::{OperationIdentity, OperationKind};
use std::{collections::HashMap, path::PathBuf, time::Duration};

pub(crate) struct CapturedRemote {
    catalog: CatalogCapture,
    daemons: HashMap<PluginId, CapturedDaemon>,
}

struct CapturedDaemon {
    instance: Option<DaemonInstance>,
    token: Option<String>,
    endpoint: Option<PathBuf>,
    artifact: Option<qol_artifact::InspectedArtifact>,
}

pub(crate) struct PreparedRemote {
    body: OperationBody,
    selected: SelectedOperation,
    instance: DaemonInstance,
    token: String,
    endpoint: PathBuf,
    artifact: qol_artifact::InspectedArtifact,
    declaration: String,
    payload: Vec<u8>,
}

impl PreparedRemote {
    pub(crate) fn capture(manager: &PluginManager) -> Result<CapturedRemote, Failure> {
        if manager.lifecycle_cancellation().is_cancelled() {
            return Err(Failure::Unavailable);
        }
        let catalog = CatalogCapture::capture(manager);
        let daemons = manager
            .plugins()
            .map(|plugin| {
                let id = plugin.id.as_str();
                (
                    plugin.id.clone(),
                    CapturedDaemon {
                        instance: manager.current_daemon_instance(id),
                        token: crate::plugins::daemon_lifecycle::current_daemon_token(id),
                        endpoint: crate::plugins::daemon_lifecycle::current_daemon_endpoint(id),
                        artifact: crate::plugins::daemon_lifecycle::current_daemon_artifact(id),
                    },
                )
            })
            .collect();
        Ok(CapturedRemote { catalog, daemons })
    }

    pub(crate) fn prepare(
        mut captured: CapturedRemote,
        body: &OperationBody,
    ) -> Result<Self, Failure> {
        let catalog = captured.catalog.resolve();
        let selected = catalog
            .select(&body.key)
            .filter(|selected| selected.operation.peer.is_some())
            .ok_or(Failure::ChangedDeclaration)?;
        let operation = &selected.operation;
        if !matches!(operation.key.identity, OperationIdentity::Stable(_))
            || operation.key.kind == OperationKind::Stream
        {
            return Err(Failure::Unsupported);
        }
        if operation
            .action_kind
            .is_some_and(|kind| kind != crate::plugins::ActionType::Run)
        {
            return Err(Failure::Unsupported);
        }
        let daemon = captured
            .daemons
            .remove(&selected.plugin.id)
            .ok_or(Failure::Unavailable)?;
        let instance = daemon.instance.ok_or(Failure::Unavailable)?;
        let token = daemon.token.ok_or(Failure::UpdateRequired)?;
        let endpoint = daemon.endpoint.ok_or(Failure::Unsupported)?;
        let artifact = verify_artifact(&selected.plugin)?;
        if daemon.artifact.as_ref() != Some(&artifact) {
            return Err(Failure::Artifact);
        }
        let declaration = selected.declaration_digest(&artifact)?;
        let request = qol_runtime::protocol::FencedDaemonRequest {
            fence_version: 1,
            instance: token.clone(),
            request: qol_runtime::protocol::DaemonRequest {
                action: operation.key.name.clone(),
                input: serde_json::from_str(&body.arguments).map_err(|_| Failure::InvalidBody)?,
            },
        };
        let mut payload = serde_json::to_vec(&request).map_err(|_| Failure::InvalidBody)?;
        payload.push(b'\n');
        if payload.len() > qol_runtime::local_ipc::MAX_MESSAGE_BYTES {
            return Err(Failure::InvalidBody);
        }
        Ok(Self {
            body: body.clone(),
            selected,
            instance,
            token,
            endpoint,
            artifact,
            declaration,
            payload,
        })
    }

    pub(crate) fn declaration(&self) -> &str {
        &self.declaration
    }

    pub(crate) fn probe(&self, timeout: Duration) -> Result<(), Failure> {
        if !action_transport::probe_operation_instance(&self.endpoint, &self.token, timeout) {
            return Err(Failure::UpdateRequired);
        }
        Ok(())
    }

    pub(crate) fn refresh(&self, captured: CapturedRemote) -> Result<Self, Failure> {
        let refreshed = Self::prepare(captured, &self.body)?;
        if refreshed.instance != self.instance
            || refreshed.token != self.token
            || refreshed.endpoint != self.endpoint
        {
            return Err(Failure::ChangedInstance);
        }
        if refreshed.artifact != self.artifact || refreshed.declaration != self.declaration {
            return Err(Failure::ChangedDeclaration);
        }
        Ok(refreshed)
    }

    pub(crate) fn recheck(&self, manager: &PluginManager) -> Result<(), Failure> {
        if manager.lifecycle_cancellation().is_cancelled() {
            return Err(Failure::Unavailable);
        }
        let plugin_id = self.selected.plugin.id.as_str();
        if manager.current_daemon_instance(plugin_id) != Some(self.instance)
            || crate::plugins::daemon_lifecycle::current_daemon_token(plugin_id).as_ref()
                != Some(&self.token)
            || crate::plugins::daemon_lifecycle::current_daemon_endpoint(plugin_id).as_ref()
                != Some(&self.endpoint)
        {
            return Err(Failure::ChangedInstance);
        }
        if !self.selected.matches_manager(manager) {
            return Err(Failure::ChangedDeclaration);
        }
        if crate::plugins::daemon_lifecycle::current_daemon_artifact(plugin_id).as_ref()
            != Some(&self.artifact)
        {
            return Err(Failure::Artifact);
        }
        Ok(())
    }

    pub(crate) fn execute(self, timeout: Duration) -> Outcome {
        match action_transport::dispatch_prepared(&self.endpoint, &self.payload, timeout) {
            DaemonActionDispatch::Handled { payload: None } => Outcome::Acknowledged,
            DaemonActionDispatch::Handled {
                payload: Some(value),
            } => match serde_json::to_string(&value) {
                Ok(document) if document.len() <= qol_peers::operations::MAX_RESULT_BYTES => {
                    Outcome::Result { document }
                }
                _ => Outcome::Unknown,
            },
            DaemonActionDispatch::Error(_) => Outcome::HandlerError,
            DaemonActionDispatch::Fallback => Outcome::HandlerFallback,
            DaemonActionDispatch::NotSent => Outcome::Refused {
                reason: Failure::Unavailable,
            },
            DaemonActionDispatch::NotReady { .. } => Outcome::Refused {
                reason: Failure::Unavailable,
            },
            DaemonActionDispatch::OutcomeUnknown => Outcome::Unknown,
        }
    }
}

fn verify_artifact(plugin: &PluginDeclaration) -> Result<qol_artifact::InspectedArtifact, Failure> {
    let daemon = plugin
        .manifest
        .daemon
        .as_ref()
        .filter(|daemon| daemon.enabled)
        .ok_or(Failure::Unsupported)?;
    let path = crate::plugins::resolve_plugin_command_path_for_source(
        &plugin.path,
        &daemon.command,
        Some(&plugin.source),
    )
    .ok_or(Failure::Artifact)?;
    #[cfg(all(test, target_os = "linux"))]
    if let Some(artifact) = fixture_artifact(&path) {
        return Ok(artifact);
    }
    let artifact = qol_artifact::inspect_path(&path).map_err(|_| Failure::Artifact)?;
    for slice in &artifact.slices {
        use qol_conventions::artifact::{BuildProfile, BuildRole};
        let identity = &slice.identity;
        let binary = std::path::Path::new(&daemon.command)
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or(Failure::Artifact)?;
        let expectation = if plugin.source.is_live_source() {
            match identity.flavor.profile {
                BuildProfile::Debug => qol_artifact::ArtifactExpectation::development_debug(
                    binary,
                    plugin.id.as_str(),
                    BuildRole::Plugin,
                    identity.flavor.dev_features,
                ),
                BuildProfile::Release => qol_artifact::ArtifactExpectation::development_release(
                    binary,
                    plugin.id.as_str(),
                    BuildRole::Plugin,
                    identity.flavor.dev_features,
                ),
                BuildProfile::Sandbox => return Err(Failure::Artifact),
            }
        } else {
            qol_artifact::ArtifactExpectation::production(
                binary,
                plugin.id.as_str(),
                BuildRole::Plugin,
            )
        };
        let target = &qol_conventions::artifact::current()
            .ok_or(Failure::Artifact)?
            .target;
        qol_artifact::verify_identity(
            identity,
            &expectation
                .with_version(&plugin.manifest.plugin.version)
                .with_compatible_target(target),
        )
        .map_err(|_| Failure::Artifact)?;
    }
    if artifact.slices.is_empty() {
        return Err(Failure::Artifact);
    }
    Ok(artifact)
}

#[cfg(all(test, target_os = "linux"))]
struct FixtureArtifact {
    artifact: qol_artifact::InspectedArtifact,
    pause: Option<FixtureVerificationPause>,
}

#[cfg(all(test, target_os = "linux"))]
struct FixtureVerificationPause {
    skip: usize,
    entered: std::sync::mpsc::Sender<()>,
    resume: std::sync::mpsc::Receiver<()>,
}

#[cfg(all(test, target_os = "linux"))]
fn fixture_artifact(path: &std::path::Path) -> Option<qol_artifact::InspectedArtifact> {
    let mut fixtures = FIXTURE_ARTIFACTS
        .get_or_init(Default::default)
        .lock()
        .ok()?;
    let fixture = fixtures.get_mut(path)?;
    let artifact = fixture.artifact.clone();
    let pause = if fixture.pause.as_ref().is_some_and(|pause| pause.skip == 0) {
        fixture.pause.take()
    } else {
        if let Some(pause) = &mut fixture.pause {
            pause.skip -= 1;
        }
        None
    };
    drop(fixtures);
    if let Some(pause) = pause {
        pause.entered.send(()).unwrap();
        pause.resume.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    Some(artifact)
}

#[cfg(all(test, target_os = "linux"))]
static FIXTURE_ARTIFACTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeMap<PathBuf, FixtureArtifact>>,
> = std::sync::OnceLock::new();

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn register_fixture_verification_pause(
    path: &std::path::Path,
    skip: usize,
) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
    let (entered, wait) = std::sync::mpsc::channel();
    let (resume, incoming) = std::sync::mpsc::channel();
    FIXTURE_ARTIFACTS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .get_mut(path)
        .unwrap()
        .pause = Some(FixtureVerificationPause {
        skip,
        entered,
        resume: incoming,
    });
    (wait, resume)
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) fn register_fixture_artifact(path: PathBuf) -> qol_artifact::InspectedArtifact {
    let artifact = qol_artifact::InspectedArtifact {
        format: qol_artifact::ArtifactFormat::Elf,
        slices: Vec::new(),
    };
    FIXTURE_ARTIFACTS
        .get_or_init(Default::default)
        .lock()
        .unwrap()
        .insert(
            path,
            FixtureArtifact {
                artifact: artifact.clone(),
                pause: None,
            },
        );
    artifact
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::Plugin;

    #[test]
    fn peer_operation_artifact_refuses_an_unidentified_executable() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("unverified"), "not an artifact").unwrap();
        let manifest = toml::from_str(
            r#"
[plugin]
id = "qol-unverified"
uid = "unverified-uid"
name = "Unverified"
description = ""
version = "1.0.0"
[menu]
label = "Unverified"
items = []
[daemon]
enabled = true
command = "unverified"
socket = "unverified.sock"
"#,
        )
        .unwrap();
        let plugin = Plugin::new(
            crate::plugins::PluginId::new("qol-unverified"),
            manifest,
            root.path().into(),
        );
        assert_eq!(
            verify_artifact(&PluginDeclaration::capture(&plugin)).err(),
            Some(Failure::Artifact)
        );
    }
}
