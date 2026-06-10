use crate::progress::{print_hint, print_title, step_label, StepKind};
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

mod control;
mod discovery;
mod live;
mod machine;
mod platform;
mod qmp;

use discovery::DiscoveryContext;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Environment {
    id: String,
    name: String,
    backend: String,
    arch: String,
    image_path: PathBuf,
    source: String,
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
    pub(crate) name: String,
    pub(crate) backend: String,
    pub(crate) arch: String,
    pub(crate) state: ResolveState,
    pub(crate) reason: String,
    pub(crate) last_run: Option<LastRun>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LastRun {
    pub(crate) status: String,
    pub(crate) finished_at_unix_ms: u64,
    pub(crate) qemu_version: Option<String>,
}

#[derive(Clone, Debug)]
struct Resolution {
    state: ResolveState,
    reason: String,
    image_path: PathBuf,
    qemu_system: Option<PathBuf>,
    qemu_img: Option<PathBuf>,
    acceleration: &'static str,
}

pub(crate) fn run(args: &[OsString], verbose: bool) -> Result<()> {
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        print_emu_help();
        return Ok(());
    };
    let rest = &args[1..];
    match command {
        "list" => cmd_list(rest, verbose),
        "doctor" => cmd_doctor(rest, verbose),
        "up" => cmd_up(rest, verbose),
        "shot" => control::cmd_shot(rest, verbose),
        "key" => control::cmd_key(rest, verbose),
        "insert" => control::cmd_insert(rest, verbose),
        "pull" => control::cmd_pull(rest, verbose),
        "snap" => control::cmd_snap(rest, verbose),
        "down" => control::cmd_down(rest, verbose),
        "help" | "-h" | "--help" => {
            print_emu_help();
            Ok(())
        }
        other => bail!("unknown emu command `{other}`\n\n{}", emu_help_text()),
    }
}

