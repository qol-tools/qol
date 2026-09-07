use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const MARKER_PREFIX: &str = "takeover-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub component: String,
    pub restore_hint: String,
}

pub fn claims_dir(plugin_id: &str) -> Option<PathBuf> {
    qol_config::data_subdir("host-takeover").map(|path| path.join(plugin_id))
}

fn marker_path(dir: &Path, component: &str) -> PathBuf {
    dir.join(format!("{MARKER_PREFIX}{component}"))
}

pub fn is_claimed(dir: &Path, component: &str) -> bool {
    marker_path(dir, component).is_file()
}

pub fn record(dir: &Path, claim: &Claim) -> Result<()> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create takeover dir {}", dir.display()))?;
    let path = marker_path(dir, &claim.component);
    std::fs::write(&path, &claim.restore_hint)
        .with_context(|| format!("failed to record takeover marker {}", path.display()))
}

pub fn clear(dir: &Path, component: &str) -> Result<()> {
    let path = marker_path(dir, component);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to clear takeover marker {}", path.display())),
    }
}

pub fn outstanding(dir: &Path) -> Vec<Claim> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut claims = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let component = name.strip_prefix(MARKER_PREFIX)?;
            Some(Claim {
                component: component.to_string(),
                restore_hint: std::fs::read_to_string(&path).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    claims.sort_by(|left, right| left.component.cmp(&right.component));
    claims
}

pub fn claim(dir: &Path, claim: &Claim, stop: impl FnOnce() -> Result<()>) -> Result<()> {
    record(dir, claim)?;
    match stop() {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = clear(dir, &claim.component);
            Err(error)
        }
    }
}

pub fn restore(dir: &Path, component: &str, start: impl FnOnce() -> Result<()>) -> Result<()> {
    if !is_claimed(dir, component) {
        return Ok(());
    }
    start()?;
    clear(dir, component)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_records_a_marker_and_restore_clears_it() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("plugin-x");
        let entry = Claim {
            component: "blueman-applet".into(),
            restore_hint: "blueman-applet".into(),
        };

        claim(&dir, &entry, || Ok(())).expect("claim");
        assert!(is_claimed(&dir, "blueman-applet"));
        assert_eq!(outstanding(&dir), vec![entry.clone()]);

        restore(&dir, "blueman-applet", || Ok(())).expect("restore");
        assert!(!is_claimed(&dir, "blueman-applet"));
        assert!(outstanding(&dir).is_empty());
    }

    #[test]
    fn a_failed_stop_leaves_no_marker_behind() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("plugin-x");
        let entry = Claim {
            component: "blueman-applet".into(),
            restore_hint: "blueman-applet".into(),
        };

        let result = claim(&dir, &entry, || anyhow::bail!("stop refused"));
        assert!(result.is_err());
        assert!(
            !is_claimed(&dir, "blueman-applet"),
            "a component we failed to stop must not look claimed"
        );
    }

    #[test]
    fn a_marker_surviving_a_kill_is_still_restorable() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("plugin-x");
        let entry = Claim {
            component: "blueman-applet".into(),
            restore_hint: "blueman-applet".into(),
        };
        record(&dir, &entry).expect("record");

        let pending = outstanding(&dir);
        assert_eq!(pending, vec![entry]);

        let mut restarted = Vec::new();
        for claim in &pending {
            restore(&dir, &claim.component, || {
                restarted.push(claim.restore_hint.clone());
                Ok(())
            })
            .expect("restore");
        }
        assert_eq!(restarted, vec!["blueman-applet".to_string()]);
        assert!(outstanding(&dir).is_empty());
    }

    #[test]
    fn restoring_an_unclaimed_component_never_starts_it() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("plugin-x");
        let mut started = false;
        restore(&dir, "blueman-applet", || {
            started = true;
            Ok(())
        })
        .expect("restore");
        assert!(!started, "qol must not start something it never stopped");
    }
}
