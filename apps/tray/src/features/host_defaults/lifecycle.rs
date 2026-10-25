use super::catalog;
use super::claim::{self, Env};
use super::intent::{self, IntentStore, Reclaim};
use super::service;
use super::slot::{LiveSlots, HOTKEY_STORE};
use super::store;
use crate::hotkeys::HotkeyLayers;
use anyhow::Result;
use qol_host_defaults::{
    host, release_portable_handlers, ClaimRecord, HostDefaults, Lifetime, Role,
};
use std::collections::BTreeSet;

pub fn release_portable_on_start() {
    release_portable_session("startup");
    reclaim_intended_roles();
}

pub fn release_portable_on_exit() {
    release_portable_session("shutdown");
}

fn release_portable_session(phase: &str) {
    let failures = match run_release() {
        Ok(failures) => failures,
        Err(error) => {
            log::warn!("[host-defaults] {phase} release failed: {error:#}");
            return;
        }
    };
    for failure in failures {
        log::warn!("[host-defaults] {phase} release failed: {failure}");
    }
}

fn reclaim_intended_roles() {
    let intents = match IntentStore::active().and_then(|intents| intents.load()) {
        Ok(intents) => intents,
        Err(error) => {
            log::warn!("[host-defaults] reading the chosen roles failed: {error:#}");
            return;
        }
    };
    if intents.roles.is_empty() {
        return;
    }
    let host = host();
    let ledger = store::ledger().ok();
    let results = intent::reclaim(
        &host,
        store::lifetime_for_role,
        &intents,
        |plugin_id, role| catalog::load_plugin(plugin_id)?.declaration_for(role),
        |declaration| service::is_held(&host, ledger.as_ref(), declaration),
        service::claim_on_host,
    );
    for (role, result) in results {
        log_reclaim(&role, &result);
    }
}

fn log_reclaim(role: &str, result: &Reclaim) {
    match result {
        Reclaim::Claimed => log::info!("[host-defaults] retook {role} for the chosen plugin"),
        Reclaim::AlreadyHeld => {
            log::debug!("[host-defaults] {role} is already held; nothing to retake")
        }
        Reclaim::Refused(reason) => log::warn!("[host-defaults] {role} not retaken: {reason}"),
        Reclaim::PluginGone => log::warn!(
            "[host-defaults] {role} not retaken: the chosen plugin is not installed or no longer declares it"
        ),
        Reclaim::UnknownRole => {
            log::warn!("[host-defaults] {role} not retaken: not a role this build knows")
        }
    }
}

fn run_release() -> Result<Vec<String>> {
    let ledger = store::ledger()?;
    if store::claims(&ledger)?.is_empty() {
        return Ok(Vec::new());
    }
    let layers = HotkeyLayers::active()?;
    let host = host();
    let env = Env {
        host: &host,
        ledger: &ledger,
        layers: &layers,
        slots: &LiveSlots,
        exec: std::path::PathBuf::new(),
        lifetime: Lifetime::PortableSession,
    };
    Ok(release_portable(&env))
}

pub(super) fn release_portable<H: HostDefaults>(env: &Env<'_, H>) -> Vec<String> {
    let mut failures = Vec::new();
    for (store_key, result) in release_portable_handlers(env.host, env.ledger) {
        match result {
            Ok(outcome) => log::info!(
                "[host-defaults] released portable claim {store_key}: {}",
                claim::outcome_label(&outcome)
            ),
            Err(error) => failures.push(format!("{store_key}: {error:#}")),
        }
    }
    let records = match store::claims(env.ledger) {
        Ok(records) => records,
        Err(error) => {
            failures.push(format!("ledger: {error:#}"));
            return failures;
        }
    };
    for role in slot_roles(&records) {
        match claim::release_slot_role(env, role, &[]) {
            Ok(outcome) => log::info!(
                "[host-defaults] released portable claim {}: {}",
                role.id(),
                claim::outcome_label(&outcome)
            ),
            Err(error) => failures.push(format!("{}: {error:#}", role.id())),
        }
    }
    failures
}

