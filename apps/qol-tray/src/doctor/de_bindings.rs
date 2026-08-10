use crate::hotkeys::takeover::MatchPolicy;
use std::collections::BTreeSet;

const MODIFIERS: &[&str] = &["ctrl", "shift", "alt", "super"];

pub(super) fn filter_shadowed(
    entries: &[String],
    qol_combos: &[String],
    policy: MatchPolicy,
) -> Option<Vec<String>> {
    let targets: Vec<String> = qol_combos
        .iter()
        .filter_map(|combo| normalize_combo(combo))
        .collect();
    if targets.is_empty() {
        return None;
    }
    Some(
        entries
            .iter()
            .filter(|entry| {
                normalize_combo(entry).is_none_or(|norm| {
                    !targets
                        .iter()
                        .any(|target| matches_shadow(&norm, target, policy))
                })
            })
            .cloned()
            .collect(),
    )
}

pub(super) fn matches_shadow(host: &str, qol: &str, policy: MatchPolicy) -> bool {
    let (host_mods, host_key) = split_combo(host);
    let (qol_mods, qol_key) = split_combo(qol);
    if host_key != qol_key {
        return false;
    }
    match policy {
        MatchPolicy::Exact => host_mods == qol_mods,
        MatchPolicy::Subset => {
            let strict_subset = !host_mods.is_empty() && host_mods.is_subset(&qol_mods);
            host_mods == qol_mods || strict_subset
        }
    }
}

fn split_combo(normalized: &str) -> (BTreeSet<&str>, String) {
    let tokens: Vec<&str> = normalized.split('+').collect();
    let mut mods = BTreeSet::new();
    let mut key_tokens = Vec::new();
    let mut saw_key = false;
    for token in tokens {
        if !saw_key && MODIFIERS.contains(&token) {
            mods.insert(token);
        } else {
            saw_key = true;
            key_tokens.push(token);
        }
    }
    (mods, key_tokens.join("+"))
}

pub(super) fn normalize_combo(input: &str) -> Option<String> {
    let lower = input.to_ascii_lowercase();
    let tokens = tokenize(&lower);

    let mut mods = BTreeSet::new();
    let mut keys = Vec::new();
    for token in tokens {
        match token.as_str() {
            "ctrl" | "control" | "primary" => {
                mods.insert("ctrl");
            }
            "shift" => {
                mods.insert("shift");
            }
            "alt" | "mod1" | "opt" | "option" => {
                mods.insert("alt");
            }
            "super" | "mod4" | "meta" | "win" | "cmd" | "command" => {
                mods.insert("super");
            }
            "" => {}
            _ => keys.push(token),
        }
    }
    if keys.is_empty() {
        return None;
    }
    let mods_str: Vec<&str> = mods.into_iter().collect();
    let key_str = keys.join("+");
    Some(if mods_str.is_empty() {
        key_str
    } else {
        format!("{}+{}", mods_str.join("+"), key_str)
    })
}

fn tokenize(lower: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut in_brackets = false;
    for c in lower.chars() {
        match c {
            '<' => {
                push_if_nonempty(&mut buf, &mut tokens);
                in_brackets = true;
            }
            '>' if in_brackets => {
                push_if_nonempty(&mut buf, &mut tokens);
                in_brackets = false;
            }
            '+' if !in_brackets => push_if_nonempty(&mut buf, &mut tokens),
            ' ' | '\t' => {}
            _ => buf.push(c),
        }
    }
    push_if_nonempty(&mut buf, &mut tokens);
    tokens
}

