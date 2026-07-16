use std::collections::BTreeMap;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use qol_dev_guest::{CommandSpec, GuestControlClient, ProcessState, RequestAction, ResponseResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::dev_bundle::{DevBundleDescriptor, ARTIFACT_ROOT_ENV, GUEST_BUNDLE_ROOT};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const HELLO_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const READY_TIMEOUT: Duration = Duration::from_secs(90);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const PAYLOAD_INSTALLER: &str = "/run/qol-payload/installer/qol-sandbox-payload";
const PAYLOAD_ROOT: &str = "/run/qol-payload";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DevSessionEvidence {
    pub(super) bundle_manifest_sha256: String,
    pub(super) plugin_count: usize,
    pub(super) unit: String,
}

pub(super) fn start(
    report_path: &Path,
    run_id: &str,
    environment_id: &str,
    image_revision: &str,
    bundle_manifest_sha256: &str,
    descriptor: &DevBundleDescriptor,
    mut cancelled: impl FnMut() -> bool,
) -> Result<DevSessionEvidence> {
    let mut guest = connect_guest(
        report_path,
        run_id,
        environment_id,
        image_revision,
        &mut cancelled,
    )?;
    install_bundle(&mut guest, &mut cancelled)?;
    let unit = launch_dev_unit(&mut guest, run_id, &mut cancelled)?;
    let plugin_ids = descriptor
        .plugins
        .iter()
        .map(|plugin| plugin.id.as_str())
        .collect::<Vec<_>>();
    wait_until_ready(&mut guest, &unit, &plugin_ids, &mut cancelled)?;
    Ok(DevSessionEvidence {
        bundle_manifest_sha256: bundle_manifest_sha256.to_string(),
        plugin_count: descriptor.plugins.len(),
        unit,
    })
}

fn connect_guest(
    report_path: &Path,
    run_id: &str,
    environment_id: &str,
    image_revision: &str,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<GuestControlClient> {
    let port = guest_control_port(report_path)?;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    GuestControlClient::connect_verified_identity_cancellable(
        address,
        CONNECT_TIMEOUT,
        HELLO_TIMEOUT,
        environment_id,
        image_revision,
        run_id,
        cancelled,
    )
}

fn install_bundle(
    guest: &mut GuestControlClient,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<()> {
    require_process(
        guest,
        RequestAction::Exec {
            command: command(
                "/usr/bin/python3",
                &[PAYLOAD_INSTALLER, "install", PAYLOAD_ROOT],
            ),
            timeout_ms: duration_millis(INSTALL_TIMEOUT)?,
        },
        INSTALL_TIMEOUT + REQUEST_TIMEOUT,
        cancelled,
    )
}

fn launch_dev_unit(
    guest: &mut GuestControlClient,
    run_id: &str,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<String> {
    let unit = format!("qol-dev-{run_id}");
    let program = format!("{GUEST_BUNDLE_ROOT}/bin/qol");
    let working_directory = format!("--working-directory={GUEST_BUNDLE_ROOT}");
    let artifact_root = format!("--setenv={ARTIFACT_ROOT_ENV}={GUEST_BUNDLE_ROOT}");
    require_process(
        guest,
        RequestAction::Exec {
            command: command(
                "/usr/bin/systemd-run",
                &[
                    "--user",
                    "--unit",
                    &unit,
                    "--property=StandardOutput=journal",
                    "--property=StandardError=journal",
                    &working_directory,
                    &artifact_root,
                    &program,
                    "dev",
                ],
            ),
            timeout_ms: duration_millis(REQUEST_TIMEOUT)?,
        },
        REQUEST_TIMEOUT + Duration::from_secs(1),
        cancelled,
    )?;
    Ok(unit)
}

fn guest_control_port(report_path: &Path) -> Result<u16> {
    let content = fs::read(report_path)
        .with_context(|| format!("failed to read lane report {}", report_path.display()))?;
    let report: Value = serde_json::from_slice(&content)
        .with_context(|| format!("invalid lane report {}", report_path.display()))?;
    report
        .get("guest_control")
        .and_then(|control| control.get("port"))
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .context("lane report has no valid guest-control port")
}

fn wait_until_ready(
    guest: &mut GuestControlClient,
    unit: &str,
    plugin_ids: &[&str],
    cancelled: &mut impl FnMut() -> bool,
) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    let url = format!("{}/api/installed", qol_conventions::local_base_url());
    loop {
        if cancelled() {
            bail!("guest qol dev startup cancelled");
        }
        let active_state = unit_state(guest, unit, cancelled)?;
        if active_state == "failed" || active_state == "inactive" || active_state.is_empty() {
            bail!(
                "guest qol dev unit {unit} stopped before readiness with state `{active_state}`; inspect it with `journalctl --user -u {unit}` in the guest"
            );
        }
        let installed = installed_plugins(guest, &url, cancelled)?;
        let ready = active_state == "active"
            && installed.state == ProcessState::Exited
            && installed.exit_code == Some(0)
            && plugin_ids.iter().all(|id| installed.stdout.contains(id));
        if ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for artifact-backed qol dev inside the guest");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn unit_state(
    guest: &mut GuestControlClient,
    unit: &str,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<String> {
    let outcome = process(
        guest,
        RequestAction::Exec {
            command: command(
                "/usr/bin/systemctl",
                &["--user", "show", "--property=ActiveState", "--value", unit],
            ),
            timeout_ms: duration_millis(REQUEST_TIMEOUT)?,
        },
        REQUEST_TIMEOUT + Duration::from_secs(1),
        cancelled,
    )?;
    Ok(outcome.stdout.trim().to_string())
}

fn installed_plugins(
    guest: &mut GuestControlClient,
    url: &str,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<qol_dev_guest::ProcessOutcome> {
    process(
        guest,
        RequestAction::Exec {
            command: command("/usr/bin/curl", &["--fail", "--silent", url]),
            timeout_ms: duration_millis(REQUEST_TIMEOUT)?,
        },
        REQUEST_TIMEOUT + Duration::from_secs(1),
        cancelled,
    )
}

fn require_process(
    guest: &mut GuestControlClient,
    action: RequestAction,
    timeout: Duration,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<()> {
    let outcome = process(guest, action, timeout, cancelled)?;
    if outcome.state == ProcessState::Exited && outcome.exit_code == Some(0) {
        return Ok(());
    }
    bail!(
        "guest command failed: state={:?}, exit={:?}, stderr={}",
        outcome.state,
        outcome.exit_code,
        outcome.stderr.trim()
    )
}

fn process(
    guest: &mut GuestControlClient,
    action: RequestAction,
    timeout: Duration,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<qol_dev_guest::ProcessOutcome> {
    match guest.request_cancellable(action, timeout, cancelled)? {
        ResponseResult::Process { outcome } => Ok(outcome),
        result => bail!("guest command returned an unexpected response: {result:?}"),
    }
}

fn command(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        cwd: None,
        env: BTreeMap::new(),
    }
}

fn duration_millis(duration: Duration) -> Result<u64> {
    u64::try_from(duration.as_millis()).context("guest command timeout is too large")
}
