use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const TAR: &str = "/usr/bin/tar";
const CODESIGN: &str = "/usr/bin/codesign";
const AD_HOC: &str = "-";
const TRAY_ARCHIVE: &str = "qol-tray-macos-aarch64.tar.gz";

pub(in crate::commands::pr) fn tray_binary(tray_dir: &Path) -> Result<PathBuf> {
    let archive = tray_dir.join(TRAY_ARCHIVE);
    if !archive.is_file() {
        bail!("the build carries the tray but no {}", archive.display());
    }
    run(Command::new(TAR)
        .arg("-xzf")
        .arg(&archive)
        .arg("-C")
        .arg(tray_dir))
    .with_context(|| format!("unpacking {}", archive.display()))?;
    let binary = bundle_binary(tray_dir);
    if !binary.is_file() {
        bail!("{} holds no {}", archive.display(), binary.display());
    }
    Ok(binary)
}

pub(in crate::commands::pr) fn prepare_tray(binary: &Path) -> Result<()> {
    let bundle = bundle_root(binary)
        .with_context(|| format!("{} is not inside an app bundle", binary.display()))?;
    let identity = qol_config::codesign_identity().unwrap_or_else(|| AD_HOC.to_string());
    sign(&identity, bundle, None)
}

pub(in crate::commands::pr) fn prepare_plugin(binary: &Path, bin_name: &str) -> Result<()> {
    sign(AD_HOC, binary, Some(bin_name))
}

fn bundle_binary(tray_dir: &Path) -> PathBuf {
    tray_dir
        .join(format!("{}.app", qol_conventions::TRAY_DISPLAY_NAME))
        .join("Contents")
        .join("MacOS")
        .join(super::super::tray::BINARY)
}

fn bundle_root(binary: &Path) -> Option<&Path> {
    let bundle = binary.parent()?.parent()?.parent()?;
    bundle
        .extension()
        .is_some_and(|extension| extension == "app")
        .then_some(bundle)
}

fn sign(identity: &str, path: &Path, identifier: Option<&str>) -> Result<()> {
    let mut command = Command::new(CODESIGN);
    command.args(["--force", "--sign", identity]);
    if let Some(identifier) = identifier {
        command.args(["--identifier", identifier]);
    }
    run(command.arg(path)).with_context(|| format!("signing {}", path.display()))
}

fn run(command: &mut Command) -> Result<()> {
    let output = command
        .output()
        .with_context(|| format!("could not run {}", command.get_program().to_string_lossy()))?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verified(path: &Path) -> bool {
        Command::new(CODESIGN)
            .args(["--verify", "--strict"])
            .arg(path)
            .status()
            .is_ok_and(|status| status.success())
    }

    fn packed_bundle(root: &Path) -> PathBuf {
        let staging = root.join("staging");
        let binary = bundle_binary(&staging);
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
        let tray_dir = root.join("tray");
        std::fs::create_dir_all(&tray_dir).unwrap();
        run(Command::new(TAR)
            .arg("-czf")
            .arg(tray_dir.join(TRAY_ARCHIVE))
            .arg("-C")
            .arg(&staging)
            .arg(format!("{}.app", qol_conventions::TRAY_DISPLAY_NAME)))
        .unwrap();
        tray_dir
    }

    #[test]
    fn the_tray_unpacks_from_its_archive_into_an_app_bundle() {
        let root = tempfile::tempdir().unwrap();
        let tray_dir = packed_bundle(root.path());
        let binary = tray_binary(&tray_dir).unwrap();
        assert_eq!(
            binary,
            tray_dir.join("QoL Tray.app/Contents/MacOS/qol-tray")
        );
        assert!(binary.is_file());
        assert_eq!(
            bundle_root(&binary),
            Some(tray_dir.join("QoL Tray.app").as_path())
        );
    }

    #[test]
    fn a_build_without_the_tray_archive_says_so() {
        let root = tempfile::tempdir().unwrap();
        let error = tray_binary(root.path()).unwrap_err().to_string();
        assert!(error.contains(TRAY_ARCHIVE), "{error}");
    }

    #[test]
    fn only_a_binary_inside_an_app_bundle_has_a_bundle_root() {
        assert_eq!(
            bundle_root(Path::new("/c/QoL Tray.app/Contents/MacOS/qol-tray")),
            Some(Path::new("/c/QoL Tray.app"))
        );
        assert_eq!(bundle_root(Path::new("/c/tray/x/qol-tray")), None);
        assert_eq!(bundle_root(Path::new("/qol-tray")), None);
    }

    #[test]
    fn a_plugin_binary_is_signed_ad_hoc_in_place() {
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("qol-demo-macos-aarch64");
        std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
        prepare_plugin(&binary, "qol-demo").unwrap();
        assert!(verified(&binary));
        let details = Command::new(CODESIGN)
            .args(["--display", "--verbose=2"])
            .arg(&binary)
            .output()
            .unwrap();
        let details = String::from_utf8_lossy(&details.stderr);
        assert!(details.contains("Identifier=qol-demo"), "{details}");
        assert!(details.contains("Signature=adhoc"), "{details}");
    }

    #[test]
    fn a_signing_failure_names_the_file() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("qol-demo");
        let error = format!("{:#}", prepare_plugin(&missing, "qol-demo").unwrap_err());
        assert!(error.contains("signing"), "{error}");
        assert!(error.contains("qol-demo"), "{error}");
    }
}
