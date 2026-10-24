use super::catalog::{self, Declaration};
use super::store;
use crate::hotkeys::{HotkeyBinding, HotkeyLayers};
use crate::plugins::PluginUid;
use anyhow::{anyhow, Context, Result};
use qol_host_defaults::{ClaimRecord, ClaimState, Ledger, Lifetime, ReleaseOutcome, Role};
use std::collections::BTreeSet;

pub(super) const HOTKEY_STORE: &str = "hotkey";

pub(super) trait SlotHost {
    fn normalize(&self, combo: &str) -> Option<String>;
    fn free(&self, combos: &[String]) -> Result<()>;
    fn restore(&self, combos: &[String]);
    fn reload(&self);
    fn declared(&self, plugin_id: &str, role: Role) -> Option<Declaration>;
    fn holder_label(&self, uid: &PluginUid, action: &str) -> String;
}

pub(super) struct LiveSlots;

impl SlotHost for LiveSlots {
    fn normalize(&self, combo: &str) -> Option<String> {
        crate::doctor::normalize_combo(combo)
    }

    fn free(&self, combos: &[String]) -> Result<()> {
        crate::doctor::free_host_bindings(combos)
    }

    fn restore(&self, combos: &[String]) {
        let summary = crate::hotkeys::takeover::restore_combos(combos);
        for failure in &summary.failures {
            log::warn!("host defaults: restoring a system binding failed: {failure}");
        }
    }

    fn reload(&self) {
        crate::hotkeys::trigger_reload();
    }

    fn declared(&self, plugin_id: &str, role: Role) -> Option<Declaration> {
        catalog::load_plugin(plugin_id)?.declaration_for(role)
    }

    fn holder_label(&self, uid: &PluginUid, action: &str) -> String {
        let plugins = catalog::installed();
        let holder = plugins.iter().find(|plugin| plugin.uid() == *uid);
        catalog::holder_label(holder, uid, action)
    }
}

pub(super) struct SlotContext<'a> {
    pub layers: &'a HotkeyLayers,
    pub ledger: &'a Ledger,
    pub host: &'a dyn SlotHost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NewBinding {
    pub id: String,
    pub key: String,
}

pub(super) fn binding_id(role: Role, ordinal: usize) -> String {
    format!("host-default-{}-{ordinal}", role.id())
}

pub(super) fn plan_bindings(
    existing: &[HotkeyBinding],
    declaration: &Declaration,
    keys: &[String],
    normalize: impl Fn(&str) -> Option<String>,
    holder_label: impl Fn(&HotkeyBinding) -> String,
) -> Result<Vec<NewBinding>> {
    let mut planned = Vec::new();
    for (index, key) in keys.iter().enumerate() {
        let wanted = normalize(key).ok_or_else(|| anyhow!("{key} is not a key combination"))?;
        let holders: Vec<&HotkeyBinding> = existing
            .iter()
            .filter(|binding| binding.enabled)
            .filter(|binding| normalize(&binding.key).as_deref() == Some(wanted.as_str()))
            .collect();
        let ours = |binding: &&HotkeyBinding| {
            binding.plugin_uid == declaration.uid && binding.action == declaration.action
        };
        if holders.iter().any(ours) {
            continue;
        }
        if let Some(other) = holders.first() {
            return Err(anyhow!(
                "{key} is already bound to {}. Change that key in Hotkeys first.",
                holder_label(other)
            ));
        }
        planned.push(NewBinding {
            id: binding_id(declaration.role, index + 1),
            key: key.clone(),
        });
    }
    Ok(planned)
}

pub(super) fn add_bindings(
    bindings: &mut Vec<HotkeyBinding>,
    declaration: &Declaration,
    planned: &[NewBinding],
) {
    bindings.retain(|binding| !planned.iter().any(|new| new.id == binding.id));
    bindings.extend(planned.iter().map(|new| HotkeyBinding {
        id: new.id.clone(),
        key: new.key.clone(),
        plugin_uid: declaration.uid.clone(),
        action: declaration.action.clone(),
        enabled: true,
        os_only: false,
    }));
}

