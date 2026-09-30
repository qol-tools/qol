use std::ffi::OsString;
use std::path::{Path, PathBuf};

const AD_HOC: &str = "-";

pub(crate) fn codesign_bundle(bundle_root: &Path) {
    let identity = configured_identity().unwrap_or_else(|| AD_HOC.to_string());
    let output = std::process::Command::new("codesign")
        .args(codesign_args(&identity, bundle_root))
        .output();
    match output {
        Ok(out) if out.status.success() => {
            log::info!("codesigned {} as {identity}", bundle_root.display());
        }
        Ok(out) => log::warn!(
            "codesign as {identity} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => log::warn!("codesign not available: {e}"),
    }
}

pub(crate) fn configured_identity() -> Option<String> {
    let identity = qol_config::codesign_identity()?;
    if let Some(path) = identity_path() {
        write_identity(&path, &identity);
    }
    Some(identity)
}

fn codesign_args(identity: &str, bundle_root: &Path) -> Vec<OsString> {
    vec![
        OsString::from("--force"),
        OsString::from("--sign"),
        OsString::from(identity),
        bundle_root.as_os_str().to_os_string(),
    ]
}

fn identity_path() -> Option<PathBuf> {
    qol_config::codesign_identity_path()
}

fn write_identity(path: &Path, identity: &str) {
    if qol_config::codesign_identity_at(path).as_deref() == Some(identity) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, identity);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qol-codesign-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn configured_identity_is_signed_with_instead_of_ad_hoc() {
        let args = codesign_args("Test Signing Identity", Path::new("/tmp/QoL Tray.app"));
        assert_eq!(args[1], OsString::from("--sign"));
        assert_eq!(args[2], OsString::from("Test Signing Identity"));
        assert_ne!(args[2], OsString::from(AD_HOC));
    }

    #[test]
    fn bundle_path_is_the_last_argument() {
        let bundle = Path::new("/Users/someone/Applications/QoL Tray.app");
        let args = codesign_args("Test Signing Identity", bundle);
        assert_eq!(args.last().unwrap(), bundle.as_os_str());
    }

    #[test]
    fn identity_round_trips_through_the_config_file() {
        let dir = scratch_dir("round-trip");
        let path = dir.join(qol_conventions::CODESIGN_IDENTITY_FILE);

        assert_eq!(qol_config::codesign_identity_at(&path), None);
        write_identity(&path, "Test Signing Identity");
        assert_eq!(
            qol_config::codesign_identity_at(&path).as_deref(),
            Some("Test Signing Identity")
        );

        write_identity(&path, "OTHER");
        assert_eq!(
            qol_config::codesign_identity_at(&path).as_deref(),
            Some("OTHER")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembering_creates_the_config_directory() {
        let dir = scratch_dir("create-parent");
        let path = dir
            .join("nested")
            .join(qol_conventions::CODESIGN_IDENTITY_FILE);

        write_identity(&path, "Test Signing Identity");

        assert_eq!(
            qol_config::codesign_identity_at(&path).as_deref(),
            Some("Test Signing Identity")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
