pub(crate) mod dconf;
mod ledger;
mod platform;

pub(crate) use dconf::{BindingEntry, BindingReach, MatchPolicy};

use qol_host_fixes::residency::HostResidency;
use std::fmt;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BindingRoot {
    pub dir: &'static str,
    pub match_policy: MatchPolicy,
    pub schema: Option<&'static str>,
}

const BINDING_ROOTS: &[BindingRoot] = &[
    BindingRoot {
        dir: "/org/cinnamon/desktop/keybindings/",
        match_policy: MatchPolicy::Subset,
        schema: Some("org.cinnamon.desktop.keybindings"),
    },
    BindingRoot {
        dir: "/org/cinnamon/desktop/keybindings/wm/",
        match_policy: MatchPolicy::Subset,
        schema: Some("org.cinnamon.desktop.keybindings.wm"),
    },
    BindingRoot {
        dir: "/org/cinnamon/desktop/keybindings/media-keys/",
        match_policy: MatchPolicy::Subset,
        schema: Some("org.cinnamon.desktop.keybindings.media-keys"),
    },
    BindingRoot {
        dir: "/org/cinnamon/muffin/keybindings/",
        match_policy: MatchPolicy::Exact,
        schema: Some("org.cinnamon.muffin.keybindings"),
    },
    BindingRoot {
        dir: "/org/gnome/desktop/wm/keybindings/",
        match_policy: MatchPolicy::Exact,
        schema: Some("org.gnome.desktop.wm.keybindings"),
    },
    BindingRoot {
        dir: "/org/gnome/settings-daemon/plugins/media-keys/",
        match_policy: MatchPolicy::Exact,
        schema: Some("org.gnome.settings-daemon.plugins.media-keys"),
    },
    BindingRoot {
        dir: "/org/gnome/shell/keybindings/",
        match_policy: MatchPolicy::Exact,
        schema: Some("org.gnome.shell.keybindings"),
    },
    BindingRoot {
        dir: "/desktop/ibus/general/hotkey/",
        match_policy: MatchPolicy::Exact,
        schema: None,
    },
];

pub(crate) fn match_policy_for(dir: &str) -> MatchPolicy {
    BINDING_ROOTS
        .iter()
        .filter(|root| dir.starts_with(root.dir))
        .max_by_key(|root| root.dir.len())
        .map_or(MatchPolicy::Exact, |root| root.match_policy)
}

pub(crate) fn schema_for(dir: &str) -> Option<&'static str> {
    BINDING_ROOTS
        .iter()
        .filter(|root| dir.starts_with(root.dir))
        .max_by_key(|root| root.dir.len())
        .and_then(|root| root.schema)
}

const DEFAULT_RESTART_HINT: &str = "log out and back in";

const RESTART_HINTS: &[(&str, &str)] = &[
    ("cinnamon", "press Ctrl+Alt+Escape to restart Cinnamon"),
    ("muffin", "press Ctrl+Alt+Escape to restart Cinnamon"),
    (
        "gnome-shell",
        "press Alt+F2, type r and press Enter to restart GNOME Shell",
    ),
    (
        "mutter",
        "press Alt+F2, type r and press Enter to restart GNOME Shell",
    ),
    ("kwin_x11", "run kwin_x11 --replace"),
    ("kwin_wayland", "log out and back in"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostFailure {
    pub command: String,
    pub detail: String,
    pub tool_missing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TakeoverError {
    Unavailable(String),
    HostRejected { command: String, detail: String },
    NoLedgerDir,
    Ledger(String),
}

impl From<HostFailure> for TakeoverError {
    fn from(failure: HostFailure) -> Self {
        let HostFailure {
            command,
            detail,
            tool_missing,
        } = failure;
        if tool_missing {
            return Self::Unavailable(format!("{command}: {detail}"));
        }
        Self::HostRejected { command, detail }
    }
}

impl fmt::Display for TakeoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(reason) => {
                write!(f, "desktop keybinding store is unavailable: {reason}")
            }
            Self::HostRejected { command, detail } => write!(f, "{command} failed: {detail}"),
            Self::NoLedgerDir => {
                write!(f, "could not resolve the hotkey takeover ledger directory")
            }
            Self::Ledger(detail) => write!(f, "hotkey takeover ledger: {detail}"),
        }
    }
}

