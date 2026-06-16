use crate::progress::{begin_run_log, print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod arch;
mod control;
mod discovery;
mod guest;
mod live;
mod machine;
mod media;
mod platform;
mod qmp;
mod registry;
mod serial;
mod workflow;

#[allow(unused_imports)]
pub(crate) use arch::{Firmware, GuestArch};
pub(crate) use discovery::{parse_emu_dir, Discovered, DiscoveryContext, ImageCandidate};
pub(crate) use media::BootMedia;
pub(crate) use registry::register_image;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Environment {
    id: String,
    name: String,
    backend: String,
    arch: GuestArch,
    image_path: PathBuf,
    source: String,
    firmware: Firmware,
    media: BootMedia,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolveState {
    Ready,
    Missing,
    Unsupported,
}

impl ResolveState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Missing => "missing",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvironmentStatus {
    pub(crate) id: String,
    pub(crate) backend: String,
    pub(crate) state: ResolveState,
    pub(crate) reason: String,
    pub(crate) last_run: Option<LastRun>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LastRun {
    pub(crate) status: String,
    pub(crate) finished_at_unix_ms: u64,
}

#[derive(Clone, Debug)]
struct Resolution {
    state: ResolveState,
    reason: String,
    image_path: PathBuf,
    qemu_system: Option<PathBuf>,
    qemu_img: Option<PathBuf>,
    acceleration: &'static str,
    firmware: Option<PathBuf>,
}

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        print_emu_help();
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "list" => cmd_list(rest, verbose),
        "add" => cmd_add(rest, verbose),
        "open" => cmd_open(rest, verbose),
        "doctor" => cmd_doctor(rest, verbose),
        "up" => cmd_up(rest, verbose),
        "run" => cmd_run(rest, verbose),
        "check" => cmd_check(rest, verbose),
        "shot" => control::cmd_shot(rest, verbose),
        "key" => control::cmd_key(rest, verbose),
        "insert" => control::cmd_insert(rest, verbose),
        "pull" => control::cmd_pull(rest, verbose),
        "snap" => control::cmd_snap(rest, verbose),
        "sh" => control::cmd_sh(rest, verbose),
        "down" => control::cmd_down(rest, verbose),
        "help" | "-h" | "--help" => {
            print_emu_help();
            Ok(())
        }
        other => bail!("unknown emu command `{other}`\n\n{}", emu_help_text()),
    }
}

fn statuses_for(environments: Vec<Environment>) -> Vec<EnvironmentStatus> {
    let mut last_runs = last_runs_by_id();
    environments
        .into_iter()
        .map(|environment| {
            let resolution = resolve_environment(&environment);
            EnvironmentStatus {
                last_run: last_runs.remove(&environment.id),
                id: environment.id,
                backend: environment.backend,
                state: resolution.state,
                reason: resolution.reason,
            }
        })
        .collect()
}

pub(crate) fn emu_scan() -> Result<(Vec<EnvironmentStatus>, Vec<ImageCandidate>)> {
    let discovered = discover_all()?;
    Ok((statuses_for(discovered.environments), discovered.candidates))
}

fn last_runs_by_id() -> HashMap<String, LastRun> {
    let mut latest = HashMap::new();
    let Some(root) = repo_root().ok() else {
        return latest;
    };
    let Ok(entries) = fs::read_dir(root.join("target/qol-emu")) else {
        return latest;
    };
    for entry in entries.flatten() {
        let Ok(content) = fs::read_to_string(entry.path().join("report.json")) else {
            continue;
        };
        let Ok(report) = serde_json::from_str(&content) else {
            continue;
        };
        let Some((id, run)) = last_run_from_report(&report) else {
            continue;
        };
        let newer = latest
            .get(&id)
            .is_none_or(|existing| run.finished_at_unix_ms > existing.finished_at_unix_ms);
        if newer {
            latest.insert(id, run);
        }
    }
    latest
}

pub(crate) struct RunDetail {
    pub(crate) run_dir: PathBuf,
    pub(crate) arch: String,
    pub(crate) image_path: String,
    pub(crate) acceleration: String,
}

impl RunDetail {
    pub(crate) fn run_log(&self) -> PathBuf {
        self.run_dir.join("run.log")
    }
}

pub(crate) fn newest_run_detail(id: &str) -> Option<RunDetail> {
    let root = repo_root().ok()?;
    let entries = fs::read_dir(root.join("target/qol-emu")).ok()?;
    let mut best: Option<(u64, RunDetail)> = None;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(content) = fs::read_to_string(dir.join("report.json")) else {
            continue;
        };
        let Ok(report) = serde_json::from_str::<serde_json::Value>(&content) else {
            continue;
        };
        if report_str(&report, &["environment", "id"]) != Some(id.to_string()) {
            continue;
        }
        let finished = report
            .get("finished_at_unix_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if best.as_ref().is_some_and(|(newest, _)| *newest >= finished) {
            continue;
        }
        best = Some((
            finished,
            RunDetail {
                run_dir: dir,
                arch: report_str(&report, &["environment", "arch"]).unwrap_or_default(),
                image_path: report_str(&report, &["environment", "image_path"]).unwrap_or_default(),
                acceleration: report_str(&report, &["resolution", "acceleration"])
                    .unwrap_or_default(),
            },
        ));
    }
    best.map(|(_, detail)| detail)
}

fn report_str(report: &serde_json::Value, path: &[&str]) -> Option<String> {
    let mut node = report;
    for key in path {
        node = node.get(key)?;
    }
    node.as_str().map(str::to_string)
}

fn last_run_from_report(report: &serde_json::Value) -> Option<(String, LastRun)> {
    let id = report.get("environment")?.get("id")?.as_str()?.to_string();
    let status = report.get("status")?.as_str()?.to_string();
    let finished_at_unix_ms = report.get("finished_at_unix_ms")?.as_u64()?;
    Some((
        id,
        LastRun {
            status,
            finished_at_unix_ms,
        },
    ))
}

pub(crate) fn emu_config_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join("emu.toml"))
}

