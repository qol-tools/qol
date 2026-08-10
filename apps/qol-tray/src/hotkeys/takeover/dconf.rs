const NON_BINDING_KEYS: &[&str] = &["custom-list", "custom-keybindings"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MatchPolicy {
    Exact,
    Subset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BindingEntry {
    pub dir: String,
    pub key: String,
    pub values: Vec<String>,
    pub reach: BindingReach,
    pub match_policy: MatchPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BindingReach {
    Managed,
    LegacyOrphan,
}

impl BindingReach {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Managed => "desktop shortcut",
            Self::LegacyOrphan => "orphaned legacy shortcut",
        }
    }
}

pub(crate) fn full_key(dir: &str, key: &str) -> String {
    format!("{dir}{key}")
}

pub(crate) struct CustomEntry {
    pub list_key: String,
    pub name: String,
}

pub(crate) fn custom_entry(dir: &str) -> Option<CustomEntry> {
    let (parent, name) = dir.trim_end_matches('/').rsplit_once('/')?;
    let keybindings_root = parent.strip_suffix("custom-keybindings")?;
    Some(CustomEntry {
        list_key: format!("{keybindings_root}custom-list"),
        name: name.to_string(),
    })
}

pub(crate) fn parse_dump(root: &str, match_policy: MatchPolicy, dump: &str) -> Vec<BindingEntry> {
    let mut entries = Vec::new();
    let mut dir = root.to_string();
    for line in dump.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            dir = section_dir(root, section);
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if NON_BINDING_KEYS.contains(&key) {
            continue;
        }
        let Some(values) = parse_string_array(raw.trim()) else {
            continue;
        };
        entries.push(BindingEntry {
            dir: dir.clone(),
            key: key.to_string(),
            values,
            reach: reach_of(root, &dir),
            match_policy,
        });
    }
    entries
}

pub(crate) fn parse_gsettings_list(
    root: &str,
    match_policy: MatchPolicy,
    output: &str,
) -> Vec<BindingEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((_, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some((key, raw)) = rest.split_once(' ') else {
            continue;
        };
        if NON_BINDING_KEYS.contains(&key) {
            continue;
        }
        let Some(values) = parse_string_array(raw.trim()) else {
            continue;
        };
        entries.push(BindingEntry {
            dir: root.to_string(),
            key: key.to_string(),
            values,
            reach: BindingReach::Managed,
            match_policy,
        });
    }
    entries
}

fn section_dir(root: &str, section: &str) -> String {
    let relative = section.trim_matches('/');
    if relative.is_empty() {
        return root.to_string();
    }
    format!("{root}{relative}/")
}

fn reach_of(root: &str, dir: &str) -> BindingReach {
    let Some(relative) = dir.strip_prefix(root) else {
        return BindingReach::Managed;
    };
    let mut segments = relative.split('/').filter(|s| !s.is_empty());
    let Some(first) = segments.next() else {
        return BindingReach::Managed;
    };
    if segments.next().is_some() {
        return BindingReach::Managed;
    }
    if root.ends_with("/keybindings/") && first.starts_with("custom") {
        return BindingReach::LegacyOrphan;
    }
    BindingReach::Managed
}

pub(crate) fn parse_string_array(raw: &str) -> Option<Vec<String>> {
    let trimmed = raw.trim().strip_prefix("@as").unwrap_or(raw.trim()).trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        return Some(Vec::new());
    }
    inner
        .split(',')
        .map(|token| unquote(token.trim()))
        .collect()
}

fn unquote(token: &str) -> Option<String> {
    let body = token.strip_prefix('\'')?.strip_suffix('\'')?;
    Some(body.replace("\\'", "'"))
}

