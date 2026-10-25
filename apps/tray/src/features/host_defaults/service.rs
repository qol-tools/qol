use super::catalog::{self, Declaration};
use super::claim::{self, Env};
use super::held;
use super::intent::IntentStore;
use super::slot::LiveSlots;
use super::store;
use crate::hotkeys::HotkeyLayers;
use anyhow::{Context, Result};
use qol_host_defaults::{host, row_text, HostDefaults, Ledger, ReleaseOutcome, Role, RowControl};

pub(crate) const ROW_QUERY: &str = "host_default";
pub(crate) const ROW_ACTION: &str = "host_default_toggle";

#[derive(Clone, Debug)]
pub(crate) struct PluginRow {
    pub label: String,
    pub description: String,
    pub control: RowControl,
}

pub(crate) fn row_for_plugin(plugin_id: &str) -> Option<PluginRow> {
    let declaration = catalog::load_plugin(plugin_id)?.main_declaration()?;
    let host = host();
    let ledger = store::ledger().ok();
    let held = is_held(&host, ledger.as_ref(), &declaration);
    let report = host.probe(declaration.role);
    let text = row_text(&report, held, store::lifetime_for_role(declaration.role))?;
    Some(PluginRow {
        label: text.label,
        description: text.description,
        control: text.control,
    })
}

pub(crate) fn toggle_for_plugin(plugin_id: &str) -> Result<Option<PluginRow>> {
    let Some(declaration) = catalog::load_plugin(plugin_id).and_then(|p| p.main_declaration())
    else {
        return Ok(None);
    };
    let held = is_held(&host(), store::ledger().ok().as_ref(), &declaration);
    if held {
        release_role(declaration.role)?;
    } else {
        claim_declared(&declaration)?;
    }
    Ok(row_for_plugin(plugin_id))
}

pub(super) fn claim_declared(declaration: &Declaration) -> Result<()> {
    let intents = IntentStore::active()?;
    remember_claim(&intents, declaration, claim_on_host)
}

pub(super) fn release_role(role: Role) -> Result<ReleaseOutcome> {
    let intents = IntentStore::active()?;
    forget_release(&intents, role, release_on_host)
}

fn remember_claim(
    intents: &IntentStore,
    declaration: &Declaration,
    claim: impl FnOnce(&Declaration) -> Result<()>,
) -> Result<()> {
    claim(declaration)?;
    intents
        .set(declaration.role, &declaration.plugin_id)
        .context("the host changed but your choice could not be saved to the profile")
}

fn forget_release(
    intents: &IntentStore,
    role: Role,
    release: impl FnOnce(Role) -> Result<ReleaseOutcome>,
) -> Result<ReleaseOutcome> {
    let outcome = release(role)?;
    intents
        .clear(role)
        .context("the host changed back but your choice could not be cleared in the profile")?;
    Ok(outcome)
}

pub(super) fn claim_on_host(declaration: &Declaration) -> Result<()> {
    let host = host();
    let ledger = store::ledger()?;
    let layers = HotkeyLayers::active()?;
    let env = Env {
        host: &host,
        ledger: &ledger,
        layers: &layers,
        slots: &LiveSlots,
        exec: std::env::current_exe().context("locating the qol-tray binary")?,
        lifetime: store::lifetime_for_role(declaration.role),
    };
    claim::claim(&env, declaration)
}

fn release_on_host(role: Role) -> Result<ReleaseOutcome> {
    let host = host();
    let ledger = store::ledger()?;
    let layers = HotkeyLayers::active()?;
    let env = Env {
        host: &host,
        ledger: &ledger,
        layers: &layers,
        slots: &LiveSlots,
        exec: std::path::PathBuf::new(),
        lifetime: store::lifetime_for_role(role),
    };
    let declared: Vec<Declaration> = catalog::installed()
        .iter()
        .filter_map(|plugin| plugin.declaration_for(role))
        .collect();
    claim::release(&env, role, &declared)
}

pub(crate) fn row_json(row: &PluginRow) -> serde_json::Value {
    let (control, on, button) = match &row.control {
        RowControl::Switch { on } => ("switch", *on, None),
        RowControl::Button { label } => ("button", false, Some(label.as_str())),
        RowControl::ReadOnly => ("read_only", false, None),
    };
    serde_json::json!({
        "label": row.label,
        "description": row.description,
        "on": on,
        "control": control,
        "button": button,
    })
}

pub(super) fn is_held(
    host: &impl HostDefaults,
    ledger: Option<&Ledger>,
    declaration: &Declaration,
) -> bool {
    let holder = ledger.and_then(|ledger| match ledger.holder(declaration.role) {
        Ok(holder) => holder,
        Err(error) => {
            log::warn!("host defaults: ledger unreadable: {error:#}");
            None
        }
    });
    held::is_held(
        holder.as_deref(),
        &declaration.plugin_id,
        || declaration.role.is_slot() && slot_is_bound(host, declaration),
        || slot_is_shadowed(host, declaration),
    )
}