pub(crate) fn environment_statuses() -> Result<Vec<EnvironmentStatus>> {
    let mut last_runs = last_runs_by_id();
    Ok(discover_environments()?
        .into_iter()
        .map(|environment| {
            let resolution = resolve_environment(&environment);
            EnvironmentStatus {
                last_run: last_runs.remove(&environment.id),
                id: environment.id,
                name: environment.name,
                backend: environment.backend,
                arch: environment.arch,
                state: resolution.state,
                reason: resolution.reason,
            }
        })
        .collect())
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

fn last_run_from_report(report: &serde_json::Value) -> Option<(String, LastRun)> {
    let id = report.get("environment")?.get("id")?.as_str()?.to_string();
    let status = report.get("status")?.as_str()?.to_string();
    let finished_at_unix_ms = report.get("finished_at_unix_ms")?.as_u64()?;
    let qemu_version = report
        .get("qmp")
        .and_then(|qmp| qmp.get("qemu_version"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Some((
        id,
        LastRun {
            status,
            finished_at_unix_ms,
            qemu_version,
        },
    ))
}

pub(crate) fn emu_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("qol-tray/emu.toml"))
}

fn cmd_list(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol emu list");
    }
    print_title("qol emu");
    print_hint(verbose);
    let statuses = environment_statuses()?;
    if statuses.is_empty() {
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
    Ok(())
}

fn cmd_doctor(args: &[OsString], verbose: bool) -> Result<()> {
    if !args.is_empty() {
        bail!("usage: qol emu doctor");
    }
    print_title("qol emu doctor");
    print_hint(verbose);
    match find_on_path("qemu-system-x86_64") {
        Some(path) => step_label("qemu", StepKind::Success, &path.display().to_string()),
        None => step_label("qemu", StepKind::Info, "missing qemu-system-x86_64"),
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
    step_label("accel", StepKind::Info, platform::acceleration());
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
    Ok(())
}

fn cmd_up(args: &[OsString], verbose: bool) -> Result<()> {
    if args.len() != 1 {
        bail!("usage: qol emu up <environment>");
    }
    let target = args[0]
        .to_str()
        .ok_or_else(|| anyhow!("environment id is not valid UTF-8"))?;
    let root = repo_root()?;
    let environments = discover_environments()?;
    if environments.is_empty() {
        bail!("no emus found; create a libvirt/QEMU VM or add [images] to ~/.config/qol-tray/emu.toml");
    }
    let environment = environments
        .iter()
        .find(|environment| environment.id == target)
        .ok_or_else(|| anyhow!("unknown emu `{target}`; run `qol emu list`"))?
        .clone();
    let resolution = resolve_environment(&environment);
    let started_at = unix_millis()?;
    let run_dir = root
        .join("target/qol-emu")
        .join(format!("{}-{started_at}", environment.id));
    fs::create_dir_all(&run_dir)
        .with_context(|| format!("failed to create {}", run_dir.display()))?;

    print_title("qol emu up");
    print_hint(verbose);
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
            teardown: None,
            next: vec!["Inspect the qemu-img output, remove the run directory if needed, then rerun `qol emu up`.".to_string()],
            started_at,
        })?;
        write_report(&run_dir, &report)?;
        bail!("qemu-img failed with {status}");
    }

    let qemu_system = resolution
        .qemu_system
        .clone()
        .ok_or_else(|| anyhow!("ready environment has no qemu-system path"))?;
    let qmp_port = machine::free_qmp_port()?;
    let qemu_args = qemu_args(
        &environment,
        &overlay,
        resolution.acceleration,
        platform::display(),
        qmp_port,
    );
    let qemu_command = command_line(&qemu_system, &qemu_args);
    let qemu_command_path = run_dir.join("qemu-command.txt");
    fs::write(&qemu_command_path, format!("{qemu_command}\n"))
        .with_context(|| format!("failed to write {}", qemu_command_path.display()))?;
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
        json!({
            "program": qemu_system,
            "args": qemu_args,
        }),
    ];

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
                overlay: Some(&overlay),
                qemu_command: Some(&qemu_command_path),
                commands,
                qmp: Some(json!({ "port": qmp_port, "error": error.to_string() })),
                teardown: Some(json!({ "removed": removed })),
                next: vec!["Inspect the qemu output above, then rerun `qol emu up`.".to_string()],
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
        overlay: Some(&overlay),
        qemu_command: Some(&qemu_command_path),
        commands: commands.clone(),
        qmp: Some(json!({ "port": qmp_port, "qemu_version": qemu_version, "status": vm_status })),
        teardown: None,
        next: vec!["Close the VM window (or shut the guest down) to end the run.".to_string()],
        started_at,
    })?;
    write_report(&run_dir, &report)?;
    step_label(
        "running",
        StepKind::Success,
        "close the VM window to end the run",
    );

    let exit = child.wait().context("failed to wait for qemu")?;
    let removed = machine::teardown(&run_dir)?;
    let final_status = if exit.success() { "pass" } else { "failed" };
    let report = report_json(ReportInput {
        environment: &environment,
        resolution: &resolution,
        run_dir: &run_dir,
        status: final_status,
        overlay: None,
        qemu_command: Some(&qemu_command_path),
        commands,
        qmp: Some(json!({ "port": qmp_port, "qemu_version": qemu_version, "status": vm_status })),
        teardown: Some(json!({ "removed": removed, "exit": exit.to_string() })),
        next: vec!["Rerun `qol emu up` for a fresh disposable clone.".to_string()],
        started_at,
    })?;
    write_report(&run_dir, &report)?;
    step_label(
        "clean",
        StepKind::Success,
        &format!("removed {} disposable file(s)", removed.len()),
    );
    step_label(
        "report",
        StepKind::Info,
        &run_dir.join("report.json").display().to_string(),
    );
    if !exit.success() {
        bail!("qemu exited with {exit}");
    }
    Ok(())
}

fn print_emu_help() {
    print!("{}", emu_help_text());
}

fn emu_help_text() -> &'static str {
    "qol emu commands:\n  qol emu list\n  qol emu doctor\n  qol emu up <environment>\n\nEmus are discovered from libvirt/QEMU domains plus optional local config:\n  ~/.config/qol-tray/emu.toml\n\nExample config:\n  [images]\n  my-windows = \"/path/to/windows.qcow2\"\n"
}

