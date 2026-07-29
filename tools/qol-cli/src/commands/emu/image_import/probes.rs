use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use qol_dev_env::EnvironmentDefinition;
use qol_dev_guest::{CommandSpec, GuestControlClient, ProcessState, RequestAction, ResponseResult};
use serde_json::{json, Value};

use super::super::guest::GuestAdapter;
use super::plan::required_capability;
use super::{ImageImportPlan, ImportCancellation};

const GUEST_CONNECT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GUEST_HELLO_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const GUEST_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub(super) struct Verification {
    pub(super) verdict: &'static str,
    pub(super) probes: Vec<Value>,
    pub(super) error: Option<String>,
}

impl Verification {
    pub(super) fn pass(probes: Vec<Value>) -> Self {
        Self {
            verdict: "pass",
            probes,
            error: None,
        }
    }

    pub(super) fn failed(probes: Vec<Value>, error: impl Into<String>) -> Self {
        Self {
            verdict: "failed",
            probes,
            error: Some(error.into()),
        }
    }

    pub(super) fn cancelled(probes: Vec<Value>) -> Self {
        Self {
            verdict: "cancelled",
            probes,
            error: Some("image import cancelled".to_string()),
        }
    }
}

#[derive(Clone, Debug)]
struct ProbeSpec {
    id: &'static str,
    command: CommandSpec,
    expected: String,
}

pub(super) fn verify_guest(
    plan: &ImageImportPlan,
    vm: &super::super::BootedVm,
    cancellation: &ImportCancellation,
) -> Result<Verification> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), vm.guest_control_port);
    let mut guest = GuestControlClient::connect_verified_identity_cancellable(
        address,
        GUEST_CONNECT_TIMEOUT,
        GUEST_HELLO_TIMEOUT,
        &plan.environment.id,
        &plan.guest_revision,
        &plan.run_id,
        || cancellation.is_requested(),
    )?;
    let specs = match plan.guest_adapter {
        GuestAdapter::MintCinnamon => mint_probe_specs(&plan.environment)?,
        GuestAdapter::DebianNocloud => {
            bail!("guest adapter `debian-nocloud` has no verified image-import probe set")
        }
        GuestAdapter::MacosDesktop | GuestAdapter::WindowsDesktop => bail!(
            "guest adapter `{}` has no verified image-import probe set",
            plan.guest_adapter.as_str()
        ),
    };
    let mut probes = Vec::with_capacity(specs.len());
    for spec in specs {
        if cancellation.is_requested() {
            return Ok(Verification::cancelled(probes));
        }
        match run_probe(&mut guest, &spec, cancellation) {
            Ok(probe) => probes.push(probe),
            Err(_error) if cancellation.is_requested() => {
                return Ok(Verification::cancelled(probes))
            }
            Err(error) => {
                probes.push(json!({
                    "id": spec.id,
                    "verdict": "failed",
                    "expected": spec.expected,
                    "error": format!("{error:#}"),
                }));
                return Ok(Verification::failed(probes, format!("{error:#}")));
            }
        }
    }
    Ok(Verification::pass(probes))
}

fn mint_probe_specs(definition: &EnvironmentDefinition) -> Result<Vec<ProbeSpec>> {
    let release = required_capability(definition, "mint_release")?;
    let edition = required_capability(definition, "mint_edition")?;
    let cinnamon = required_capability(definition, "cinnamon_version")?;
    Ok(vec![
        ProbeSpec {
            id: "linux-mint-release",
            command: command(
                "/usr/bin/grep",
                &["-Fx", &format!("RELEASE={release}"), "/etc/linuxmint/info"],
            ),
            expected: format!("RELEASE={release}"),
        },
        ProbeSpec {
            id: "linux-mint-edition",
            command: command(
                "/usr/bin/grep",
                &[
                    "-Fx",
                    &format!("EDITION=\"{edition}\""),
                    "/etc/linuxmint/info",
                ],
            ),
            expected: format!("EDITION=\"{edition}\""),
        },
        ProbeSpec {
            id: "cinnamon-version",
            command: command("/usr/bin/cinnamon", &["--version"]),
            expected: format!("Cinnamon {cinnamon}"),
        },
    ])
}

fn run_probe(
    guest: &mut GuestControlClient,
    spec: &ProbeSpec,
    cancellation: &ImportCancellation,
) -> Result<Value> {
    let timeout_ms = u64::try_from(GUEST_COMMAND_TIMEOUT.as_millis())
        .context("guest command timeout is too large")?;
    let response = guest.request_cancellable(
        RequestAction::Exec {
            command: spec.command.clone(),
            timeout_ms,
        },
        GUEST_COMMAND_TIMEOUT + Duration::from_secs(2),
        || cancellation.is_requested(),
    )?;
    let ResponseResult::Process { outcome } = response else {
        bail!("guest probe `{}` returned an unexpected response", spec.id);
    };
    if outcome.state != ProcessState::Exited || outcome.exit_code != Some(0) {
        bail!(
            "guest probe `{}` failed: state={:?}, exit={:?}, stderr={}",
            spec.id,
            outcome.state,
            outcome.exit_code,
            outcome.stderr.trim()
        );
    }
    let observed = outcome.stdout.trim();
    if observed != spec.expected {
        bail!(
            "guest probe `{}` expected `{}`, got `{observed}`",
            spec.id,
            spec.expected
        );
    }
    Ok(json!({
        "id": spec.id,
        "verdict": "pass",
        "expected": spec.expected,
        "observed": observed,
        "program": spec.command.program,
        "args": spec.command.args,
    }))
}

fn command(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_string(),
        args: args.iter().map(|value| (*value).to_string()).collect(),
        cwd: None,
        env: BTreeMap::new(),
    }
}