pub(crate) fn emu_dir() -> Option<PathBuf> {
    let override_dir = emu_config_path()
        .filter(|path| path.is_file())
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|content| parse_emu_dir(&content, dirs::home_dir().as_ref()));
    resolve_emu_dir(override_dir, qol_config::data_subdir("emu"))
}

fn resolve_emu_dir(override_dir: Option<PathBuf>, default_dir: Option<PathBuf>) -> Option<PathBuf> {
    override_dir.or(default_dir)
}

fn cmd_list(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol emu list");
    }
    print_title("qol emu");
    print_hint(verbose);
    let (statuses, candidates) = emu_scan()?;
    if statuses.is_empty() && candidates.is_empty() {
        step_label("env", StepKind::Info, "no emus found");
        if let Some(path) = emu_config_path() {
            step_label("config", StepKind::Info, &path.display().to_string());
        }
        return Ok(());
    }
    for status in statuses {
        step_label(
            "env",
            kind_for_resolution(status.state),
            &format!(
                "{} · {} · {} · {}",
                status.id,
                status.state.as_str(),
                status.backend,
                status.reason
            ),
        );
    }
    for candidate in candidates {
        step_label(
            candidate.media.as_str(),
            StepKind::Info,
            &format!(
                "{} · candidate · {} · `qol emu up {}` to boot",
                candidate.id,
                candidate.arch.as_str(),
                candidate.id
            ),
        );
    }
    Ok(())
}

fn cmd_doctor(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol emu doctor");
    }
    print_title("qol emu doctor");
    print_hint(verbose);
    for arch in GuestArch::ALL {
        match find_on_path(arch.qemu_system_binary()) {
            Some(path) => step_label(
                arch.as_str(),
                StepKind::Success,
                &format!("{} \u{b7} {}", path.display(), platform::acceleration(arch)),
            ),
            None => step_label(
                arch.as_str(),
                StepKind::Info,
                &format!("missing {}", arch.qemu_system_binary()),
            ),
        }
    }
    match find_on_path("qemu-img") {
        Some(path) => step_label("qemu-img", StepKind::Success, &path.display().to_string()),
        None => step_label("qemu-img", StepKind::Info, "missing qemu-img"),
    }
    match find_on_path("virsh") {
        Some(path) => step_label("virsh", StepKind::Success, &path.display().to_string()),
        None => step_label(
            "virsh",
            StepKind::Info,
            "missing virsh (libvirt discovery disabled)",
        ),
    }
    if let Some(path) = emu_config_path() {
        step_label("config", StepKind::Info, &path.display().to_string());
    }
    let found = discover_environments()?.len();
    step_label("found", StepKind::Info, &format!("{found} emus"));
    let root = repo_root()?;
    step_label(
        "runs",
        StepKind::Info,
        &root.join("target/qol-emu").display().to_string(),
    );
    if let Some(dir) = emu_dir() {
        step_label("emu-dir", StepKind::Info, &dir.display().to_string());
    }
    Ok(())
}

fn cmd_up(args: &[OsString], verbose: bool) -> Result<()> {
    if args.len() != 1 {
        bail!("usage: qol emu up <environment>");
    }
    let target = args[0]
        .to_str()
        .ok_or_else(|| anyhow!("environment id is not valid UTF-8"))?;
    print_title("qol emu up");
    print_hint(verbose);
    let mut vm = boot_vm(target, "up", verbose)?;
    step_label(
        "running",
        StepKind::Success,
        "close the VM window to end the run",
    );
    let exit = vm.child.wait().context("failed to wait for qemu")?;
    let (report_path, removed) = finalize_vm(vm, exit, None, "up")?;
    step_label(
        "clean",
        StepKind::Success,
        &format!("removed {} disposable file(s)", removed.len()),
    );
    step_label("report", StepKind::Info, &report_path.display().to_string());
    if !exit.success() {
        bail!("qemu exited with {exit}");
    }
    Ok(())
}

fn cmd_run(args: &[OsString], verbose: bool) -> Result<()> {
    if args.len() != 2 {
        bail!("usage: qol emu run <workflow> <environment>");
    }
    let workflow_id = args[0]
        .to_str()
        .ok_or_else(|| anyhow!("workflow id is not valid UTF-8"))?;
    let target = args[1]
        .to_str()
        .ok_or_else(|| anyhow!("environment id is not valid UTF-8"))?;
    run_workflow(workflow_id, target, "run", verbose)
}

fn cmd_check(args: &[OsString], verbose: bool) -> Result<()> {
    if args.len() != 1 {
        bail!("usage: qol emu check <environment>");
    }
    let target = args[0]
        .to_str()
        .ok_or_else(|| anyhow!("environment id is not valid UTF-8"))?;
    run_workflow("leaves-no-trace", target, "check", verbose)
}

fn run_workflow(workflow_id: &str, target: &str, command_name: &str, verbose: bool) -> Result<()> {
    let Some(workflow_fn) = workflow::find(workflow_id) else {
        bail!(
            "unknown workflow `{workflow_id}`; available: {}",
            workflow::ids().join(", ")
        );
    };
    print_title(&format!("qol emu {command_name}"));
    print_hint(verbose);
    let mut vm = boot_vm(target, command_name, verbose)?;
    let outcome = drive_workflow(&vm, workflow_fn);
    let exit = shutdown_vm(&mut vm)?;
    let workflow_report = match &outcome {
        Ok(verdict) => json!({
            "id": workflow_id,
            "verdict": if verdict.pass { "pass" } else { "fail" },
            "traces": verdict.traces,
        }),
        Err(error) => json!({
            "id": workflow_id,
            "verdict": "error",
            "error": error.to_string(),
        }),
    };
    let (report_path, removed) = finalize_vm(vm, exit, Some(workflow_report), command_name)?;
    step_label(
        "clean",
        StepKind::Success,
        &format!("removed {} disposable file(s)", removed.len()),
    );
    step_label("report", StepKind::Info, &report_path.display().to_string());
    let verdict = outcome?;
    if !verdict.pass {
        bail!(
            "{workflow_id} failed; traces: {}",
            verdict.traces.join(", ")
        );
    }
    step_label("verdict", StepKind::Success, "pass · no qol traces survive");
    Ok(())
}