fn push_if_nonempty(buf: &mut String, out: &mut Vec<String>) {
    let token = std::mem::take(buf);
    let trimmed = token.trim();
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_handles_qol_format() {
        let cases = [
            ("Super+Space", Some("super+space")),
            ("Shift+Super+R", Some("shift+super+r")),
            ("Ctrl+Alt+Shift+Down", Some("alt+ctrl+shift+down")),
            ("Alt+Tab", Some("alt+tab")),
            ("Super+Down", Some("super+down")),
        ];
        for (input, want) in cases {
            assert_eq!(
                normalize_combo(input).as_deref(),
                want,
                "qol input: {input}"
            );
        }
    }

    #[test]
    fn normalize_handles_gtk_format() {
        let cases = [
            ("<Super>space", Some("super+space")),
            ("<Shift><Super>R", Some("shift+super+r")),
            ("<Primary><Alt>Delete", Some("alt+ctrl+delete")),
            ("<Mod4>F1", Some("super+f1")),
            ("<Alt>Tab", Some("alt+tab")),
        ];
        for (input, want) in cases {
            assert_eq!(
                normalize_combo(input).as_deref(),
                want,
                "gtk input: {input}"
            );
        }
    }

    #[test]
    fn qol_and_gtk_match_on_canonical_form() {
        let pairs = [
            ("Super+Space", "<Super>space"),
            ("Shift+Super+R", "<Shift><Super>R"),
            ("Ctrl+Alt+Delete", "<Primary><Alt>Delete"),
        ];
        for (qol, gtk) in pairs {
            assert_eq!(normalize_combo(qol), normalize_combo(gtk), "{qol} vs {gtk}");
        }
    }

    #[test]
    fn normalize_maps_cmd_command_opt_and_option_aliases() {
        let cases = [
            ("Cmd+Tab", Some("super+tab")),
            ("Command+S", Some("super+s")),
            ("Command+Shift+S", Some("shift+super+s")),
            ("opt+space", Some("alt+space")),
            ("Option+F1", Some("alt+f1")),
        ];
        for (input, want) in cases {
            assert_eq!(
                normalize_combo(input).as_deref(),
                want,
                "alias input: {input}"
            );
        }
    }

    #[test]
    fn normalize_returns_none_for_modifier_only_or_special() {
        assert_eq!(normalize_combo("<Super>"), None);
        assert_eq!(normalize_combo("Shift+Ctrl"), None);
        assert_eq!(normalize_combo(""), None);
    }

    #[test]
    fn normalize_treats_xf86_keyboard_as_distinct_token() {
        assert_eq!(
            normalize_combo("XF86Keyboard").as_deref(),
            Some("xf86keyboard")
        );
        assert!(normalize_combo("XF86Keyboard") != normalize_combo("Super+Space"));
    }

    #[test]
    fn filter_shadowed_removes_only_conflicting_entry() {
        let entries: Vec<String> = ["<Super>space", "XF86Keyboard"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_shadowed(&entries, &["Super+Space".into()], MatchPolicy::Exact)
            .expect("normalized");
        assert_eq!(filtered, vec!["XF86Keyboard".to_string()]);
    }

    #[test]
    fn filter_shadowed_removes_host_entries_whose_modifiers_are_a_subset_of_the_qol_combo() {
        let entries: Vec<String> = ["<Super>s", "<Shift><Super>s", "<Super>d"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_shadowed(&entries, &["Shift+Super+S".into()], MatchPolicy::Subset)
            .expect("normalized");
        assert_eq!(
            filtered,
            vec!["<Super>d".to_string()],
            "a host grab on Super+s intercepts Shift+Super+S and must be withdrawn too"
        );
    }

    #[test]
    fn filter_shadowed_keeps_host_entries_with_more_modifiers_than_the_qol_combo() {
        let entries: Vec<String> = ["<Shift><Super>s"].into_iter().map(String::from).collect();
        let filtered =
            filter_shadowed(&entries, &["Super+S".into()], MatchPolicy::Exact).expect("normalized");
        assert_eq!(
            filtered,
            vec!["<Shift><Super>s".to_string()],
            "a host grab with extra modifiers does not intercept the bare combo"
        );
    }

    #[test]
    fn matches_shadow_exact_policy_requires_identical_modifiers() {
        let cases = [
            ("super+s", "super+s", true),
            ("super+s", "shift+super+s", false),
            ("shift+super+s", "shift+super+s", true),
            ("s", "shift+super+s", false),
            ("super+s", "super+t", false),
        ];
        for (host, qol, want) in cases {
            assert_eq!(
                matches_shadow(host, qol, MatchPolicy::Exact),
                want,
                "exact host={host} qol={qol}"
            );
        }
    }

    #[test]
    fn matches_shadow_subset_policy_flags_fewer_modifiers_on_the_same_key() {
        let cases = [
            ("super+s", "super+s", true),
            ("super+s", "shift+super+s", true),
            ("super+s", "ctrl+shift+super+s", true),
            ("shift+super+s", "ctrl+shift+super+s", true),
            ("s", "shift+super+s", false),
            ("shift+super+s", "super+s", false),
            ("super+s", "super+t", false),
            ("super+space", "shift+super+s", false),
            ("alt+super+s", "shift+super+s", false),
        ];
        for (host, qol, want) in cases {
            assert_eq!(
                matches_shadow(host, qol, MatchPolicy::Subset),
                want,
                "subset host={host} qol={qol}"
            );
        }
    }

    #[test]
    fn split_combo_keeps_unknown_tokens_in_the_key() {
        assert_eq!(split_combo("hyper+s"), (BTreeSet::new(), "hyper+s".into()));
        assert_eq!(
            split_combo("ctrl+alt+a+b"),
            (["alt", "ctrl"].into_iter().collect(), "a+b".to_string())
        );
    }

    #[test]
    fn filter_shadowed_returns_empty_when_only_conflict_present() {
        let entries: Vec<String> = ["<Super>space"].into_iter().map(String::from).collect();
        let filtered = filter_shadowed(&entries, &["Super+Space".into()], MatchPolicy::Exact)
            .expect("normalized");
        assert!(filtered.is_empty(), "got: {filtered:?}");
    }

    #[test]
    fn filter_shadowed_keeps_non_matching_entries() {
        let entries: Vec<String> = ["<Alt>Tab", "<Super>r"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_shadowed(&entries, &["Super+Space".into()], MatchPolicy::Exact)
            .expect("normalized");
        assert_eq!(filtered, entries);
    }

    #[test]
    fn filter_shadowed_keeps_unparseable_entries() {
        let entries: Vec<String> = ["<Super>space", "garbage-no-key", "<Super>"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_shadowed(&entries, &["Super+Space".into()], MatchPolicy::Exact)
            .expect("normalized");
        assert_eq!(
            filtered,
            vec!["garbage-no-key".to_string(), "<Super>".to_string()]
        );
    }

    #[test]
    fn filter_shadowed_returns_none_for_unnormalizable_qol_combo() {
        let entries: Vec<String> = ["<Super>space"].into_iter().map(String::from).collect();
        assert!(filter_shadowed(&entries, &["<Super>".into()], MatchPolicy::Exact).is_none());
        assert!(filter_shadowed(&entries, &["".into()], MatchPolicy::Exact).is_none());
    }

    #[test]
    fn filter_shadowed_matches_across_qol_and_gtk_forms() {
        let entries: Vec<String> = ["<Primary><Alt>Delete", "<Mod4>F1"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_shadowed(&entries, &["Ctrl+Alt+Delete".into()], MatchPolicy::Exact)
            .expect("normalized");
        assert_eq!(filtered, vec!["<Mod4>F1".to_string()]);
    }
}
