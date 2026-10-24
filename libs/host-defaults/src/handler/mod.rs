pub mod backends;

use anyhow::{bail, Result};

use crate::claims::{ClaimRecord, ClaimState, Ledger, ReleaseOutcome};
use crate::report::Lifetime;
use crate::role::Role;
use backends::{desktop_entry, mimeapps};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerStore {
    Mimeapps,
    DesktopEntry,
}

impl HandlerStore {
    pub const ALL: [HandlerStore; 2] = [HandlerStore::Mimeapps, HandlerStore::DesktopEntry];

    pub fn name(self) -> &'static str {
        match self {
            HandlerStore::Mimeapps => "mimeapps",
            HandlerStore::DesktopEntry => "desktop_entry",
        }
    }

    pub fn from_name(name: &str) -> Option<HandlerStore> {
        HandlerStore::ALL
            .into_iter()
            .find(|store| store.name() == name)
    }

    pub fn read(self, key: &str) -> Result<Option<String>> {
        match self {
            HandlerStore::Mimeapps => mimeapps::read_key(key),
            HandlerStore::DesktopEntry => desktop_entry::read_key(key),
        }
    }

    pub fn write(self, key: &str, value: Option<&str>) -> Result<()> {
        match self {
            HandlerStore::Mimeapps => mimeapps::write_key(key, value),
            HandlerStore::DesktopEntry => desktop_entry::write_key(key, value),
        }
    }
}

pub struct Claim<'a> {
    pub role: Role,
    pub plugin_id: &'a str,
    pub lifetime: Lifetime,
    pub store: HandlerStore,
    pub key: &'a str,
    pub applied: &'a str,
}

pub fn claim_value(ledger: &Ledger, claim: &Claim) -> Result<()> {
    let current = claim.store.read(claim.key)?;
    let previous = original_previous(ledger, claim, &current)?;
    let record = |state| ClaimRecord {
        role: claim.role.id().to_string(),
        plugin_id: claim.plugin_id.to_string(),
        store: claim.store.name().to_string(),
        key: claim.key.to_string(),
        previous: previous.clone(),
        applied: claim.applied.to_string(),
        lifetime: claim.lifetime,
        state,
    };
    ledger.insert(record(ClaimState::Pending))?;
    claim.store.write(claim.key, Some(claim.applied))?;
    let after = claim.store.read(claim.key)?;
    if after.as_deref() != Some(claim.applied) {
        ledger.drop_record(&record(ClaimState::Pending))?;
        bail!(
            "{} kept {:?} at {} instead of {:?}",
            claim.store.name(),
            after,
            claim.key,
            claim.applied
        );
    }
    ledger.insert(record(ClaimState::Applied))
}

fn original_previous(
    ledger: &Ledger,
    claim: &Claim,
    current: &Option<String>,
) -> Result<Option<String>> {
    let held = ledger.load()?.into_iter().find(|record| {
        record.role == claim.role.id()
            && record.store == claim.store.name()
            && record.key == claim.key
    });
    Ok(match held {
        Some(record) if current.as_deref() == Some(record.applied.as_str()) => record.previous,
        Some(_) | None => current.clone(),
    })
}

pub fn release_role(role: Role, ledger: &Ledger) -> Result<ReleaseOutcome> {
    let records = ledger
        .load()?
        .into_iter()
        .filter(|record| record.role == role.id())
        .filter_map(|record| Some((HandlerStore::from_name(&record.store)?, record)))
        .collect::<Vec<_>>();
    let mut outcomes = Vec::new();
    for (store, record) in &records {
        outcomes.push(undo(*store, record)?);
        ledger.drop_record(record)?;
    }
    Ok(combine(&outcomes))
}

fn undo(store: HandlerStore, record: &ClaimRecord) -> Result<ReleaseOutcome> {
    let current = store.read(&record.key)?;
    if current.as_deref() == Some(record.applied.as_str()) {
        store.write(&record.key, record.previous.as_deref())?;
        return Ok(ReleaseOutcome::Restored);
    }
    if current == record.previous {
        return Ok(ReleaseOutcome::AlreadyGone);
    }
    Ok(ReleaseOutcome::LeftUserChange)
}

