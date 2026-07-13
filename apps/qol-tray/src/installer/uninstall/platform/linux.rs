use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use super::PlatformOps;
use crate::installer::uninstall::model::{
    ArtifactId, ArtifactSpec, Operation, OwnershipProof, PreserveSpec, ProcessTargets,
    UninstallContext,
};

const ICON_64: &[u8] = include_bytes!("../../../../assets/icons/64.png");
const ICON_128: &[u8] = include_bytes!("../../../../assets/icons/128.png");
const ICON_256: &[u8] = include_bytes!("../../../../assets/icons/256.png");
const AUTOSTART_MARKERS: &[&str] = &[
    "[Desktop Entry]",
    "Name=QoL Tray",
    "Icon=qol-tray",
    "X-GNOME-Autostart-enabled=true",
];
const DESKTOP_MARKERS: &[&str] = &[
    "[Desktop Entry]",
    "Name=QoL Tray",
    "Icon=qol-tray",
    "MimeType=x-scheme-handler/qol;",
];

pub(in crate::installer::uninstall) struct Platform;

impl PlatformOps for Platform {
    fn context(&self) -> Result<UninstallContext> {
        resolve_context()
    }

    fn managed_processes(&self) -> Vec<crate::plugins::daemon_tracker::ManagedProcess> {
        crate::plugins::daemon_tracker::managed_processes()
    }

    fn stop_processes(&self, targets: &ProcessTargets) -> Result<()> {
        if let Some(binary) = &targets.installed_binary {
            crate::installer::platform::stop_running(binary)?;
        }
        crate::plugins::daemon_tracker::kill_managed_processes(&targets.plugins);
        Ok(())
    }

    fn refresh_desktop_caches(&self, context: &UninstallContext) -> Result<()> {
        refresh_caches(&context.refresh_root);
        Ok(())
    }
}

fn resolve_context() -> Result<UninstallContext> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let config_home = dirs::config_dir().context("Could not determine config directory")?;
    let data_home = dirs::data_dir().context("Could not determine data directory")?;
    let config_root = crate::paths::shared_config_dir()?;
    let data_root = crate::paths::base_data_dir()?;
    let install_dir = crate::installer::platform::install_dir()?;
    Ok(context_from_roots(
        &home,
        &config_home,
        &data_home,
        &config_root,
        &data_root,
        &install_dir,
        crate::paths::runtime_dir(),
    ))
}

fn context_from_roots(
    home: &Path,
    config_home: &Path,
    data_home: &Path,
    config_root: &Path,
    data_root: &Path,
    install_dir: &Path,
    runtime_dir: PathBuf,
) -> UninstallContext {
    let roots = ResolvedRoots {
        home: home.to_path_buf(),
        config_home: config_home.to_path_buf(),
        data_home: data_home.to_path_buf(),
        config_root: config_root.to_path_buf(),
        data_root: data_root.to_path_buf(),
        install_dir: install_dir.to_path_buf(),
        runtime_dir,
    };
    UninstallContext {
        platform: "linux",
        artifacts: artifacts(&roots),
        purge_artifacts: purge_artifacts(config_root, data_root),
        preserved: preserved_roots(config_root, data_root),
        refresh_root: data_home.to_path_buf(),
    }
}

struct ResolvedRoots {
    home: PathBuf,
    config_home: PathBuf,
    data_home: PathBuf,
    config_root: PathBuf,
    data_root: PathBuf,
    install_dir: PathBuf,
    runtime_dir: PathBuf,
}

fn artifacts(roots: &ResolvedRoots) -> Vec<ArtifactSpec> {
    let mut artifacts = desktop_artifacts(roots);
    artifacts.extend(runtime_artifacts(roots));
    artifacts
}

fn desktop_artifacts(roots: &ResolvedRoots) -> Vec<ArtifactSpec> {
    let app_dir = roots.data_home.join("applications");
    let icon_root = roots.data_home.join("icons/hicolor");
    vec![
        shell_hook(ArtifactId::ShellHookBash, roots.home.join(".bashrc")),
        shell_hook(ArtifactId::ShellHookZsh, roots.home.join(".zshrc")),
        text_file(
            ArtifactId::Autostart,
            roots.config_home.join("autostart/qol-tray.desktop"),
            AUTOSTART_MARKERS,
        ),
        text_file(
            ArtifactId::DesktopEntry,
            app_dir.join("qol-tray.desktop"),
            DESKTOP_MARKERS,
        ),
        mime_file(
            ArtifactId::MimeDefault,
            roots.config_home.join("mimeapps.list"),
        ),
        mime_file(ArtifactId::MimeData, app_dir.join("mimeapps.list")),
        mime_file(ArtifactId::MimeCache, app_dir.join("mimeinfo.cache")),
        exact_file(
            ArtifactId::Icon64,
            icon_root.join("64x64/apps/qol-tray.png"),
            ICON_64,
        ),
        exact_file(
            ArtifactId::Icon128,
            icon_root.join("128x128/apps/qol-tray.png"),
            ICON_128,
        ),
        exact_file(
            ArtifactId::Icon256,
            icon_root.join("256x256/apps/qol-tray.png"),
            ICON_256,
        ),
    ]
}