#[derive(Debug)]
pub(super) struct Undo {
    pub kept: Vec<HotkeyBinding>,
    pub removed: usize,
    pub user_edited: usize,
}

fn untouched(
    binding: &HotkeyBinding,
    record: &ClaimRecord,
    expected: Option<&Declaration>,
) -> bool {
    binding.enabled
        && record.applied == binding.key
        && expected.is_none_or(|declaration| {
            binding.plugin_uid == declaration.uid && binding.action == declaration.action
        })
}

pub(super) fn undo_bindings(
    bindings: Vec<HotkeyBinding>,
    records: &[ClaimRecord],
    expected: Option<&Declaration>,
) -> Undo {
    let mut kept = Vec::with_capacity(bindings.len());
    let mut removed = 0;
    let mut user_edited = 0;
    for binding in bindings {
        let Some(record) = records.iter().find(|record| record.key == binding.id) else {
            kept.push(binding);
            continue;
        };
        if untouched(&binding, record, expected) {
            removed += 1;
            continue;
        }
        user_edited += 1;
        kept.push(binding);
    }
    Undo {
        kept,
        removed,
        user_edited,
    }
}

pub(super) fn hotkey_records(ledger: &Ledger, role: Role) -> Result<Vec<ClaimRecord>> {
    Ok(store::claims(ledger)?
        .into_iter()
        .filter(|record| record.role == role.id() && record.store == HOTKEY_STORE)
        .collect())
}

pub(super) fn claim_slot(
    context: &SlotContext<'_>,
    declaration: &Declaration,
    keys: &[String],
    lifetime: Lifetime,
) -> Result<()> {
    let mut config = context
        .layers
        .load()
        .context("reading the hotkey bindings")?;
    let planned = plan_bindings(
        &config.hotkeys,
        declaration,
        keys,
        |combo| context.host.normalize(combo),
        |binding| {
            context
                .host
                .holder_label(&binding.plugin_uid, &binding.action)
        },
    )?;
    for new in &planned {
        context
            .ledger
            .insert(ClaimRecord {
                role: declaration.role.id().to_string(),
                plugin_id: declaration.plugin_id.clone(),
                store: HOTKEY_STORE.to_string(),
                key: new.id.clone(),
                previous: None,
                applied: new.key.clone(),
                lifetime,
                state: ClaimState::Applied,
            })
            .context("recording the hotkey claim")?;
    }
    add_bindings(&mut config.hotkeys, declaration, &planned);
    if let Err(error) = context.layers.save(&config) {
        let _ = context.ledger.remove_store(declaration.role, HOTKEY_STORE);
        return Err(error.context("saving the hotkey bindings"));
    }
    if let Err(error) = context.host.free(keys) {
        if let Err(undo) = release_slot(context, declaration.role, keys) {
            log::warn!("host defaults: could not undo a refused slot claim: {undo:#}");
        }
        return Err(error);
    }
    context.host.reload();
    Ok(())
}