fn slot_keys(host: &impl HostDefaults, declaration: &Declaration) -> Vec<String> {
    host.slot_route(declaration.role)
        .map(|route| route.keys)
        .unwrap_or_default()
}

fn slot_is_bound(host: &impl HostDefaults, declaration: &Declaration) -> bool {
    let keys = slot_keys(host, declaration);
    let bindings = HotkeyLayers::active()
        .and_then(|layers| layers.load())
        .map(|config| config.hotkeys);
    let Ok(bindings) = bindings else {
        return false;
    };
    held::bound_on_every_key(
        &keys,
        &bindings,
        &declaration.uid,
        &declaration.action,
        crate::doctor::normalize_combo,
    )
}

fn slot_is_shadowed(host: &impl HostDefaults, declaration: &Declaration) -> bool {
    let keys = slot_keys(host, declaration);
    !crate::doctor::shadowed_combos(&keys).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginUid;
    use qol_host_defaults::{Lifetime, Owner, Report, Restore, State};

    fn declaration() -> Declaration {
        Declaration {
            plugin_id: "qol-shot".to_string(),
            uid: PluginUid::new("uid-shot"),
            role: Role::SlotScreenshot,
            action: "screenshot".to_string(),
        }
    }

    fn intent_store(dir: &tempfile::TempDir) -> IntentStore {
        IntentStore::at(dir.path().join("core").join("default-roles.json"))
    }

    fn remembered(store: &IntentStore) -> Vec<(String, String)> {
        store.load().expect("load").roles.into_iter().collect()
    }

    #[test]
    fn a_successful_claim_writes_the_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = intent_store(&dir);

        remember_claim(&store, &declaration(), |_| Ok(())).expect("claim");

        assert_eq!(
            remembered(&store),
            vec![("slot.screenshot".to_string(), "qol-shot".to_string())]
        );
    }

    #[test]
    fn a_refused_claim_writes_no_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = intent_store(&dir);

        let result = remember_claim(&store, &declaration(), |_| {
            Err(anyhow::anyhow!("the system keeps a binding"))
        });

        assert!(result.is_err());
        assert!(remembered(&store).is_empty());
    }

    #[test]
    fn a_successful_release_clears_the_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = intent_store(&dir);
        remember_claim(&store, &declaration(), |_| Ok(())).expect("claim");

        let outcome = forget_release(&store, Role::SlotScreenshot, |_| {
            Ok(ReleaseOutcome::Restored)
        })
        .expect("release");

        assert_eq!(outcome, ReleaseOutcome::Restored);
        assert!(remembered(&store).is_empty());
    }

    #[test]
    fn a_failed_release_keeps_the_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = intent_store(&dir);
        remember_claim(&store, &declaration(), |_| Ok(())).expect("claim");

        let result = forget_release(&store, Role::SlotScreenshot, |_| {
            Err(anyhow::anyhow!("dconf is gone"))
        });

        assert!(result.is_err());
        assert_eq!(remembered(&store).len(), 1);
    }

    #[test]
    fn a_slot_row_on_a_resident_host_says_it_goes_back() {
        let report = Report {
            role: Role::SlotWindowSwitcher,
            state: State::Silent,
            owner: Owner::Named("Cinnamon".to_string()),
            triggers: vec!["Alt+Tab".to_string()],
            kept: Vec::new(),
            restore: Restore::Exact,
            lifetimes: vec![Lifetime::PortableSession],
        };
        let lifetime = store::lifetime_for_role(report.role);

        let row = row_text(&report, false, lifetime).expect("row");

        assert_eq!(row.control, RowControl::Switch { on: false });
        assert_eq!(
            row.description,
            "Takes over: Alt+Tab. Goes back when qol stops."
        );
    }

    #[test]
    fn json_shape_for_each_control() {
        let cases = [
            (
                RowControl::Switch { on: true },
                serde_json::json!({"label": "L", "description": "D", "on": true, "control": "switch", "button": null}),
            ),
            (
                RowControl::Button {
                    label: "Allow".to_string(),
                },
                serde_json::json!({"label": "L", "description": "D", "on": false, "control": "button", "button": "Allow"}),
            ),
            (
                RowControl::ReadOnly,
                serde_json::json!({"label": "L", "description": "D", "on": false, "control": "read_only", "button": null}),
            ),
        ];
        for (control, expected) in cases {
            let row = PluginRow {
                label: "L".to_string(),
                description: "D".to_string(),
                control,
            };
            assert_eq!(row_json(&row), expected);
        }
    }

    #[test]
    fn the_settings_contract_names_are_stable() {
        assert_eq!(ROW_QUERY, "host_default");
        assert_eq!(ROW_ACTION, "host_default_toggle");
    }
}