pub(crate) fn serialize_string_array(values: &[String]) -> String {
    if values.is_empty() {
        return "@as []".to_string();
    }
    let quoted: Vec<String> = values
        .iter()
        .map(|value| format!("'{}'", value.replace('\'', "\\'")))
        .collect();
    format!("[{}]", quoted.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const CINNAMON: &str = "/org/cinnamon/desktop/keybindings/";

    #[test]
    fn parse_string_array_accepts_only_gvariant_arrays() {
        let cases: [(&str, Option<Vec<&str>>); 10] = [
            ("@as []", Some(vec![])),
            ("[]", Some(vec![])),
            ("['<Super>space']", Some(vec!["<Super>space"])),
            (
                "['<Super>space', 'XF86Keyboard']",
                Some(vec!["<Super>space", "XF86Keyboard"]),
            ),
            ("  ['<Shift><Super>s']  ", Some(vec!["<Shift><Super>s"])),
            ("'flameshot gui'", None),
            ("'Emoji Picker'", None),
            ("true", None),
            ("uint32 3", None),
            ("['unterminated", None),
        ];
        for (raw, want) in cases {
            let got = parse_string_array(raw);
            let want = want.map(|v| v.into_iter().map(String::from).collect::<Vec<_>>());
            assert_eq!(got, want, "raw: {raw}");
        }
    }

    #[test]
    fn serialize_uses_typed_empty_array_dconf_accepts() {
        assert_eq!(
            serialize_string_array(&[]),
            "@as []",
            "bare [] is an untyped gvariant literal and dconf write rejects it"
        );
        assert_eq!(
            serialize_string_array(&["<Super>space".to_string()]),
            "['<Super>space']"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn serialize_then_parse_round_trips(
            values in proptest::collection::vec("[A-Za-z0-9<>_ '\\\\-]{0,12}", 0..6)
        ) {
            let serialized = serialize_string_array(&values);
            let parsed = parse_string_array(&serialized);
            prop_assert_eq!(parsed, Some(values));
        }
    }

    #[test]
    fn parse_dump_reads_root_and_section_keys() {
        let dump = "[/]\ncustom-list=['custom1']\n\n[wm]\nclose=['<Super>w']\nmaximize=@as []\n\n[custom-keybindings/custom1]\nbinding=['<Super>t']\ncommand='xterm'\nname='Terminal'\n";
        let entries = parse_dump(CINNAMON, MatchPolicy::Subset, dump);
        let seen: Vec<(&str, &str, &[String])> = entries
            .iter()
            .map(|e| (e.dir.as_str(), e.key.as_str(), e.values.as_slice()))
            .collect();
        assert_eq!(
            seen,
            vec![
                (
                    "/org/cinnamon/desktop/keybindings/wm/",
                    "close",
                    ["<Super>w".to_string()].as_slice()
                ),
                (
                    "/org/cinnamon/desktop/keybindings/wm/",
                    "maximize",
                    [].as_slice()
                ),
                (
                    "/org/cinnamon/desktop/keybindings/custom-keybindings/custom1/",
                    "binding",
                    ["<Super>t".to_string()].as_slice()
                ),
            ],
            "custom-list must be skipped and command/name are not arrays"
        );
    }

    #[test]
    fn parse_dump_classifies_root_level_custom_sections_as_legacy_orphans() {
        let dump = "[custom2]\nbinding=['<Shift><Super>s']\ncommand='flameshot gui'\n\n[custom-keybindings/custom2]\nbinding=['<Super>e']\n\n[wm]\nclose=['<Super>w']\n";
        let entries = parse_dump(CINNAMON, MatchPolicy::Subset, dump);
        let reaches: Vec<(&str, BindingReach)> =
            entries.iter().map(|e| (e.dir.as_str(), e.reach)).collect();
        assert_eq!(
            reaches,
            vec![
                (
                    "/org/cinnamon/desktop/keybindings/custom2/",
                    BindingReach::LegacyOrphan
                ),
                (
                    "/org/cinnamon/desktop/keybindings/custom-keybindings/custom2/",
                    BindingReach::Managed
                ),
                (
                    "/org/cinnamon/desktop/keybindings/wm/",
                    BindingReach::Managed
                ),
            ]
        );
    }

    #[test]
    fn parse_gsettings_list_reads_effective_values_including_defaults() {
        let output = "org.cinnamon.desktop.keybindings show-desklets ['<Super>s']\norg.cinnamon.desktop.keybindings custom-list ['custom1']\norg.cinnamon.desktop.keybindings looking-glass-keybinding ['<Super>l']\norg.cinnamon.desktop.keybindings pointer-previous-monitor @as []\norg.cinnamon.desktop.keybindings something-else ''\n";
        let entries = parse_gsettings_list(CINNAMON, MatchPolicy::Subset, output);
        let seen: Vec<(&str, &[String])> = entries
            .iter()
            .map(|e| (e.key.as_str(), e.values.as_slice()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("show-desklets", ["<Super>s".to_string()].as_slice()),
                (
                    "looking-glass-keybinding",
                    ["<Super>l".to_string()].as_slice()
                ),
                ("pointer-previous-monitor", [].as_slice()),
            ],
            "custom-list is a name list, strings are not bindings, empty arrays are scanned"
        );
        assert!(entries.iter().all(|e| e.reach == BindingReach::Managed));
        assert!(entries
            .iter()
            .all(|e| e.match_policy == MatchPolicy::Subset));
    }

    #[test]
    fn parse_dump_carries_the_roots_match_policy_on_every_entry() {
        let dump = "[wm]\nclose=['<Super>w']\n";
        let subset = parse_dump(CINNAMON, MatchPolicy::Subset, dump);
        assert_eq!(subset.len(), 1);
        assert_eq!(subset[0].match_policy, MatchPolicy::Subset);
        let exact = parse_dump("/desktop/ibus/general/hotkey/", MatchPolicy::Exact, dump);
        assert_eq!(exact[0].match_policy, MatchPolicy::Exact);
    }

    #[test]
    fn parse_dump_never_marks_non_keybinding_roots_as_orphaned() {
        let dump = "[custom-thing]\ntriggers=['<Super>space']\n";
        let entries = parse_dump("/desktop/ibus/general/hotkey/", MatchPolicy::Exact, dump);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].reach, BindingReach::Managed);
    }

    #[test]
    fn full_key_joins_dir_and_key_without_double_slash() {
        assert_eq!(
            full_key("/org/cinnamon/desktop/keybindings/custom2/", "binding"),
            "/org/cinnamon/desktop/keybindings/custom2/binding"
        );
    }
}