impl std::error::Error for TakeoverError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Compositor {
    pub name: String,
    pub started_at: SystemTime,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Scan {
    pub available: bool,
    pub entries: Vec<BindingEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BindingMutation {
    pub dir: String,
    pub key: String,
    pub next: String,
    pub qol_combo: String,
    pub reach: BindingReach,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RestoreSummary {
    pub restored: usize,
    pub abandoned: usize,
    pub quarantined: usize,
    pub settled: usize,
    pub failures: Vec<String>,
}

pub(crate) fn scan() -> Scan {
    assemble_scan(BINDING_ROOTS.iter().map(|root| {
        let sources = RootSources {
            dconf: Some(platform::dump(root.dir)),
            effective: root.schema.map(platform::list_schema),
        };
        (*root, sources)
    }))
}

#[derive(Clone, Debug, Default)]
struct RootSources {
    dconf: Option<Result<String, HostFailure>>,
    effective: Option<Result<String, HostFailure>>,
}

fn assemble_scan(sources: impl Iterator<Item = (BindingRoot, RootSources)>) -> Scan {
    let mut scan = Scan::default();
    let mut seen: std::collections::BTreeSet<(String, String)> = std::collections::BTreeSet::new();
    for (root, sources) in sources {
        if let Some(result) = sources.dconf {
            match result {
                Ok(dump) => {
                    scan.available = true;
                    extend_unique(
                        &mut scan.entries,
                        &mut seen,
                        dconf::parse_dump(root.dir, root.match_policy, &dump),
                    );
                }
                Err(failure) => {
                    scan.available |= !failure.tool_missing;
                    log_source_failure(&failure);
                }
            }
        }
        if let Some(result) = sources.effective {
            match result {
                Ok(output) => {
                    scan.available = true;
                    extend_unique(
                        &mut scan.entries,
                        &mut seen,
                        dconf::parse_gsettings_list(root.dir, root.match_policy, &output),
                    );
                }
                Err(failure) => {
                    scan.available |= !failure.tool_missing;
                    log_source_failure(&failure);
                }
            }
        }
    }
    scan
}

fn log_source_failure(failure: &HostFailure) {
    if failure.tool_missing {
        log::debug!(
            "hotkey takeover: {} skipped: {}",
            failure.command,
            failure.detail
        );
    } else {
        log::warn!(
            "hotkey takeover: {} failed, conflicts may be hidden: {}",
            failure.command,
            failure.detail
        );
    }
}

fn extend_unique(
    entries: &mut Vec<BindingEntry>,
    seen: &mut std::collections::BTreeSet<(String, String)>,
    incoming: Vec<BindingEntry>,
) {
    for entry in incoming {
        if seen.insert((entry.dir.clone(), entry.key.clone())) {
            entries.push(entry);
        }
    }
}

pub(crate) trait BindingStore {
    fn read(&mut self, full_key: &str) -> Result<String, TakeoverError>;
    fn read_effective(&mut self, dir: &str, key: &str) -> Result<String, TakeoverError>;
    fn write(&mut self, full_key: &str, value: &str) -> Result<(), TakeoverError>;
    fn reset(&mut self, full_key: &str) -> Result<(), TakeoverError>;
}

struct HostStore;

impl BindingStore for HostStore {
    fn read(&mut self, full_key: &str) -> Result<String, TakeoverError> {
        platform::read(full_key).map_err(TakeoverError::from)
    }

    fn read_effective(&mut self, dir: &str, key: &str) -> Result<String, TakeoverError> {
        let full_key = dconf::full_key(dir, key);
        match schema_for(dir) {
            None => self.read(&full_key),
            Some(schema) => match platform::get_schema_value(schema, key) {
                Ok(value) => Ok(value),
                Err(failure) => {
                    if failure.tool_missing {
                        Err(TakeoverError::from(failure))
                    } else {
                        log::debug!(
                            "hotkey takeover: {} failed for {full_key}, falling back to dconf: {}",
                            failure.command,
                            failure.detail
                        );
                        self.read(&full_key)
                    }
                }
            },
        }
    }

    fn write(&mut self, full_key: &str, value: &str) -> Result<(), TakeoverError> {
        platform::write(full_key, value).map_err(TakeoverError::from)
    }

    fn reset(&mut self, full_key: &str) -> Result<(), TakeoverError> {
        platform::reset(full_key).map_err(TakeoverError::from)
    }
}

pub(crate) fn read_effective_binding(dir: &str, key: &str) -> Result<String, TakeoverError> {
    HostStore.read_effective(dir, key)
}

pub(crate) fn take_over(mutation: &BindingMutation) -> Result<(), TakeoverError> {
    let root = ledger::claims_dir().ok_or(TakeoverError::NoLedgerDir)?;
    take_over_in(&root, &mut HostStore, mutation)
}

pub fn restore_all() -> RestoreSummary {
    let Some(root) = ledger::claims_dir() else {
        return RestoreSummary::default();
    };
    let compositor_started_at = platform::compositor().map(|found| found.started_at);
    restore_all_in(&root, &mut HostStore, compositor_started_at)
}

pub fn restore_on_exit() -> RestoreSummary {
    let Some(root) = ledger::claims_dir() else {
        return RestoreSummary::default();
    };
    let compositor_started_at = platform::compositor().map(|found| found.started_at);
    let resident = HostResidency::current().is_resident();
    restore_on_exit_in(&root, &mut HostStore, resident, compositor_started_at)
}

fn restore_on_exit_in(
    root: &std::path::Path,
    store: &mut dyn BindingStore,
    resident: bool,
    compositor_started_at: Option<SystemTime>,
) -> RestoreSummary {
    if resident {
        return RestoreSummary::default();
    }
    restore_all_in(root, store, compositor_started_at)
}

fn take_over_in(
    root: &std::path::Path,
    store: &mut dyn BindingStore,
    mutation: &BindingMutation,
) -> Result<(), TakeoverError> {
    let full_key = dconf::full_key(&mutation.dir, &mutation.key);
    let current = store.read(&full_key)?;
    let outstanding = ledger::outstanding(root);
    let prior = outstanding
        .iter()
        .find(|claim| claim.dir == mutation.dir && claim.key == mutation.key);
    let user_rebound = prior.is_some_and(|claim| {
        claim.previous_unset && !current.trim().is_empty() && current.trim() != claim.applied.trim()
    });
    let previous = prior.map_or_else(
        || current.clone(),
        |claim| {
            if user_rebound {
                current.clone()
            } else if claim.previous_unset {
                String::new()
            } else {
                claim.previous.clone()
            }
        },
    );
    let current_unset = if user_rebound {
        false
    } else {
        current.trim().is_empty() || prior.is_some_and(|claim| claim.previous_unset)
    };

    let custom_list = withdrawn_custom_list(store, &mutation.dir);
    let claim = ledger::Claim {
        dir: mutation.dir.clone(),
        key: mutation.key.clone(),
        previous,
        applied: mutation.next.clone(),
        qol_combo: mutation.qol_combo.clone(),
        reach: mutation.reach,
        recorded_at: SystemTime::now(),
        previous_unset: current_unset,
        custom_list: custom_list.clone(),
    };
    ledger::record(root, &claim).map_err(|error| TakeoverError::Ledger(error.to_string()))?;
    if let Err(error) = store.write(&full_key, &mutation.next) {
        let _ = ledger::clear(root, &claim);
        qol_runtime::probe!(
            "HOTKEY_TAKEOVER",
            "phase=takeover-failed key={} combo={} reason={}",
            qol_runtime::probe::token(&full_key),
            qol_runtime::probe::token(&mutation.qol_combo),
            qol_runtime::probe::token(&error.to_string())
        );
        return Err(error);
    }
    qol_runtime::probe!(
        "HOTKEY_TAKEOVER",
        "phase=takeover key={} combo={} reach={}",
        qol_runtime::probe::token(&full_key),
        qol_runtime::probe::token(&mutation.qol_combo),
        qol_runtime::probe::token(mutation.reach.label())
    );
    if let Some(list) = custom_list {
        if let Err(error) = store.write(&list.key, &list.applied) {
            log::warn!(
                "hotkey takeover: {} kept its entry in {}: {error}",
                mutation.qol_combo,
                list.key
            );
        }
    }
    Ok(())
}

fn withdrawn_custom_list(
    store: &mut dyn BindingStore,
    dir: &str,
) -> Option<ledger::CustomListClaim> {
    let entry = dconf::custom_entry(dir)?;
    let previous = store.read(&entry.list_key).ok()?;
    let names = dconf::parse_string_array(&previous)?;
    if !names.iter().any(|name| name == &entry.name) {
        return None;
    }
    let remaining: Vec<String> = names
        .into_iter()
        .filter(|name| name != &entry.name)
        .collect();
    Some(ledger::CustomListClaim {
        key: entry.list_key,
        applied: dconf::serialize_string_array(&remaining),
        previous,
    })
}

fn restore_all_in(
    root: &std::path::Path,
    store: &mut dyn BindingStore,
    compositor_started_at: Option<SystemTime>,
) -> RestoreSummary {
    let mut summary = RestoreSummary::default();
    for claim in ledger::outstanding(root) {
        let full_key = dconf::full_key(&claim.dir, &claim.key);
        let current = store.read(&full_key).ok();
        let decision = ledger::decide_restore(&claim, current.as_deref(), compositor_started_at);
        let clear_claim = match decision {
            ledger::RestoreDecision::Abandon => {
                summary.abandoned += 1;
                true
            }
            ledger::RestoreDecision::Rewrite => {
                let restore = if claim.previous_unset {
                    store.reset(&full_key)
                } else {
                    store.write(&full_key, &claim.previous)
                };
                if let Err(error) = restore {
                    summary.failures.push(format!("{full_key}: {error}"));
                    continue;
                }
                if let Some(list) = &claim.custom_list {
                    if let Err(error) = store.write(&list.key, &list.previous) {
                        summary.failures.push(format!("{}: {error}", list.key));
                    }
                }
                summary.restored += 1;
                true
            }
            ledger::RestoreDecision::Quarantine => {
                summary.quarantined += 1;
                false
            }
            ledger::RestoreDecision::Settle => {
                summary.settled += 1;
                true
            }
        };
        qol_runtime::probe!(
            "HOTKEY_TAKEOVER",
            "phase=restore decision={} key={} combo={}",
            decision.trace_label(),
            qol_runtime::probe::token(&full_key),
            qol_runtime::probe::token(&claim.qol_combo)
        );
        if !clear_claim {
            continue;
        }
        if let Err(error) = ledger::clear(root, &claim) {
            summary.failures.push(format!("{full_key}: {error}"));
        }
    }
    summary
}

pub(crate) fn restart_advice() -> Option<String> {
    let root = ledger::claims_dir()?;
    let claims = ledger::outstanding(&root);
    if ledger::restart_pending(&claims, None).is_empty() {
        return None;
    }
    let compositor = platform::compositor();
    let pending = ledger::restart_pending(&claims, compositor.as_ref().map(|c| c.started_at));
    if pending.is_empty() {
        return None;
    }
    let combos: Vec<&str> = pending
        .iter()
        .map(|claim| claim.qol_combo.as_str())
        .collect();
    Some(format_restart_advice(
        compositor.as_ref().map(|c| c.name.as_str()),
        &combos,
    ))
}

fn format_restart_advice(compositor: Option<&str>, combos: &[&str]) -> String {
    let hint = compositor.map_or(DEFAULT_RESTART_HINT, restart_hint);
    let owner = compositor.unwrap_or("the desktop");
    format!(
        "{} still holds a stale key grab for {} from an orphaned shortcut entry; \
         the entry is cleared but the running session keeps its grab until you {hint}",
        owner,
        combos.join(", ")
    )
}

fn restart_hint(name: &str) -> &'static str {
    RESTART_HINTS
        .iter()
        .find(|(known, _)| *known == name)
        .map_or(DEFAULT_RESTART_HINT, |(_, hint)| *hint)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::time::Duration;

    const ORPHAN_DIR: &str = "/org/cinnamon/desktop/keybindings/custom2/";
    const ORIGINAL: &str = "['<Shift><Super>s']";

    #[derive(Default)]
    struct FakeStore {
        values: BTreeMap<String, String>,
        write_failure: Option<String>,
    }

    impl FakeStore {
        fn with(full_key: &str, value: &str) -> Self {
            let mut values = BTreeMap::new();
            values.insert(full_key.to_string(), value.to_string());
            Self {
                values,
                write_failure: None,
            }
        }

        fn get(&self, full_key: &str) -> Option<&str> {
            self.values.get(full_key).map(String::as_str)
        }
    }

    impl BindingStore for FakeStore {
        fn read(&mut self, full_key: &str) -> Result<String, TakeoverError> {
            Ok(self.values.get(full_key).cloned().unwrap_or_default())
        }

        fn read_effective(&mut self, dir: &str, key: &str) -> Result<String, TakeoverError> {
            self.read(&dconf::full_key(dir, key))
        }

        fn write(&mut self, full_key: &str, value: &str) -> Result<(), TakeoverError> {
            if self.write_failure.as_deref() == Some(full_key) {
                return Err(TakeoverError::HostRejected {
                    command: format!("dconf write {full_key}"),
                    detail: "denied".into(),
                });
            }
            self.values.insert(full_key.to_string(), value.to_string());
            Ok(())
        }

        fn reset(&mut self, full_key: &str) -> Result<(), TakeoverError> {
            if self.write_failure.as_deref() == Some(full_key) {
                return Err(TakeoverError::HostRejected {
                    command: format!("dconf reset {full_key}"),
                    detail: "denied".into(),
                });
            }
            self.values.remove(full_key);
            Ok(())
        }
    }

    fn orphan_mutation() -> BindingMutation {
        BindingMutation {
            dir: ORPHAN_DIR.into(),
            key: "binding".into(),
            next: "@as []".into(),
            qol_combo: "Shift+Super+S".into(),
            reach: BindingReach::LegacyOrphan,
        }
    }

    fn managed_mutation() -> BindingMutation {
        BindingMutation {
            reach: BindingReach::Managed,
            ..orphan_mutation()
        }
    }

    fn full_key() -> String {
        dconf::full_key(ORPHAN_DIR, "binding")
    }

    #[test]
    fn portable_exit_restores_bindings_and_clears_the_ledger() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);
        take_over_in(root.path(), &mut store, &managed_mutation()).expect("take over");
        assert_eq!(store.get(&full_key()), Some("@as []"));

        let summary = restore_on_exit_in(root.path(), &mut store, false, None);
        assert_eq!(summary.restored, 1);
        assert_eq!(
            store.get(&full_key()),
            Some(ORIGINAL),
            "a portable exit hands the desktop bindings back"
        );
        assert!(
            ledger::outstanding(root.path()).is_empty(),
            "the ledger entry is retired after a portable shutdown"
        );
    }

