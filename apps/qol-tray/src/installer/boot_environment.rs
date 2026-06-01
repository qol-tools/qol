use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub trait BootEnvironment: Send + Sync {
    fn canonical_binary(&self) -> Result<PathBuf>;
    fn read_autostart_target(&self) -> Result<Option<PathBuf>>;
    fn write_autostart_target(&self, binary: &Path) -> Result<()>;
    fn honors_dev_selection(&self) -> bool;
}

#[cfg(test)]
pub(crate) struct InMemoryBootEnvironment {
    pub canonical: PathBuf,
    pub honors_dev: bool,
    pub autostart: std::sync::Mutex<Option<PathBuf>>,
    pub fail_write: bool,
}

#[cfg(test)]
impl InMemoryBootEnvironment {
    pub(crate) fn new(canonical: PathBuf, honors_dev: bool) -> Self {
        Self {
            canonical,
            honors_dev,
            autostart: std::sync::Mutex::new(None),
            fail_write: false,
        }
    }

    pub(crate) fn with_autostart(self, target: PathBuf) -> Self {
        *self.autostart.lock().unwrap() = Some(target);
        self
    }
}

#[cfg(test)]
impl BootEnvironment for InMemoryBootEnvironment {
    fn canonical_binary(&self) -> Result<PathBuf> {
        Ok(self.canonical.clone())
    }
    fn read_autostart_target(&self) -> Result<Option<PathBuf>> {
        Ok(self.autostart.lock().unwrap().clone())
    }
    fn write_autostart_target(&self, binary: &Path) -> Result<()> {
        if self.fail_write {
            anyhow::bail!("simulated write failure");
        }
        *self.autostart.lock().unwrap() = Some(binary.to_path_buf());
        Ok(())
    }
    fn honors_dev_selection(&self) -> bool {
        self.honors_dev
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_round_trip() {
        let env = InMemoryBootEnvironment::new(PathBuf::from("/canonical"), true);
        assert_eq!(env.read_autostart_target().unwrap(), None);
        env.write_autostart_target(Path::new("/x")).unwrap();
        assert_eq!(
            env.read_autostart_target().unwrap(),
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
            env.read_autostart_target().unwrap(),
            Some(PathBuf::from("/pre"))
        );
    }

    #[test]
    fn in_memory_write_failure() {
        let env = InMemoryBootEnvironment {
            canonical: PathBuf::from("/c"),
            honors_dev: false,
            autostart: std::sync::Mutex::new(None),
            fail_write: true,
        };
        assert!(env.write_autostart_target(Path::new("/x")).is_err());
    }
}

/// Resolves the binary the autostart artifact should point at when no dev
/// worktree is selected, or when the selected worktree is unusable.
///
/// Priority (from the spec's "canonical_binary priority chain"):
///   1. (handled by InstallBootEnvironment short-circuit, not by this fn)
///   2. current_exe if it equals the installed binary
///   3. main-clone-debug when honors_dev_selection is true
///   4. installed binary, if it exists
///   5. current_exe
pub fn canonical_boot_binary() -> Result<PathBuf> {
    let install_binary = installed_binary_path().ok();
    let install_exists = install_binary.as_ref().is_some_and(|p| p.is_file());
    let main_clone = main_clone_debug_path();
    let main_exists = main_clone.as_ref().is_some_and(|p| p.is_file());
    let current_exe = std::env::current_exe()?;

    let probes = CanonicalProbes {
        install_binary,
        install_binary_exists: install_exists,
        main_clone_debug: main_clone,
        main_clone_debug_exists: main_exists,
        current_exe,
    };
    Ok(canonical_boot_binary_inner(&probes, cfg!(feature = "dev")))
}

#[derive(Default)]
struct CanonicalProbes {
    install_binary: Option<PathBuf>,
    install_binary_exists: bool,
    main_clone_debug: Option<PathBuf>,
    main_clone_debug_exists: bool,
    current_exe: PathBuf,
}

fn canonical_boot_binary_inner(probes: &CanonicalProbes, honors_dev: bool) -> PathBuf {
    // Step 2: current_exe == installed binary
    if let Some(install) = probes.install_binary.as_ref() {
        if probes.install_binary_exists && paths_equal_canonicalized(&probes.current_exe, install) {
            return install.clone();
        }
    }
    // Step 3: main-clone-debug when honors_dev_selection
    if honors_dev {
        if let Some(main) = probes.main_clone_debug.as_ref() {
            if probes.main_clone_debug_exists {
                return main.clone();
            }
        }
    }
    // Step 4: installed binary
    if let Some(install) = probes.install_binary.as_ref() {
        if probes.install_binary_exists {
            return install.clone();
        }
    }
    // Step 5: current_exe
    probes.current_exe.clone()
}

fn paths_equal_canonicalized(a: &Path, b: &Path) -> bool {
    let canon_a = a.canonicalize().unwrap_or_else(|_| a.to_path_buf());
    let canon_b = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    canon_a == canon_b
}

fn installed_binary_path() -> Result<PathBuf> {
    let dir = crate::installer::platform::install_dir()?;
    Ok(dir.join(crate::installer::platform::binary_filename()))
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
    fn step_2_current_exe_equals_install() {
        let probes = CanonicalProbes {
            install_binary: Some(PathBuf::from("/install")),
            install_binary_exists: true,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/install"),
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/install"));
    }

    #[test]
    fn step_3_dev_prefers_main_clone_over_install() {
        let probes = CanonicalProbes {
            install_binary: Some(PathBuf::from("/install")),
            install_binary_exists: true,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/worktree"),
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/main"));
    }

    #[test]
    fn step_3_skipped_in_prod() {
        let probes = CanonicalProbes {
            install_binary: Some(PathBuf::from("/install")),
            install_binary_exists: true,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: true,
            current_exe: PathBuf::from("/worktree"),
        };
        assert_eq!(pick(&probes, false), PathBuf::from("/install"));
    }

    #[test]
    fn step_4_installed_when_no_main_clone() {
        let probes = CanonicalProbes {
            install_binary: Some(PathBuf::from("/install")),
            install_binary_exists: true,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: false,
            current_exe: PathBuf::from("/worktree"),
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/install"));
    }

    #[test]
    fn step_5_current_exe_when_nothing_else() {
        let probes = CanonicalProbes {
            install_binary: Some(PathBuf::from("/install")),
            install_binary_exists: false,
            main_clone_debug: Some(PathBuf::from("/main")),
            main_clone_debug_exists: false,
            current_exe: PathBuf::from("/worktree"),
        };
        assert_eq!(pick(&probes, true), PathBuf::from("/worktree"));
    }
}

pub struct PlatformBootEnvironment;

impl BootEnvironment for PlatformBootEnvironment {
    fn canonical_binary(&self) -> Result<PathBuf> {
        canonical_boot_binary()
    }
    fn read_autostart_target(&self) -> Result<Option<PathBuf>> {
        crate::installer::autostart::read_target()
    }
    fn write_autostart_target(&self, binary: &Path) -> Result<()> {
        crate::installer::autostart::write_target(binary)
    }
    fn honors_dev_selection(&self) -> bool {
        cfg!(feature = "dev")
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
}

impl BootEnvironment for InstallBootEnvironment {
    fn canonical_binary(&self) -> Result<PathBuf> {
        Ok(self.installed_binary.clone())
    }
    fn read_autostart_target(&self) -> Result<Option<PathBuf>> {
        crate::installer::autostart::read_target()
    }
    fn write_autostart_target(&self, binary: &Path) -> Result<()> {
        crate::installer::autostart::write_target(binary)
    }
    fn honors_dev_selection(&self) -> bool {
        self.honors_dev_selection
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
        };
        assert_eq!(
            env.canonical_binary().unwrap(),
            PathBuf::from("/usr/local/qol-tray")
        );
        assert!(!env.honors_dev_selection());
    }
}
