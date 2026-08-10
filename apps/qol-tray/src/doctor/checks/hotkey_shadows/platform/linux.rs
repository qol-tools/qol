#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::super::{DetectedShadow, ShadowKind};
use crate::doctor::de_bindings::{matches_shadow, normalize_combo};
use crate::doctor::diagnosis::FixAction;
use crate::hotkeys::takeover::{self, BindingEntry, BindingReach};
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::doctor::checks::hotkey_shadows) fn collect_shadows(
    qol_index: &BTreeMap<String, String>,
) -> Vec<DetectedShadow> {
    let scan = takeover::scan();
    if !scan.available {
        return Vec::new();
    }
    shadows_from_entries(qol_index, &scan.entries)
}

pub(crate) fn shadows_from_entries(
    qol_index: &BTreeMap<String, String>,
    entries: &[BindingEntry],
) -> Vec<DetectedShadow> {
    entries
        .iter()
        .filter_map(|entry| {
            let combos: BTreeSet<String> = entry
                .values
                .iter()
                .filter_map(|value| normalize_combo(value))
                .flat_map(|norm| {
                    qol_index
                        .iter()
                        .filter(move |(qol_norm, _)| {
                            matches_shadow(&norm, qol_norm, entry.match_policy)
                        })
                        .map(|(_, qol_combo)| qol_combo.clone())
                })
                .collect();
            if combos.is_empty() {
                return None;
            }
            Some(shadow(entry, combos.into_iter().collect()))
        })
        .collect()
}

