use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::installer::autostart::AutostartState;

pub trait BootEnvironment: Send + Sync {
    fn canonical_binary(&self) -> Result<PathBuf>;
    fn read_autostart_state(&self) -> Result<AutostartState>;
    fn write_autostart_target(&self, binary: &Path) -> Result<bool>;
    fn honors_dev_selection(&self) -> bool;
    fn production_install(&self) -> Option<PathBuf>;
}

#[cfg(test)]
pub(crate) struct InMemoryBootEnvironment {
    pub canonical: PathBuf,
    pub honors_dev: bool,
    pub autostart: std::sync::Mutex<Option<PathBuf>>,
    pub disabled_by_user: bool,
    pub fail_write: bool,
    pub production: Option<PathBuf>,
}

#[cfg(test)]
impl InMemoryBootEnvironment {
    pub(crate) fn new(canonical: PathBuf, honors_dev: bool) -> Self {
        Self {
            canonical,
            honors_dev,
            autostart: std::sync::Mutex::new(None),
            disabled_by_user: false,
            fail_write: false,
            production: None,
        }
    }

    #[cfg(feature = "dev")]
    pub(crate) fn with_production(mut self, binary: PathBuf) -> Self {
        self.production = Some(binary);
        self
    }

    pub(crate) fn with_autostart(self, target: PathBuf) -> Self {
        *self.autostart.lock().unwrap() = Some(target);
        self
    }

    #[cfg(feature = "dev")]
    pub(crate) fn with_autostart_disabled_by_user(mut self) -> Self {
        self.disabled_by_user = true;
        self
    }
}

#[cfg(test)]
impl BootEnvironment for InMemoryBootEnvironment {
    fn canonical_binary(&self) -> Result<PathBuf> {
        Ok(self.canonical.clone())
    }
    fn read_autostart_state(&self) -> Result<AutostartState> {
        if self.disabled_by_user {
            return Ok(AutostartState::DisabledByUser);
        }
        Ok(AutostartState::from_target(
            self.autostart.lock().unwrap().clone(),
        ))
    }
    fn write_autostart_target(&self, binary: &Path) -> Result<bool> {
        if self.fail_write {
            anyhow::bail!("simulated write failure");
        }
        *self.autostart.lock().unwrap() = Some(binary.to_path_buf());
        Ok(true)
    }
    fn honors_dev_selection(&self) -> bool {
        self.honors_dev
    }
    fn production_install(&self) -> Option<PathBuf> {
        self.production.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_round_trip() {
        let env = InMemoryBootEnvironment::new(PathBuf::from("/canonical"), true);
        assert_eq!(env.read_autostart_state().unwrap().into_target(), None);
        env.write_autostart_target(Path::new("/x")).unwrap();
        assert_eq!(
            env.read_autostart_state().unwrap().into_target(),
            Some(PathBuf::from("/x"))
        );
        assert_eq!(env.canonical_binary().unwrap(), PathBuf::from("/canonical"));
        assert!(env.honors_dev_selection());
    }

    #[test]
    fn in_memory_with_autostart_prefills_target() {
        let env = InMemoryBootEnvironment::new(PathBuf::from("/c"), false)
            .with_autostart(PathBuf::from("/pre"));
        assert_eq!(
            env.read_autostart_state().unwrap().into_target(),
            Some(PathBuf::from("/pre"))
        );
    }

    #[test]
    fn in_memory_write_failure() {
        let env = InMemoryBootEnvironment {
            canonical: PathBuf::from("/c"),
            honors_dev: false,
            autostart: std::sync::Mutex::new(None),
            disabled_by_user: false,
            fail_write: true,
            production: None,
        };
        assert!(env.write_autostart_target(Path::new("/x")).is_err());
    }
}

/// Resolves the binary the autostart artifact should point at when no dev
/// worktree is selected, or when the selected worktree is unusable.
pub fn canonical_boot_binary() -> Result<PathBuf> {
    let main_clone = main_clone_debug_path();
    let main_exists = main_clone.as_ref().is_some_and(|p| p.is_file());
    let current_exe = std::env::current_exe()?;
    let current_role = qol_conventions::artifact::current()
        .map(|identity| identity.role)
        .ok_or_else(|| anyhow::anyhow!("running build identity is unavailable"))?;

    let probes = CanonicalProbes {
        production_install: existing_production_binary(),
        main_clone_debug: main_clone,
        main_clone_debug_exists: main_exists,
        current_exe,
        current_role,
    };
    Ok(canonical_boot_binary_inner(&probes, cfg!(feature = "dev")))
}

struct CanonicalProbes {
    production_install: Option<PathBuf>,
    main_clone_debug: Option<PathBuf>,
    main_clone_debug_exists: bool,
    current_exe: PathBuf,
    current_role: qol_conventions::artifact::BuildRole,
}

fn canonical_boot_binary_inner(probes: &CanonicalProbes, honors_dev: bool) -> PathBuf {
    if let Some(install) = probes.production_install.as_ref() {
        return install.clone();
    }
    if honors_dev {
        if let Some(main) = probes.main_clone_debug.as_ref() {
            if probes.main_clone_debug_exists {
                return main.clone();
            }
        }
    }
    if probes.current_role != qol_conventions::artifact::BuildRole::Host {
        return probes
            .current_exe
            .parent()
            .map(|dir| dir.join(crate::installer::platform::binary_filename()))
            .unwrap_or_else(|| probes.current_exe.clone());
    }
    probes.current_exe.clone()
}

pub(crate) fn paths_equal_canonicalized(a: &Path, b: &Path) -> bool {
    let canon_a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let canon_b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    canon_a == canon_b
}

fn existing_production_binary() -> Option<PathBuf> {
    crate::paths::tray_installed_binary(qol_apps::tray_install::InstallProfile::Production)
        .ok()
        .filter(|path| path.is_file() && super::has_install_marker(path))
}

#[cfg(test)]
mod production_install_tests {
    use super::*;
    use qol_apps::tray_install::InstallProfile;

