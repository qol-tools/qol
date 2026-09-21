use std::ffi::OsString;
use std::path::{Path, PathBuf};

const IDENTITY_ENV: &str = "QOL_CODESIGN_IDENTITY";
const IDENTITY_FILE: &str = "codesign-identity";
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
    let identity = resolve_identity(identity_from_env(), || {
        identity_path().and_then(|path| read_identity(&path))
    })?;
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

fn resolve_identity(
    from_env: Option<String>,
    remembered: impl FnOnce() -> Option<String>,
) -> Option<String> {
    from_env.or_else(remembered)
}

fn identity_from_env() -> Option<String> {
    non_empty(std::env::var(IDENTITY_ENV).ok()?)
}

fn identity_path() -> Option<PathBuf> {
    qol_config::config_dir().map(|dir| dir.join(IDENTITY_FILE))
}

fn read_identity(path: &Path) -> Option<String> {
    non_empty(std::fs::read_to_string(path).ok()?)
}

fn write_identity(path: &Path, identity: &str) {
    if read_identity(path).as_deref() == Some(identity) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, identity);
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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
    fn missing_identity_falls_back_to_ad_hoc() {
        let identity = resolve_identity(None, || None).unwrap_or_else(|| AD_HOC.to_string());
        let args = codesign_args(&identity, Path::new("/tmp/QoL Tray.app"));
        assert_eq!(args[2], OsString::from(AD_HOC));
    }

    #[test]
    fn bundle_path_is_the_last_argument() {
        let bundle = Path::new("/Users/someone/Applications/QoL Tray.app");
        let args = codesign_args("Test Signing Identity", bundle);
        assert_eq!(args.last().unwrap(), bundle.as_os_str());
    }

    #[test]
    fn env_identity_wins_over_remembered() {
        let identity = resolve_identity(Some("FROM-ENV".into()), || Some("REMEMBERED".into()));
        assert_eq!(identity.as_deref(), Some("FROM-ENV"));
    }

    #[test]
    fn remembered_identity_is_used_when_env_is_unset() {
        let identity = resolve_identity(None, || Some("REMEMBERED".into()));
        assert_eq!(identity.as_deref(), Some("REMEMBERED"));
    }

    #[test]
    fn blank_identity_is_rejected() {
        assert_eq!(non_empty(String::new()), None);
        assert_eq!(non_empty("   \n".into()), None);
        assert_eq!(
            non_empty("  Test Signing Identity \n".into()).as_deref(),
            Some("Test Signing Identity")
        );
    }

    #[test]
    fn identity_round_trips_through_the_config_file() {
        let dir = scratch_dir("round-trip");
        let path = dir.join(IDENTITY_FILE);

        assert_eq!(read_identity(&path), None);
        write_identity(&path, "Test Signing Identity");
        assert_eq!(
            read_identity(&path).as_deref(),
            Some("Test Signing Identity")
        );

        write_identity(&path, "OTHER");
        assert_eq!(read_identity(&path).as_deref(), Some("OTHER"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembering_creates_the_config_directory() {
        let dir = scratch_dir("create-parent");
        let path = dir.join("nested").join(IDENTITY_FILE);

        write_identity(&path, "Test Signing Identity");

        assert_eq!(
            read_identity(&path).as_deref(),
            Some("Test Signing Identity")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