fn shadow(entry: &BindingEntry, qol_combos: Vec<String>) -> DetectedShadow {
    let fix = FixAction::UnshadowDeBinding {
        dir: entry.dir.clone(),
        key: entry.key.clone(),
        qol_combos: qol_combos.clone(),
        orphaned: entry.reach == BindingReach::LegacyOrphan,
    };
    DetectedShadow {
        qol_combos,
        source_label: format!("{}{} ({})", entry.dir, entry.key, entry.reach.label()),
        kind: ShadowKind::Fixable(fix),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::takeover::{dconf, MatchPolicy};

    const CINNAMON: &str = "/org/cinnamon/desktop/keybindings/";

    fn index(combos: &[&str]) -> BTreeMap<String, String> {
        combos
            .iter()
            .filter_map(|c| normalize_combo(c).map(|n| (n, (*c).to_string())))
            .collect()
    }

    fn detect(qol: &[&str], root: &str, policy: MatchPolicy, dump: &str) -> Vec<DetectedShadow> {
        shadows_from_entries(&index(qol), &dconf::parse_dump(root, policy, dump))
    }

    fn detect_cinnamon(qol: &[&str], dump: &str) -> Vec<DetectedShadow> {
        detect(qol, CINNAMON, MatchPolicy::Subset, dump)
    }

    #[test]
    fn an_orphaned_root_level_custom_entry_is_detected_and_flagged_as_orphaned() {
        let dump =
            "[custom2]\nbinding=['<Shift><Super>s']\ncommand='flameshot gui'\nname='Screenshot'\n";
        let shadows = detect_cinnamon(&["Shift+Super+S"], dump);
        assert_eq!(shadows.len(), 1, "got: {shadows:?}");
        assert_eq!(shadows[0].qol_combos, vec!["Shift+Super+S"]);
        assert_eq!(
            shadows[0].source_label,
            "/org/cinnamon/desktop/keybindings/custom2/binding (orphaned legacy shortcut)"
        );
        assert!(
            matches!(
                &shadows[0].kind,
                ShadowKind::Fixable(FixAction::UnshadowDeBinding { dir, key, orphaned, .. })
                    if dir == "/org/cinnamon/desktop/keybindings/custom2/"
                        && key == "binding"
                        && *orphaned
            ),
            "orphaned entries must carry the flag that drives the restart advice: {:?}",
            shadows[0].kind
        );
    }

    #[test]
    fn a_custom_entry_missing_from_custom_list_is_still_detected() {
        let dump = "[/]\ncustom-list=['custom1']\n\n[custom0]\nbinding=['<Super>space']\ncommand='ulauncher'\n\n[custom-keybindings/custom1]\nbinding=['<Super>t']\n";
        let shadows = detect_cinnamon(&["Super+Space"], dump);
        assert_eq!(
            shadows.len(),
            1,
            "custom-list membership must not gate detection: {shadows:?}"
        );
        assert!(shadows[0].source_label.contains("custom0/binding"));
    }

    #[test]
    fn a_host_binding_with_fewer_modifiers_on_the_same_key_shadows_the_qol_combo() {
        let dump = "[wm]\nshow-desklets=['<Super>s']\n";
        let shadows = detect_cinnamon(&["Shift+Super+S"], dump);
        assert_eq!(shadows.len(), 1, "got: {shadows:?}");
        assert_eq!(shadows[0].qol_combos, vec!["Shift+Super+S"]);
        assert!(shadows[0].source_label.contains("wm/show-desklets"));
        assert!(
            matches!(
                &shadows[0].kind,
                ShadowKind::Fixable(FixAction::UnshadowDeBinding {
                    orphaned: false,
                    ..
                })
            ),
            "show-desklets is a managed schema binding: {:?}",
            shadows[0].kind
        );
    }

    #[test]
    fn one_host_binding_shadowing_several_qol_combos_merges_into_a_single_shadow() {
        let dump = "[wm]\nshow-desklets=['<Super>s']\n";
        let shadows = detect_cinnamon(&["Super+S", "Shift+Super+S", "Ctrl+Super+S"], dump);
        assert_eq!(shadows.len(), 1, "got: {shadows:?}");
        assert_eq!(
            shadows[0].qol_combos,
            vec!["Ctrl+Super+S", "Shift+Super+S", "Super+S"],
            "one binding produces one merged fix, never one fix per combo"
        );
    }

    #[test]
    fn a_multi_value_binding_array_merges_all_shadowing_values_into_one_shadow() {
        let dump = "[custom0]\nbinding=['<Super>s', '<Shift><Super>s']\n";
        let shadows = detect_cinnamon(&["Shift+Super+S"], dump);
        assert_eq!(shadows.len(), 1, "got: {shadows:?}");
        assert_eq!(shadows[0].qol_combos, vec!["Shift+Super+S"]);
        assert!(matches!(
            &shadows[0].kind,
            ShadowKind::Fixable(FixAction::UnshadowDeBinding { orphaned: true, .. })
        ));
    }

    #[test]
    fn duplicate_values_in_a_binding_array_produce_one_shadow() {
        let dump = "[wm]\nshow-desklets=['<Super>s', '<Super>s']\n";
        let shadows = detect_cinnamon(&["Shift+Super+S"], dump);
        assert_eq!(shadows.len(), 1, "got: {shadows:?}");
        assert_eq!(shadows[0].qol_combos, vec!["Shift+Super+S"]);
    }

    #[test]
    fn subset_matching_is_gated_per_root_by_match_policy() {
        let dump = "[wm]\nshow-desklets=['<Super>s']\n";
        let exact = detect(&["Shift+Super+S"], CINNAMON, MatchPolicy::Exact, dump);
        assert!(
            exact.is_empty(),
            "an exact-mask root must not flag Super+s for Shift+Super+S: {exact:?}"
        );
        let subset = detect(&["Shift+Super+S"], CINNAMON, MatchPolicy::Subset, dump);
        assert_eq!(subset.len(), 1, "got: {subset:?}");
    }

    #[test]
    fn a_host_binding_with_more_modifiers_never_shadows_the_qol_combo() {
        let dump = "[wm]\nclose=['<Shift><Super>s']\n";
        assert!(detect_cinnamon(&["Super+S"], dump).is_empty());
    }

    #[test]
    fn managed_wm_and_media_key_bindings_are_detected_without_the_orphan_flag() {
        let cases = [
            ("[wm]\nclose=['<Super>w']\n", "Super+W", "wm/close"),
            (
                "[media-keys]\nscreensaver=['<Super>l']\n",
                "Super+L",
                "media-keys/screensaver",
            ),
        ];
        for (dump, combo, label) in cases {
            let shadows = detect_cinnamon(&[combo], dump);
            assert_eq!(shadows.len(), 1, "combo {combo}: {shadows:?}");
            assert!(
                shadows[0].source_label.contains(label),
                "combo {combo} label: {}",
                shadows[0].source_label
            );
            assert!(
                matches!(
                    &shadows[0].kind,
                    ShadowKind::Fixable(FixAction::UnshadowDeBinding { orphaned, .. })
                        if !*orphaned
                ),
                "schema-backed bindings are re-read live and must not ask for a restart"
            );
        }
    }

    #[test]
    fn ibus_triggers_are_detected_under_the_ibus_root() {
        let shadows = detect(
            &["Super+Space"],
            "/desktop/ibus/general/hotkey/",
            MatchPolicy::Exact,
            "[/]\ntriggers=['<Super>space']\n",
        );
        assert_eq!(shadows.len(), 1);
        assert!(shadows[0]
            .source_label
            .starts_with("/desktop/ibus/general/hotkey/triggers"));
    }

    #[test]
    fn ibus_triggers_never_match_fewer_modifiers_under_the_exact_policy() {
        let shadows = detect(
            &["Shift+Super+S"],
            "/desktop/ibus/general/hotkey/",
            MatchPolicy::Exact,
            "[/]\ntriggers=['<Super>s']\n",
        );
        assert!(shadows.is_empty(), "got: {shadows:?}");
    }

    #[test]
    fn one_combo_bound_in_several_places_yields_one_shadow_per_binding() {
        let dump =
            "[custom0]\nbinding=['<Super>space']\n\n[wm]\npanel-main-menu=['<Super>space']\n";
        let shadows = detect_cinnamon(&["Super+Space"], dump);
        assert_eq!(shadows.len(), 2, "got: {shadows:?}");
    }

    #[test]
    fn non_binding_values_never_produce_a_shadow() {
        let cases = [
            "[/]\ncustom-list=['custom1']\n",
            "[custom0]\ncommand='Super+Space'\nname='Super+Space'\n",
            "[wm]\nclose=@as []\n",
            "",
        ];
        for dump in cases {
            assert!(
                detect_cinnamon(&["Super+Space", "custom1"], dump).is_empty(),
                "dump must not produce shadows: {dump}"
            );
        }
    }

    #[test]
    fn bindings_that_do_not_overlap_are_left_alone() {
        let dump = "[wm]\nclose=['<Super>w']\nshow-desktop=['<Super>d']\n";
        assert!(detect_cinnamon(&["Ctrl+Alt+Shift+F12"], dump).is_empty());
    }
}