pub(super) fn slot_roles(records: &[ClaimRecord]) -> Vec<Role> {
    let ids: BTreeSet<&str> = records
        .iter()
        .filter(|record| record.store == HOTKEY_STORE)
        .map(|record| record.role.as_str())
        .collect();
    Role::ALL
        .into_iter()
        .filter(|role| ids.contains(role.id()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::host_defaults::catalog::Declaration;
    use crate::features::host_defaults::slot::{self, SlotHost};
    use crate::hotkeys::HotkeyBinding;
    use crate::plugins::PluginUid;
    use qol_host_defaults::{
        ClaimState, Ledger, OpenerSpec, Owner, ReleaseOutcome, Report, Restore, SlotRoute, State,
    };
    use std::cell::RefCell;

    struct RouteHost;

    impl HostDefaults for RouteHost {
        fn probe(&self, role: Role) -> Report {
            Report {
                role,
                state: State::Silent,
                owner: Owner::NoHostTool,
                triggers: Vec::new(),
                kept: Vec::new(),
                restore: Restore::Exact,
                lifetimes: vec![Lifetime::PortableSession],
            }
        }

        fn slot_route(&self, _role: Role) -> Option<SlotRoute> {
            Some(SlotRoute {
                keys: vec!["Print".to_string()],
                reaches: vec!["Print".to_string()],
                owner: "Cinnamon".to_string(),
                answered_in: vec!["/org/cinnamon/".to_string()],
            })
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

    #[derive(Default)]
    struct RecordingSlots {
        freed: RefCell<Vec<Vec<String>>>,
        restored: RefCell<Vec<Vec<String>>>,
    }

    impl SlotHost for RecordingSlots {
        fn normalize(&self, combo: &str) -> Option<String> {
            Some(combo.to_lowercase())
        }

        fn free(&self, combos: &[String]) -> Result<()> {
            self.freed.borrow_mut().push(combos.to_vec());
            Ok(())
        }

        fn restore(&self, combos: &[String]) {
            self.restored.borrow_mut().push(combos.to_vec());
        }

        fn reload(&self) {}

        fn declared(&self, _plugin_id: &str, _role: Role) -> Option<Declaration> {
            Some(shot_declaration())
        }

        fn holder_label(&self, uid: &PluginUid, action: &str) -> String {
            format!("{} {action}", uid.as_str())
        }
    }

    fn shot_declaration() -> Declaration {
        Declaration {
            plugin_id: "qol-shot".to_string(),
            uid: PluginUid::new("uid-shot"),
            role: Role::SlotScreenshot,
            action: "screenshot".to_string(),
        }
    }

    fn own_binding() -> HotkeyBinding {
        HotkeyBinding {
            id: "shot-default".to_string(),
            key: "Print".to_string(),
            plugin_uid: PluginUid::new("uid-shot"),
            action: "screenshot".to_string(),
            enabled: true,
            os_only: false,
        }
    }

    fn layers_in(dir: &std::path::Path) -> HotkeyLayers {
        HotkeyLayers {
            core: dir.join("core-hotkeys.json"),
            os: dir.join("os-hotkeys.json"),
        }
    }

    fn env_in<'a>(
        layers: &'a HotkeyLayers,
        ledger: &'a Ledger,
        slots: &'a RecordingSlots,
    ) -> Env<'a, RouteHost> {
        Env {
            host: &RouteHost,
            ledger,
            layers,
            slots,
            exec: std::path::PathBuf::new(),
            lifetime: Lifetime::PortableSession,
        }
    }

    fn record(role: &str, store: &str, lifetime: Lifetime) -> ClaimRecord {
        ClaimRecord {
            role: role.to_string(),
            plugin_id: "qol-shot".to_string(),
            store: store.to_string(),
            key: "k".to_string(),
            previous: None,
            applied: "v".to_string(),
            lifetime,
            state: ClaimState::Applied,
        }
    }

    #[test]
    fn every_slot_hotkey_claim_is_released_whatever_its_recorded_lifetime() {
        let records = [
            record("slot.screenshot", HOTKEY_STORE, Lifetime::PortableSession),
            record("slot.screenshot", HOTKEY_STORE, Lifetime::PortableSession),
            record("slot.launcher", HOTKEY_STORE, Lifetime::ResidentPolicy),
            record("folder.open", "mimeapps", Lifetime::PortableSession),
            record("slot.nonsense", HOTKEY_STORE, Lifetime::PortableSession),
            record("slot.dictation", HOTKEY_STORE, Lifetime::PortableSession),
        ];
        assert_eq!(
            slot_roles(&records),
            vec![
                Role::SlotLauncher,
                Role::SlotScreenshot,
                Role::SlotDictation
            ]
        );
    }

    #[test]
    fn an_empty_ledger_replays_nothing() {
        assert!(slot_roles(&[]).is_empty());
    }

    #[test]
    fn a_session_end_release_keeps_the_own_binding_enabled_and_restores_the_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layers = layers_in(dir.path());
        let ledger = Ledger::at(dir.path().join("host-defaults.json"));
        let slots = RecordingSlots::default();
        let env = env_in(&layers, &ledger, &slots);
        let mut config = layers.load().expect("load");
        config.hotkeys.push(own_binding());
        config.hotkeys.push(HotkeyBinding {
            id: slot::binding_id(Role::SlotScreenshot, 1),
            os_only: true,
            ..own_binding()
        });
        layers.save(&config).expect("save");
        let mut claimed = record("slot.screenshot", HOTKEY_STORE, Lifetime::PortableSession);
        claimed.key = slot::binding_id(Role::SlotScreenshot, 1);
        claimed.applied = "Print".to_string();
        ledger.insert(claimed).expect("record");

        let failures = release_portable(&env);

        assert!(failures.is_empty(), "{failures:?}");
        let saved = layers.load().expect("load").hotkeys;
        assert_eq!(saved.len(), 1);
        assert!(saved[0].enabled && saved[0].id == "shot-default");
        assert_eq!(
            slots.restored.borrow().as_slice(),
            [vec!["Print".to_string()]]
        );
    }

    #[test]
    fn the_start_after_a_user_release_claims_and_frees_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layers = layers_in(dir.path());
        let ledger = Ledger::at(dir.path().join("host-defaults.json"));
        let slots = RecordingSlots::default();
        let env = env_in(&layers, &ledger, &slots);
        let mut config = layers.load().expect("load");
        config.hotkeys.push(own_binding());
        layers.save(&config).expect("save");
        claim::release(&env, Role::SlotScreenshot, &[shot_declaration()]).expect("user release");

        let failures = release_portable(&env);
        let claimed = RefCell::new(Vec::new());
        let results = intent::reclaim(
            &RouteHost,
            |_| Lifetime::PortableSession,
            &intent::Intents::default(),
            |_, _| Some(shot_declaration()),
            |_| false,
            |declaration| {
                claimed.borrow_mut().push(declaration.plugin_id.clone());
                Ok(())
            },
        );

        assert!(failures.is_empty(), "{failures:?}");
        assert!(results.is_empty());
        assert!(claimed.borrow().is_empty());
        assert!(slots.freed.borrow().is_empty());
        let saved = layers.load().expect("load").hotkeys;
        assert_eq!(saved.len(), 1);
        assert!(!saved[0].enabled);
    }
}
