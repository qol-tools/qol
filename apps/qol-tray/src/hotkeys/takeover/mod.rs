pub(crate) mod dconf;
mod ledger;
mod platform;

pub(crate) use dconf::{BindingEntry, BindingReach};

use std::fmt;
use std::time::SystemTime;

const BINDING_ROOTS: &[&str] = &[
    "/org/cinnamon/desktop/keybindings/",
    "/org/cinnamon/muffin/keybindings/",
    "/org/gnome/desktop/wm/keybindings/",
    "/org/gnome/settings-daemon/plugins/media-keys/",
    "/org/gnome/shell/keybindings/",
    "/desktop/ibus/general/hotkey/",
];

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
    assemble_scan(
        BINDING_ROOTS
            .iter()
            .map(|root| (*root, platform::dump(root))),
    )
}

fn assemble_scan<'a>(dumps: impl Iterator<Item = (&'a str, Result<String, HostFailure>)>) -> Scan {
    let mut scan = Scan::default();
    for (root, result) in dumps {
        match result {
            Ok(dump) => {
                scan.available = true;
                scan.entries.extend(dconf::parse_dump(root, &dump));
            }
            Err(failure) => {
                scan.available |= !failure.tool_missing;
                log::debug!(
                    "hotkey takeover: {} skipped: {}",
                    failure.command,
                    failure.detail
                );
            }
        }
    }
    scan
}

pub(crate) trait BindingStore {
    fn read(&mut self, full_key: &str) -> Result<String, TakeoverError>;
    fn write(&mut self, full_key: &str, value: &str) -> Result<(), TakeoverError>;
}

struct HostStore;

impl BindingStore for HostStore {
    fn read(&mut self, full_key: &str) -> Result<String, TakeoverError> {
        platform::read(full_key).map_err(TakeoverError::from)
    }

    fn write(&mut self, full_key: &str, value: &str) -> Result<(), TakeoverError> {
        platform::write(full_key, value).map_err(TakeoverError::from)
    }
}

pub(crate) fn read_binding(dir: &str, key: &str) -> Result<String, TakeoverError> {
    HostStore.read(&dconf::full_key(dir, key))
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

fn take_over_in(
    root: &std::path::Path,
    store: &mut dyn BindingStore,
    mutation: &BindingMutation,
) -> Result<(), TakeoverError> {
    let full_key = dconf::full_key(&mutation.dir, &mutation.key);
    let current = store.read(&full_key)?;
    if current.trim().is_empty() {
        return Err(TakeoverError::HostRejected {
            command: format!("dconf read {full_key}"),
            detail: "key is unset, so there is nothing to take back".into(),
        });
    }
    let previous = ledger::outstanding(root)
        .into_iter()
        .find(|claim| claim.dir == mutation.dir && claim.key == mutation.key)
        .map_or(current, |claim| claim.previous);

    let custom_list = withdrawn_custom_list(store, &mutation.dir);
    let claim = ledger::Claim {
        dir: mutation.dir.clone(),
        key: mutation.key.clone(),
        previous,
        applied: mutation.next.clone(),
        qol_combo: mutation.qol_combo.clone(),
        reach: mutation.reach,
        recorded_at: SystemTime::now(),
        custom_list: custom_list.clone(),
    };
    ledger::record(root, &claim).map_err(|error| TakeoverError::Ledger(error.to_string()))?;
    if let Err(error) = store.write(&full_key, &mutation.next) {
        let _ = ledger::clear(root, &claim);
        return Err(error);
    }
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
                if let Err(error) = store.write(&full_key, &claim.previous) {
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
            self.values
                .get(full_key)
                .cloned()
                .ok_or_else(|| TakeoverError::HostRejected {
                    command: format!("dconf read {full_key}"),
                    detail: "unset".into(),
                })
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
    fn a_scan_is_unavailable_only_when_every_root_reports_a_missing_tool() {
        type Dump = (&'static str, Result<String, HostFailure>);
        let missing = |root: &'static str| {
            (
                root,
                Err(HostFailure {
                    command: format!("dconf dump {root}"),
                    detail: "not found".into(),
                    tool_missing: true,
                }),
            )
        };
        let refused = |root: &'static str| {
            (
                root,
                Err(HostFailure {
                    command: format!("dconf dump {root}"),
                    detail: "denied".into(),
                    tool_missing: false,
                }),
            )
        };
        let dumped = |root: &'static str| (root, Ok("[wm]\nclose=['<Super>w']\n".to_string()));

        type Case = (&'static str, Vec<Dump>, bool, usize);
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
        for (label, dumps, available, entries) in cases {
            let scan = assemble_scan(dumps.into_iter());
            assert_eq!(scan.available, available, "case: {label}");
            assert_eq!(scan.entries.len(), entries, "case: {label}");
        }
    }

    #[test]
    fn an_unset_key_is_never_claimed_so_restore_cannot_write_an_empty_gvariant() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut store = FakeStore::with(&full_key(), "");

        let error = take_over_in(root.path(), &mut store, &orphan_mutation())
            .expect_err("an unset key has nothing to take back");
        assert!(
            error.to_string().contains("nothing to take back"),
            "{error}"
        );
        assert_eq!(
            restore_all_in(root.path(), &mut store, None),
            RestoreSummary::default()
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
        let mut sorted = BINDING_ROOTS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), BINDING_ROOTS.len(), "duplicate binding root");
        for root in BINDING_ROOTS {
            assert!(root.starts_with('/'), "root must be absolute: {root}");
            assert!(root.ends_with('/'), "root must be a dconf dir: {root}");
        }
        assert!(BINDING_ROOTS.contains(&"/org/cinnamon/desktop/keybindings/"));
        assert!(BINDING_ROOTS.contains(&"/desktop/ibus/general/hotkey/"));
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
