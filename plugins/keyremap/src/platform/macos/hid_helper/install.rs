use std::ffi::{c_char, c_int, c_uint, CString};
use std::fs;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, ensure, Context, Result};
use qol_hotkeys::layout::physical_layout;
use qol_hotkeys::macos_keycode::PhysicalLayout;

use super::peer_signature::{client_requirement, is_cdhash};
use super::protocol::SOCKET_PATH;
use crate::platform::macos::doctor::INPUT_MONITORING_FIX;
use crate::platform::macos::virtual_hid::client::request::{
    VIRTUAL_KEYBOARD_PRODUCT_ID, VIRTUAL_KEYBOARD_VENDOR_ID,
};
use crate::platform::macos::virtual_hid::{
    OWN_DAEMON_LABEL, PQRS_DAEMON_BINARY, PQRS_DAEMON_LABEL, PQRS_MANAGER_BINARY,
};

pub(crate) const HELPER_LABEL: &str = "com.qol-tools.keyremap.hid-helper";
const PRIVILEGED_DIR: &str = "/Library/PrivilegedHelperTools";
pub(crate) const HELPER_BINARY: &str =
    "/Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper";
pub(crate) const CLIENT_IDENTIFIER: &str = "com.qol-tools.keyremap";
pub(crate) const REQUIREMENT_PATH: &str =
    "/Library/PrivilegedHelperTools/com.qol-tools.keyremap.hid-helper.requirement";
pub(crate) const HELPER_PLIST: &str =
    "/Library/LaunchDaemons/com.qol-tools.keyremap.hid-helper.plist";
pub(crate) const OWN_DAEMON_PLIST: &str =
    "/Library/LaunchDaemons/com.qol-tools.keyremap.vhid-daemon.plist";
