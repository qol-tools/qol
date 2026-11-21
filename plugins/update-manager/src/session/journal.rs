use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::updates::{Finish, Update};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Journal {
    #[serde(default)]
    pub(crate) boot: Option<String>,
    #[serde(default)]
    pub(crate) finish: Vec<Finish>,
}

impl Journal {
    pub(crate) fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                log::warn!(
                    "starting a new journal, {} could not be read: {error}",
                    path.display()
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        qol_fs::atomic_write(path, &bytes)
    }

    pub(crate) fn start_boot(&mut self, boot: Option<String>) -> bool {
        if boot.is_none() || self.boot == boot {
            return false;
        }
        self.boot = boot;
        self.finish.clear();
        true
    }

    pub(crate) fn installed(&mut self, updates: &[Update]) {
        for update in updates.iter().filter(|update| update.restart) {
            if self.finish.iter().any(|row| row.id == update.id) {
                continue;
            }
            self.finish.push(Finish {
                id: update.id.clone(),
                label: update.label.clone(),
                name: update.name.clone(),
                finish: update.finish.clone(),
                short: "Finishes when the computer restarts".into(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_boot_forgets_what_the_restart_finished() {
        let mut journal = Journal {
            boot: Some("a".into()),
            ..Journal::default()
        };
        journal.finish.push(Finish {
            id: "apt:x".into(),
            label: "X".into(),
            name: "X".into(),
            finish: String::new(),
            short: String::new(),
        });
        assert!(!journal.start_boot(Some("a".into())));
        assert!(!journal.start_boot(None));
        assert_eq!(journal.finish.len(), 1);
        assert!(journal.start_boot(Some("b".into())));
        assert!(journal.finish.is_empty());
    }

    #[test]
    fn the_journal_survives_a_round_trip_and_a_bad_file() {
        let dir =
            std::env::temp_dir().join(format!("qol-update-manager-journal-{}", std::process::id()));
        let path = dir.join("journal.json");
        let journal = Journal {
            boot: Some("b".into()),
            finish: vec![Finish {
                id: "apt:x".into(),
                label: "X".into(),
                name: "X".into(),
                finish: String::new(),
                short: String::new(),
            }],
        };
        journal.save(&path).expect("save");
        assert_eq!(Journal::load(&path), journal);
        fs::write(&path, b"{").expect("write");
        assert_eq!(Journal::load(&path), Journal::default());
        let _ = fs::remove_dir_all(dir);
    }
}
