mod platform;

use std::collections::HashMap;
use std::path::PathBuf;

use super::parse::KittyLs;
use super::spawn::SESSION_KEY_TAG;
use crate::{Chord, SpawnKey};

const CACHE_OVERRIDE: &str = "KITTY_CACHE_DIRECTORY";
const PROCESS_ID: &str = "KITTY_PID";
const EFFECTIVE_CONFIGS: &str = "effective-config";
const BETWEEN_KEYS: &str = ">";

pub(super) fn config_line(key: &SpawnKey, chord: &Chord) -> String {
    let bytes = chord
        .keys()
        .iter()
        .map(|key| format!("\\x{:02x}", key.control_byte()))
        .collect::<String>();
    format!(
        "map --when-focus-on var:{SESSION_KEY_TAG}={key} {} send_text all {bytes}",
        chord.spelled(BETWEEN_KEYS)
    )
}

pub(super) fn override_argument(line: &str) -> String {
    line.replacen(' ', "=", 1)
}

pub(super) fn missing(lines: Vec<String>, effective: Option<&str>) -> Vec<String> {
    let Some(effective) = effective else {
        return lines;
    };
    lines
        .into_iter()
        .filter(|line| !effective.lines().any(|applied| applied.trim_end() == line))
        .collect()
}

pub(super) fn effective_config_file(listing: &KittyLs) -> Option<PathBuf> {
    let env = listing
        .0
        .iter()
        .flat_map(|os_window| &os_window.tabs)
        .flat_map(|tab| &tab.windows)
        .map(|window| &window.env)
        .find(|env| env.contains_key(PROCESS_ID))?;
    let process = env.get(PROCESS_ID)?.parse::<u32>().ok()?;
    Some(
        cache_dir(env)?
            .join(EFFECTIVE_CONFIGS)
            .join(process.to_string()),
    )
}

fn cache_dir(env: &HashMap<String, String>) -> Option<PathBuf> {
    match env.get(CACHE_OVERRIDE).filter(|dir| !dir.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute()),
        None => platform::default_cache_dir(env),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kitty::{backend_id, parse_ls};

    fn listed(windows: serde_json::Value) -> KittyLs {
        let listing = format!(r#"[{{"id":1,"tabs":[{{"windows":{windows}}}]}}]"#);
        parse_ls(&listing, backend_id()).unwrap()
    }

    fn line(letters: &str) -> String {
        config_line(
            &SpawnKey::new("switchboard").unwrap(),
            &Chord::ctrl(letters).unwrap(),
        )
    }

    #[test]
    fn a_chord_becomes_a_mapping_that_only_the_keyed_window_gets() {
        assert_eq!(
            line("xc"),
            r"map --when-focus-on var:qol_session_key=switchboard ctrl+x>ctrl+c send_text all \x18\x03"
        );
        assert_eq!(
            override_argument(&line("xx")),
            r"map=--when-focus-on var:qol_session_key=switchboard ctrl+x>ctrl+x send_text all \x18\x18"
        );
    }

    #[test]
    fn only_lines_the_running_kitty_does_not_hold_yet_are_missing() {
        let effective = format!(
            "font_size 12.5\nmap ctrl+c copy_to_clipboard\n{}\n",
            line("xc")
        );
        assert_eq!(
            missing(vec![line("xx"), line("xc")], Some(&effective)),
            [line("xx")]
        );
        assert_eq!(
            missing(vec![line("xx"), line("xc")], None),
            [line("xx"), line("xc")]
        );
        assert!(missing(vec![line("xc")], Some(&effective)).is_empty());
    }

    #[test]
    fn the_effective_config_is_named_by_the_kitty_process_of_a_listed_window() {
        let cache = std::env::temp_dir().join("kitty-cache");
        let listing = listed(serde_json::json!([
            { "id": 7, "title": "shell", "pid": 900 },
            { "id": 8, "title": "shell", "pid": 901, "env": {
                "KITTY_PID": "12140",
                "KITTY_CACHE_DIRECTORY": cache,
            } },
        ]));
        assert_eq!(
            effective_config_file(&listing),
            Some(cache.join("effective-config").join("12140"))
        );
    }

    #[test]
    fn a_listing_that_names_no_kitty_process_has_no_effective_config() {
        let cache = std::env::temp_dir().join("kitty-cache");
        let envs = [
            serde_json::json!({}),
            serde_json::json!({ "KITTY_PID": "../12140", "KITTY_CACHE_DIRECTORY": cache }),
            serde_json::json!({ "KITTY_PID": "12140", "KITTY_CACHE_DIRECTORY": "relative" }),
        ];
        for env in envs {
            let listing = listed(serde_json::json!([
                { "id": 7, "title": "shell", "pid": 900, "env": env },
            ]));
            assert_eq!(effective_config_file(&listing), None, "{env}");
        }
        assert_eq!(effective_config_file(&KittyLs(Vec::new())), None);
    }
}
