use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use qol_headless::DoctorCheckResult;
use serde_json::json;

const REQUIRED_BINARIES: [&str; 4] = ["xdotool", "xprop", "wmctrl", "xrandr"];

pub(crate) fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "Linux is declared and supported through the Cinnamon X11 backend",
    )
}

pub(crate) fn required_binaries_check() -> DoctorCheckResult {
    let found = REQUIRED_BINARIES
        .iter()
        .filter_map(|name| resolve_command(name).map(|path| ((*name).to_string(), path)))
        .collect::<Vec<_>>();
    let missing = REQUIRED_BINARIES
        .iter()
        .filter(|name| !found.iter().any(|(found, _)| found == *name))
        .copied()
        .collect::<Vec<_>>();
    let session = env::var("XDG_SESSION_TYPE").ok();
    let desktop = env::var("XDG_CURRENT_DESKTOP").ok();
    let details = json!({
        "found": found
            .iter()
            .map(|(name, path)| (name, path.display().to_string()))
            .collect::<std::collections::BTreeMap<_, _>>(),
        "missing": missing,
        "session_type": session,
        "desktop": desktop,
        "executed": false,
    });

    if !missing.is_empty() {
        return DoctorCheckResult::fail(
            "required_binaries",
            format!("Missing required Linux tools: {}", missing.join(", ")),
        )
        .with_fix("Install xdotool, xprop, wmctrl, and xrandr")
        .with_details(details);
    }
    if session
        .as_deref()
        .is_some_and(|session| !session.eq_ignore_ascii_case("x11"))
    {
        return DoctorCheckResult::fail(
            "required_binaries",
            format!(
                "The Window Actions Linux backend requires X11, but the session is {}",
                session.as_deref().unwrap_or("unknown")
            ),
        )
        .with_fix("Log into a Cinnamon X11 session")
        .with_details(details);
    }
    if desktop
        .as_deref()
        .is_some_and(|desktop| !is_cinnamon_desktop(desktop))
    {
        return DoctorCheckResult::warn(
            "required_binaries",
            "Required tools are available, but the current desktop is not reported as Cinnamon",
        )
        .with_fix("Run Window Actions under Cinnamon")
        .with_details(details);
    }

    DoctorCheckResult::ok(
        "required_binaries",
        "Required Cinnamon X11 tools are available by executable metadata",
    )
    .with_details(details)
}

fn is_cinnamon_desktop(desktop: &str) -> bool {
    desktop
        .split(':')
        .any(|name| name.to_ascii_lowercase().contains("cinnamon"))
}

fn resolve_command(command: &str) -> Option<PathBuf> {
    command_search_dirs()
        .into_iter()
        .map(|dir| dir.join(command))
        .find(|path| is_executable_file(path))
}

fn command_search_dirs() -> Vec<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {
    use super::is_cinnamon_desktop;

    #[test]
    fn recognizes_cinnamon_desktop_tokens() {
        let cases = [
            ("Cinnamon", true),
            ("X-Cinnamon", true),
            ("X-Cinnamon:GNOME", true),
            ("GNOME:Cinnamon", true),
            ("GNOME", false),
            ("", false),
        ];

        for (desktop, expected) in cases {
            assert_eq!(is_cinnamon_desktop(desktop), expected, "desktop={desktop}");
        }
    }
}
