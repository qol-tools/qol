use std::path::PathBuf;

pub(super) fn discover_installed_apps() -> Vec<qol_apps::InstalledApp> {
    let roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
    ];
    qol_apps::macos_installed_apps(&roots, qol_apps::Spotlight::Roots(&roots))
}
