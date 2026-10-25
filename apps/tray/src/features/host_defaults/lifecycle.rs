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
    for role in portable_slot_roles(&records) {
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

pub(super) fn portable_slot_roles(records: &[ClaimRecord]) -> Vec<Role> {
    let ids: BTreeSet<&str> = records
        .iter()
        .filter(|record| record.store == HOTKEY_STORE)
        .filter(|record| record.lifetime == Lifetime::PortableSession)
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
    use qol_host_defaults::ClaimState;

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
    fn only_portable_hotkey_claims_are_replayed() {
        let records = [
            record("slot.screenshot", HOTKEY_STORE, Lifetime::PortableSession),
            record("slot.screenshot", HOTKEY_STORE, Lifetime::PortableSession),
            record("slot.launcher", HOTKEY_STORE, Lifetime::ResidentPolicy),
            record("folder.open", "mimeapps", Lifetime::PortableSession),
            record("slot.nonsense", HOTKEY_STORE, Lifetime::PortableSession),
            record("slot.dictation", HOTKEY_STORE, Lifetime::PortableSession),
        ];
        assert_eq!(
            portable_slot_roles(&records),
            vec![Role::SlotScreenshot, Role::SlotDictation]
        );
    }

    #[test]
    fn an_empty_ledger_replays_nothing() {
        assert!(portable_slot_roles(&[]).is_empty());
    }
}
