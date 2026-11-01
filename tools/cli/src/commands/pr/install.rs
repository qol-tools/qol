use anyhow::{bail, Context, Result};
use qol_conventions::artifact::BuildIntent;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const ROOT: &str = "pr-try";

pub(super) struct Places {
    pub(super) plugins: PathBuf,
    pub(super) pids: PathBuf,
    pub(super) state: PathBuf,
    backups: PathBuf,
    downloads: PathBuf,
}

impl Places {
    pub(super) fn current() -> Result<Self> {
        let config = qol_config::config_dir().context("could not find the qol config directory")?;
        let data = qol_config::data_dir().context("could not find the qol data directory")?;
        let cache = qol_config::cache_dir().context("could not find the qol cache directory")?;
        Ok(Self::under(&config, &data, &cache))
    }

    fn under(config: &Path, data: &Path, cache: &Path) -> Self {
        Self {
            plugins: config.join("plugins"),
            pids: data.join("runtime").join("pids"),
            state: data.join(ROOT).join("state.json"),
            backups: data.join(ROOT).join("backups"),
            downloads: cache.join(ROOT),
        }
    }

    pub(super) fn download_dir(&self, number: u64, head: &str) -> PathBuf {
        self.downloads
            .join(format!("{number}-{}", super::short(head)))
    }

    pub(super) fn backup_for(&self, id: &str, binary: &str) -> PathBuf {
        self.backups.join(id).join(binary)
    }
}

#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct State {
    pub(super) pull_request: Option<u64>,
    pub(super) head_sha: Option<String>,
    pub(super) plugins: Vec<Swapped>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Swapped {
    pub(super) id: String,
    pub(super) target: PathBuf,
    pub(super) backup: PathBuf,
}

impl State {
    pub(super) fn load(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("{} is not a qol pr try record", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub(super) fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        qol_fs::atomic_write(path, serde_json::to_string_pretty(self)?.as_bytes())
            .with_context(|| format!("writing {}", path.display()))
    }

    pub(super) fn clear(path: &Path) -> Result<()> {
        match fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(()),
        }
    }
}

pub(super) fn verify_binary(path: &Path, binary: &str) -> Result<()> {
    let artifact = match qol_artifact::inspect_path(path) {
        Ok(artifact) => artifact,
        Err(qol_artifact::InspectionError::MissingIdentity) => return Ok(()),
        Err(error) => bail!("{} is not an executable: {error}", path.display()),
    };
    let built = artifact.slices.iter().any(|slice| {
        slice.identity.binary == binary && slice.identity.intent == BuildIntent::Production
    });
    if !built {
        bail!("{} is not a production build of {binary}", path.display());
    }
    Ok(())
}

pub(super) struct Swap<'a> {
    pub(super) id: &'a str,
    pub(super) source: &'a Path,
    pub(super) target: &'a Path,
    pub(super) backup: &'a Path,
}

pub(super) fn replace(swap: &Swap<'_>, state: &mut State, state_path: &Path) -> Result<()> {
    if !state.plugins.iter().any(|swapped| swapped.id == swap.id) {
        if !swap.backup.is_file() {
            back_up(swap.target, swap.backup)?;
        }
        state.plugins.push(Swapped {
            id: swap.id.to_string(),
            target: swap.target.to_path_buf(),
            backup: swap.backup.to_path_buf(),
        });
    }
    state.save(state_path)?;
    swap_in(swap.source, swap.target)
}