pub(super) fn release_slot(
    context: &SlotContext<'_>,
    role: Role,
    route_keys: &[String],
) -> Result<ReleaseOutcome> {
    let records = hotkey_records(context.ledger, role)?;
    let mut config = context
        .layers
        .load()
        .context("reading the hotkey bindings")?;
    let expected = records
        .first()
        .and_then(|record| context.host.declared(&record.plugin_id, role));
    let undo = undo_bindings(
        std::mem::take(&mut config.hotkeys),
        &records,
        expected.as_ref(),
    );
    config.hotkeys = undo.kept;
    if undo.removed > 0 {
        context
            .layers
            .save(&config)
            .context("saving the hotkey bindings")?;
    }
    context.ledger.remove_store(role, HOTKEY_STORE)?;
    let combos: Vec<String> = route_keys
        .iter()
        .cloned()
        .chain(records.iter().map(|record| record.applied.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    context.host.restore(&combos);
    context.host.reload();
    Ok(outcome_of(&records, undo.removed, undo.user_edited))
}

fn outcome_of(records: &[ClaimRecord], removed: usize, user_edited: usize) -> ReleaseOutcome {
    if records.is_empty() {
        return ReleaseOutcome::AlreadyGone;
    }
    if user_edited > 0 && removed == 0 {
        return ReleaseOutcome::LeftUserChange;
    }
    ReleaseOutcome::Restored
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeSlots {
        freed: RefCell<Vec<Vec<String>>>,
        restored: RefCell<Vec<Vec<String>>>,
        reloads: RefCell<usize>,
        refuse: bool,
    }

    impl SlotHost for FakeSlots {
        fn normalize(&self, combo: &str) -> Option<String> {
            Some(combo.to_lowercase())
        }

        fn free(&self, combos: &[String]) -> Result<()> {
            if self.refuse {
                return Err(anyhow!("the system keeps a binding"));
            }
            self.freed.borrow_mut().push(combos.to_vec());
            Ok(())
        }

        fn restore(&self, combos: &[String]) {
            self.restored.borrow_mut().push(combos.to_vec());
        }

        fn reload(&self) {
            *self.reloads.borrow_mut() += 1;
        }

        fn declared(&self, _plugin_id: &str, _role: Role) -> Option<Declaration> {
            Some(declaration())
        }

        fn holder_label(&self, uid: &PluginUid, action: &str) -> String {
            format!("{} {action}", uid.as_str())
        }
    }

    fn declaration() -> Declaration {
        Declaration {
            plugin_id: "qol-shot".to_string(),
            uid: PluginUid::new("uid-shot"),
            role: Role::SlotScreenshot,
            action: "screenshot".to_string(),
        }
    }

    fn keys(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn binding(id: &str, key: &str, uid: &str, action: &str, enabled: bool) -> HotkeyBinding {
        HotkeyBinding {
            id: id.to_string(),
            key: key.to_string(),
            plugin_uid: PluginUid::new(uid),
            action: action.to_string(),
            enabled,
            os_only: false,
        }
    }

    fn record(key: &str, applied: &str) -> ClaimRecord {
        ClaimRecord {
            role: "slot.screenshot".to_string(),
            plugin_id: "qol-shot".to_string(),
            store: HOTKEY_STORE.to_string(),
            key: key.to_string(),
            previous: None,
            applied: applied.to_string(),
            lifetime: Lifetime::PortableSession,
            state: ClaimState::Applied,
        }
    }

    fn lower(combo: &str) -> Option<String> {
        Some(combo.to_lowercase())
    }

    fn label_of(binding: &HotkeyBinding) -> String {
        format!("{} {}", binding.plugin_uid.as_str(), binding.action)
    }

    #[test]
    fn ids_follow_the_role_and_the_key_position() {
        let planned = plan_bindings(
            &[],
            &declaration(),
            &keys(&["Print", "Shift+Print"]),
            lower,
            label_of,
        )
        .expect("plan");
        assert_eq!(
            planned,
            vec![
                NewBinding {
                    id: "host-default-slot.screenshot-1".to_string(),
                    key: "Print".to_string()
                },
                NewBinding {
                    id: "host-default-slot.screenshot-2".to_string(),
                    key: "Shift+Print".to_string()
                },
            ]
        );
    }

    #[test]
    fn an_identical_enabled_binding_is_left_alone() {
        let existing = [binding("mine", "print", "uid-shot", "screenshot", true)];
        let planned = plan_bindings(
            &existing,
            &declaration(),
            &keys(&["Print"]),
            lower,
            label_of,
        )
        .expect("plan");
        assert!(planned.is_empty());
    }

    #[test]
    fn a_disabled_identical_binding_is_replaced_by_a_new_one() {
        let existing = [binding("mine", "Print", "uid-shot", "screenshot", false)];
        let planned = plan_bindings(
            &existing,
            &declaration(),
            &keys(&["Print"]),
            lower,
            label_of,
        )
        .expect("plan");
        assert_eq!(planned.len(), 1);
    }

    #[test]
    fn a_key_another_plugin_binds_refuses_the_claim() {
        let existing = [binding("theirs", "Print", "uid-other", "grab", true)];
        let error = plan_bindings(
            &existing,
            &declaration(),
            &keys(&["Print"]),
            lower,
            label_of,
        )
        .expect_err("conflict");
        assert_eq!(
            error.to_string(),
            "Print is already bound to uid-other grab. Change that key in Hotkeys first."
        );
    }

    #[test]
    fn a_key_that_is_not_a_combination_refuses_the_claim() {
        let result = plan_bindings(&[], &declaration(), &keys(&["Print"]), |_| None, label_of);
        assert!(result.is_err());
    }

    #[test]
    fn adding_replaces_a_stale_binding_with_the_same_id() {
        let mut bindings = vec![
            binding(
                "host-default-slot.screenshot-1",
                "F1",
                "uid-shot",
                "old",
                false,
            ),
            binding("user", "F2", "uid-x", "y", true),
        ];
        let planned = vec![NewBinding {
            id: "host-default-slot.screenshot-1".to_string(),
            key: "Print".to_string(),
        }];
        add_bindings(&mut bindings, &declaration(), &planned);
        assert_eq!(bindings.len(), 2);
        let added = bindings
            .iter()
            .find(|binding| binding.id == "host-default-slot.screenshot-1")
            .expect("added");
        assert!(added.enabled && added.key == "Print" && added.action == "screenshot");
    }

    #[test]
    fn undo_deletes_matching_bindings_and_leaves_user_edits() {
        let bindings = vec![
            binding(
                "host-default-slot.screenshot-1",
                "Print",
                "uid-shot",
                "screenshot",
                true,
            ),
            binding(
                "host-default-slot.screenshot-2",
                "Ctrl+Print",
                "uid-shot",
                "screenshot",
                true,
            ),
            binding("user", "F2", "uid-x", "y", true),
        ];
        let records = [
            record("host-default-slot.screenshot-1", "Print"),
            record("host-default-slot.screenshot-2", "Shift+Print"),
        ];
        let undo = undo_bindings(bindings, &records, Some(&declaration()));
        assert_eq!(undo.removed, 1);
        assert_eq!(undo.user_edited, 1);
        assert_eq!(
            undo.kept.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            vec!["host-default-slot.screenshot-2", "user"]
        );
    }

    #[test]
    fn undo_leaves_a_binding_the_user_disabled_or_repointed() {
        let name = "host-default-slot.screenshot-1";
        let cases = [
            binding(name, "Print", "uid-shot", "screenshot", false),
            binding(name, "Print", "uid-other", "screenshot", true),
            binding(name, "Print", "uid-shot", "other", true),
        ];
        let records = [record(name, "Print")];
        for changed in cases {
            let undo = undo_bindings(vec![changed], &records, Some(&declaration()));
            assert_eq!((undo.removed, undo.user_edited), (0, 1));
            assert_eq!(undo.kept.len(), 1);
        }
    }

    #[test]
    fn undo_without_a_resolvable_plugin_checks_key_and_enabled_only() {
        let name = "host-default-slot.screenshot-1";
        let records = [record(name, "Print")];
        let kept = undo_bindings(
            vec![binding(name, "Print", "uid-gone", "screenshot", true)],
            &records,
            None,
        );
        assert_eq!(kept.removed, 1);
        let disabled = undo_bindings(
            vec![binding(name, "Print", "uid-gone", "screenshot", false)],
            &records,
            None,
        );
        assert_eq!(disabled.user_edited, 1);
    }

    #[test]
    fn a_binding_the_user_deleted_is_simply_gone() {
        let records = [record("host-default-slot.screenshot-1", "Print")];
        let undo = undo_bindings(Vec::new(), &records, Some(&declaration()));
        assert_eq!((undo.removed, undo.user_edited), (0, 0));
    }

    #[test]
    fn outcomes_name_what_happened() {
        assert!(matches!(outcome_of(&[], 0, 0), ReleaseOutcome::AlreadyGone));
        let one = [record("a", "Print")];
        assert!(matches!(outcome_of(&one, 1, 0), ReleaseOutcome::Restored));
        assert!(matches!(
            outcome_of(&one, 0, 1),
            ReleaseOutcome::LeftUserChange
        ));
    }

    fn layers_in(dir: &std::path::Path) -> HotkeyLayers {
        HotkeyLayers {
            core: dir.join("core-hotkeys.json"),
            os: dir.join("os-hotkeys.json"),
        }
    }

    #[test]
    fn claim_then_release_round_trips_bindings_and_system_keys() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layers = layers_in(dir.path());
        let ledger = Ledger::at(dir.path().join("host-defaults.json"));
        let host = FakeSlots::default();
        let context = SlotContext {
            layers: &layers,
            ledger: &ledger,
            host: &host,
        };
        let route = keys(&["Print"]);

        claim_slot(&context, &declaration(), &route, Lifetime::PortableSession).expect("claim");

        let saved = layers.load().expect("load").hotkeys;
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, "host-default-slot.screenshot-1");
        assert!(saved[0].enabled);
        assert_eq!(host.freed.borrow().as_slice(), std::slice::from_ref(&route));
        assert_eq!(
            ledger.holder(Role::SlotScreenshot).expect("holder"),
            Some("qol-shot".to_string())
        );

        let outcome = release_slot(&context, Role::SlotScreenshot, &route).expect("release");

        assert!(matches!(outcome, ReleaseOutcome::Restored));
        assert!(layers.load().expect("load").hotkeys.is_empty());
        assert_eq!(host.restored.borrow().as_slice(), [route]);
        assert_eq!(ledger.holder(Role::SlotScreenshot).expect("holder"), None);
    }

    #[test]
    fn a_refused_free_rolls_the_bindings_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layers = layers_in(dir.path());
        let ledger = Ledger::at(dir.path().join("host-defaults.json"));
        let host = FakeSlots {
            refuse: true,
            ..FakeSlots::default()
        };
        let context = SlotContext {
            layers: &layers,
            ledger: &ledger,
            host: &host,
        };

        let result = claim_slot(
            &context,
            &declaration(),
            &keys(&["Print"]),
            Lifetime::PortableSession,
        );

        assert!(result.is_err());
        assert!(layers.load().expect("load").hotkeys.is_empty());
        assert_eq!(ledger.holder(Role::SlotScreenshot).expect("holder"), None);
    }

    #[test]
    fn releasing_leaves_a_binding_the_user_edited() {
        let dir = tempfile::tempdir().expect("tempdir");
        let layers = layers_in(dir.path());
        let ledger = Ledger::at(dir.path().join("host-defaults.json"));
        let host = FakeSlots::default();
        let context = SlotContext {
            layers: &layers,
            ledger: &ledger,
            host: &host,
        };
        claim_slot(
            &context,
            &declaration(),
            &keys(&["Print"]),
            Lifetime::PortableSession,
        )
        .expect("claim");
        let mut config = layers.load().expect("load");
        config.hotkeys[0].key = "Ctrl+Print".to_string();
        layers.save(&config).expect("save");

        let outcome =
            release_slot(&context, Role::SlotScreenshot, &keys(&["Print"])).expect("release");

        assert!(matches!(outcome, ReleaseOutcome::LeftUserChange));
        assert_eq!(layers.load().expect("load").hotkeys.len(), 1);
    }
}