const HELPER_LOG: &str = "/var/log/com.qol-tools.keyremap.hid-helper.log";
const LAUNCHCTL: &str = "/bin/launchctl";
const BOOTSTRAP_ATTEMPTS: u32 = 5;
const DEFAULTS: &str = "/usr/bin/defaults";
const CODESIGN: &str = "/usr/bin/codesign";
const LOGIN_KEYCHAIN: &str = "Library/Keychains/login.keychain-db";
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
            let cdhash = install_signed(&current, &identity)?;
            steps.push(format!(
                "Copied {} to {HELPER_BINARY} and signed it as {identity}.",
                current.display()
            ));
            steps.push(format!(
                "Pinned {} (cdhash {cdhash}) as the only Key Remap build the helper trusts. Install the helper again after every Key Remap update.",
                current.display()
            ));
        }
        None => {
            install_file(&current, HELPER_BINARY, 0o755)?;
            steps.push(format!("Copied {} to {HELPER_BINARY}.", current.display()));
            steps.push(
                "No code signing identity is configured, so the helper cannot verify keyremap and Key Remap stays on the event tap. Set QOL_CODESIGN_IDENTITY, start qol-tray once to remember it, then install again."
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
    for path in [HELPER_PLIST, HELPER_BINARY, REQUIREMENT_PATH, SOCKET_PATH] {
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

fn install_signed(source: &Path, identity: &str) -> Result<String> {
    let keychain = login_keychain()?;
    let client = fs::canonicalize(source).context("resolve the running qol-keyremap binary")?;
    let stage = Stage::create_in(Path::new(PRIVILEGED_DIR))?;
    let staged = stage.copy_of(&client)?;
    sign(&codesign_arguments(
        identity,
        &keychain,
        CLIENT_IDENTIFIER,
        &staged,
    ))?;
    let cdhash = client_cdhash(&staged)?;
    let requirement = client_requirement(CLIENT_IDENTIFIER, &cdhash)?;
    let helper = format!("{HELPER_BINARY}.new");
    fs::copy(&staged, &helper).with_context(|| format!("copy to {helper}"))?;
    let signed = sign(&codesign_arguments(
        identity,
        &keychain,
        HELPER_LABEL,
        &helper,
    ));
    if signed.is_err() {
        let _ = fs::remove_file(&helper);
    }
    signed?;
    finish_file(&helper, HELPER_BINARY, 0o755)?;
    place_signed_client(Path::new(&staged), &client)?;
    write_file(REQUIREMENT_PATH, &requirement, 0o644)?;
    Ok(cdhash)
}

struct Stage(std::path::PathBuf);

impl Stage {
    fn create_in(root: &Path) -> Result<Self> {
        let dir = root.join(format!("{HELPER_LABEL}.stage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir(&dir).with_context(|| format!("create {}", dir.display()))?;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("chmod {}", dir.display()))?;
        Ok(Self(dir))
    }

    fn copy_of(&self, client: &Path) -> Result<String> {
        let staged = self.0.join("qol-keyremap");
        fs::copy(client, &staged).with_context(|| format!("copy to {}", staged.display()))?;
        staged
            .to_str()
            .map(str::to_owned)
            .context("the staging path is not valid UTF-8")
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

extern "C" {
    fn openat(dir: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    fn renameat(from_dir: c_int, from: *const c_char, to_dir: c_int, to: *const c_char) -> c_int;
    fn unlinkat(dir: c_int, path: *const c_char, flags: c_int) -> c_int;
    fn fchown(fd: c_int, uid: u32, gid: u32) -> c_int;
    fn fchmod(fd: c_int, mode: u16) -> c_int;
}

const O_WRONLY: c_int = 0x1;
const O_CREAT: c_int = 0x200;
const O_EXCL: c_int = 0x800;
const O_NOFOLLOW: c_int = 0x100;
const O_DIRECTORY: c_int = 0x0010_0000;
const O_CLOEXEC: c_int = 0x0100_0000;

fn place_signed_client(staged: &Path, client: &Path) -> Result<()> {
    let folder = client
        .parent()
        .with_context(|| format!("{} has no folder", client.display()))?;
    let name = client
        .file_name()
        .with_context(|| format!("{} has no name", client.display()))?;
    let original =
        fs::symlink_metadata(client).with_context(|| format!("stat {}", client.display()))?;
    ensure!(
        original.is_file(),
        "{} is not a regular file",
        client.display()
    );
    let folder_fd = fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_DIRECTORY | O_NOFOLLOW)
        .open(folder)
        .with_context(|| format!("open {}", folder.display()))?;
    let temp_name = c_name(&format!(
        ".{}.signed-{}",
        name.to_string_lossy(),
        std::process::id()
    ))?;
    let fd = unsafe {
        openat(
            folder_fd.as_raw_fd(),
            temp_name.as_ptr(),
            O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC,
            0o700 as c_uint,
        )
    };
    ensure!(
        fd >= 0,
        "cannot create the signed copy in {}: {}",
        folder.display(),
        std::io::Error::last_os_error()
    );
    let mut file = unsafe { fs::File::from_raw_fd(fd) };
    let placed = fill_and_swap(
        &mut file,
        staged,
        &folder_fd,
        &temp_name,
        &c_name(&name.to_string_lossy())?,
        &original,
    );
    if placed.is_err() {
        let _ = unsafe { unlinkat(folder_fd.as_raw_fd(), temp_name.as_ptr(), 0) };
    }
    placed
}

fn fill_and_swap(
    file: &mut fs::File,
    staged: &Path,
    folder_fd: &fs::File,
    temp_name: &CString,
    name: &CString,
    original: &fs::Metadata,
) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    std::io::copy(
        &mut fs::File::open(staged).with_context(|| format!("open {}", staged.display()))?,
        file,
    )
    .context("write the signed copy")?;
    file.sync_all().context("flush the signed copy")?;
    let fd = file.as_raw_fd();
    ensure!(
        unsafe { fchown(fd, original.uid(), original.gid()) } == 0,
        "cannot give the signed copy back to its owner: {}",
        std::io::Error::last_os_error()
    );
    ensure!(
        unsafe { fchmod(fd, (original.mode() & 0o7777) as u16) } == 0,
        "cannot restore the signed copy's mode: {}",
        std::io::Error::last_os_error()
    );
    let folder = folder_fd.as_raw_fd();
    ensure!(
        unsafe { renameat(folder, temp_name.as_ptr(), folder, name.as_ptr()) } == 0,
        "cannot move the signed copy over the client: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

fn c_name(name: &str) -> Result<CString> {
    CString::new(name).context("the file name holds a NUL byte")
}

fn sign(arguments: &[String]) -> Result<()> {
    run(
        CODESIGN,
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

fn client_cdhash(path: &str) -> Result<String> {
    let output = Command::new(CODESIGN)
        .args(["--display", "--verbose=4", path])
        .output()
        .with_context(|| format!("run {CODESIGN}"))?;
    ensure!(
        output.status.success(),
        "{CODESIGN} --display {path} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    parse_cdhash(&String::from_utf8_lossy(&output.stderr))
        .with_context(|| format!("{CODESIGN} printed no CDHash for {path}"))
}

fn parse_cdhash(display: &str) -> Option<String> {
    display
        .lines()
        .find_map(|line| line.trim().strip_prefix("CDHash="))
        .map(str::trim)
        .filter(|hash| is_cdhash(hash))
        .map(str::to_owned)
}

fn login_keychain() -> Result<String> {
    let home =
        std::env::var("HOME").context("HOME is not set, so the login keychain is unknown")?;
    let keychain = Path::new(&home).join(LOGIN_KEYCHAIN);
    ensure!(
        keychain.exists(),
        "no login keychain at {}",
        keychain.display()
    );
    Ok(keychain.to_string_lossy().into_owned())
}

fn codesign_arguments(identity: &str, keychain: &str, identifier: &str, path: &str) -> Vec<String> {
    [
        "--force",
        "--sign",
        identity,
        "--keychain",
        keychain,
        "--options",
        "runtime",
        "--identifier",
        identifier,
        path,
    ]
    .map(str::to_owned)
    .to_vec()
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
    fn the_helper_is_signed_with_the_login_keychain_under_a_stable_identifier() {
        assert_eq!(
            codesign_arguments(
                "KMRH47",
                "/Users/me/Library/Keychains/login.keychain-db",
                HELPER_LABEL,
                "/tmp/helper"
            ),
            [
                "--force",
                "--sign",
                "KMRH47",
                "--keychain",
                "/Users/me/Library/Keychains/login.keychain-db",
                "--options",
                "runtime",
                "--identifier",
                "com.qol-tools.keyremap.hid-helper",
                "/tmp/helper"
            ]
        );
    }

    #[test]
    fn the_client_is_signed_in_place_under_its_own_identifier() {
        assert_eq!(
            codesign_arguments(
                "KMRH47",
                "/Users/me/Library/Keychains/login.keychain-db",
                CLIENT_IDENTIFIER,
                "/plugins/keyremap/qol-keyremap"
            ),
            [
                "--force",
                "--sign",
                "KMRH47",
                "--keychain",
                "/Users/me/Library/Keychains/login.keychain-db",
                "--options",
                "runtime",
                "--identifier",
                "com.qol-tools.keyremap",
                "/plugins/keyremap/qol-keyremap"
            ]
        );
    }

    #[test]
    fn the_cdhash_is_read_from_the_codesign_display_output() {
        let display = "Executable=/plugins/keyremap/qol-keyremap\n\
Identifier=com.qol-tools.keyremap\n\
Format=Mach-O thin (arm64)\n\
CodeDirectory v=20500 size=1234 flags=0x10000(runtime) hashes=30+2 location=embedded\n\
Hash type=sha256 size=32\n\
CandidateCDHash sha256=1111111111111111111111111111111111111111\n\
CandidateCDHashFull sha256=2222222222222222222222222222222222222222222222222222222222222222\n\
CDHash=0123456789abcdef0123456789abcdef01234567\n\
Signature size=9000\n";
        assert_eq!(
            parse_cdhash(display).as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(parse_cdhash("Identifier=x\nCDHash=xyz\n"), None);
        assert_eq!(parse_cdhash("Identifier=x\n"), None);
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qol-keyremap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_stage_is_a_private_folder_removed_when_dropped() {
        let root = scratch("stage");
        let source = root.join("source");
        fs::write(&source, b"binary").unwrap();
        let stage = Stage::create_in(&root).unwrap();
        let dir = stage.0.clone();
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let staged = stage.copy_of(&source).unwrap();
        assert_eq!(fs::read(&staged).unwrap(), b"binary");
        drop(stage);
        assert!(!dir.exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_signed_copy_replaces_the_client_and_keeps_its_mode() {
        let root = scratch("place");
        let client = root.join("plugins").join("qol-keyremap");
        fs::create_dir_all(client.parent().unwrap()).unwrap();
        fs::write(&client, b"old").unwrap();
        fs::set_permissions(&client, fs::Permissions::from_mode(0o750)).unwrap();
        let staged = root.join("staged");
        fs::write(&staged, b"signed").unwrap();
        place_signed_client(&staged, &client).unwrap();
        assert_eq!(fs::read(&client).unwrap(), b"signed");
        assert_eq!(
            fs::metadata(&client).unwrap().permissions().mode() & 0o777,
            0o750
        );
        assert_eq!(fs::read_dir(client.parent().unwrap()).unwrap().count(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_planted_entry_at_the_temporary_name_stops_the_placement() {
        let root = scratch("planted");
        let client = root.join("qol-keyremap");
        fs::write(&client, b"old").unwrap();
        let planted = root.join(format!(".qol-keyremap.signed-{}", std::process::id()));
        std::os::unix::fs::symlink(root.join("victim"), &planted).unwrap();
        let staged = root.join("staged");
        fs::write(&staged, b"signed").unwrap();
        assert!(place_signed_client(&staged, &client).is_err());
        assert_eq!(fs::read(&client).unwrap(), b"old");
        assert!(!root.join("victim").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installing_without_root_names_sudo() {
        let error = install().unwrap_err().to_string();
        assert!(error.contains("sudo"), "{error}");
        let error = uninstall().unwrap_err().to_string();
        assert!(error.contains("sudo"), "{error}");
    }
}
