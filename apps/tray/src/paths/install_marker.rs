use std::path::{Path, PathBuf};

pub(crate) const INSTALL_ID_FILE: &str = "qol-tray.install-id";

const MACOS_DIR: &str = "MacOS";
const CONTENTS_DIR: &str = "Contents";
const RESOURCES_DIR: &str = "Resources";

/// Where the install marker belongs for a binary at `binary_path`.
///
/// Inside a macOS app bundle the marker goes in `Contents/Resources`, not
/// beside the binary: `codesign` treats every file in `Contents/MacOS` as a
/// nested code object and refuses to sign the bundle when a plain text file
/// sits there.
pub(crate) fn marker_path(binary_path: &Path) -> Option<PathBuf> {
    let parent = binary_path.parent()?;
    match bundle_contents(parent) {
        Some(contents) => Some(contents.join(RESOURCES_DIR).join(INSTALL_ID_FILE)),
        None => Some(parent.join(INSTALL_ID_FILE)),
    }
}

/// The pre-bundle-aware location, kept so installs made before the move are
/// still recognised and can be cleaned up.
pub(crate) fn legacy_marker_path(binary_path: &Path) -> Option<PathBuf> {
    let parent = binary_path.parent()?;
    bundle_contents(parent).map(|_| parent.join(INSTALL_ID_FILE))
}

/// The marker that is actually on disk, preferring the current location.
pub(crate) fn existing_marker_path(binary_path: &Path) -> Option<PathBuf> {
    marker_path(binary_path)
        .filter(|path| path.exists())
        .or_else(|| legacy_marker_path(binary_path).filter(|path| path.exists()))
}

fn bundle_contents(binary_dir: &Path) -> Option<&Path> {
    if binary_dir.file_name()? != MACOS_DIR {
        return None;
    }
    let contents = binary_dir.parent()?;
    (contents.file_name()? == CONTENTS_DIR).then_some(contents)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE_BINARY: &str = "/Users/someone/Applications/QoL Tray.app/Contents/MacOS/qol-tray";

    #[test]
    fn bundle_marker_stays_out_of_the_signed_code_directory() {
        let marker = marker_path(Path::new(BUNDLE_BINARY)).expect("marker path");
        assert!(
            !marker.starts_with("/Users/someone/Applications/QoL Tray.app/Contents/MacOS"),
            "marker must not sit in Contents/MacOS, got {marker:?}"
        );
        assert_eq!(
            marker,
            PathBuf::from(
                "/Users/someone/Applications/QoL Tray.app/Contents/Resources/qol-tray.install-id"
            )
        );
    }

    #[test]
    fn plain_install_keeps_the_marker_beside_the_binary() {
        let marker = marker_path(Path::new("/home/someone/.local/bin/qol-tray")).expect("marker");
        assert_eq!(
            marker,
            PathBuf::from("/home/someone/.local/bin/qol-tray.install-id")
        );
    }

    #[test]
    fn legacy_location_only_exists_for_bundles() {
        assert_eq!(
            legacy_marker_path(Path::new(BUNDLE_BINARY)),
            Some(PathBuf::from(
                "/Users/someone/Applications/QoL Tray.app/Contents/MacOS/qol-tray.install-id"
            ))
        );
        assert_eq!(
            legacy_marker_path(Path::new("/home/someone/.local/bin/qol-tray")),
            None
        );
    }

    #[test]
    fn a_macos_directory_outside_a_bundle_is_not_treated_as_one() {
        let marker = marker_path(Path::new("/tmp/MacOS/qol-tray")).expect("marker");
        assert_eq!(marker, PathBuf::from("/tmp/MacOS/qol-tray.install-id"));
    }

    #[test]
    fn existing_marker_prefers_the_current_location_then_falls_back() {
        let root = std::env::temp_dir().join(format!("qol-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let macos = root.join("QoL Tray.app").join(CONTENTS_DIR).join(MACOS_DIR);
        let resources = root
            .join("QoL Tray.app")
            .join(CONTENTS_DIR)
            .join(RESOURCES_DIR);
        std::fs::create_dir_all(&macos).expect("macos dir");
        std::fs::create_dir_all(&resources).expect("resources dir");
        let binary = macos.join("qol-tray");
        std::fs::write(&binary, b"binary").expect("binary");

        assert_eq!(existing_marker_path(&binary), None);

        std::fs::write(macos.join(INSTALL_ID_FILE), b"legacy").expect("legacy marker");
        assert_eq!(
            existing_marker_path(&binary),
            Some(macos.join(INSTALL_ID_FILE))
        );

        std::fs::write(resources.join(INSTALL_ID_FILE), b"current").expect("current marker");
        assert_eq!(
            existing_marker_path(&binary),
            Some(resources.join(INSTALL_ID_FILE))
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