fn drive_workflow(vm: &BootedVm, workflow_fn: workflow::Workflow) -> Result<workflow::Verdict> {
    let qemu_img = vm
        .resolution
        .qemu_img
        .clone()
        .ok_or_else(|| anyhow!("ready environment has no qemu-img path"))?;
    let stick = machine::ensure_usb_stick(&vm.run_dir, &qemu_img)?;
    let mut qmp = qmp::connect(vm.qmp_port, Duration::from_secs(10))?;
    let mut serial = serial::connect(vm.serial_port, Duration::from_secs(10))?;
    let os = guest::DebianNocloud;
    step_label("login", StepKind::Pending, "waiting for a root shell");
    guest::GuestOs::ensure_root_shell(&os, &mut serial)?;
    step_label("login", StepKind::Success, "root shell over serial");
    let mut run = workflow::Run {
        qmp: &mut qmp,
        serial: &mut serial,
        os: &os,
        stick: &stick,
    };
    workflow_fn(&mut run)
}

fn shutdown_vm(vm: &mut BootedVm) -> Result<ExitStatus> {
    match qmp::connect(vm.qmp_port, Duration::from_secs(5)) {
        Ok(mut client) => {
            let _ = client.fire("quit");
        }
        Err(_) => {
            let _ = vm.child.kill();
        }
    }
    vm.child.wait().context("failed to wait for qemu")
}

struct BootedVm {
    environment: Environment,
    resolution: Resolution,
    run_dir: PathBuf,
    qemu_command_path: PathBuf,
    commands: Vec<serde_json::Value>,
    qmp_port: u16,
    serial_port: u16,
    qemu_version: String,
    vm_status: String,
    child: std::process::Child,
    started_at: u64,
}

fn environment_from_candidate(candidate: &ImageCandidate) -> Environment {
    Environment {
        id: candidate.id.clone(),
        name: candidate.display_name.clone(),
        backend: "qemu".to_string(),
        arch: candidate.arch,
        image_path: candidate.path.clone(),
        source: "candidate".to_string(),
        firmware: candidate.firmware,
        media: candidate.media,
    }
}

fn boot_vm(target: &str, command_name: &str, verbose: bool) -> Result<BootedVm> {
    let root = repo_root()?;
    let discovered = discover_all()?;
    if discovered.environments.is_empty() && discovered.candidates.is_empty() {
        bail!("no emus found; drop a disk image or .iso into the emu dir (`qol emu open`), create a libvirt/QEMU VM, or add [images] to ~/.config/qol-tray/emu.toml");
    }
    let environment = discovered
        .environments
        .iter()
        .find(|environment| environment.id == target)
        .cloned()
        .or_else(|| {
            discovered
                .candidates
                .iter()
                .find(|candidate| candidate.id == target)
                .map(environment_from_candidate)
        })
        .ok_or_else(|| anyhow!("unknown emu `{target}`; run `qol emu list`"))?;
    let resolution = resolve_environment(&environment);
    let started_at = unix_millis()?;
    let run_dir = root
        .join("target/qol-emu")
        .join(format!("{}-{started_at}", environment.id));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;
    begin_run_log(&run_dir.join("run.log"));

    step_label(
        "resolve",
        kind_for_resolution(resolution.state),
        &format!("{} · {}", environment.id, resolution.reason),
    );

    if resolution.state != ResolveState::Ready {
        let report = report_json(ReportInput {
            environment: &environment,
            resolution: &resolution,
            run_dir: &run_dir,
            status: "skipped",
            overlay: None,
            qemu_command: None,
            commands: Vec::new(),
            qmp: None,
            serial: None,
            workflow: None,
            teardown: None,
            next: next_for_resolution(&environment, &resolution),
            started_at,
        })?;
        write_report(&run_dir, &report)?;
        step_label(
            "report",
            StepKind::Info,
            &run_dir.join("report.json").display().to_string(),
        );
        bail!("emu `{}` is {}", environment.id, resolution.state.as_str());
    }

    let (boot_disk, overlay_artifact, disk_commands) = match environment.media {
        BootMedia::Iso => (resolution.image_path.clone(), None, Vec::new()),
        BootMedia::Disk => {
            let qemu_img = resolution
                .qemu_img
                .clone()
                .ok_or_else(|| anyhow!("ready environment has no qemu-img path"))?;
            let image_format = detect_image_format(&qemu_img, &resolution.image_path, verbose)?;
            let overlay = run_dir.join("overlay.qcow2");
            let create_args = vec![
                "create".to_string(),
                "-f".to_string(),
                "qcow2".to_string(),
                "-F".to_string(),
                image_format.clone(),
                "-b".to_string(),
                resolution.image_path.display().to_string(),
                overlay.display().to_string(),
            ];
            step_label("clone", StepKind::Pending, &overlay.display().to_string());
            let status = run_child_status(&qemu_img, &create_args, verbose)?;
            if !status.success() {
                let report = report_json(ReportInput {
                    environment: &environment,
                    resolution: &resolution,
                    run_dir: &run_dir,
                    status: "failed",
                    overlay: Some(&overlay),
                    qemu_command: None,
                    commands: vec![json!({
                        "program": qemu_img,
                        "args": create_args,
                        "status": status.to_string(),
                    })],
                    qmp: None,
                    serial: None,
                    workflow: None,
                    teardown: None,
                    next: vec![format!("Inspect the qemu-img output, remove the run directory if needed, then rerun `qol emu {command_name}`.")],
                    started_at,
                })?;
                write_report(&run_dir, &report)?;
                bail!("qemu-img failed with {status}");
            }
            let commands = vec![
                json!({
                    "program": qemu_img,
                    "args": ["info", "--output=json", &resolution.image_path.display().to_string()],
                    "detected_format": image_format,
                }),
                json!({
                    "program": qemu_img,
                    "args": create_args,
                    "status": status.to_string(),
                }),
            ];
            (overlay.clone(), Some(overlay), commands)
        }
    };

    let qemu_system = resolution
        .qemu_system
        .clone()
        .ok_or_else(|| anyhow!("ready environment has no qemu-system path"))?;
    let qmp_port = machine::free_qmp_port()?;
    let serial_port = machine::free_qmp_port()?;
    let qemu_args = qemu_args(
        &environment,
        &boot_disk,
        resolution.acceleration,
        platform::display(),
        qmp_port,
        serial_port,
        resolution.firmware.as_deref(),
    );
    let qemu_command = command_line(&qemu_system, &qemu_args);
    let qemu_command_path = run_dir.join("qemu-command.txt");
    fs::write(&qemu_command_path, format!("{qemu_command}\n"))
        .with_context(|| format!("failed to write {}", qemu_command_path.display()))?;
    let mut commands = disk_commands;
    commands.push(json!({
        "program": qemu_system,
        "args": qemu_args,
    }));

    step_label(
        "boot",
        StepKind::Pending,
        &format!("{} · qmp 127.0.0.1:{qmp_port}", environment.id),
    );
    let mut child = machine::spawn_qemu(&qemu_system, &qemu_args)?;
    let handshake = qmp::connect(qmp_port, Duration::from_secs(10)).and_then(|mut client| {
        let status = client.query_status()?;
        Ok((client.qemu_version.clone(), status))
    });
    let (qemu_version, vm_status) = match handshake {
        Ok(values) => values,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            let removed = machine::teardown(&run_dir)?;
            let report = report_json(ReportInput {
                environment: &environment,
                resolution: &resolution,
                run_dir: &run_dir,
                status: "failed",
                overlay: overlay_artifact.as_deref(),
                qemu_command: Some(&qemu_command_path),
                commands,
                qmp: Some(json!({ "port": qmp_port, "error": error.to_string() })),
                serial: Some(json!({ "port": serial_port })),
                workflow: None,
                teardown: Some(json!({ "removed": removed })),
                next: vec![format!(
                    "Inspect the qemu output above, then rerun `qol emu {command_name}`."
                )],
                started_at,
            })?;
            write_report(&run_dir, &report)?;
            bail!("qmp handshake failed: {error:#}");
        }
    };
    step_label(
        "qmp",
        StepKind::Success,
        &format!("qemu {qemu_version} · {vm_status}"),
    );
    let report = report_json(ReportInput {
        environment: &environment,
        resolution: &resolution,
        run_dir: &run_dir,
        status: "running",
        overlay: overlay_artifact.as_deref(),
        qemu_command: Some(&qemu_command_path),
        commands: commands.clone(),
        qmp: Some(json!({ "port": qmp_port, "qemu_version": qemu_version, "status": vm_status })),
        serial: Some(json!({ "port": serial_port })),
        workflow: None,
        teardown: None,
        next: vec!["Close the VM window (or shut the guest down) to end the run.".to_string()],
        started_at,
    })?;
    write_report(&run_dir, &report)?;
    Ok(BootedVm {
        environment,
        resolution,
        run_dir,
        qemu_command_path,
        commands,
        qmp_port,
        serial_port,
        qemu_version,
        vm_status,
        child,
        started_at,
    })
}