fn runtime_artifacts(roots: &ResolvedRoots) -> Vec<ArtifactSpec> {
    let binary = roots.install_dir.join(crate::installer::binary_filename());
    let marker = roots.install_dir.join("qol-tray.install-id");
    vec![
        directory(ArtifactId::RuntimeDirectory, roots.runtime_dir.clone()),
        file(ArtifactId::ModeConfig, roots.config_root.join("mode.json")),
        valid_install_id_file(
            ArtifactId::ActiveInstallId,
            roots.data_root.join(qol_config::ACTIVE_INSTALL_ID_FILE),
        ),
        binary_file(
            ArtifactId::StagedBinary,
            binary.with_extension("new"),
            &marker,
        ),
        binary_file(ArtifactId::Binary, binary, &marker),
        install_marker(marker),
    ]
}

fn purge_artifacts(config_root: &Path, data_root: &Path) -> Vec<ArtifactSpec> {
    let mut config = directory(ArtifactId::ConfigDirectory, config_root.to_path_buf());
    let mut data = directory(ArtifactId::DataDirectory, data_root.to_path_buf());
    let dependencies = vec![ArtifactId::Binary, ArtifactId::InstallMarker];
    config.depends_on = dependencies.clone();
    data.depends_on = dependencies;
    vec![config, data]
}

fn preserved_roots(config_root: &Path, data_root: &Path) -> Vec<PreserveSpec> {
    vec![
        PreserveSpec {
            id: ArtifactId::ConfigDirectory,
            path: config_root.to_path_buf(),
            reason: "profiles, plugins, settings, and migration history are user data",
        },
        PreserveSpec {
            id: ArtifactId::DataDirectory,
            path: data_root.to_path_buf(),
            reason: "plugin state, logs, and reusable application data are user data",
        },
    ]
}

fn file(id: ArtifactId, path: PathBuf) -> ArtifactSpec {
    artifact(id, Operation::RemoveFile, path, OwnershipProof::AnyFile)
}

fn valid_install_id_file(id: ArtifactId, path: PathBuf) -> ArtifactSpec {
    artifact(
        id,
        Operation::RemoveFile,
        path,
        OwnershipProof::ValidInstallId,
    )
}

fn directory(id: ArtifactId, path: PathBuf) -> ArtifactSpec {
    artifact(
        id,
        Operation::RemoveDirectory,
        path,
        OwnershipProof::AnyDirectory,
    )
}

fn binary_file(id: ArtifactId, path: PathBuf, marker: &Path) -> ArtifactSpec {
    let mut spec = artifact(
        id,
        Operation::RemoveFile,
        path,
        OwnershipProof::BinaryWithMarker(marker.to_path_buf()),
    );
    spec.depends_on = vec![ArtifactId::StopProcesses];
    spec
}

fn install_marker(path: PathBuf) -> ArtifactSpec {
    let mut spec = artifact(
        ArtifactId::InstallMarker,
        Operation::RemoveFile,
        path,
        OwnershipProof::ValidInstallId,
    );
    spec.depends_on = vec![ArtifactId::StagedBinary, ArtifactId::Binary];
    spec
}

fn exact_file(id: ArtifactId, path: PathBuf, bytes: &'static [u8]) -> ArtifactSpec {
    artifact(
        id,
        Operation::RemoveFile,
        path,
        OwnershipProof::ExactBytes(bytes),
    )
}

fn text_file(id: ArtifactId, path: PathBuf, markers: &'static [&'static str]) -> ArtifactSpec {
    artifact(
        id,
        Operation::RemoveFile,
        path,
        OwnershipProof::TextMarkers(markers),
    )
}

fn shell_hook(id: ArtifactId, path: PathBuf) -> ArtifactSpec {
    artifact(
        id,
        Operation::EditShellHook,
        path,
        OwnershipProof::ShellHook,
    )
}

fn mime_file(id: ArtifactId, path: PathBuf) -> ArtifactSpec {
    artifact(
        id,
        Operation::EditMimeAssociation,
        path,
        OwnershipProof::MimeAssociation,
    )
}

fn artifact(
    id: ArtifactId,
    operation: Operation,
    path: PathBuf,
    ownership: OwnershipProof,
) -> ArtifactSpec {
    ArtifactSpec {
        id,
        operation,
        path,
        ownership,
        depends_on: Vec::new(),
    }
}

fn refresh_caches(data_home: &Path) {
    let _ = std::process::Command::new("update-desktop-database")
        .arg(data_home.join("applications"))
        .output();
    let _ = std::process::Command::new("gtk-update-icon-cache")
        .arg(data_home.join("icons/hicolor"))
        .output();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_maps_every_artifact_under_the_supplied_roots() {
        let context = context_from_roots(
            Path::new("/home/u"),
            Path::new("/cfg"),
            Path::new("/data"),
            Path::new("/cfg/qol-tray"),
            Path::new("/data/qol-tray"),
            Path::new("/home/u/.local/bin"),
            PathBuf::from("/runtime/qol-tray"),
        );

        let binary = context
            .artifacts
            .iter()
            .find(|artifact| artifact.id == ArtifactId::Binary)
            .unwrap();
        let active_install_id = context
            .artifacts
            .iter()
            .find(|artifact| artifact.id == ArtifactId::ActiveInstallId)
            .unwrap();
        assert_eq!(binary.path, PathBuf::from("/home/u/.local/bin/qol-tray"));
        assert!(matches!(
            active_install_id.ownership,
            OwnershipProof::ValidInstallId
        ));
        assert_eq!(context.refresh_root, PathBuf::from("/data"));
        assert_eq!(context.preserved[0].path, PathBuf::from("/cfg/qol-tray"));
        assert_eq!(context.preserved[1].path, PathBuf::from("/data/qol-tray"));
    }
}
