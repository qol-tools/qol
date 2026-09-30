use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use qol_hotkeys::layout::physical_layout;
use qol_hotkeys::macos_keycode::PhysicalLayout;

use super::protocol::SOCKET_PATH;
use crate::platform::macos::virtual_hid::client::request::{
    VIRTUAL_KEYBOARD_PRODUCT_ID, VIRTUAL_KEYBOARD_VENDOR_ID,
};
use crate::platform::macos::virtual_hid::{
    OWN_DAEMON_LABEL, PQRS_DAEMON_BINARY, PQRS_DAEMON_LABEL, PQRS_MANAGER_BINARY,
};
use crate::platform::INPUT_MONITORING_FIX;

pub(crate) const HELPER_LABEL: &str = "com.qol-tools.keyremap.hid-helper";
pub(crate) const HELPER_BINARY: &str =
    "/Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper";
pub(crate) const HELPER_PLIST: &str =
    "/Library/LaunchDaemons/com.qol-tools.keyremap.hid-helper.plist";
pub(crate) const OWN_DAEMON_PLIST: &str =
    "/Library/LaunchDaemons/com.qol-tools.keyremap.vhid-daemon.plist";
const HELPER_LOG: &str = "/var/log/com.qol-tools.keyremap.hid-helper.log";
const LAUNCHCTL: &str = "/bin/launchctl";
const BOOTSTRAP_ATTEMPTS: u32 = 5;
const DEFAULTS: &str = "/usr/bin/defaults";
const CODESIGN: &str = "/usr/bin/codesign";
const SUDO: &str = "/usr/bin/sudo";
const SIGNING_ROOT: &str = "/private/tmp";
const KEYBOARD_TYPES: &str = "/Library/Preferences/com.apple.keyboardtype";

pub(crate) fn install() -> Result<String> {
    ensure_root("install-hid-helper")?;
    ensure!(
        Path::new(PQRS_DAEMON_BINARY).exists(),
        "the virtual keyboard driver is not installed; install Karabiner-DriverKit-VirtualHIDDevice first"
    );
    let mut steps = Vec::new();
    let current = std::env::current_exe().context("find the running qol-keyremap binary")?;
    match qol_config::codesign_identity() {
        Some(identity) => {
            let signed = signed_copy(&current, &identity)?;
            let installed = install_file(&signed, HELPER_BINARY, 0o755);
            if let Some(folder) = signed.parent() {
                remove_signing_folder(folder);
            }
            installed?;
            steps.push(format!(
                "Copied {} to {HELPER_BINARY} and signed it as {identity}.",
                current.display()
            ));
        }
        None => {
            install_file(&current, HELPER_BINARY, 0o755)?;
            steps.push(format!("Copied {} to {HELPER_BINARY}.", current.display()));
            steps.push(
                "No code signing identity is configured, so macOS forgets the Input Monitoring grant every time you reinstall. Set QOL_CODESIGN_IDENTITY and start qol-tray once to remember one."
                    .to_string(),
            );
        }
    }
    write_file(HELPER_PLIST, &helper_plist(), 0o644)?;
    if launchd_loaded(PQRS_DAEMON_LABEL) {
        steps.push(format!("Left {PQRS_DAEMON_LABEL} running the pqrs daemon."));
    } else {
        write_file(OWN_DAEMON_PLIST, &daemon_plist(), 0o644)?;
        reload(OWN_DAEMON_LABEL, OWN_DAEMON_PLIST)?;
        steps.push(format!("Started the pqrs daemon as {OWN_DAEMON_LABEL}."));
    }
    if !virtual_keyboard_type_known() {
        let layout = physical_layout();
        let arguments = keyboard_type_arguments(layout);
        run(DEFAULTS, &arguments.each_ref().map(String::as_str))?;
        steps.push(format!(
            "Told macOS the virtual keyboard is {layout:?}, so the Keyboard Setup Assistant does not ask."
        ));
    }
    reload(HELPER_LABEL, HELPER_PLIST)?;
    steps.push(format!("Started {HELPER_LABEL}."));
    run(PQRS_MANAGER_BINARY, &["activate"])?;
    steps.push("Activated the driver extension.".to_string());
    steps.push(format!("Next: {INPUT_MONITORING_FIX}"));
    Ok(steps.join("\n"))
}

pub(crate) fn uninstall() -> Result<String> {
    ensure_root("uninstall-hid-helper")?;
    let mut steps = Vec::new();
    bootout(HELPER_LABEL);
    for path in [HELPER_PLIST, HELPER_BINARY, SOCKET_PATH] {
        remove_if_present(path)?;
    }
    steps.push(format!("Stopped and removed {HELPER_LABEL}."));
    if Path::new(OWN_DAEMON_PLIST).exists() {
        bootout(OWN_DAEMON_LABEL);
        remove_if_present(OWN_DAEMON_PLIST)?;
        steps.push(format!("Stopped and removed {OWN_DAEMON_LABEL}."));
    }
    steps.push("Left the driver package installed.".to_string());
    Ok(steps.join("\n"))
}