fn finalize_vm(
    vm: BootedVm,
    exit: ExitStatus,
    workflow: Option<serde_json::Value>,
    command_name: &str,
) -> Result<(PathBuf, Vec<PathBuf>)> {
    let removed = machine::teardown(&vm.run_dir)?;
    let final_status = if exit.success() { "pass" } else { "failed" };
    let report = report_json(ReportInput {
        environment: &vm.environment,
        resolution: &vm.resolution,
        run_dir: &vm.run_dir,
        status: final_status,
        overlay: None,
        qemu_command: Some(&vm.qemu_command_path),
        commands: vm.commands.clone(),
        qmp: Some(
            json!({ "port": vm.qmp_port, "qemu_version": vm.qemu_version, "status": vm.vm_status }),
        ),
        serial: Some(json!({ "port": vm.serial_port })),
        workflow,
        teardown: Some(json!({ "removed": removed, "exit": exit.to_string() })),
        next: vec![format!(
            "Rerun `qol emu {command_name}` for a fresh disposable clone."
        )],
        started_at: vm.started_at,
    })?;
    write_report(&vm.run_dir, &report)?;
    Ok((vm.run_dir.join("report.json"), removed))
}

const ADD_SYNTAX: &str =
    "qol emu add <path> [--arch x86_64|aarch64] [--firmware bios|uefi] [--id <id>]";

struct AddArgs {
    path: PathBuf,
    arch: Option<GuestArch>,
    firmware: Option<Firmware>,
    id: Option<String>,
}

fn parse_add_args(args: &[OsString]) -> Result<AddArgs> {
    let mut path: Option<PathBuf> = None;
    let mut arch: Option<GuestArch> = None;
    let mut firmware: Option<Firmware> = None;
    let mut id: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--arch") => {
                let value = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .context("--arch needs a value")?;
                arch = Some(
                    GuestArch::parse(value)
                        .ok_or_else(|| anyhow!("--arch must be one of: x86_64, aarch64"))?,
                );
            }
            Some("--firmware") => {
                let value = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .context("--firmware needs a value")?;
                firmware = Some(
                    Firmware::parse(value)
                        .ok_or_else(|| anyhow!("--firmware must be one of: bios, uefi"))?,
                );
            }
            Some("--id") => {
                let value = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .context("--id needs a value")?;
                id = Some(sanitize_id(value));
            }
            _ => {
                if path.is_some() {
                    bail!("usage: {ADD_SYNTAX}");
                }
                path = Some(PathBuf::from(arg));
            }
        }
    }
    Ok(AddArgs {
        path: path.with_context(|| format!("usage: {ADD_SYNTAX}"))?,
        arch,
        firmware,
        id,
    })
}

