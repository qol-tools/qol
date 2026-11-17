use super::install::{back_up, swap_in};
use super::manifest::CONTRACT_FILES;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const ABSENT_SUFFIX: &str = ".absent";

pub(super) struct Swap<'a> {
    pub(super) build: &'a Path,
    pub(super) installed: &'a Path,
    pub(super) backups: &'a Path,
    pub(super) carried: &'a [String],
}

pub(super) fn replace(swap: &Swap<'_>) -> Result<()> {
    for name in CONTRACT_FILES {
        let target = swap.installed.join(name);
        remember(&target, &swap.backups.join(name))?;
        if swap.carried.iter().any(|carried| carried == name) {
            swap_in(&swap.build.join(name), &target)?;
        } else {
            remove(&target)?;
        }
    }
    Ok(())
}

pub(super) fn restore(installed: &Path, backups: &Path) -> Result<()> {
    for name in CONTRACT_FILES {
        let target = installed.join(name);
        let backup = backups.join(name);
        let absent = absent_marker(&backup);
        if backup.is_file() {
            if installed.is_dir() {
                swap_in(&backup, &target)?;
            }
            remove(&backup)?;
        } else if absent.is_file() {
            remove(&target)?;
            remove(&absent)?;
        }
    }
    Ok(())
}

fn remember(target: &Path, backup: &Path) -> Result<()> {
    let absent = absent_marker(backup);
    if backup.is_file() || absent.is_file() {
        return Ok(());
    }
    if target.is_file() {
        return back_up(target, backup);
    }
    let parent = backup
        .parent()
        .with_context(|| format!("{} has no folder", backup.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    fs::write(&absent, b"").with_context(|| format!("writing {}", absent.display()))
}

fn absent_marker(backup: &Path) -> PathBuf {
    let mut name = backup.as_os_str().to_os_string();
    name.push(ABSENT_SUFFIX);
    PathBuf::from(name)
}

fn remove(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "qol-config.toml";
    const QUERIES: &str = "qol-runtime.toml";

    struct Fixture {
        _root: tempfile::TempDir,
        build: PathBuf,
        installed: PathBuf,
        backups: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let fixture = Self {
                build: root.path().join("build"),
                installed: root.path().join("installed"),
                backups: root.path().join("backups"),
                _root: root,
            };
            fs::create_dir_all(&fixture.build).unwrap();
            fs::create_dir_all(&fixture.installed).unwrap();
            fixture
        }

        fn replace(&self, carried: &[&str]) {
            let carried = carried
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>();
            replace(&Swap {
                build: &self.build,
                installed: &self.installed,
                backups: &self.backups,
                carried: &carried,
            })
            .unwrap();
        }

        fn restore(&self) {
            restore(&self.installed, &self.backups).unwrap();
        }

        fn installed(&self, name: &str) -> Option<String> {
            fs::read_to_string(self.installed.join(name)).ok()
        }

        fn backups_left(&self) -> usize {
            fs::read_dir(&self.backups).map_or(0, Iterator::count)
        }
    }

    #[test]
    fn off_puts_back_a_file_the_build_replaced_added_or_dropped() {
        let cases: [(Option<&str>, Option<&str>); 4] = [
            (Some("installed"), Some("pull request")),
            (None, Some("pull request")),
            (Some("installed"), None),
            (None, None),
        ];
        for (before, built) in cases {
            let fixture = Fixture::new();
            if let Some(text) = before {
                fs::write(fixture.installed.join(PAGE), text).unwrap();
            }
            if let Some(text) = built {
                fs::write(fixture.build.join(PAGE), text).unwrap();
            }
            let carried: &[&str] = if built.is_some() { &[PAGE] } else { &[] };
            fixture.replace(carried);
            assert_eq!(
                fixture.installed(PAGE).as_deref(),
                built,
                "{before:?} {built:?}"
            );
            fixture.restore();
            assert_eq!(
                fixture.installed(PAGE).as_deref(),
                before,
                "{before:?} {built:?}"
            );
            assert_eq!(fixture.backups_left(), 0, "{before:?} {built:?}");
        }
    }

    #[test]
    fn a_repeated_try_keeps_the_first_install() {
        let fixture = Fixture::new();
        fs::write(fixture.installed.join(PAGE), "installed").unwrap();
        fs::write(fixture.build.join(PAGE), "first push").unwrap();
        fs::write(fixture.build.join(QUERIES), "first push").unwrap();
        fixture.replace(&[PAGE, QUERIES]);
        fs::write(fixture.build.join(PAGE), "second push").unwrap();
        fixture.replace(&[PAGE]);
        assert_eq!(fixture.installed(PAGE).as_deref(), Some("second push"));
        assert_eq!(fixture.installed(QUERIES), None);
        fixture.restore();
        assert_eq!(fixture.installed(PAGE).as_deref(), Some("installed"));
        assert_eq!(fixture.installed(QUERIES), None);
        assert_eq!(fixture.backups_left(), 0);
    }

    #[test]
    fn off_drops_the_backups_of_a_plugin_that_was_uninstalled() {
        let fixture = Fixture::new();
        fs::write(fixture.installed.join(PAGE), "installed").unwrap();
        fs::write(fixture.build.join(PAGE), "pull request").unwrap();
        fixture.replace(&[PAGE]);
        fs::remove_dir_all(&fixture.installed).unwrap();
        fixture.restore();
        assert!(!fixture.installed.exists());
        assert_eq!(fixture.backups_left(), 0);
    }

    #[test]
    fn off_without_a_swap_changes_nothing() {
        let fixture = Fixture::new();
        fs::write(fixture.installed.join(PAGE), "installed").unwrap();
        fixture.restore();
        assert_eq!(fixture.installed(PAGE).as_deref(), Some("installed"));
    }
}