pub(crate) fn launchd_loaded(label: &str) -> bool {
    Command::new(LAUNCHCTL)
        .args(["print", &format!("system/{label}")])
        .output()
        .is_ok_and(|output| output.status.success())
}

pub(crate) fn helper_plist() -> String {
    plist(
        HELPER_LABEL,
        &[HELPER_BINARY, "hid-helper"],
        Some(HELPER_LOG),
    )
}

pub(crate) fn daemon_plist() -> String {
    plist(OWN_DAEMON_LABEL, &[PQRS_DAEMON_BINARY], None)
}

fn plist(label: &str, arguments: &[&str], log: Option<&str>) -> String {
    let arguments: String = arguments
        .iter()
        .map(|argument| format!("        <string>{argument}</string>\n"))
        .collect();
    let log = log.map_or(String::new(), |path| {
        format!("    <key>StandardErrorPath</key>\n    <string>{path}</string>\n")
    });
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{label}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n\
{arguments}    </array>\n\
    <key>RunAtLoad</key>\n\
    <true/>\n\
    <key>KeepAlive</key>\n\
    <true/>\n\
    <key>ProcessType</key>\n\
    <string>Interactive</string>\n\
{log}</dict>\n\
</plist>\n"
    )
}

fn ensure_root(command: &str) -> Result<()> {
    if !super::is_root() {
        bail!("{command} changes system files; run it with sudo: sudo qol-keyremap {command}");
    }
    Ok(())
}

fn install_file(source: &Path, target: &str, mode: u32) -> Result<()> {
    let staging = format!("{target}.new");
    fs::copy(source, &staging).with_context(|| format!("copy to {staging}"))?;
    finish_file(&staging, target, mode)
}

fn write_file(target: &str, contents: &str, mode: u32) -> Result<()> {
    let staging = format!("{target}.new");
    fs::write(&staging, contents).with_context(|| format!("write {staging}"))?;
    finish_file(&staging, target, mode)
}

fn finish_file(staging: &str, target: &str, mode: u32) -> Result<()> {
    std::os::unix::fs::chown(staging, Some(0), Some(0))
        .with_context(|| format!("chown {staging}"))?;
    fs::set_permissions(staging, fs::Permissions::from_mode(mode))
        .with_context(|| format!("chmod {staging}"))?;
    fs::rename(staging, target).with_context(|| format!("move {staging} to {target}"))
}

fn remove_if_present(path: &str) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("remove {path}"))
        }
        _ => Ok(()),
    }
}

fn reload(label: &str, plist: &str) -> Result<()> {
    bootout(label);
    let mut last = Ok(());
    for _ in 0..BOOTSTRAP_ATTEMPTS {
        last = run(LAUNCHCTL, &["bootstrap", "system", plist]);
        if last.is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    last
}

fn bootout(label: &str) {
    let _ = Command::new(LAUNCHCTL)
        .args(["bootout", &format!("system/{label}")])
        .output();
}

fn virtual_keyboard_type_known() -> bool {
    Command::new(DEFAULTS)
        .args(["read", KEYBOARD_TYPES, "keyboardtype"])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .contains(&format!("\"{}\"", virtual_keyboard_id()))
        })
}

fn signed_copy(source: &Path, identity: &str) -> Result<PathBuf> {
    let folder = Path::new(SIGNING_ROOT).join(format!("{HELPER_LABEL}.{}", std::process::id()));
    fs::create_dir(&folder).with_context(|| format!("create {}", folder.display()))?;
    let signed = sign_in(&folder, source, identity);
    if signed.is_err() {
        remove_signing_folder(&folder);
    }
    signed
}

fn remove_signing_folder(folder: &Path) {
    let folder = folder.to_string_lossy();
    let _ = match signing_user() {
        Some(user) => run(SUDO, &["-u", &user, "/bin/rm", "-rf", &folder]),
        None => fs::remove_dir_all(folder.as_ref()).map_err(Into::into),
    };
}

fn signing_user() -> Option<String> {
    std::env::var("SUDO_USER")
        .ok()
        .filter(|user| !user.is_empty())
}