fn cmd_add(args: &[OsString], verbose: bool) -> Result<()> {
    print_title("qol emu add");
    print_hint(verbose);
    let parsed = parse_add_args(args)?;
    let mut candidate = discovery::infer_candidate(&parsed.path);
    if let Some(arch) = parsed.arch {
        candidate.arch = arch;
        candidate.arch_inferred = false;
    }
    if let Some(firmware) = parsed.firmware {
        candidate.firmware = firmware;
    }
    if let Some(id) = parsed.id {
        candidate.id = id;
    }
    let qemu_img = find_on_path("qemu-img").context("missing qemu-img")?;
    let emu_toml = emu_config_path().context("could not determine emu.toml path")?;
    let id = register_image(&emu_toml, &candidate, &qemu_img)?;
    step_label("add", StepKind::Info, &format!("registered {id}"));
    Ok(())
}

fn cmd_open(args: &[OsString], _verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol emu open");
    }
    let dir = emu_dir().context("could not determine emu dir")?;
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    print_title("qol emu open");
    step_label("dir", StepKind::Info, &dir.display().to_string());
    if env::var_os("DISPLAY").is_none()
        && env::var_os("WAYLAND_DISPLAY").is_none()
        && crate::host_facade::os_name() == "linux"
    {
        return Ok(());
    }
    crate::host_facade::open_path(&dir);
    Ok(())
}

fn print_emu_help() {
    print!("{}", emu_help_text());
}

fn emu_help_text() -> String {
    format!("qol emu commands:\n  qol emu list\n  {ADD_SYNTAX}\n  qol emu open\n  qol emu doctor\n  qol emu up <environment>\n  qol emu run <workflow> <environment>\n  qol emu check <environment>\n  qol emu shot <environment>\n  qol emu key <environment> <qcode>...\n  qol emu insert <environment>\n  qol emu pull <environment>\n  qol emu snap <environment>\n  qol emu sh <environment> <command>...\n  qol emu down <environment>\n\nControl verbs target the newest running `qol emu up` for that environment.\n\nEmus are discovered from libvirt/QEMU domains plus optional local config:\n  ~/.config/qol-tray/emu.toml\n\nDrop a disk image or .iso into the emu dir (`qol emu open`) and it appears in\n`qol emu list`; `qol emu up <id>` boots it (an .iso boots as a disposable live CD).\n\nExample config:\n  [images]\n  my-windows = \"/path/to/windows.qcow2\"\n")
}

pub(crate) fn discover_all() -> Result<Discovered> {
    discovery::discover(DiscoveryContext {
        config_path: emu_config_path(),
        home_dir: dirs::home_dir(),
        virsh: find_on_path("virsh"),
        libvirt_uris: platform::libvirt_uris(),
        emu_dir: emu_dir().unwrap_or_default(),
    })
}

fn discover_environments() -> Result<Vec<Environment>> {
    Ok(discover_all()?.environments)
}

fn resolve_environment(environment: &Environment) -> Resolution {
    let qemu_system = find_on_path(environment.arch.qemu_system_binary());
    let qemu_img = find_on_path("qemu-img");
    let acceleration = platform::acceleration(environment.arch);
    if qemu_system.is_none() {
        return Resolution {
            state: ResolveState::Unsupported,
            reason: format!("missing {}", environment.arch.qemu_system_binary()),
            image_path: environment.image_path.clone(),
            qemu_system,
            qemu_img,
            acceleration,
            firmware: None,
        };
    }
    if environment.media == BootMedia::Disk && qemu_img.is_none() {
        return Resolution {
            state: ResolveState::Unsupported,
            reason: "missing qemu-img".to_string(),
            image_path: environment.image_path.clone(),
            qemu_system,
            qemu_img,
            acceleration,
            firmware: None,
        };
    }
    let firmware = match qemu_system.as_deref() {
        Some(path) => match locate_firmware(path, environment.arch, environment.firmware) {
            Ok(firmware) => firmware,
            Err(reason) => {
                return Resolution {
                    state: ResolveState::Unsupported,
                    reason,
                    image_path: environment.image_path.clone(),
                    qemu_system,
                    qemu_img,
                    acceleration,
                    firmware: None,
                }
            }
        },
        None => None,
    };
    match image_path_status(&environment.image_path) {
        Ok(canonical) => Resolution {
            state: ResolveState::Ready,
            reason: format!("{} · {}", environment.source, canonical.display()),
            image_path: canonical,
            qemu_system,
            qemu_img,
            acceleration,
            firmware,
        },
        Err((state, reason)) => Resolution {
            state,
            reason,
            image_path: environment.image_path.clone(),
            qemu_system,
            qemu_img,
            acceleration,
            firmware,
        },
    }
}

const FIRMWARE_FALLBACK_DIRS: [&str; 3] =
    ["/usr/share/qemu", "/usr/share/OVMF", "/usr/share/edk2/x64"];

fn locate_firmware(
    qemu_system: &Path,
    arch: GuestArch,
    firmware: Firmware,
) -> std::result::Result<Option<PathBuf>, String> {
    locate_firmware_in(qemu_system, arch, firmware, &FIRMWARE_FALLBACK_DIRS)
}

fn locate_firmware_in(
    qemu_system: &Path,
    arch: GuestArch,
    firmware: Firmware,
    fallback_dirs: &[&str],
) -> std::result::Result<Option<PathBuf>, String> {
    let candidates = arch.firmware_file(firmware);
    if candidates.is_empty() {
        return Ok(None);
    }
    let Some(bin_dir) = qemu_system.parent() else {
        return Err(format!("{} has no parent directory", qemu_system.display()));
    };
    let mut search_dirs = vec![bin_dir.join("../share/qemu")];
    search_dirs.extend(fallback_dirs.iter().map(PathBuf::from));
    for dir in &search_dirs {
        for file in &candidates {
            let candidate = dir.join(file);
            if let Ok(path) = candidate.canonicalize() {
                if path.is_file() {
                    return Ok(Some(path));
                }
            }
        }
    }
    Err(format!(
        "missing firmware ({}) under {}",
        candidates.join(", "),
        search_dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn image_path_status(path: &Path) -> std::result::Result<PathBuf, (ResolveState, String)> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            Ok(path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
        }
        Ok(_) => Err((
            ResolveState::Missing,
            format!("image path is not a file: {}", path.display()),
        )),
        Err(error) if error.kind() == ErrorKind::NotFound => Err((
            ResolveState::Missing,
            format!("image not found at {}", path.display()),
        )),
        Err(error) => Err((
            ResolveState::Unsupported,
            format!("cannot inspect {}: {error}", path.display()),
        )),
    }
}

pub(crate) fn find_on_path(program: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|path| path.join(program))
        .find(|candidate| candidate.is_file())
}

