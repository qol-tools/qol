use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const ICNS_DATA: &[u8] = include_bytes!("../../../assets/qol-tray.icns");
const APP_NAME: &str = "QoL Tray";
const BUNDLE_ID: &str = "com.qol-tools.qol-tray";

pub(super) fn install_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home
        .join("Applications")
        .join(format!("{APP_NAME}.app"))
        .join("Contents")
        .join("MacOS"))
}

pub(super) fn autostart_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{BUNDLE_ID}.plist")))
}

pub(super) fn write_autostart_entry(binary_path: &Path) -> Result<()> {
    let path = autostart_path()?;
    let binary = xml_escape(&binary_path.display().to_string());
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>Label</key>\n\
         <string>{BUNDLE_ID}</string>\n\
         <key>ProgramArguments</key>\n\
         <array>\n\
         <string>{}</string>\n\
         </array>\n\
         <key>RunAtLoad</key>\n\
         <true/>\n\
         <key>KeepAlive</key>\n\
         <false/>\n\
         </dict>\n\
         </plist>\n",
        binary
    );
    super::write_text_file(&path, &plist)
}

pub(super) fn register_application(binary_path: &Path) -> Result<()> {
    let bundle_root = bundle_root_from_binary(binary_path)?;
    write_info_plist(&bundle_root)?;
    write_icon(&bundle_root)?;
    codesign_bundle(&bundle_root);
    Ok(())
}

pub(super) fn warn_system_install_conflict() {}

pub(super) fn remove_legacy_install() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let legacy_binary = home.join(".local").join("bin").join("qol-tray");
    let legacy_marker = home.join(".local").join("bin").join("qol-tray.install-id");
    if legacy_binary.exists() {
        println!("Removing legacy install at {}", legacy_binary.display());
        let _ = std::fs::remove_file(&legacy_binary);
    }
    if legacy_marker.exists() {
        let _ = std::fs::remove_file(&legacy_marker);
    }
}

pub(super) fn start_now(binary_path: &Path) -> Result<()> {
    if super::unix_common::is_running("qol-tray") {
        return Ok(());
    }
    super::unix_common::start_now(binary_path)
}

pub(super) fn stop_running(binary_path: &Path) -> Result<()> {
    super::unix_common::stop_running(binary_path, "qol-tray")
}

pub(super) fn set_executable_permissions(path: &Path) -> Result<()> {
    super::unix_common::set_executable_permissions(path)
}

pub(super) fn prepare_atomic_replace(_: &Path) -> Result<()> {
    Ok(())
}

fn bundle_root_from_binary(binary_path: &Path) -> Result<PathBuf> {
    binary_path
        .parent()
        .and_then(|macos| macos.parent())
        .and_then(|contents| contents.parent())
        .map(|root| root.to_path_buf())
        .context("Binary path is not inside an .app bundle")
}

fn write_info_plist(bundle_root: &Path) -> Result<()> {
    let plist_path = bundle_root.join("Contents").join("Info.plist");
    let version = env!("CARGO_PKG_VERSION");
    let plist = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>CFBundleIdentifier</key>\n\
         <string>{BUNDLE_ID}</string>\n\
         <key>CFBundleName</key>\n\
         <string>{APP_NAME}</string>\n\
         <key>CFBundleExecutable</key>\n\
         <string>qol-tray</string>\n\
         <key>CFBundleIconFile</key>\n\
         <string>qol-tray</string>\n\
         <key>CFBundleVersion</key>\n\
         <string>{version}</string>\n\
         <key>CFBundleShortVersionString</key>\n\
         <string>{version}</string>\n\
         <key>LSUIElement</key>\n\
         <true/>\n\
         <key>LSMinimumSystemVersion</key>\n\
         <string>11.0</string>\n\
         </dict>\n\
         </plist>\n"
    );
    super::write_text_file(&plist_path, &plist)
}

fn write_icon(bundle_root: &Path) -> Result<()> {
    let resources = bundle_root.join("Contents").join("Resources");
    std::fs::create_dir_all(&resources)?;
    std::fs::write(resources.join("qol-tray.icns"), ICNS_DATA).context("Failed to write icon")
}

fn codesign_bundle(bundle_root: &Path) {
    let status = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(bundle_root)
        .output();
    match status {
        Ok(out) if out.status.success() => {}
        Ok(out) => log::warn!(
            "codesign ad-hoc failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => log::warn!("codesign not available: {e}"),
    }
}

fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