fn sign_in(folder: &Path, source: &Path, identity: &str) -> Result<PathBuf> {
    let staging = folder.join("qol-keyremap");
    fs::copy(source, &staging).with_context(|| format!("copy to {}", staging.display()))?;
    let signer = signing_user();
    if signer.is_some() {
        let id = |key| std::env::var(key).ok().and_then(|value| value.parse().ok());
        for path in [folder, staging.as_path()] {
            std::os::unix::fs::chown(path, id("SUDO_UID"), id("SUDO_GID"))
                .with_context(|| format!("chown {}", path.display()))?;
        }
    }
    let staging_path = staging.to_string_lossy();
    let (program, arguments) = codesign_command(identity, &staging_path, signer.as_deref());
    let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
    run(program, &arguments)?;
    Ok(staging)
}

fn codesign_command(
    identity: &str,
    path: &str,
    signer: Option<&str>,
) -> (&'static str, Vec<String>) {
    let codesign = [
        CODESIGN,
        "--force",
        "--sign",
        identity,
        "--identifier",
        HELPER_LABEL,
        path,
    ];
    match signer {
        Some(user) => (
            SUDO,
            ["-u", user]
                .into_iter()
                .chain(codesign)
                .map(str::to_owned)
                .collect(),
        ),
        None => (
            CODESIGN,
            codesign[1..].iter().map(|arg| (*arg).to_owned()).collect(),
        ),
    }
}

fn virtual_keyboard_id() -> String {
    format!("{VIRTUAL_KEYBOARD_PRODUCT_ID}-{VIRTUAL_KEYBOARD_VENDOR_ID}-0")
}

fn keyboard_type_arguments(layout: PhysicalLayout) -> [String; 7] {
    let code = match layout {
        PhysicalLayout::Ansi => "40",
        PhysicalLayout::Iso => "41",
        PhysicalLayout::Jis => "42",
    };
    [
        "write",
        KEYBOARD_TYPES,
        "keyboardtype",
        "-dict-add",
        &virtual_keyboard_id(),
        "-int",
        code,
    ]
    .map(str::to_owned)
}

fn run(program: &str, arguments: &[&str]) -> Result<()> {
    let output = Command::new(program)
        .args(arguments)
        .output()
        .with_context(|| format!("run {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lint(contents: &str) {
        let path =
            std::env::temp_dir().join(format!("qol-keyremap-plist-{}.plist", std::process::id()));
        std::fs::write(&path, contents).unwrap();
        let status = std::process::Command::new("/usr/bin/plutil")
            .arg("-lint")
            .arg(&path)
            .status()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(status.success(), "plutil rejected:\n{contents}");
    }

    #[test]
    fn the_helper_plist_runs_the_root_copy_as_hid_helper() {
        let plist = helper_plist();
        lint(&plist);
        assert!(plist.contains("<string>com.qol-tools.keyremap.hid-helper</string>"));
        assert!(plist.contains(&format!(
            "<string>{HELPER_BINARY}</string>\n        <string>hid-helper</string>"
        )));
        assert!(plist.contains("<key>KeepAlive</key>"));
    }

    #[test]
    fn the_daemon_plist_runs_the_pqrs_daemon_under_our_label() {
        let plist = daemon_plist();
        lint(&plist);
        assert!(plist.contains("<string>com.qol-tools.keyremap.vhid-daemon</string>"));
        assert!(plist.contains(PQRS_DAEMON_BINARY));
    }

    #[test]
    fn the_virtual_keyboard_is_recorded_with_the_physical_keyboard_type() {
        for (layout, code) in [
            (PhysicalLayout::Ansi, "40"),
            (PhysicalLayout::Iso, "41"),
            (PhysicalLayout::Jis, "42"),
        ] {
            assert_eq!(
                keyboard_type_arguments(layout),
                [
                    "write",
                    "/Library/Preferences/com.apple.keyboardtype",
                    "keyboardtype",
                    "-dict-add",
                    "10203-5824-0",
                    "-int",
                    code
                ]
            );
        }
    }

    #[test]
    fn the_helper_is_signed_as_the_invoking_user_under_a_stable_identifier() {
        let (program, arguments) = codesign_command("KMRH47", "/tmp/helper", Some("kaho"));
        assert_eq!(program, "/usr/bin/sudo");
        assert_eq!(
            arguments,
            [
                "-u",
                "kaho",
                "/usr/bin/codesign",
                "--force",
                "--sign",
                "KMRH47",
                "--identifier",
                "com.qol-tools.keyremap.hid-helper",
                "/tmp/helper"
            ]
        );
        let (program, arguments) = codesign_command("KMRH47", "/tmp/helper", None);
        assert_eq!(program, "/usr/bin/codesign");
        assert_eq!(arguments[0], "--force");
        assert_eq!(arguments.len(), 6);
    }

    #[test]
    fn installing_without_root_names_sudo() {
        let error = install().unwrap_err().to_string();
        assert!(error.contains("sudo"), "{error}");
        let error = uninstall().unwrap_err().to_string();
        assert!(error.contains("sudo"), "{error}");
    }
}
