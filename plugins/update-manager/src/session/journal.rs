use std::fs;
use std::io;
use std::path::Path;

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

use crate::updates::{AutoKey, Finish, Hours, Update};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Journal {
    #[serde(default)]
    pub(crate) boot: Option<String>,
    #[serde(default)]
    pub(crate) finish: Vec<Finish>,
    #[serde(default)]
    pub(crate) hours: Option<HoursChange>,
    #[serde(default)]
    pub(crate) auto: Vec<AutoChange>,
    #[serde(default)]
    pub(crate) restart: Option<NaiveDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct HoursChange {
    pub(crate) was: Option<Hours>,
    pub(crate) at: NaiveDateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct AutoChange {
    pub(crate) key: AutoKey,
    pub(crate) was: bool,
    pub(crate) at: NaiveDateTime,
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
        self.restart = None;
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

    pub(crate) fn hours_written(&mut self, was: Option<Hours>, ours: bool, at: NaiveDateTime) {
        match (ours, self.hours.is_some()) {
            (true, false) => self.hours = Some(HoursChange { was, at }),
            (true, true) => {
                if let Some(change) = self.hours.as_mut() {
                    change.at = at;
                }
            }
            (false, _) => self.hours = None,
        }
    }

    pub(crate) fn auto_written(&mut self, key: AutoKey, was: bool, on: bool, at: NaiveDateTime) {
        match self.auto.iter().position(|change| change.key == key) {
            Some(index) if self.auto[index].was == on => {
                self.auto.remove(index);
            }
            Some(index) => self.auto[index].at = at,
            None if was != on => self.auto.push(AutoChange { key, was, at }),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::Clock;

    fn at(text: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M").expect("time")
    }

    #[test]
    fn a_new_boot_forgets_what_the_restart_finished() {
        let mut journal = Journal {
            boot: Some("a".into()),
            restart: Some(at("2026-10-14 09:00")),
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
        assert_eq!(journal.restart, None);
    }

    #[test]
    fn changes_remember_the_first_value_until_set_back() {
        let mut journal = Journal::default();
        let first = Hours {
            from: Clock::hm(2, 0),
            until: Clock::hm(3, 0),
            startup: true,
        };
        journal.hours_written(None, true, at("2026-10-14 09:00"));
        journal.hours_written(Some(first), true, at("2026-10-14 10:00"));
        assert_eq!(
            journal.hours,
            Some(HoursChange {
                was: None,
                at: at("2026-10-14 10:00")
            })
        );
        journal.hours_written(Some(first), false, at("2026-10-14 11:00"));
        assert_eq!(journal.hours, None);

        journal.auto_written(AutoKey::Flatpak, false, true, at("2026-10-14 09:00"));
        assert_eq!(journal.auto.len(), 1);
        journal.auto_written(AutoKey::Flatpak, true, false, at("2026-10-14 09:30"));
        assert!(journal.auto.is_empty());
        journal.auto_written(AutoKey::Spices, true, true, at("2026-10-14 09:00"));
        assert!(journal.auto.is_empty());
    }

    #[test]
    fn the_journal_survives_a_round_trip_and_a_bad_file() {
        let dir =
            std::env::temp_dir().join(format!("qol-update-manager-journal-{}", std::process::id()));
        let path = dir.join("journal.json");
        let journal = Journal {
            boot: Some("b".into()),
            auto: vec![AutoChange {
                key: AutoKey::Spices,
                was: true,
                at: at("2026-10-14 09:00"),
            }],
            ..Journal::default()
        };
        journal.save(&path).expect("save");
        assert_eq!(Journal::load(&path), journal);
        fs::write(&path, b"{").expect("write");
        assert_eq!(Journal::load(&path), Journal::default());
        let _ = fs::remove_dir_all(dir);
    }
}
