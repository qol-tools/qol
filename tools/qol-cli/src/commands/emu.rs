use crate::progress::{print_hint, print_title, step_label, StepKind};
use crate::workspace::repo_root;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::json;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::{SystemTime, UNIX_EPOCH};

mod discovery;
mod platform;

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
        "help" | "-h" | "--help" => {
            print_emu_help();
            Ok(())
        }
        other => bail!("unknown emu command `{other}`\n\n{}", emu_help_text()),
    }
}

pub(crate) fn environment_statuses() -> Result<Vec<EnvironmentStatus>> {
    Ok(discover_environments()?
        .into_iter()
        .map(|environment| {
            let resolution = resolve_environment(&environment);
            EnvironmentStatus {
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
    let qemu_args = qemu_args(&environment, &overlay, resolution.acceleration);
    let qemu_command = command_line(&qemu_system, &qemu_args);
    let qemu_command_path = run_dir.join("qemu-command.txt");
    fs::write(&qemu_command_path, format!("{qemu_command}\n"))
        .with_context(|| format!("failed to write {}", qemu_command_path.display()))?;
    let report = report_json(ReportInput {
        environment: &environment,
        resolution: &resolution,
        run_dir: &run_dir,
        status: "pass",
        overlay: Some(&overlay),
        qemu_command: Some(&qemu_command_path),
        commands: vec![
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
        ],
        next: vec![format!(
            "Run `{qemu_command}` when you want to launch this disposable clone."
        )],
        started_at,
    })?;
    write_report(&run_dir, &report)?;

    step_label("ready", StepKind::Success, &run_dir.display().to_string());
    step_label(
        "command",
        StepKind::Info,
        &qemu_command_path.display().to_string(),
    );
    step_label(
        "report",
        StepKind::Info,
        &run_dir.join("report.json").display().to_string(),
    );
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

fn find_on_path(program: &str) -> Option<PathBuf> {
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

fn qemu_args(environment: &Environment, overlay: &Path, acceleration: &str) -> Vec<String> {
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
        format!("file={},if=virtio,format=qcow2", overlay.display()),
        "-nic".to_string(),
        "user,model=virtio-net-pci".to_string(),
        "-display".to_string(),
        "gtk".to_string(),
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

fn unix_millis() -> Result<u64> {
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