    #[test]
    fn resident_exit_keeps_the_ledger_and_never_restores_bindings() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);
        take_over_in(root.path(), &mut store, &managed_mutation()).expect("take over");
        assert_eq!(store.get(&full_key()), Some("@as []"));

        let summary = restore_on_exit_in(root.path(), &mut store, true, None);
        assert_eq!(summary, RestoreSummary::default());
        assert_eq!(
            store.get(&full_key()),
            Some("@as []"),
            "a resident host keeps the user's bindings"
        );
        assert_eq!(
            ledger::outstanding(root.path()).len(),
            1,
            "the ledger entry survives a resident shutdown so disabling residency can restore it"
        );
    }

    #[test]
    fn managed_take_over_then_restore_leaves_the_host_exactly_as_found() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);

        take_over_in(root.path(), &mut store, &managed_mutation()).expect("take over");
        assert_eq!(store.get(&full_key()), Some("@as []"));

        let summary = restore_all_in(root.path(), &mut store, None);
        assert_eq!(summary.restored, 1);
        assert_eq!(summary.abandoned, 0);
        assert!(summary.failures.is_empty(), "{:?}", summary.failures);
        assert_eq!(
            store.get(&full_key()),
            Some(ORIGINAL),
            "the desktop shortcut must come back byte-identical"
        );
        assert_eq!(
            restore_all_in(root.path(), &mut store, None),
            RestoreSummary::default(),
            "a second restore must be a no-op, not a second rewrite"
        );
    }

    const CUSTOM_DIR: &str = "/org/cinnamon/desktop/keybindings/custom-keybindings/custom9/";
    const CUSTOM_LIST: &str = "/org/cinnamon/desktop/keybindings/custom-list";

    fn custom_mutation() -> BindingMutation {
        BindingMutation {
            dir: CUSTOM_DIR.into(),
            ..managed_mutation()
        }
    }

    fn custom_store() -> FakeStore {
        let mut store = FakeStore::with(&dconf::full_key(CUSTOM_DIR, "binding"), ORIGINAL);
        store
            .values
            .insert(CUSTOM_LIST.to_string(), "['custom4', 'custom9']".into());
        store
    }

    #[test]
    fn a_custom_shortcut_is_withdrawn_from_the_list_so_the_desktop_re_reads_it_and_drops_the_grab()
    {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = custom_store();

        take_over_in(root.path(), &mut store, &custom_mutation()).expect("take over");

        assert_eq!(
            store.get(CUSTOM_LIST),
            Some("['custom4']"),
            "emptying the binding alone leaves the entry in the list the desktop iterates, and it keeps the grab"
        );
    }

    #[test]
    fn restoring_a_custom_shortcut_puts_its_entry_back_in_the_list() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = custom_store();

        take_over_in(root.path(), &mut store, &custom_mutation()).expect("take over");
        let summary = restore_all_in(root.path(), &mut store, None);

        assert_eq!(summary.restored, 1);
        assert!(summary.failures.is_empty(), "{:?}", summary.failures);
        assert_eq!(store.get(CUSTOM_LIST), Some("['custom4', 'custom9']"));
        assert_eq!(
            store.get(&dconf::full_key(CUSTOM_DIR, "binding")),
            Some(ORIGINAL)
        );
    }

    #[test]
    fn a_binding_outside_the_custom_keybindings_tree_never_touches_a_list() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);

        take_over_in(root.path(), &mut store, &managed_mutation()).expect("take over");

        assert_eq!(store.get(CUSTOM_LIST), None);
    }

    #[test]
    fn orphan_takeover_stays_cleared_and_claimed_across_restore() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);

        take_over_in(root.path(), &mut store, &orphan_mutation()).expect("take over");
        let summary = restore_all_in(root.path(), &mut store, None);

        assert_eq!(summary.restored, 0);
        assert_eq!(summary.abandoned, 0);
        assert_eq!(summary.quarantined, 1);
        assert_eq!(summary.settled, 0);
        assert!(summary.failures.is_empty(), "{:?}", summary.failures);
        assert_eq!(store.get(&full_key()), Some("@as []"));
        let claims = ledger::outstanding(root.path());
        assert_eq!(claims.len(), 1);

        let summary = restore_all_in(
            root.path(),
            &mut store,
            Some(claims[0].recorded_at + Duration::from_secs(1)),
        );
        assert_eq!(summary.settled, 1);
        assert_eq!(store.get(&full_key()), Some("@as []"));
        assert!(ledger::outstanding(root.path()).is_empty());
    }

    #[test]
    fn a_second_take_over_of_the_same_key_keeps_the_original_value_to_restore() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);

        take_over_in(root.path(), &mut store, &managed_mutation()).expect("first take over");
        let mut second = managed_mutation();
        second.next = "['XF86Keyboard']".into();
        take_over_in(root.path(), &mut store, &second).expect("second take over");

        restore_all_in(root.path(), &mut store, None);
        assert_eq!(
            store.get(&full_key()),
            Some(ORIGINAL),
            "re-applying must not record the already-cleared value as the original"
        );
    }

    #[test]
    fn a_failed_write_records_no_claim_so_restore_never_invents_a_binding() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);
        store.write_failure = Some(full_key());

        let error =
            take_over_in(root.path(), &mut store, &orphan_mutation()).expect_err("write denied");
        assert!(error.to_string().contains("denied"), "{error}");
        assert_eq!(store.get(&full_key()), Some(ORIGINAL));
        assert_eq!(
            restore_all_in(root.path(), &mut store, None),
            RestoreSummary::default()
        );
    }

    #[test]
    fn a_binding_the_user_rebound_while_qol_ran_is_left_to_the_user() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), ORIGINAL);
        take_over_in(root.path(), &mut store, &orphan_mutation()).expect("take over");
        store
            .write(&full_key(), "['<Super>F9']")
            .expect("user rebinds");

        let summary = restore_all_in(root.path(), &mut store, None);
        assert_eq!(summary.abandoned, 1);
        assert_eq!(summary.restored, 0);
        assert_eq!(
            store.get(&full_key()),
            Some("['<Super>F9']"),
            "qol must never clobber a choice the user made after the takeover"
        );
    }

    #[test]
    fn a_scan_is_unavailable_only_when_every_source_reports_a_missing_tool() {
        type Source = (BindingRoot, RootSources);
        let root = |dir: &'static str| BindingRoot {
            dir,
            match_policy: MatchPolicy::Exact,
            schema: None,
        };
        let missing = |dir: &'static str| {
            (
                root(dir),
                RootSources {
                    dconf: Some(Err(HostFailure {
                        command: format!("dconf dump {dir}"),
                        detail: "not found".into(),
                        tool_missing: true,
                    })),
                    effective: None,
                },
            )
        };
        let refused = |dir: &'static str| {
            (
                root(dir),
                RootSources {
                    dconf: Some(Err(HostFailure {
                        command: format!("dconf dump {dir}"),
                        detail: "denied".into(),
                        tool_missing: false,
                    })),
                    effective: None,
                },
            )
        };
        let dumped = |dir: &'static str| {
            (
                root(dir),
                RootSources {
                    dconf: Some(Ok("[wm]\nclose=['<Super>w']\n".to_string())),
                    effective: None,
                },
            )
        };

        type Case = (&'static str, Vec<Source>, bool, usize);
        let cases: [Case; 4] = [
            (
                "no dconf at all",
                vec![missing("/a/"), missing("/b/")],
                false,
                0,
            ),
            (
                "one root dumped",
                vec![missing("/a/"), dumped("/b/")],
                true,
                1,
            ),
            ("read refused", vec![refused("/a/")], true, 0),
            (
                "every root dumped",
                vec![dumped("/a/"), dumped("/b/")],
                true,
                2,
            ),
        ];
        for (label, sources, available, entries) in cases {
            let scan = assemble_scan(sources.into_iter());
            assert_eq!(scan.available, available, "case: {label}");
            assert_eq!(scan.entries.len(), entries, "case: {label}");
        }
    }

    #[test]
    fn a_schema_default_binding_is_scanned_without_being_set_in_dconf() {
        let root = BindingRoot {
            dir: "/org/cinnamon/desktop/keybindings/",
            match_policy: MatchPolicy::Subset,
            schema: Some("org.cinnamon.desktop.keybindings"),
        };
        let sources = RootSources {
            dconf: Some(Ok("[/]\n".to_string())),
            effective: Some(Ok(
                "org.cinnamon.desktop.keybindings show-desklets ['<Super>s']\norg.cinnamon.desktop.keybindings custom-list ['custom1']\n"
                    .to_string(),
            )),
        };
        let scan = assemble_scan(std::iter::once((root, sources)));
        assert_eq!(scan.entries.len(), 1, "custom-list is not a binding");
        assert_eq!(scan.entries[0].key, "show-desklets");
        assert_eq!(scan.entries[0].values, vec!["<Super>s".to_string()]);
        assert_eq!(scan.entries[0].match_policy, MatchPolicy::Subset);
    }

    #[test]
    fn an_effective_default_never_duplicates_an_explicitly_set_value() {
        let root = BindingRoot {
            dir: "/org/cinnamon/desktop/keybindings/",
            match_policy: MatchPolicy::Subset,
            schema: Some("org.cinnamon.desktop.keybindings"),
        };
        let sources = RootSources {
            dconf: Some(Ok("[/]\nshow-desklets=['<Super>s']\n".to_string())),
            effective: Some(Ok(
                "org.cinnamon.desktop.keybindings show-desklets ['<Super>s']\n".to_string(),
            )),
        };
        let scan = assemble_scan(std::iter::once((root, sources)));
        assert_eq!(scan.entries.len(), 1, "set and effective must dedupe");
    }

    #[test]
    fn taking_over_an_unset_key_restores_it_by_resetting_the_key() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::default();

        take_over_in(root.path(), &mut store, &managed_mutation()).expect("take over");
        assert_eq!(store.get(&full_key()), Some("@as []"));

        let summary = restore_all_in(root.path(), &mut store, None);
        assert_eq!(summary.restored, 1);
        assert!(summary.failures.is_empty(), "{:?}", summary.failures);
        assert_eq!(
            store.get(&full_key()),
            None,
            "an unset key must be reset, not written an empty value"
        );
        assert_eq!(
            restore_all_in(root.path(), &mut store, None),
            RestoreSummary::default()
        );
    }

    #[test]
    fn a_second_takeover_of_an_unset_key_keeps_reset_semantics() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::default();

        take_over_in(root.path(), &mut store, &managed_mutation()).expect("first take over");
        let mut second = managed_mutation();
        second.next = "@as []".into();
        take_over_in(root.path(), &mut store, &second).expect("second take over");

        restore_all_in(root.path(), &mut store, None);
        assert_eq!(
            store.get(&full_key()),
            None,
            "re-applying must not turn a reset into a written empty value"
        );
    }

    #[test]
    fn a_missing_tool_and_a_refused_write_map_to_different_error_classes() {
        let cases = [
            (
                HostFailure {
                    command: "dconf read /a/b".into(),
                    detail: "No such file".into(),
                    tool_missing: true,
                },
                TakeoverError::Unavailable("dconf read /a/b: No such file".into()),
            ),
            (
                HostFailure {
                    command: "dconf write /a/b".into(),
                    detail: "permission denied".into(),
                    tool_missing: false,
                },
                TakeoverError::HostRejected {
                    command: "dconf write /a/b".into(),
                    detail: "permission denied".into(),
                },
            ),
        ];
        for (failure, expected) in cases {
            assert_eq!(
                TakeoverError::from(failure.clone()),
                expected,
                "failure: {failure:?}"
            );
        }
    }

    #[test]
    fn binding_roots_cover_cinnamon_gnome_and_ibus_without_duplicates() {
        let mut sorted: Vec<&str> = BINDING_ROOTS.iter().map(|root| root.dir).collect();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), BINDING_ROOTS.len(), "duplicate binding root");
        for root in BINDING_ROOTS {
            assert!(
                root.dir.starts_with('/'),
                "root must be absolute: {}",
                root.dir
            );
            assert!(
                root.dir.ends_with('/'),
                "root must be a dconf dir: {}",
                root.dir
            );
        }
        assert!(BINDING_ROOTS
            .iter()
            .any(|root| root.dir == "/org/cinnamon/desktop/keybindings/"
                && root.match_policy == MatchPolicy::Subset));
        assert!(BINDING_ROOTS
            .iter()
            .any(|root| root.dir == "/desktop/ibus/general/hotkey/"
                && root.match_policy == MatchPolicy::Exact));
    }

    #[test]
    fn schema_and_policy_for_sub_roots_resolve_to_the_longest_matching_root() {
        assert_eq!(
            schema_for("/org/cinnamon/desktop/keybindings/show-desklets"),
            Some("org.cinnamon.desktop.keybindings")
        );
        assert_eq!(
            schema_for("/org/cinnamon/desktop/keybindings/wm/close"),
            Some("org.cinnamon.desktop.keybindings.wm")
        );
        assert_eq!(
            schema_for("/org/cinnamon/desktop/keybindings/media-keys/screenshot"),
            Some("org.cinnamon.desktop.keybindings.media-keys")
        );
        assert_eq!(
            schema_for("/org/gnome/desktop/wm/keybindings/close"),
            Some("org.gnome.desktop.wm.keybindings")
        );
        assert_eq!(schema_for("/desktop/ibus/general/hotkey/triggers"), None);
        assert_eq!(schema_for("/org/unknown/root/key"), None);
        assert_eq!(
            match_policy_for("/org/cinnamon/desktop/keybindings/wm/close"),
            MatchPolicy::Subset
        );
        assert_eq!(
            match_policy_for("/org/cinnamon/muffin/keybindings/toggle-maximized"),
            MatchPolicy::Exact
        );
    }

    #[test]
    fn re_takeover_after_unset_preserves_a_user_rebound_value() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::default();

        take_over_in(root.path(), &mut store, &managed_mutation()).expect("first take over");
        assert_eq!(store.get(&full_key()), Some("@as []"));
        assert!(ledger::outstanding(root.path())[0].previous_unset);

        store.values.insert(full_key(), "['<Super>x']".into());
        take_over_in(root.path(), &mut store, &managed_mutation()).expect("re take over");
        let claims = ledger::outstanding(root.path());
        assert_eq!(claims.len(), 1);
        assert!(!claims[0].previous_unset);
        assert_eq!(claims[0].previous, "['<Super>x']");
        assert_eq!(store.get(&full_key()), Some("@as []"));

        let summary = restore_all_in(root.path(), &mut store, None);
        assert_eq!(summary.restored, 1);
        assert!(summary.failures.is_empty(), "{:?}", summary.failures);
        assert_eq!(store.get(&full_key()), Some("['<Super>x']"));
    }

    #[test]
    fn restart_hint_is_desktop_specific_with_a_safe_default() {
        let cases = [
            ("cinnamon", "press Ctrl+Alt+Escape to restart Cinnamon"),
            ("muffin", "press Ctrl+Alt+Escape to restart Cinnamon"),
            (
                "gnome-shell",
                "press Alt+F2, type r and press Enter to restart GNOME Shell",
            ),
            ("kwin_x11", "run kwin_x11 --replace"),
            ("kwin_wayland", "log out and back in"),
            ("some-unknown-wm", "log out and back in"),
            ("", "log out and back in"),
        ];
        for (name, want) in cases {
            assert_eq!(restart_hint(name), want, "compositor: {name}");
        }
    }

    #[test]
    fn restart_advice_names_the_owner_the_combo_and_the_single_keystroke() {
        let advice = format_restart_advice(Some("cinnamon"), &["Shift+Super+S"]);
        assert!(advice.contains("cinnamon"), "{advice}");
        assert!(advice.contains("Shift+Super+S"), "{advice}");
        assert!(advice.contains("Ctrl+Alt+Escape"), "{advice}");

        let unknown = format_restart_advice(None, &["Super+Space", "Alt+Tab"]);
        assert!(unknown.contains("the desktop"), "{unknown}");
        assert!(unknown.contains("Super+Space, Alt+Tab"), "{unknown}");
        assert!(unknown.contains("log out and back in"), "{unknown}");
    }
}
