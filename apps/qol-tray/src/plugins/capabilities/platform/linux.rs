use super::PermissionPlatform;
use crate::plugins::capabilities::{PermissionState, PermissionStatus};
use crate::plugins::manifest::Capabilities;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(super) struct Platform;

struct CapabilityRule {
    name: &'static str,
    is_required: fn(&Capabilities) -> bool,
    check: fn() -> PermissionStatus,
    request: fn() -> PermissionStatus,
}

const REGISTRY: &[CapabilityRule] = &[CapabilityRule {
    name: "serial",
    is_required: |capabilities| capabilities.serial,
    check: check_serial,
    request: request_serial,
}];

impl PermissionPlatform for Platform {
    fn check_plugin_permissions(capabilities: &Capabilities) -> HashMap<String, PermissionStatus> {
        REGISTRY
            .iter()
            .filter(|rule| (rule.is_required)(capabilities))
            .map(|rule| (rule.name.to_string(), (rule.check)()))
            .collect()
    }

    fn check_permission(name: &str) -> Option<PermissionStatus> {
        REGISTRY
            .iter()
            .find(|rule| rule.name == name)
            .map(|rule| (rule.check)())
    }

    fn request_permission(name: &str) -> Option<PermissionStatus> {
        REGISTRY
            .iter()
            .find(|rule| rule.name == name)
            .map(|rule| (rule.request)())
    }
}

fn check_serial() -> PermissionStatus {
    let in_session = user_is_in_group("dialout");
    let device_accessible = serial_devices().iter().any(|path| can_access_device(path));
    let persistent = user_in_persistent_group("dialout");
    let pkexec = pkexec_command_path().is_ok();
    resolve_serial_check_state(in_session, device_accessible, persistent, pkexec)
}

fn user_is_in_group(group: &str) -> bool {
    let output = Command::new("id").arg("-nG").output();
    let Ok(output) = output else { return false };
    let groups = String::from_utf8_lossy(&output.stdout);
    groups.split_whitespace().any(|current| current == group)
}

fn parse_group_members(getent_output: &str) -> impl Iterator<Item = &str> {
    getent_output
        .split(':')
        .nth(3)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|member| !member.is_empty())
}

fn user_in_persistent_group(group: &str) -> bool {
    let output = Command::new("getent").args(["group", group]).output();
    let Ok(output) = output else { return false };
    let line = String::from_utf8_lossy(&output.stdout);
    let Ok(user) = current_user_name() else {
        return false;
    };
    let found = parse_group_members(&line).any(|member| member == user);
    found
}

fn resolve_serial_check_state(
    in_session_group: bool,
    device_accessible: bool,
    in_persistent_group: bool,
    pkexec_available: bool,
) -> PermissionStatus {
    if in_session_group || device_accessible {
        return PermissionStatus {
            state: PermissionState::Granted,
            hint: None,
        };
    }
    if in_persistent_group {
        return PermissionStatus {
            state: PermissionState::RequiresLogout,
            hint: Some("Log out and back in to activate serial access".to_string()),
        };
    }
    if pkexec_available {
        return PermissionStatus {
            state: PermissionState::Fixable,
            hint: Some("Serial port access requires password prompt".to_string()),
        };
    }
    PermissionStatus {
        state: PermissionState::Denied,
        hint: Some("Install policykit to enable serial access".to_string()),
    }
}

fn resolve_serial_request_state(
    device_accessible: bool,
    in_persistent_group: bool,
    in_session_group: bool,
) -> PermissionStatus {
    if in_session_group || device_accessible {
        return PermissionStatus {
            state: PermissionState::Granted,
            hint: None,
        };
    }
    if in_persistent_group {
        return PermissionStatus {
            state: PermissionState::RequiresLogout,
            hint: Some("Log out and back in to activate serial access".to_string()),
        };
    }
    PermissionStatus {
        state: PermissionState::Denied,
        hint: Some("Could not configure serial access".to_string()),
    }
}

