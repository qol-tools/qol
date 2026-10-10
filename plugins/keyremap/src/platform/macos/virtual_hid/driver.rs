use std::process::Command;

use super::{OWN_DAEMON_LABEL, PQRS_DAEMON_LABEL};
use crate::platform::macos::doctor::{DriverState, ExtensionState, Probe};

const PACKAGE_ID: &str = "org.pqrs.Karabiner-DriverKit-VirtualHIDDevice";

pub(crate) fn driver_state() -> Probe<DriverState> {
    let installed_version = match Command::new("/usr/sbin/pkgutil")
        .args(["--pkg-info", PACKAGE_ID])
        .output()
    {
        Ok(output) if output.status.success() => {
            parse_pkg_version(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(_) => None,
        Err(error) => return Probe::Unknown(format!("could not run pkgutil: {error}")),
    };
    let extension = match Command::new("/usr/bin/systemextensionsctl")
        .arg("list")
        .output()
    {
        Ok(output) => parse_extension_state(&String::from_utf8_lossy(&output.stdout)),
        Err(error) => return Probe::Unknown(format!("could not run systemextensionsctl: {error}")),
    };
    Probe::Known(DriverState {
        installed_version,
        extension,
    })
}

pub(crate) fn daemon_running() -> Probe<bool> {
    for label in [PQRS_DAEMON_LABEL, OWN_DAEMON_LABEL] {
        match Command::new("/bin/launchctl")
            .args(["print", &format!("system/{label}")])
            .output()
        {
            Ok(output) if output.status.success() => {
                if parse_launchd_running(&String::from_utf8_lossy(&output.stdout)) {
                    return Probe::Known(true);
                }
            }
            Ok(_) => {}
            Err(error) => return Probe::Unknown(format!("could not run launchctl: {error}")),
        }
    }
    Probe::Known(false)
}

fn parse_pkg_version(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("version: "))
        .map(|version| version.trim().to_string())
}

fn parse_extension_state(output: &str) -> ExtensionState {
    let mut lines = output
        .lines()
        .filter(|line| line.contains(PACKAGE_ID))
        .peekable();
    if lines.peek().is_none() {
        return ExtensionState::Missing;
    }
    if lines.any(|line| line.contains("[activated enabled]")) {
        ExtensionState::Activated
    } else {
        ExtensionState::NotActivated
    }
}

fn parse_launchd_running(output: &str) -> bool {
    output.lines().any(|line| line.trim() == "state = running")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKGUTIL: &str = "package-id: org.pqrs.Karabiner-DriverKit-VirtualHIDDevice\nversion: 8.6.0\nvolume: /\nlocation: \ninstall-time: 1790761368\n";
    const EXTENSIONS: &str = "3 extension(s)\n--- com.apple.system_extension.driver_extension\nenabled\tactive\tteamID\tbundleID (version)\tname\t[state]\n*\t*\tG43BCU2T37\torg.pqrs.Karabiner-DriverKit-VirtualHIDDevice (1.8.0/1.8.0)\torg.pqrs.Karabiner-DriverKit-VirtualHIDDevice\t[activated enabled]\n";

    #[test]
    fn the_package_version_is_read_from_pkgutil() {
        assert_eq!(parse_pkg_version(PKGUTIL), Some("8.6.0".to_string()));
        assert_eq!(parse_pkg_version("No receipt for 'x' found at '/'."), None);
    }

    #[test]
    fn the_extension_state_is_read_from_systemextensionsctl() {
        assert_eq!(parse_extension_state(EXTENSIONS), ExtensionState::Activated);
        assert_eq!(
            parse_extension_state(
                &EXTENSIONS.replace("[activated enabled]", "[activated waiting for user]")
            ),
            ExtensionState::NotActivated
        );
        assert_eq!(
            parse_extension_state("0 extension(s)\n"),
            ExtensionState::Missing
        );
    }

    #[test]
    fn a_running_launchd_job_says_so() {
        assert!(parse_launchd_running(
            "system/x = {\n\tstate = running\n\tpid = 10\n}"
        ));
        assert!(!parse_launchd_running(
            "system/x = {\n\tstate = not running\n}"
        ));
    }
}
