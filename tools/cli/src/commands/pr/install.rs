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
    let artifact = qol_artifact::inspect_path(path)
        .map_err(|error| anyhow::anyhow!("{} is not a qol build: {error:?}", path.display()))?;
    let built = artifact.slices.iter().any(|slice| {
        slice.identity.binary == binary && slice.identity.intent == BuildIntent::Production
    });
    if !built {
        bail!("{} is not a production build of {binary}", path.display());
    }
    Ok(())
}

pub(super) fn replace(
    source: &Path,
    target: &Path,
    backup: &Path,
    id: &str,
    state: &mut State,
) -> Result<()> {
    if !state.plugins.iter().any(|swapped| swapped.id == id) {
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        fs::copy(target, backup)
            .with_context(|| format!("backing up {} to {}", target.display(), backup.display()))?;
        state.plugins.push(Swapped {
            id: id.to_string(),
            target: target.to_path_buf(),
            backup: backup.to_path_buf(),
        });
    }
    swap_in(source, target)
}

pub(super) fn restore(swapped: &Swapped) -> Result<()> {
    if !swapped.backup.is_file() {
        bail!(
            "the backup of {} is gone from {}",
            swapped.id,
            swapped.backup.display()
        );
    }
    swap_in(&swapped.backup, &swapped.target)?;
    fs::remove_file(&swapped.backup)
        .with_context(|| format!("removing {}", swapped.backup.display()))
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
        .with_context(|| format!("reading {}", target.display()))?
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

    #[test]
    fn replace_keeps_the_first_install_through_repeated_tries() {
        let fixture = fixture();
        let backup = fixture.places.backup_for("qol-demo", "qol-demo");
        let mut state = State::default();
        replace(
            &fixture.source,
            &fixture.target,
            &backup,
            "qol-demo",
            &mut state,
        )
        .unwrap();
        assert_eq!(fs::read(&fixture.target).unwrap(), b"pull request");
        fs::write(&fixture.source, b"newer push").unwrap();
        replace(
            &fixture.source,
            &fixture.target,
            &backup,
            "qol-demo",
            &mut state,
        )
        .unwrap();
        assert_eq!(fs::read(&fixture.target).unwrap(), b"newer push");
        assert_eq!(fs::read(&backup).unwrap(), b"installed");
        assert_eq!(state.plugins.len(), 1);

        restore(&state.plugins[0]).unwrap();
        assert_eq!(fs::read(&fixture.target).unwrap(), b"installed");
        assert!(!backup.exists(), "a restore removes its backup");
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
            &fixture.source,
            &fixture.target,
            &backup,
            "qol-demo",
            &mut State::default(),
        )
        .unwrap();
        let mode = fs::metadata(&fixture.target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn restore_refuses_a_missing_backup_and_leaves_the_build() {
        let fixture = fixture();
        let swapped = Swapped {
            id: "qol-demo".into(),
            target: fixture.target.clone(),
            backup: fixture.places.backup_for("qol-demo", "qol-demo"),
        };
        assert!(restore(&swapped).is_err());
        assert_eq!(fs::read(&fixture.target).unwrap(), b"installed");
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
            plugins: vec![Swapped {
                id: "qol-demo".into(),
                target: fixture.target.clone(),
                backup: fixture.places.backup_for("qol-demo", "qol-demo"),
            }],
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
}