fn detect_image_format(program: &Path, image_path: &Path, verbose: bool) -> Result<String> {
    let args = vec![
        "info".to_string(),
        "--output=json".to_string(),
        image_path.display().to_string(),
    ];
    let output = run_child_output(program, &args, verbose)?;
    if !output.status.success() {
        bail!("qemu-img info failed with {}", output.status);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(registry::parse_qemu_img_info(&stdout)?.format)
}

fn qemu_args(
    environment: &Environment,
    boot_disk: &Path,
    acceleration: &str,
    display: &str,
    qmp_port: u16,
    serial_port: u16,
    firmware: Option<&Path>,
) -> Vec<String> {
    let mut args = vec![
        "-name".to_string(),
        format!("qol-emu-{}", environment.id),
        "-machine".to_string(),
        environment.arch.machine_type().to_string(),
        "-accel".to_string(),
        acceleration.to_string(),
        "-m".to_string(),
        "4096".to_string(),
        "-smp".to_string(),
        "2".to_string(),
    ];
    match environment.arch {
        GuestArch::X86_64 => {}
        GuestArch::Aarch64 => {
            let cpu = if acceleration == "tcg" { "max" } else { "host" };
            args.extend(["-cpu".to_string(), cpu.to_string()]);
        }
    }
    if let Some(firmware) = firmware {
        args.extend([
            "-drive".to_string(),
            format!(
                "if=pflash,format=raw,readonly=on,file={}",
                firmware.display()
            ),
        ]);
    }
    match environment.media {
        BootMedia::Disk => args.extend([
            "-drive".to_string(),
            format!(
                "file={},id=qoldisk,if=virtio,format=qcow2",
                boot_disk.display()
            ),
        ]),
        BootMedia::Iso => args.extend([
            "-boot".to_string(),
            "d".to_string(),
            "-cdrom".to_string(),
            boot_disk.display().to_string(),
        ]),
    }
    args.extend([
        "-nic".to_string(),
        "user,model=virtio-net-pci".to_string(),
        "-device".to_string(),
        "qemu-xhci,id=xhci".to_string(),
        "-device".to_string(),
        "virtio-rng-pci".to_string(),
        "-display".to_string(),
        display.to_string(),
        "-qmp".to_string(),
        format!("tcp:127.0.0.1:{qmp_port},server,nowait"),
        "-serial".to_string(),
        format!("tcp:127.0.0.1:{serial_port},server,nowait"),
    ]);
    args
}

fn run_child_status(program: &Path, args: &[String], verbose: bool) -> Result<ExitStatus> {
    Ok(run_child_output(program, args, verbose)?.status)
}

fn run_child_output(
    program: &Path,
    args: &[String],
    verbose: bool,
) -> Result<std::process::Output> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to spawn {}", program.display()))?;
    if verbose || !output.status.success() {
        eprint!("{}", String::from_utf8_lossy(&output.stdout));
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(output)
}

struct ReportInput<'a> {
    environment: &'a Environment,
    resolution: &'a Resolution,
    run_dir: &'a Path,
    status: &'a str,
    overlay: Option<&'a Path>,
    qemu_command: Option<&'a Path>,
    commands: Vec<serde_json::Value>,
    qmp: Option<serde_json::Value>,
    serial: Option<serde_json::Value>,
    workflow: Option<serde_json::Value>,
    teardown: Option<serde_json::Value>,
    next: Vec<String>,
    started_at: u64,
}

fn report_json(input: ReportInput<'_>) -> Result<serde_json::Value> {
    let finished_at = unix_millis()?;
    Ok(json!({
        "name": "qol-emu-up",
        "started_at_unix_ms": input.started_at,
        "finished_at_unix_ms": finished_at,
        "status": input.status,
        "environment": {
            "id": input.environment.id,
            "name": input.environment.name,
            "backend": input.environment.backend,
            "arch": input.environment.arch.as_str(),
            "image_path": input.environment.image_path,
            "source": input.environment.source,
            "firmware": input.environment.firmware.as_str(),
            "media": input.environment.media.as_str(),
        },
        "resolution": {
            "state": input.resolution.state.as_str(),
            "reason": input.resolution.reason,
            "image_path": input.resolution.image_path,
            "qemu_system": input.resolution.qemu_system,
            "qemu_img": input.resolution.qemu_img,
            "acceleration": input.resolution.acceleration,
        },
        "artifacts": {
            "run_dir": input.run_dir,
            "overlay": input.overlay,
            "qemu_command": input.qemu_command,
            "report": input.run_dir.join("report.json"),
        },
        "commands": input.commands,
        "qmp": input.qmp,
        "serial": input.serial,
        "workflow": input.workflow,
        "teardown": input.teardown,
        "next": input.next,
    }))
}

fn write_report(run_dir: &Path, report: &serde_json::Value) -> Result<()> {
    let path = run_dir.join("report.json");
    let content = serde_json::to_string_pretty(report).context("failed to serialize report")?;
    fs::write(&path, format!("{content}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn next_for_resolution(environment: &Environment, resolution: &Resolution) -> Vec<String> {
    match resolution.state {
        ResolveState::Ready => Vec::new(),
        ResolveState::Missing => vec![format!(
            "Fix the discovered image path for `{}` or add a valid [images].{} entry to ~/.config/qol-tray/emu.toml.",
            environment.id, environment.id
        )],
        ResolveState::Unsupported => {
            vec!["Install QEMU tooling or fix permissions for the discovered image path.".to_string()]
        }
    }
}

fn kind_for_resolution(state: ResolveState) -> StepKind {
    match state {
        ResolveState::Ready => StepKind::Success,
        ResolveState::Missing => StepKind::Info,
        ResolveState::Unsupported => StepKind::Info,
    }
}

pub(crate) fn unix_millis() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_millis();
    u64::try_from(millis).context("system time overflowed u64 milliseconds")
}

fn command_line(program: &Path, args: &[String]) -> String {
    std::iter::once(shell_quote(&program.display().to_string()))
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'/' | b'.' | b'-' | b'_' | b':' | b'=' | b',')
    }) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn sanitize_id(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "emu".to_string()
    } else {
        out
    }
}