fn back_up(target: &Path, backup: &Path) -> Result<()> {
    let parent = backup
        .parent()
        .with_context(|| format!("{} has no folder", backup.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let staged = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("staging in {}", parent.display()))?;
    fs::copy(target, staged.path()).with_context(|| format!("backing up {}", target.display()))?;
    staged
        .persist(backup)
        .with_context(|| format!("writing {}", backup.display()))?;
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Restored {
    PutBack,
    BackupGone,
    PluginGone,
}

impl Restored {
    pub(super) fn describe(&self) -> &'static str {
        match self {
            Self::PutBack => "put back",
            Self::BackupGone => "its backup is gone, so the pull request build stays",
            Self::PluginGone => "no longer installed, backup dropped",
        }
    }
}

pub(super) fn restore(swapped: &Swapped) -> Result<Restored> {
    if !swapped.backup.is_file() {
        return Ok(Restored::BackupGone);
    }
    let installed = swapped.target.parent().is_some_and(Path::is_dir);
    if installed {
        swap_in(&swapped.backup, &swapped.target)?;
    }
    fs::remove_file(&swapped.backup)
        .with_context(|| format!("removing {}", swapped.backup.display()))?;
    Ok(if installed {
        Restored::PutBack
    } else {
        Restored::PluginGone
    })
}

fn swap_in(source: &Path, target: &Path) -> Result<()> {
    let parent = target
        .parent()
        .with_context(|| format!("{} has no folder", target.display()))?;
    let staged = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("staging in {}", parent.display()))?;
    fs::copy(source, staged.path())
        .with_context(|| format!("copying {} into {}", source.display(), parent.display()))?;
    let permissions = fs::metadata(target)
        .or_else(|_| fs::metadata(source))
        .with_context(|| format!("reading {}", source.display()))?
        .permissions();
    fs::set_permissions(staged.path(), permissions)?;
    staged
        .persist(target)
        .with_context(|| format!("replacing {}", target.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _root: tempfile::TempDir,
        places: Places,
        target: PathBuf,
        source: PathBuf,
    }

    fn fixture() -> Fixture {
        let root = tempfile::tempdir().unwrap();
        let places = Places::under(
            &root.path().join("config"),
            &root.path().join("data"),
            &root.path().join("cache"),
        );
        let target = places.plugins.join("qol-demo").join("qol-demo");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, b"installed").unwrap();
        let source = root.path().join("downloaded");
        fs::write(&source, b"pull request").unwrap();
        Fixture {
            _root: root,
            places,
            target,
            source,
        }
    }

    fn swap<'a>(fixture: &'a Fixture, backup: &'a Path) -> Swap<'a> {
        Swap {
            id: "qol-demo",
            source: &fixture.source,
            target: &fixture.target,
            backup,
        }
    }

    fn swapped(fixture: &Fixture) -> Swapped {
        Swapped {
            id: "qol-demo".into(),
            target: fixture.target.clone(),
            backup: fixture.places.backup_for("qol-demo", "qol-demo"),
        }
    }

    #[test]
    fn replace_keeps_the_first_install_through_repeated_tries() {
        let fixture = fixture();
        let backup = fixture.places.backup_for("qol-demo", "qol-demo");
        let mut state = State::default();
        replace(&swap(&fixture, &backup), &mut state, &fixture.places.state).unwrap();
        assert_eq!(fs::read(&fixture.target).unwrap(), b"pull request");
        assert_eq!(State::load(&fixture.places.state).unwrap(), state);
        fs::write(&fixture.source, b"newer push").unwrap();
        replace(&swap(&fixture, &backup), &mut state, &fixture.places.state).unwrap();
        assert_eq!(fs::read(&fixture.target).unwrap(), b"newer push");
        assert_eq!(fs::read(&backup).unwrap(), b"installed");
        assert_eq!(state.plugins.len(), 1);

        assert_eq!(restore(&state.plugins[0]).unwrap(), Restored::PutBack);
        assert_eq!(fs::read(&fixture.target).unwrap(), b"installed");
        assert!(!backup.exists(), "a restore removes its backup");
    }

    #[test]
    fn a_backup_left_without_a_record_is_never_overwritten() {
        let fixture = fixture();
        let backup = fixture.places.backup_for("qol-demo", "qol-demo");
        fs::create_dir_all(backup.parent().unwrap()).unwrap();
        fs::write(&backup, b"first install").unwrap();
        fs::write(&fixture.target, b"pull request").unwrap();
        let mut state = State::default();
        replace(&swap(&fixture, &backup), &mut state, &fixture.places.state).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), b"first install");
        assert_eq!(State::load(&fixture.places.state).unwrap().plugins.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn replace_keeps_the_installed_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = fixture();
        fs::set_permissions(&fixture.target, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&fixture.source, fs::Permissions::from_mode(0o644)).unwrap();
        let backup = fixture.places.backup_for("qol-demo", "qol-demo");
        replace(
            &swap(&fixture, &backup),
            &mut State::default(),
            &fixture.places.state,
        )
        .unwrap();
        let mode = fs::metadata(&fixture.target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn restore_settles_a_missing_backup_or_plugin_instead_of_failing() {
        let fixture = fixture();
        let swapped = swapped(&fixture);
        assert_eq!(restore(&swapped).unwrap(), Restored::BackupGone);
        assert_eq!(fs::read(&fixture.target).unwrap(), b"installed");

        fs::create_dir_all(swapped.backup.parent().unwrap()).unwrap();
        fs::write(&swapped.backup, b"first install").unwrap();
        fs::remove_dir_all(fixture.target.parent().unwrap()).unwrap();
        assert_eq!(restore(&swapped).unwrap(), Restored::PluginGone);
        assert!(!swapped.backup.exists());
    }

    #[test]
    fn restore_puts_back_a_binary_that_was_deleted() {
        let fixture = fixture();
        let swapped = swapped(&fixture);
        fs::create_dir_all(swapped.backup.parent().unwrap()).unwrap();
        fs::write(&swapped.backup, b"first install").unwrap();
        fs::remove_file(&fixture.target).unwrap();
        assert_eq!(restore(&swapped).unwrap(), Restored::PutBack);
        assert_eq!(fs::read(&fixture.target).unwrap(), b"first install");
    }

    #[test]
    fn state_round_trips_and_clears() {
        let fixture = fixture();
        assert_eq!(
            State::load(&fixture.places.state).unwrap(),
            State::default()
        );
        let state = State {
            pull_request: Some(412),
            head_sha: Some("abc".into()),
            plugins: vec![swapped(&fixture)],
        };
        state.save(&fixture.places.state).unwrap();
        assert_eq!(State::load(&fixture.places.state).unwrap(), state);
        State::clear(&fixture.places.state).unwrap();
        State::clear(&fixture.places.state).unwrap();
        assert_eq!(
            State::load(&fixture.places.state).unwrap(),
            State::default()
        );
    }

    #[test]
    fn verify_binary_refuses_a_file_that_is_not_a_build() {
        let fixture = fixture();
        assert!(verify_binary(&fixture.source, "qol-demo").is_err());
    }

    #[test]
    fn verify_binary_accepts_an_executable_that_declares_no_identity() {
        let test_binary = std::env::current_exe().unwrap();
        verify_binary(&test_binary, "qol-demo").unwrap();
    }
}