    fn place_production_binary(with_marker: bool) -> PathBuf {
        let binary = crate::paths::tray_installed_binary(InstallProfile::Production).unwrap();
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, b"binary").unwrap();
        if with_marker {
            let marker = crate::paths::install_marker::marker_path(&binary).unwrap();
            std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
            std::fs::write(&marker, "install-test\n").unwrap();
        }
        binary
    }

    #[test]
    fn a_binary_without_its_install_marker_is_not_a_production_install() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());
        place_production_binary(false);
        assert_eq!(existing_production_binary(), None);
    }

    #[test]
    fn a_binary_with_its_install_marker_is_the_production_install() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());
        let binary = place_production_binary(true);
        assert_eq!(existing_production_binary(), Some(binary));
    }
}

#[cfg(feature = "dev")]
fn main_clone_debug_path() -> Option<PathBuf> {
    Some(
        crate::paths::repo_root_from_manifest_dir()
            .join("target")
            .join("debug")
            .join(crate::installer::platform::binary_filename()),
    )
}

#[cfg(not(feature = "dev"))]
fn main_clone_debug_path() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod canonical_tests {
    use super::*;

    fn pick(probes: &CanonicalProbes, honors_dev: bool) -> PathBuf {
        canonical_boot_binary_inner(probes, honors_dev)
    }

    #[test]
    fn production_install_wins_over_main_clone_in_a_dev_build() {
        let probes = CanonicalProbes {
            production_install: Some(PathBuf::from("/install")),
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/worktree"),
            current_role: qol_conventions::artifact::BuildRole::Host,
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/install"));
    }

    #[test]
    fn production_install_wins_in_a_production_build() {
        let probes = CanonicalProbes {
            production_install: Some(PathBuf::from("/install")),
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/worktree"),
            current_role: qol_conventions::artifact::BuildRole::Host,
        };
        assert_eq!(pick(&probes, false), PathBuf::from("/install"));
    }

    #[test]
    fn main_clone_wins_in_a_dev_build_without_a_production_install() {
        let probes = CanonicalProbes {
            production_install: None,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/worktree"),
            current_role: qol_conventions::artifact::BuildRole::Host,
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/main"));
    }

    #[test]
    fn main_clone_is_ignored_in_a_production_build() {
        let probes = CanonicalProbes {
            production_install: None,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/worktree"),
            current_role: qol_conventions::artifact::BuildRole::Host,
        };
        assert_eq!(pick(&probes, false), PathBuf::from("/worktree"));
    }

    #[test]
    fn current_exe_when_no_install_and_no_built_main_clone() {
        let probes = CanonicalProbes {
            production_install: None,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: false,
            current_exe: PathBuf::from("/worktree"),
            current_role: qol_conventions::artifact::BuildRole::Host,
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/worktree"));
    }

    #[test]
    fn non_host_roles_fall_back_to_sibling_tray() {
        let roles = [
            qol_conventions::artifact::BuildRole::Doctor,
            qol_conventions::artifact::BuildRole::Installer,
            qol_conventions::artifact::BuildRole::Migrator,
        ];
        for current_role in roles {
            let probes = CanonicalProbes {
                production_install: None,
                main_clone_debug: None,
                main_clone_debug_exists: false,
                current_exe: PathBuf::from("/repo/target/debug/renamed-tool.exe"),
                current_role,
            };
            assert_eq!(
                pick(&probes, false),
                PathBuf::from("/repo/target/debug")
                    .join(crate::installer::platform::binary_filename()),
                "role: {current_role:?}"
            );
        }
    }

    #[test]
    fn host_exe_still_resolves_to_itself() {
        let probes = CanonicalProbes {
            production_install: None,
            main_clone_debug: None,
            main_clone_debug_exists: false,
            current_exe: PathBuf::from("/repo/target/debug/qol-tray"),
            current_role: qol_conventions::artifact::BuildRole::Host,
        };
        assert_eq!(
            pick(&probes, false),
            PathBuf::from("/repo/target/debug/qol-tray")
        );
    }
}

pub struct PlatformBootEnvironment;

impl BootEnvironment for PlatformBootEnvironment {
    fn canonical_binary(&self) -> Result<PathBuf> {
        canonical_boot_binary()
    }
    fn read_autostart_state(&self) -> Result<AutostartState> {
        crate::installer::autostart::read_state()
    }
    fn write_autostart_target(&self, binary: &Path) -> Result<bool> {
        crate::installer::autostart::write_target(binary)?;
        Ok(true)
    }
    fn honors_dev_selection(&self) -> bool {
        cfg!(feature = "dev")
    }
    fn production_install(&self) -> Option<PathBuf> {
        existing_production_binary()
    }
}

pub fn default_boot_environment() -> Arc<dyn BootEnvironment> {
    Arc::new(PlatformBootEnvironment)
}

/// `BootEnvironment` for the install/update path: canonical_binary short-circuits
/// to the path the installer just wrote to, so `boot_contract::set_selected_worktree`
/// can be called before the file exists on disk.
pub struct InstallBootEnvironment {
    pub installed_binary: PathBuf,
    pub honors_dev_selection: bool,
    pub owns_autostart: bool,
}

impl BootEnvironment for InstallBootEnvironment {
    fn canonical_binary(&self) -> Result<PathBuf> {
        Ok(self.installed_binary.clone())
    }
    fn read_autostart_state(&self) -> Result<AutostartState> {
        crate::installer::autostart::read_state()
    }
    fn write_autostart_target(&self, binary: &Path) -> Result<bool> {
        if !self.owns_autostart {
            return Ok(false);
        }
        crate::installer::autostart::write_target(binary)?;
        Ok(true)
    }
    fn honors_dev_selection(&self) -> bool {
        self.honors_dev_selection
    }
    fn production_install(&self) -> Option<PathBuf> {
        existing_production_binary()
    }
}

#[cfg(test)]
mod install_env_tests {
    use super::*;

    #[test]
    fn install_env_canonical_short_circuits_to_installed_binary() {
        let env = InstallBootEnvironment {
            installed_binary: PathBuf::from("/usr/local/qol-tray"),
            honors_dev_selection: false,
            owns_autostart: true,
        };
        assert_eq!(
            env.canonical_binary().unwrap(),
            PathBuf::from("/usr/local/qol-tray")
        );
        assert!(!env.honors_dev_selection());
    }

    #[test]
    fn a_development_install_never_writes_the_autostart_artifact() {
        let env = InstallBootEnvironment {
            installed_binary: PathBuf::from("/home/u/.local/share/qol-tray/dev/bin/qol-tray"),
            honors_dev_selection: true,
            owns_autostart: false,
        };
        assert!(!env
            .write_autostart_target(Path::new("/home/u/.local/bin/qol-tray"))
            .unwrap());
    }
}