pub(crate) fn humanize_id(id: &str) -> String {
    id.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_domain_names_for_cli_ids() {
        assert_eq!(sanitize_id("Windows 11 Pro"), "windows-11-pro");
        assert_eq!(sanitize_id("!!!"), "emu");
    }

    #[test]
    fn report_json_serializes_firmware() {
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
            firmware: Firmware::Uefi,
            media: BootMedia::Disk,
        };
        let resolution = Resolution {
            state: ResolveState::Ready,
            reason: "ready".to_string(),
            image_path: PathBuf::from("/a/b/base.qcow2"),
            qemu_system: None,
            qemu_img: None,
            acceleration: "kvm",
            firmware: None,
        };
        let report = report_json(ReportInput {
            environment: &environment,
            resolution: &resolution,
            run_dir: Path::new("/a/b/run"),
            status: "ok",
            overlay: None,
            qemu_command: None,
            commands: Vec::new(),
            qmp: None,
            serial: None,
            workflow: None,
            teardown: None,
            next: Vec::new(),
            started_at: 0,
        })
        .unwrap();
        assert_eq!(report["environment"]["firmware"], "uefi");
    }

    #[test]
    fn statuses_for_maps_each_environment_to_a_status() {
        let environments = vec![
            Environment {
                id: "alpha".to_string(),
                name: "Alpha".to_string(),
                backend: "qemu".to_string(),
                arch: GuestArch::X86_64,
                image_path: PathBuf::from("/a/b/alpha.qcow2"),
                source: "config".to_string(),
                firmware: Firmware::Bios,
                media: BootMedia::Disk,
            },
            Environment {
                id: "beta".to_string(),
                name: "Beta".to_string(),
                backend: "qemu".to_string(),
                arch: GuestArch::Aarch64,
                image_path: PathBuf::from("/a/b/beta.qcow2"),
                source: "config".to_string(),
                firmware: Firmware::Uefi,
                media: BootMedia::Disk,
            },
        ];
        let statuses = statuses_for(environments);
        let ids: Vec<&str> = statuses.iter().map(|status| status.id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "beta"], "statuses: {statuses:?}");
        assert!(statuses.iter().all(|status| status.backend == "qemu"));
    }

    #[test]
    fn resolve_emu_dir_prefers_parsed_override() {
        let parsed = Some(PathBuf::from("/home/me/vms"));
        let fallback = Some(PathBuf::from("/data/qol-tray/emu"));
        let cases = [
            (
                parsed.clone(),
                fallback.clone(),
                Some(PathBuf::from("/home/me/vms")),
            ),
            (
                None,
                fallback.clone(),
                Some(PathBuf::from("/data/qol-tray/emu")),
            ),
            (None, None, None),
        ];
        for (override_dir, default_dir, expected) in cases {
            assert_eq!(
                resolve_emu_dir(override_dir.clone(), default_dir.clone()),
                expected,
                "override: {override_dir:?}, default: {default_dir:?}"
            );
        }
    }

    #[test]
    fn qemu_args_wire_accel_display_and_qmp() {
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
            firmware: Firmware::Bios,
            media: BootMedia::Disk,
        };
        let args = qemu_args(
            &environment,
            Path::new("/a/b/overlay.qcow2"),
            "kvm",
            "gtk",
            4444,
            5555,
            None,
        );
        let joined = args.join(" ");
        let expected = [
            "-accel kvm",
            "-display gtk",
            "-qmp tcp:127.0.0.1:4444,server,nowait",
            "-serial tcp:127.0.0.1:5555,server,nowait",
            "-drive file=/a/b/overlay.qcow2,id=qoldisk,if=virtio,format=qcow2",
            "-device qemu-xhci,id=xhci",
            "-device virtio-rng-pci",
        ];
        for fragment in expected {
            assert!(
                joined.contains(fragment),
                "missing `{fragment}` in: {joined}"
            );
        }
        assert!(joined.contains("-machine q35"), "machine in: {joined}");
        assert!(!joined.contains("-cpu"), "unexpected -cpu in: {joined}");
        assert!(!joined.contains("pflash"), "unexpected pflash in: {joined}");
    }

    #[test]
    fn qemu_args_wire_aarch64_machine_cpu_and_firmware() {
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::Aarch64,
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
            firmware: Firmware::Uefi,
            media: BootMedia::Disk,
        };
        let accelerated = qemu_args(
            &environment,
            Path::new("/a/b/overlay.qcow2"),
            "hvf",
            "cocoa",
            4444,
            5555,
            Some(Path::new("/fw/edk2-aarch64-code.fd")),
        )
        .join(" ");
        let expected = [
            "-machine virt",
            "-cpu host",
            "-drive if=pflash,format=raw,readonly=on,file=/fw/edk2-aarch64-code.fd",
        ];
        for fragment in expected {
            assert!(
                accelerated.contains(fragment),
                "missing `{fragment}` in: {accelerated}"
            );
        }
        let emulated = qemu_args(
            &environment,
            Path::new("/a/b/overlay.qcow2"),
            "tcg",
            "cocoa",
            4444,
            5555,
            None,
        )
        .join(" ");
        assert!(emulated.contains("-cpu max"), "cpu in: {emulated}");
        assert!(
            !emulated.contains("pflash"),
            "unexpected pflash in: {emulated}"
        );
    }

    #[test]
    fn qemu_args_wire_iso_as_cdrom_without_overlay_disk() {
        let environment = Environment {
            id: "mint".to_string(),
            name: "Mint".to_string(),
            backend: "qemu".to_string(),
            arch: GuestArch::X86_64,
            image_path: PathBuf::from("/a/b/mint.iso"),
            source: "candidate".to_string(),
            firmware: Firmware::Bios,
            media: BootMedia::Iso,
        };
        let joined = qemu_args(
            &environment,
            Path::new("/a/b/mint.iso"),
            "tcg",
            "cocoa",
            4444,
            5555,
            None,
        )
        .join(" ");
        assert!(
            joined.contains("-cdrom /a/b/mint.iso"),
            "cdrom in: {joined}"
        );
        assert!(joined.contains("-boot d"), "boot order in: {joined}");
        assert!(
            !joined.contains("id=qoldisk"),
            "iso must not attach a writable overlay disk: {joined}"
        );
        assert!(joined.contains("-machine q35"), "machine in: {joined}");
    }

    #[test]
    fn locate_firmware_finds_edk2_next_to_binary() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin");
        let share = root.path().join("share/qemu");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&share).unwrap();
        let qemu_system = bin.join("qemu-system-aarch64");
        fs::write(&qemu_system, b"x").unwrap();

        assert_eq!(
            locate_firmware_in(&qemu_system, GuestArch::X86_64, Firmware::Bios, &[]),
            Ok(None)
        );

        let arm_missing =
            locate_firmware_in(&qemu_system, GuestArch::Aarch64, Firmware::Uefi, &[]).unwrap_err();
        assert!(
            arm_missing.contains("edk2-aarch64-code.fd"),
            "error: {arm_missing}"
        );
        let x86_missing =
            locate_firmware_in(&qemu_system, GuestArch::X86_64, Firmware::Uefi, &[]).unwrap_err();
        assert!(x86_missing.contains("OVMF_CODE.fd"), "error: {x86_missing}");

        fs::write(share.join("edk2-aarch64-code.fd"), b"fw").unwrap();
        let arm_found = locate_firmware_in(&qemu_system, GuestArch::Aarch64, Firmware::Uefi, &[])
            .unwrap()
            .unwrap();
        assert!(
            arm_found.ends_with("edk2-aarch64-code.fd"),
            "found: {arm_found:?}"
        );

        fs::write(share.join("OVMF_CODE.fd"), b"fw").unwrap();
        let x86_found = locate_firmware_in(&qemu_system, GuestArch::X86_64, Firmware::Uefi, &[])
            .unwrap()
            .unwrap();
        assert!(x86_found.ends_with("OVMF_CODE.fd"), "found: {x86_found:?}");
    }

    #[test]
    fn last_run_parsing_extracts_id_and_status() {
        let full = json!({
            "environment": {"id": "foo"},
            "status": "pass",
            "finished_at_unix_ms": 42u64,
            "qmp": {"qemu_version": "9.2.0"},
        });
        let no_qmp = json!({
            "environment": {"id": "bar"},
            "status": "failed",
            "finished_at_unix_ms": 7u64,
            "qmp": null,
        });
        let unrelated = json!({"unrelated": 1});
        let cases = [
            (
                &full,
                Some((
                    "foo".to_string(),
                    LastRun {
                        status: "pass".to_string(),
                        finished_at_unix_ms: 42,
                    },
                )),
            ),
            (
                &no_qmp,
                Some((
                    "bar".to_string(),
                    LastRun {
                        status: "failed".to_string(),
                        finished_at_unix_ms: 7,
                    },
                )),
            ),
            (&unrelated, None),
        ];
        for (report, expected) in cases {
            assert_eq!(last_run_from_report(report), expected, "report: {report}");
        }
    }

    #[test]
    fn quotes_command_arguments_for_humans() {
        assert_eq!(
            command_line(
                Path::new("/usr/bin/qemu-system-x86_64"),
                &["-drive".to_string(), "file=/tmp/a b.qcow2".to_string()],
            ),
            "/usr/bin/qemu-system-x86_64 -drive 'file=/tmp/a b.qcow2'"
        );
    }

    #[test]
    fn emu_config_path_is_under_qol_config_namespace() {
        let path = emu_config_path().expect("config dir resolves in test env");
        assert!(
            path.ends_with("emu.toml"),
            "expected emu.toml leaf, got {path:?}"
        );
        let parent = path.parent().expect("emu.toml has a parent");
        assert!(
            parent.ends_with(qol_config::NAMESPACE),
            "expected parent under {} namespace, got {parent:?}",
            qol_config::NAMESPACE
        );
    }

    #[test]
    fn parse_add_args_extracts_path_and_overrides() {
        let args: Vec<OsString> = [
            "/a/b/win.qcow2",
            "--arch",
            "aarch64",
            "--firmware",
            "uefi",
            "--id",
            "My Box!",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        let parsed = parse_add_args(&args).unwrap();
        assert_eq!(parsed.path, PathBuf::from("/a/b/win.qcow2"), "path");
        assert_eq!(parsed.arch, Some(GuestArch::Aarch64), "arch");
        assert_eq!(parsed.firmware, Some(Firmware::Uefi), "firmware");
        assert_eq!(parsed.id.as_deref(), Some("my-box"), "id sanitized");
    }

    #[test]
    fn parse_add_args_requires_a_path() {
        let args: Vec<OsString> = ["--arch", "x86_64"].iter().map(OsString::from).collect();
        assert!(parse_add_args(&args).is_err(), "missing path must error");
    }

    #[test]
    fn parse_add_args_rejects_unknown_arch_and_firmware() {
        let cases = [
            (["/a/b/x.img", "--arch", "riscv"], "unknown arch must error"),
            (
                ["/a/b/x.img", "--firmware", "coreboot"],
                "unknown firmware must error",
            ),
        ];
        for (raw, why) in cases {
            let args: Vec<OsString> = raw.iter().map(OsString::from).collect();
            assert!(parse_add_args(&args).is_err(), "{why}");
        }
    }
}