fn discover_environments() -> Result<Vec<Environment>> {
    discovery::discover(DiscoveryContext {
        config_path: emu_config_path(),
        home_dir: dirs::home_dir(),
        virsh: find_on_path("virsh"),
        libvirt_uris: platform::libvirt_uris(),
        image_search_roots: platform::image_search_roots(dirs::home_dir()),
    })
}

fn resolve_environment(environment: &Environment) -> Resolution {
    let qemu_system = find_on_path("qemu-system-x86_64");
    let qemu_img = find_on_path("qemu-img");
    let acceleration = platform::acceleration();
    if qemu_system.is_none() {
        return Resolution {
            state: ResolveState::Unsupported,
            reason: "missing qemu-system-x86_64".to_string(),
            image_path: environment.image_path.clone(),
            qemu_system,
            qemu_img,
            acceleration,
        };
    }
    if qemu_img.is_none() {
        return Resolution {
            state: ResolveState::Unsupported,
            reason: "missing qemu-img".to_string(),
            image_path: environment.image_path.clone(),
            qemu_system,
            qemu_img,
            acceleration,
        };
    }
    match image_path_status(&environment.image_path) {
        Ok(canonical) => Resolution {
            state: ResolveState::Ready,
            reason: format!("{} · {}", environment.source, canonical.display()),
            image_path: canonical,
            qemu_system,
            qemu_img,
            acceleration,
        },
        Err((state, reason)) => Resolution {
            state,
            reason,
            image_path: environment.image_path.clone(),
            qemu_system,
            qemu_img,
            acceleration,
        },
    }
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
    let parsed: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("failed to parse qemu-img info JSON")?;
    parsed
        .get("format")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("qemu-img info did not report an image format"))
}

fn qemu_args(
    environment: &Environment,
    overlay: &Path,
    acceleration: &str,
    display: &str,
    qmp_port: u16,
) -> Vec<String> {
    vec![
        "-name".to_string(),
        format!("qol-emu-{}", environment.id),
        "-machine".to_string(),
        "q35".to_string(),
        "-accel".to_string(),
        acceleration.to_string(),
        "-m".to_string(),
        "4096".to_string(),
        "-smp".to_string(),
        "2".to_string(),
        "-drive".to_string(),
        format!(
            "file={},id=qoldisk,if=virtio,format=qcow2",
            overlay.display()
        ),
        "-nic".to_string(),
        "user,model=virtio-net-pci".to_string(),
        "-device".to_string(),
        "qemu-xhci,id=xhci".to_string(),
        "-display".to_string(),
        display.to_string(),
        "-qmp".to_string(),
        format!("tcp:127.0.0.1:{qmp_port},server,nowait"),
    ]
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
            "arch": input.environment.arch,
            "image_path": input.environment.image_path,
            "source": input.environment.source,
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
    fn qemu_args_wire_accel_display_and_qmp() {
        let environment = Environment {
            id: "foo".to_string(),
            name: "Foo".to_string(),
            backend: "qemu".to_string(),
            arch: "x86_64".to_string(),
            image_path: PathBuf::from("/a/b/base.qcow2"),
            source: "config".to_string(),
        };
        let args = qemu_args(
            &environment,
            Path::new("/a/b/overlay.qcow2"),
            "kvm",
            "gtk",
            4444,
        );
        let joined = args.join(" ");
        let expected = [
            "-accel kvm",
            "-display gtk",
            "-qmp tcp:127.0.0.1:4444,server,nowait",
            "-drive file=/a/b/overlay.qcow2,id=qoldisk,if=virtio,format=qcow2",
            "-device qemu-xhci,id=xhci",
        ];
        for fragment in expected {
            assert!(
                joined.contains(fragment),
                "missing `{fragment}` in: {joined}"
            );
        }
    }

    #[test]
    fn last_run_parsing_extracts_id_status_and_version() {
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
                        qemu_version: Some("9.2.0".to_string()),
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
                        qemu_version: None,
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
}