fn combine(outcomes: &[ReleaseOutcome]) -> ReleaseOutcome {
    if outcomes.contains(&ReleaseOutcome::LeftUserChange) {
        return ReleaseOutcome::LeftUserChange;
    }
    if outcomes.contains(&ReleaseOutcome::Restored) {
        return ReleaseOutcome::Restored;
    }
    ReleaseOutcome::AlreadyGone
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const MIME: &str = "inode/directory";

    struct Rig {
        _dir: tempfile::TempDir,
        list: PathBuf,
        key: String,
        ledger: Ledger,
    }

    fn rig(initial: Option<&str>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("mimeapps.list");
        if let Some(text) = initial {
            std::fs::write(&list, text).unwrap();
        }
        Rig {
            key: mimeapps::key_for(&list, MIME),
            ledger: Ledger::at(dir.path().join("claims.json")),
            list,
            _dir: dir,
        }
    }

    fn claim<'a>(rig: &'a Rig, plugin_id: &'a str, applied: &'a str) -> Claim<'a> {
        Claim {
            role: Role::FolderOpen,
            plugin_id,
            lifetime: Lifetime::PortableSession,
            store: HandlerStore::Mimeapps,
            key: &rig.key,
            applied,
        }
    }

    fn list_text(rig: &Rig) -> String {
        std::fs::read_to_string(&rig.list).unwrap_or_default()
    }

    #[test]
    fn release_puts_back_the_previous_value() {
        let original =
            "[Default Applications]\ninode/directory=thunar.desktop\ntext/html=a.desktop\n";
        let rig = rig(Some(original));
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        assert!(list_text(&rig).contains("inode/directory=qol-open.desktop"));
        let record = rig.ledger.load().unwrap().remove(0);
        assert_eq!(record.previous.as_deref(), Some("thunar.desktop"));
        assert_eq!(record.state, ClaimState::Applied);
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::Restored
        );
        assert_eq!(list_text(&rig), original);
        assert!(rig.ledger.load().unwrap().is_empty());
    }

    #[test]
    fn release_deletes_the_key_when_nothing_was_set_before() {
        let original = "[Added Associations]\nx=y\n";
        let rig = rig(Some(original));
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        assert_eq!(rig.ledger.load().unwrap()[0].previous, None);
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::Restored
        );
        assert_eq!(list_text(&rig), original);
    }

    #[test]
    fn release_leaves_a_value_the_user_changed_and_drops_the_claim() {
        let rig = rig(Some(
            "[Default Applications]\ninode/directory=thunar.desktop\n",
        ));
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        mimeapps::write_key(&rig.key, Some("dolphin.desktop")).unwrap();
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::LeftUserChange
        );
        assert_eq!(
            mimeapps::read_key(&rig.key).unwrap().as_deref(),
            Some("dolphin.desktop")
        );
        assert!(rig.ledger.load().unwrap().is_empty());
    }

    #[test]
    fn release_with_no_records_is_already_gone() {
        let rig = rig(None);
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::AlreadyGone
        );
    }

    #[test]
    fn a_claim_that_never_reached_the_store_is_already_gone() {
        let original = "[Default Applications]\ninode/directory=thunar.desktop\n";
        let rig = rig(Some(original));
        rig.ledger
            .insert(ClaimRecord {
                role: Role::FolderOpen.id().to_string(),
                plugin_id: "explorer".to_string(),
                store: HandlerStore::Mimeapps.name().to_string(),
                key: rig.key.clone(),
                previous: Some("thunar.desktop".to_string()),
                applied: "qol-open.desktop".to_string(),
                lifetime: Lifetime::PortableSession,
                state: ClaimState::Pending,
            })
            .unwrap();
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::AlreadyGone
        );
        assert_eq!(list_text(&rig), original);
        assert!(rig.ledger.load().unwrap().is_empty());
    }

    #[test]
    fn a_second_claim_keeps_the_first_previous_value() {
        let original = "[Default Applications]\ninode/directory=thunar.desktop\n";
        let rig = rig(Some(original));
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        claim_value(&rig.ledger, &claim(&rig, "other", "qol-open.desktop")).unwrap();
        let records = rig.ledger.load().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].plugin_id, "other");
        assert_eq!(records[0].previous.as_deref(), Some("thunar.desktop"));
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::Restored
        );
        assert_eq!(list_text(&rig), original);
    }

    #[test]
    fn a_claim_after_the_user_changed_the_value_records_the_new_previous() {
        let rig = rig(Some(
            "[Default Applications]\ninode/directory=thunar.desktop\n",
        ));
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        mimeapps::write_key(&rig.key, Some("dolphin.desktop")).unwrap();
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        assert_eq!(
            rig.ledger.load().unwrap()[0].previous.as_deref(),
            Some("dolphin.desktop")
        );
    }

    #[test]
    fn release_keeps_records_of_other_roles_and_other_stores() {
        let rig = rig(None);
        claim_value(&rig.ledger, &claim(&rig, "explorer", "qol-open.desktop")).unwrap();
        for (role, store, key) in [
            (Role::FolderOpen, "hotkey", "Super"),
            (Role::SlotLauncher, "mimeapps", "elsewhere"),
        ] {
            rig.ledger
                .insert(ClaimRecord {
                    role: role.id().to_string(),
                    plugin_id: "launcher".to_string(),
                    store: store.to_string(),
                    key: key.to_string(),
                    previous: None,
                    applied: "x".to_string(),
                    lifetime: Lifetime::PortableSession,
                    state: ClaimState::Applied,
                })
                .unwrap();
        }
        release_role(Role::FolderOpen, &rig.ledger).unwrap();
        let left = rig.ledger.load().unwrap();
        assert_eq!(left.len(), 2);
        assert!(left.iter().any(|record| record.store == "hotkey"));
        assert!(left.iter().any(|record| record.role == "slot.launcher"));
    }

    #[test]
    fn the_desktop_entry_store_creates_then_removes_the_file() {
        let rig = rig(None);
        let path = rig.list.with_file_name("qol-open.desktop");
        let key = path.to_string_lossy().into_owned();
        let claim = Claim {
            role: Role::FolderOpen,
            plugin_id: "explorer",
            lifetime: Lifetime::ResidentPolicy,
            store: HandlerStore::DesktopEntry,
            key: &key,
            applied: "[Desktop Entry]\nName=qol\n",
        };
        claim_value(&rig.ledger, &claim).unwrap();
        assert!(path.is_file());
        assert_eq!(
            release_role(Role::FolderOpen, &rig.ledger).unwrap(),
            ReleaseOutcome::Restored
        );
        assert!(!path.exists());
    }

    #[test]
    fn store_names_round_trip() {
        for store in HandlerStore::ALL {
            assert_eq!(HandlerStore::from_name(store.name()), Some(store));
        }
        assert_eq!(HandlerStore::from_name("hotkey"), None);
    }
}
