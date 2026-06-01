use std::collections::BTreeSet;

pub(super) fn parse_gsettings_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "@as []" || trimmed == "[]" {
        return Vec::new();
    }
    let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|tok| tok.trim().trim_matches('\'').trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

pub(super) fn serialize_gsettings_list(entries: &[String]) -> String {
    if entries.is_empty() {
        return "[]".to_string();
    }
    let quoted: Vec<String> = entries
        .iter()
        .map(|entry| format!("'{}'", entry.replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(","))
}

pub(super) fn filter_unshadow(entries: &[String], qol_combo: &str) -> Option<Vec<String>> {
    let target = normalize_combo(qol_combo)?;
    Some(
        entries
            .iter()
            .filter(|entry| normalize_combo(entry).is_none_or(|norm| norm != target))
            .cloned()
            .collect(),
    )
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
            "alt" | "mod1" => {
                mods.insert("alt");
            }
            "super" | "mod4" | "meta" | "win" => {
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
    fn parse_gsettings_list_handles_common_shapes() {
        let cases = [
            (
                "['<Super>space', 'XF86Keyboard']",
                vec!["<Super>space", "XF86Keyboard"],
            ),
            ("['<Super>space']", vec!["<Super>space"]),
            ("@as []", Vec::<&str>::new()),
            ("[]", Vec::<&str>::new()),
            ("", Vec::<&str>::new()),
        ];
        for (raw, want) in cases {
            let got: Vec<String> = parse_gsettings_list(raw);
            let want_owned: Vec<String> = want.into_iter().map(String::from).collect();
            assert_eq!(got, want_owned, "raw: {raw}");
        }
    }

    #[test]
    fn serialize_gsettings_list_matches_get_format() {
        let cases: [(&[&str], &str); 4] = [
            (
                &["<Super>space", "XF86Keyboard"],
                "['<Super>space','XF86Keyboard']",
            ),
            (&["<Super>space"], "['<Super>space']"),
            (&[], "[]"),
            (&["<Mod4>F1"], "['<Mod4>F1']"),
        ];
        for (entries, want) in cases {
            let owned: Vec<String> = entries.iter().map(|s| (*s).to_string()).collect();
            assert_eq!(
                serialize_gsettings_list(&owned),
                want,
                "entries: {entries:?}"
            );
        }
    }

    #[test]
    fn serialize_round_trips_with_parse() {
        let raw = "['<Super>space', 'XF86Keyboard', '<Mod4>F1']";
        let parsed = parse_gsettings_list(raw);
        let serialized = serialize_gsettings_list(&parsed);
        assert_eq!(parse_gsettings_list(&serialized), parsed);
    }

    #[test]
    fn filter_unshadow_removes_only_conflicting_entry() {
        let entries: Vec<String> = ["<Super>space", "XF86Keyboard"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_unshadow(&entries, "Super+Space").expect("normalized");
        assert_eq!(filtered, vec!["XF86Keyboard".to_string()]);
    }

    #[test]
    fn filter_unshadow_returns_empty_when_only_conflict_present() {
        let entries: Vec<String> = ["<Super>space"].into_iter().map(String::from).collect();
        let filtered = filter_unshadow(&entries, "Super+Space").expect("normalized");
        assert!(filtered.is_empty(), "got: {filtered:?}");
    }

    #[test]
    fn filter_unshadow_keeps_non_matching_entries() {
        let entries: Vec<String> = ["<Alt>Tab", "<Super>r"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_unshadow(&entries, "Super+Space").expect("normalized");
        assert_eq!(filtered, entries);
    }

    #[test]
    fn filter_unshadow_keeps_unparseable_entries() {
        let entries: Vec<String> = ["<Super>space", "garbage-no-key", "<Super>"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_unshadow(&entries, "Super+Space").expect("normalized");
        assert_eq!(
            filtered,
            vec!["garbage-no-key".to_string(), "<Super>".to_string()]
        );
    }

    #[test]
    fn filter_unshadow_returns_none_for_unnormalizable_qol_combo() {
        let entries: Vec<String> = ["<Super>space"].into_iter().map(String::from).collect();
        assert!(filter_unshadow(&entries, "<Super>").is_none());
        assert!(filter_unshadow(&entries, "").is_none());
    }

    #[test]
    fn filter_unshadow_matches_across_qol_and_gtk_forms() {
        let entries: Vec<String> = ["<Primary><Alt>Delete", "<Mod4>F1"]
            .into_iter()
            .map(String::from)
            .collect();
        let filtered = filter_unshadow(&entries, "Ctrl+Alt+Delete").expect("normalized");
        assert_eq!(filtered, vec!["<Mod4>F1".to_string()]);
    }
}
