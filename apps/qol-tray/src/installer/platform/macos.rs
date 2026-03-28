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

pub(super) fn should_bootstrap_current_install(binary_path: &Path) -> Result<bool> {
    let bundle_root = match bundle_root_from_binary(binary_path) {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    Ok(bootstrap_allowed_for_bundle_root(
        &bundle_root,
        dirs::home_dir().as_deref(),
    ))
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
    let macos = binary_path
        .parent()
        .context("Binary path is not inside an .app bundle")?;
    let contents = macos
        .parent()
        .context("Binary path is not inside an .app bundle")?;
    let bundle_root = contents
        .parent()
        .context("Binary path is not inside an .app bundle")?;
    if macos.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        anyhow::bail!("Binary path is not inside an .app bundle");
    }
    if contents.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        anyhow::bail!("Binary path is not inside an .app bundle");
    }
    if !bundle_root
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".app"))
    {
        anyhow::bail!("Binary path is not inside an .app bundle");
    }
    Ok(bundle_root.to_path_buf())
}

fn bootstrap_allowed_for_bundle_root(bundle_root: &Path, home: Option<&Path>) -> bool {
    if bundle_root.starts_with(Path::new("/Applications")) {
        return true;
    }

    let Some(home) = home else {
        return false;
    };
    bundle_root.starts_with(home.join("Applications"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_current_install_only_allows_installed_app_locations() {
        let home = Path::new("/Users/tester");
        let cases = [
            ("/Applications/QoL Tray.app", true),
            ("/Applications/Utilities/QoL Tray.app", true),
            ("/Users/tester/Applications/QoL Tray.app", true),
            ("/Users/tester/Applications/Utilities/QoL Tray.app", true),
            ("/Users/other/Applications/QoL Tray.app", false),
            ("/Volumes/QoL Tray/QoL Tray.app", false),
            ("/private/tmp/QoL Tray.app", false),
            ("/Users/tester/Downloads/QoL Tray.app", false),
        ];

        for (bundle_root, expected) in cases {
            assert_eq!(
                bootstrap_allowed_for_bundle_root(Path::new(bundle_root), Some(home)),
                expected,
                "bundle_root: {bundle_root}"
            );
        }
    }

    #[test]
    fn bootstrap_current_install_requires_home_for_user_applications() {
        assert!(!bootstrap_allowed_for_bundle_root(
            Path::new("/Users/tester/Applications/QoL Tray.app"),
            None
        ));
        assert!(bootstrap_allowed_for_bundle_root(
            Path::new("/Applications/QoL Tray.app"),
            None
        ));
    }

    #[test]
    fn bundle_root_from_binary_rejects_non_bundle_paths() {
        let cases = [
            "/tmp/qol-tray",
            "/Applications/QoL Tray.app",
            "/usr/local/bin/qol-tray",
        ];
        for path in cases {
            assert!(
                bundle_root_from_binary(Path::new(path)).is_err(),
                "path: {path}"
            );
        }
    }
}
