use super::catalog::Declaration;
use super::claim;
use anyhow::{Context, Result};
use qol_host_defaults::{HostDefaults, Lifetime, Role};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;

pub(super) const INTENT_FILE: &str = "default-roles.json";

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
pub(super) struct Intents {
    #[serde(default)]
    pub roles: BTreeMap<String, String>,
}

pub(super) struct IntentStore {
    path: PathBuf,
}

impl IntentStore {
    pub(super) fn at(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn active() -> Result<Self> {
        let scope = crate::features::profile::ProfileScopeStore::from_active()?;
        Ok(Self::at(scope.core_dir().join(INTENT_FILE)))
    }

    pub(super) fn load(&self) -> Result<Intents> {
        match std::fs::read(&self.path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("parsing {}", self.path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Intents::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", self.path.display())),
        }
    }

    fn save(&self, intents: &Intents) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(intents).context("encoding the role choices")?;
        qol_fs::atomic_write(&self.path, &bytes)
            .with_context(|| format!("writing {}", self.path.display()))
    }

    pub(super) fn set(&self, role: Role, plugin_id: &str) -> Result<()> {
        let mut intents = self.load()?;
        intents
            .roles
            .insert(role.id().to_string(), plugin_id.to_string());
        self.save(&intents)
    }

    pub(super) fn clear(&self, role: Role) -> Result<()> {
        let mut intents = self.load()?;
        if intents.roles.remove(role.id()).is_none() {
            return Ok(());
        }
        self.save(&intents)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Reclaim {
    Claimed,
    AlreadyHeld,
    Refused(String),
    PluginGone,
    UnknownRole,
}

pub(super) fn reclaim<H: HostDefaults>(
    host: &H,
    lifetime: Lifetime,
    intents: &Intents,
    declaration_of: impl Fn(&str, Role) -> Option<Declaration>,
    is_held: impl Fn(&Declaration) -> bool,
    claim: impl Fn(&Declaration) -> Result<()>,
) -> Vec<(String, Reclaim)> {
    intents
        .roles
        .iter()
        .map(|(role_id, plugin_id)| {
            let outcome = reclaim_one(
                host,
                lifetime,
                role_id,
                plugin_id,
                &declaration_of,
                &is_held,
                &claim,
            );
            (role_id.clone(), outcome)
        })
        .collect()
}

fn reclaim_one<H: HostDefaults>(
    host: &H,
    lifetime: Lifetime,
    role_id: &str,
    plugin_id: &str,
    declaration_of: &impl Fn(&str, Role) -> Option<Declaration>,
    is_held: &impl Fn(&Declaration) -> bool,
    claim: &impl Fn(&Declaration) -> Result<()>,
) -> Reclaim {
    let Some(role) = Role::parse(role_id) else {
        return Reclaim::UnknownRole;
    };
    let Some(declaration) = declaration_of(plugin_id, role) else {
        return Reclaim::PluginGone;
    };
    if is_held(&declaration) {
        return Reclaim::AlreadyHeld;
    }
    let report = host.probe(role);
    if let Err(error) = claim::admit(role, &report.state, &report.lifetimes, lifetime) {
        return Reclaim::Refused(format!("{error:#}"));
    }
    match claim(&declaration) {
        Ok(()) => Reclaim::Claimed,
        Err(error) => Reclaim::Refused(format!("{error:#}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginUid;
    use qol_host_defaults::{
        Ledger, OpenerSpec, Owner, ReleaseOutcome, Report, Restore, SlotRoute, State,
    };
    use std::cell::RefCell;

    struct FakeHost {
        state: State,
        lifetimes: Vec<Lifetime>,
    }

    impl FakeHost {
        fn allowing(lifetime: Lifetime) -> Self {
            Self {
                state: State::Silent,
                lifetimes: vec![lifetime],
            }
        }
    }

    impl HostDefaults for FakeHost {
        fn probe(&self, role: Role) -> Report {
            Report {
                role,
                state: self.state.clone(),
                owner: Owner::NoHostTool,
                triggers: Vec::new(),
                kept: Vec::new(),
                restore: Restore::Exact,
                lifetimes: self.lifetimes.clone(),
            }
        }

        fn slot_route(&self, _role: Role) -> Option<SlotRoute> {
            None
        }

        fn claim_handler(
            &self,
            _role: Role,
            _opener: &OpenerSpec,
            _plugin_id: &str,
            _lifetime: Lifetime,
            _ledger: &Ledger,
        ) -> Result<()> {
            anyhow::bail!("not used")
        }

        fn release(&self, _role: Role, _ledger: &Ledger) -> Result<ReleaseOutcome> {
            anyhow::bail!("not used")
        }
    }

    fn declaration(plugin_id: &str, role: Role) -> Declaration {
        Declaration {
            plugin_id: plugin_id.to_string(),
            uid: PluginUid::new(plugin_id),
            role,
            action: "screenshot".to_string(),
        }
    }

    fn intents(entries: &[(&str, &str)]) -> Intents {
        Intents {
            roles: entries
                .iter()
                .map(|(role, plugin)| ((*role).to_string(), (*plugin).to_string()))
                .collect(),
        }
    }

    fn installed(plugin_id: &str, role: Role) -> Option<Declaration> {
        Some(declaration(plugin_id, role))
    }

    fn run(
        host: &FakeHost,
        intents: &Intents,
        held: bool,
        claimed: &RefCell<Vec<String>>,
    ) -> Vec<(String, Reclaim)> {
        reclaim(
            host,
            Lifetime::PortableSession,
            intents,
            installed,
            |_| held,
            |declaration| {
                claimed.borrow_mut().push(format!(
                    "{}:{}",
                    declaration.role.id(),
                    declaration.plugin_id
                ));
                Ok(())
            },
        )
    }

    #[test]
    fn a_missing_file_holds_no_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = IntentStore::at(dir.path().join("core").join(INTENT_FILE));
        assert_eq!(store.load().expect("load"), Intents::default());
    }

    #[test]
    fn set_then_clear_round_trips_through_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = IntentStore::at(dir.path().join("core").join(INTENT_FILE));

        store.set(Role::SlotScreenshot, "qol-shot").expect("set");
        store.set(Role::FolderOpen, "qol-explorer").expect("set");
        assert_eq!(
            store.load().expect("load"),
            intents(&[
                ("slot.screenshot", "qol-shot"),
                ("folder.open", "qol-explorer")
            ])
        );

        store.clear(Role::SlotScreenshot).expect("clear");
        assert_eq!(
            store.load().expect("load"),
            intents(&[("folder.open", "qol-explorer")])
        );
    }

    #[test]
    fn setting_a_role_again_replaces_the_plugin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = IntentStore::at(dir.path().join(INTENT_FILE));
        store.set(Role::SlotLauncher, "qol-launcher").expect("set");
        store.set(Role::SlotLauncher, "qol-other").expect("set");
        assert_eq!(
            store.load().expect("load"),
            intents(&[("slot.launcher", "qol-other")])
        );
    }

    #[test]
    fn clearing_a_role_that_was_never_set_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(INTENT_FILE);
        IntentStore::at(path.clone())
            .clear(Role::SlotLauncher)
            .expect("clear");
        assert!(!path.exists());
    }

    #[test]
    fn a_corrupt_file_is_an_error_not_an_empty_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(INTENT_FILE);
        std::fs::write(&path, "not json").expect("write");
        assert!(IntentStore::at(path).load().is_err());
    }

    #[test]
    fn an_intended_role_the_probe_allows_is_claimed() {
        let claimed = RefCell::new(Vec::new());
        let results = run(
            &FakeHost::allowing(Lifetime::PortableSession),
            &intents(&[("slot.screenshot", "qol-shot")]),
            false,
            &claimed,
        );
        assert_eq!(
            results,
            vec![("slot.screenshot".to_string(), Reclaim::Claimed)]
        );
        assert_eq!(claimed.into_inner(), vec!["slot.screenshot:qol-shot"]);
    }

    #[test]
    fn a_role_already_held_is_left_alone() {
        let claimed = RefCell::new(Vec::new());
        let results = run(
            &FakeHost::allowing(Lifetime::PortableSession),
            &intents(&[("slot.screenshot", "qol-shot")]),
            true,
            &claimed,
        );
        assert_eq!(results[0].1, Reclaim::AlreadyHeld);
        assert!(claimed.borrow().is_empty());
    }

    #[test]
    fn a_probe_that_refuses_is_never_claimed_and_says_why() {
        let claimed = RefCell::new(Vec::new());
        let host = FakeHost {
            state: State::Impossible {
                reason: "Finder owns it".to_string(),
            },
            lifetimes: vec![Lifetime::PortableSession],
        };
        let results = run(
            &host,
            &intents(&[("folder.open", "qol-explorer")]),
            false,
            &claimed,
        );
        assert!(
            matches!(&results[0].1, Reclaim::Refused(reason) if reason.contains("Finder owns it")),
            "{results:?}"
        );
        assert!(claimed.borrow().is_empty());
    }

    #[test]
    fn a_lifetime_the_probe_does_not_offer_is_never_claimed() {
        let claimed = RefCell::new(Vec::new());
        let results = run(
            &FakeHost::allowing(Lifetime::ResidentPolicy),
            &intents(&[("slot.screenshot", "qol-shot")]),
            false,
            &claimed,
        );
        assert!(
            matches!(&results[0].1, Reclaim::Refused(reason) if reason.contains("portable")),
            "{results:?}"
        );
        assert!(claimed.borrow().is_empty());
    }

    #[test]
    fn a_plugin_that_no_longer_declares_the_role_is_skipped() {
        let claimed = RefCell::new(Vec::new());
        let results = reclaim(
            &FakeHost::allowing(Lifetime::PortableSession),
            Lifetime::PortableSession,
            &intents(&[("slot.screenshot", "qol-gone")]),
            |_, _| None,
            |_| false,
            |declaration| {
                claimed.borrow_mut().push(declaration.plugin_id.clone());
                Ok(())
            },
        );
        assert_eq!(results[0].1, Reclaim::PluginGone);
        assert!(claimed.borrow().is_empty());
    }

    #[test]
    fn an_unknown_role_id_is_skipped() {
        let claimed = RefCell::new(Vec::new());
        let results = run(
            &FakeHost::allowing(Lifetime::PortableSession),
            &intents(&[("slot.nonsense", "qol-shot")]),
            false,
            &claimed,
        );
        assert_eq!(results[0].1, Reclaim::UnknownRole);
        assert!(claimed.borrow().is_empty());
    }

    #[test]
    fn a_claim_the_host_rejects_is_reported_and_the_rest_still_run() {
        let results = reclaim(
            &FakeHost::allowing(Lifetime::PortableSession),
            Lifetime::PortableSession,
            &intents(&[
                ("folder.open", "qol-explorer"),
                ("slot.screenshot", "qol-shot"),
            ]),
            installed,
            |_| false,
            |declaration| match declaration.role {
                Role::FolderOpen => Err(anyhow::anyhow!("mimeapps is read-only")),
                _ => Ok(()),
            },
        );
        assert!(
            matches!(&results[0].1, Reclaim::Refused(reason) if reason.contains("read-only")),
            "{results:?}"
        );
        assert_eq!(results[1].1, Reclaim::Claimed);
    }
}