fn request_serial() -> PermissionStatus {
    let user = match current_user_name() {
        Ok(user) => user,
        Err(_) => {
            return PermissionStatus {
                state: PermissionState::Denied,
                hint: Some("Could not resolve current user".to_string()),
            }
        }
    };

    if !user_in_persistent_group("dialout") {
        if let Ok((command, args)) = serial_group_fix_command(&user) {
            let _ = run_pkexec(&command, &args);
        }
    }

    let devices = serial_devices();
    if !devices.is_empty() {
        if let Ok(command) = setfacl_command_path() {
            let args = device_access_fix_args(&user, &devices);
            let _ = run_pkexec(&command, &args);
        }
    }

    let device_accessible = devices.iter().any(|path| can_access_device(path));
    let persistent = user_in_persistent_group("dialout");
    let in_session = user_is_in_group("dialout");
    resolve_serial_request_state(device_accessible, persistent, in_session)
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

fn current_user_name() -> Result<String> {
    let command =
        existing_command_path(&["/usr/bin/id", "/bin/id"]).context("no id command found")?;
    let output = Command::new(command).arg("-un").output()?;
    if !output.status.success() {
        bail!("id -un failed");
    }
    let user = String::from_utf8(output.stdout)?.trim().to_string();
    if user.is_empty()
        || user.starts_with('-')
        || !user.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
    {
        bail!("current user name is invalid");
    }
    Ok(user)
}

fn setfacl_command_path() -> Result<PathBuf> {
    existing_command_path(&["/bin/setfacl", "/usr/bin/setfacl"]).context("no setfacl command found")
}

fn pkexec_command_path() -> Result<PathBuf> {
    existing_command_path(&["/bin/pkexec", "/usr/bin/pkexec"]).context("no pkexec command found")
}

fn device_access_fix_args(user: &str, device_paths: &[PathBuf]) -> Vec<String> {
    let mut args = vec![String::from("-m"), format!("u:{user}:rw")];
    args.extend(
        device_paths
            .iter()
            .filter_map(|path| path.to_str().map(str::to_string)),
    );
    args
}

fn existing_command_path(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn can_access_device(path: &Path) -> bool {
    let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(path.as_ptr(), libc::R_OK | libc::W_OK) == 0 }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_group_members_extracts_users() {
        let cases = [
            ("dialout:x:20:alice,bob", vec!["alice", "bob"]),
            ("dialout:x:20:alice", vec!["alice"]),
            ("dialout:x:20:", vec![]),
            ("dialout:x:20", vec![]),
            ("", vec![]),
            ("short", vec![]),
        ];
        for (input, expected) in cases {
            let members: Vec<_> = parse_group_members(input).collect();
            assert_eq!(members, expected, "input: {:?}", input);
        }
    }

    #[test]
    fn request_state_resolution() {
        let cases = [
            (true, false, false, PermissionState::Granted),
            (false, false, true, PermissionState::Granted),
            (true, true, true, PermissionState::Granted),
            (false, true, false, PermissionState::RequiresLogout),
            (false, false, false, PermissionState::Denied),
        ];
        for (device_ok, persistent, in_session, expected) in cases {
            let status = resolve_serial_request_state(device_ok, persistent, in_session);
            assert_eq!(
                status.state, expected,
                "device_ok={device_ok}, persistent={persistent}, in_session={in_session}"
            );
        }
    }

    #[test]
    fn check_state_resolution() {
        let cases = [
            (true, false, false, false, PermissionState::Granted),
            (false, true, false, false, PermissionState::Granted),
            (true, true, true, true, PermissionState::Granted),
            (false, false, true, true, PermissionState::RequiresLogout),
            (false, false, true, false, PermissionState::RequiresLogout),
            (false, false, false, true, PermissionState::Fixable),
            (false, false, false, false, PermissionState::Denied),
        ];
        for (in_session, device_ok, persistent, pkexec, expected) in cases {
            let status = resolve_serial_check_state(in_session, device_ok, persistent, pkexec);
            assert_eq!(
                status.state, expected,
                "in_session={in_session}, device_ok={device_ok}, persistent={persistent}, pkexec={pkexec}"
            );
        }
    }
}
