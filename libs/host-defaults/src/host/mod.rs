use std::path::PathBuf;

use anyhow::Result;

use crate::claims::{Ledger, ReleaseOutcome};
use crate::handler::HandlerStore;
use crate::platform::Platform;
use crate::report::{Blocker, Lifetime, Report, SlotRoute};
use crate::role::Role;

pub struct OpenerSpec {
    pub exec: PathBuf,
}

pub trait HostDefaults {
    fn probe(&self, role: Role) -> Report;
    fn slot_route(&self, role: Role) -> Option<SlotRoute>;
    fn blockers(&self) -> Vec<Blocker>;
    fn claim_handler(
        &self,
        role: Role,
        opener: &OpenerSpec,
        plugin_id: &str,
        lifetime: Lifetime,
        ledger: &Ledger,
    ) -> Result<()>;
    fn release(&self, role: Role, ledger: &Ledger) -> Result<ReleaseOutcome>;
}

pub fn host() -> impl HostDefaults {
    Platform
}

pub fn release_portable_handlers(
    host: &impl HostDefaults,
    ledger: &Ledger,
) -> Vec<(String, Result<ReleaseOutcome>)> {
    let records = match ledger.load() {
        Ok(records) => records,
        Err(error) => return vec![("ledger".to_string(), Err(error))],
    };
    Role::ALL
        .into_iter()
        .filter(|role| {
            records.iter().any(|record| {
                record.role == role.id()
                    && record.lifetime == Lifetime::PortableSession
                    && HandlerStore::from_name(&record.store).is_some()
            })
        })
        .map(|role| (role.id().to_string(), host.release(role, ledger)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{ClaimRecord, ClaimState};
    use crate::handler;

    struct ReplayHost;

    impl HostDefaults for ReplayHost {
        fn probe(&self, role: Role) -> Report {
            Report::impossible(role, "test host")
        }

        fn slot_route(&self, _role: Role) -> Option<SlotRoute> {
            None
        }

        fn blockers(&self) -> Vec<Blocker> {
            Vec::new()
        }

        fn claim_handler(
            &self,
            _role: Role,
            _opener: &OpenerSpec,
            _plugin_id: &str,
            _lifetime: Lifetime,
            _ledger: &Ledger,
        ) -> Result<()> {
            anyhow::bail!("test host")
        }

        fn release(&self, role: Role, ledger: &Ledger) -> Result<ReleaseOutcome> {
            handler::release_role(role, ledger)
        }
    }

    fn record(
        role: Role,
        store: &str,
        key: &str,
        applied: &str,
        lifetime: Lifetime,
    ) -> ClaimRecord {
        ClaimRecord {
            role: role.id().to_string(),
            plugin_id: "explorer".to_string(),
            store: store.to_string(),
            key: key.to_string(),
            previous: Some("old".to_string()),
            applied: applied.to_string(),
            lifetime,
            state: ClaimState::Applied,
        }
    }

    #[test]
    fn crash_replay_releases_only_roles_with_portable_handler_records() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::at(dir.path().join("claims.json"));
        let entry = dir.path().join("qol-open.desktop");
        std::fs::write(&entry, "[Desktop Entry]\nName=qol\n").unwrap();
        let entry_key = entry.to_string_lossy().into_owned();
        ledger
            .insert(record(
                Role::FolderOpen,
                "desktop_entry",
                &entry_key,
                "[Desktop Entry]\nName=qol\n",
                Lifetime::PortableSession,
            ))
            .unwrap();
        ledger
            .insert(record(
                Role::SlotLauncher,
                "hotkey",
                "Super",
                "x",
                Lifetime::PortableSession,
            ))
            .unwrap();
        ledger
            .insert(record(
                Role::FolderReveal,
                "desktop_entry",
                "/never/touched",
                "x",
                Lifetime::ResidentPolicy,
            ))
            .unwrap();
        let results = release_portable_handlers(&ReplayHost, &ledger);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "folder.open");
        assert_eq!(*results[0].1.as_ref().unwrap(), ReleaseOutcome::Restored);
        assert_eq!(std::fs::read_to_string(&entry).unwrap(), "old");
        let left = ledger.load().unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().all(|record| record.role != "folder.open"));
    }

    #[test]
    fn crash_replay_with_an_empty_ledger_does_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::at(dir.path().join("claims.json"));
        assert!(release_portable_handlers(&ReplayHost, &ledger).is_empty());
    }

    #[test]
    fn crash_replay_reports_an_unreadable_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("claims.json");
        std::fs::write(&path, "not json").unwrap();
        let results = release_portable_handlers(&ReplayHost, &Ledger::at(path));
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "ledger");
        assert!(results[0].1.is_err());
    }

    #[test]
    fn crash_replay_restores_an_applied_portable_claim() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::at(dir.path().join("claims.json"));
        let entry = dir.path().join("qol-open.desktop");
        let key = entry.to_string_lossy().into_owned();
        std::fs::write(&entry, "applied").unwrap();
        let mut held = record(
            Role::FolderOpen,
            "desktop_entry",
            &key,
            "applied",
            Lifetime::PortableSession,
        );
        held.previous = None;
        ledger.insert(held).unwrap();
        let results = release_portable_handlers(&ReplayHost, &ledger);
        assert_eq!(*results[0].1.as_ref().unwrap(), ReleaseOutcome::Restored);
        assert!(!entry.exists());
    }
}
