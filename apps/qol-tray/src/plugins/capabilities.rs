use crate::plugins::manifest::Capabilities;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

struct CapabilityRule {
    name: &'static str,
    os: &'static str,
    is_required: fn(&Capabilities) -> bool,
    check: fn() -> bool,
    fix: fn() -> Result<()>,
}

const REGISTRY: &[CapabilityRule] = &[CapabilityRule {
    name: "serial",
    os: "linux",
    is_required: |c| c.serial,
    check: check_serial_linux,
    fix: fix_serial_linux,
}];

pub fn check_capabilities(capabilities_list: &[&Capabilities]) -> HashMap<&'static str, bool> {
    evaluate_capabilities(capabilities_list, false)
}

pub fn ensure_capabilities(capabilities_list: &[&Capabilities]) -> HashMap<&'static str, bool> {
    evaluate_capabilities(capabilities_list, true)
}

pub fn required_capability_names(capabilities: &Capabilities) -> Vec<&'static str> {
    registry_rules_for_current_os()
        .filter(|rule| (rule.is_required)(capabilities))
        .map(|rule| rule.name)
        .collect()
}

pub fn unmet_capability_names(
    capabilities: &Capabilities,
    results: &HashMap<&'static str, bool>,
) -> Vec<&'static str> {
    required_capability_names(capabilities)
        .into_iter()
        .filter(|name| !results.get(name).copied().unwrap_or(false))
        .collect()
}

pub fn capabilities_met(
    capabilities: &Capabilities,
    results: &HashMap<&'static str, bool>,
) -> bool {
    unmet_capability_names(capabilities, results).is_empty()
}

fn evaluate_capabilities(
    capabilities_list: &[&Capabilities],
    attempt_fix: bool,
) -> HashMap<&'static str, bool> {
    let mut results = HashMap::new();

    for rule in registry_rules_for_current_os() {
        let needed = capabilities_list
            .iter()
            .any(|capabilities| (rule.is_required)(capabilities));
        if !needed {
            results.insert(rule.name, true);
            continue;
        }
        if (rule.check)() {
            results.insert(rule.name, true);
            continue;
        }
        if !attempt_fix {
            results.insert(rule.name, false);
            continue;
        }

        log::info!("capability '{}' not met, attempting fix", rule.name);
        let fixed = (rule.fix)().is_ok() && (rule.check)();
        if fixed {
            log::info!("capability '{}' fixed", rule.name);
        }
        if !fixed {
            log::warn!("capability '{}' could not be fixed", rule.name);
        }
        results.insert(rule.name, fixed);
    }

    results
}

fn registry_rules_for_current_os() -> impl Iterator<Item = &'static CapabilityRule> {
    REGISTRY
        .iter()
        .filter(|rule| rule.os == std::env::consts::OS)
}

fn check_serial_linux() -> bool {
    if user_is_in_group("dialout") {
        return true;
    }

    serial_devices()
        .into_iter()
        .any(|path| can_access_serial_device(&path))
}

fn user_is_in_group(group: &str) -> bool {
    let output = Command::new("id").arg("-nG").output();
    let Ok(output) = output else { return false };
    let groups = String::from_utf8_lossy(&output.stdout);
    groups.split_whitespace().any(|current| current == group)
}

fn fix_serial_linux() -> Result<()> {
    let user = std::env::var("USER").context("USER env var not set")?;
    let (group_command, group_args) = serial_group_fix_command(&user)?;
    run_pkexec(&group_command, &group_args)?;
    let devices = serial_devices();
    if devices.is_empty() {
        return Ok(());
    }

    let access_command = device_access_fix_command()?;
    let args = device_access_fix_args(access_command.as_path(), &user, &devices);
    let _ = run_pkexec(&access_command, &args);

    Ok(())
}

fn serial_group_fix_command(user: &str) -> Result<(PathBuf, Vec<String>)> {
    if let Some(path) = existing_command_path(&["/usr/sbin/usermod", "/sbin/usermod"]) {
        return Ok((
            path,
            vec![
                String::from("-aG"),
                String::from("dialout"),
                user.to_string(),
            ],
        ));
    }

    let path = existing_command_path(&["/usr/sbin/adduser", "/sbin/adduser"])
        .context("no adduser/usermod command found")?;
    Ok((path, vec![user.to_string(), String::from("dialout")]))
}

fn serial_devices() -> Vec<PathBuf> {
    std::fs::read_dir("/dev")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            if !is_serial_device_name(name) {
                return None;
            }
            Some(entry.path())
        })
        .collect()
}

fn is_serial_device_name(name: &str) -> bool {
    name.starts_with("ttyUSB") || name.starts_with("ttyACM")
}

fn chmod_command_path() -> Result<PathBuf> {
    existing_command_path(&["/bin/chmod", "/usr/bin/chmod"]).context("no chmod command found")
}

fn setfacl_command_path() -> Result<PathBuf> {
    existing_command_path(&["/bin/setfacl", "/usr/bin/setfacl"]).context("no setfacl command found")
}

fn pkexec_command_path() -> Result<PathBuf> {
    existing_command_path(&["/bin/pkexec", "/usr/bin/pkexec"]).context("no pkexec command found")
}

fn device_access_fix_command() -> Result<PathBuf> {
    if let Ok(path) = setfacl_command_path() {
        return Ok(path);
    }

    chmod_command_path()
}

fn device_access_fix_args(
    command_path: &Path,
    user: &str,
    device_paths: &[PathBuf],
) -> Vec<String> {
    let mut args = Vec::new();

    if is_setfacl_command(command_path) {
        args.push(String::from("-m"));
        args.push(format!("u:{}:rw", user));
        args.extend(
            device_paths
                .iter()
                .filter_map(|path| path.to_str().map(str::to_string)),
        );
        return args;
    }

    args.push(String::from("666"));
    args.extend(
        device_paths
            .iter()
            .filter_map(|path| path.to_str().map(str::to_string)),
    );
    args
}

fn is_setfacl_command(command_path: &Path) -> bool {
    command_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "setfacl")
}

fn existing_command_path(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

#[cfg(unix)]
fn can_access_serial_device(path: &Path) -> bool {
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::R_OK | libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn can_access_serial_device(_path: &Path) -> bool {
    false
}

fn run_pkexec(command_path: &Path, args: &[String]) -> Result<()> {
    let pkexec_path = pkexec_command_path()?;
    let status = Command::new(pkexec_path)
        .arg(command_path)
        .args(args)
        .status()
        .with_context(|| format!("pkexec {} {:?} failed", command_path.display(), args))?;
    if status.success() {
        return Ok(());
    }

    bail!(
        "pkexec {} {:?} exited with {}",
        command_path.display(),
        args,
        status
    )
}
