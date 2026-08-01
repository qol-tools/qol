#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::super::{DetectedShadow, ShadowKind};
use crate::doctor::de_bindings::normalize_combo;
use crate::doctor::diagnosis::FixAction;
use crate::hotkeys::takeover::{self, BindingEntry, BindingReach};
use std::collections::BTreeMap;

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
        .flat_map(|entry| {
            entry
                .values
                .iter()
                .filter_map(|value| normalize_combo(value))
                .filter_map(|norm| qol_index.get(&norm))
                .map(|qol_combo| shadow(entry, qol_combo))
        })
        .collect()
}

fn shadow(entry: &BindingEntry, qol_combo: &str) -> DetectedShadow {
    DetectedShadow {
        qol_combo: qol_combo.to_string(),
        source_label: format!("{}{} ({})", entry.dir, entry.key, entry.reach.label()),
        kind: ShadowKind::Fixable(FixAction::UnshadowDeBinding {
            dir: entry.dir.clone(),
            key: entry.key.clone(),
            qol_combo: qol_combo.to_string(),
            orphaned: entry.reach == BindingReach::LegacyOrphan,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::takeover::dconf;

    const CINNAMON: &str = "/org/cinnamon/desktop/keybindings/";

    fn index(combos: &[&str]) -> BTreeMap<String, String> {
        combos
            .iter()
            .filter_map(|c| normalize_combo(c).map(|n| (n, (*c).to_string())))
            .collect()
    }

    fn detect(qol: &[&str], root: &str, dump: &str) -> Vec<DetectedShadow> {
        shadows_from_entries(&index(qol), &dconf::parse_dump(root, dump))
    }

    #[test]
    fn an_orphaned_root_level_custom_entry_is_detected_and_flagged_as_orphaned() {
        let dump =
            "[custom2]\nbinding=['<Shift><Super>s']\ncommand='flameshot gui'\nname='Screenshot'\n";
        let shadows = detect(&["Shift+Super+S"], CINNAMON, dump);
        assert_eq!(shadows.len(), 1, "got: {shadows:?}");
        assert_eq!(shadows[0].qol_combo, "Shift+Super+S");
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
        let shadows = detect(&["Super+Space"], CINNAMON, dump);
        assert_eq!(
            shadows.len(),
            1,
            "custom-list membership must not gate detection: {shadows:?}"
        );
        assert!(shadows[0].source_label.contains("custom0/binding"));
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
            let shadows = detect(&[combo], CINNAMON, dump);
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
            "[/]\ntriggers=['<Super>space']\n",
        );
        assert_eq!(shadows.len(), 1);
        assert!(shadows[0]
            .source_label
            .starts_with("/desktop/ibus/general/hotkey/triggers"));
    }

    #[test]
    fn one_combo_bound_in_several_places_yields_one_shadow_per_binding() {
        let dump =
            "[custom0]\nbinding=['<Super>space']\n\n[wm]\npanel-main-menu=['<Super>space']\n";
        let shadows = detect(&["Super+Space"], CINNAMON, dump);
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
                detect(&["Super+Space", "custom1"], CINNAMON, dump).is_empty(),
                "dump must not produce shadows: {dump}"
            );
        }
    }

    #[test]
    fn bindings_that_do_not_overlap_are_left_alone() {
        let dump = "[wm]\nclose=['<Super>w']\nshow-desktop=['<Super>d']\n";
        assert!(detect(&["Ctrl+Alt+Shift+F12"], CINNAMON, dump).is_empty());
    }
}
