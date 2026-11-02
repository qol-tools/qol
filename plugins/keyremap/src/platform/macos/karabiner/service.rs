use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::compile::ACTIVE_VARIABLE;
use super::store::Store;
use super::CLI_PATH;
use crate::platform::macos::input::selection::Availability;

const CORE_SERVICE: &str = "Karabiner-Core-Service";
const CORE_SERVICE_COMM: &str = "Karabiner-Core-";
const INSTALL_FIX: &str =
    "brew install --cask karabiner-elements, then open Karabiner-Elements once";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Probe {
    pub(crate) availability: Availability,
    pub(crate) core_pid: Option<u32>,
}

pub(crate) fn probe(store: &Store) -> Probe {
    let core_pid = core_service_pid();
    let availability = availability(
        Path::new(CLI_PATH).exists(),
        core_pid,
        store.read_document().ok().as_ref(),
    );
    Probe {
        availability,
        core_pid,
    }
}

pub(crate) fn availability(
    installed: bool,
    core_pid: Option<u32>,
    document: Option<&Value>,
) -> Availability {
    let missing = |what: &str, fix: &str| Availability::Missing {
        what: what.to_string(),
        fix: fix.to_string(),
    };
    if !installed {
        return missing("Karabiner-Elements", INSTALL_FIX);
    }
    if core_pid.is_none() {
        return missing(
            "Karabiner-Elements running",
            "open Karabiner-Elements and finish its setup",
        );
    }
    let Some(document) = document else {
        return missing(
            "karabiner.json",
            "open Karabiner-Elements once so it writes ~/.config/karabiner/karabiner.json",
        );
    };
    if ignores_built_in_keyboard(document) {
        return missing(
            "Karabiner-Elements to modify the built-in keyboard",
            "turn on Modify events for Apple Internal Keyboard in Karabiner-Elements, Devices",
        );
    }
    Availability::Ready
}

pub(crate) fn ignores_built_in_keyboard(document: &Value) -> bool {
    let Some(profiles) = document.get("profiles").and_then(Value::as_array) else {
        return false;
    };
    profiles
        .iter()
        .filter(|profile| profile.get("selected").and_then(Value::as_bool) == Some(true))
        .filter_map(|profile| profile.get("devices").and_then(Value::as_array))
        .flatten()
        .any(|device| {
            device.get("ignore").and_then(Value::as_bool) == Some(true)
                && device.get("identifiers") == Some(&json!({ "is_keyboard": true }))
        })
}

pub(crate) fn core_service_pid() -> Option<u32> {
    let processes = qol_process::processes().ok()?;
    processes
        .into_iter()
        .filter(|process| process.exe.starts_with(CORE_SERVICE_COMM))
        .find(|process| {
            qol_process::process_image_path(process.pid).map_or(true, |path| {
                path.file_name().is_some_and(|name| name == CORE_SERVICE)
            })
        })
        .map(|process| process.pid)
}

pub(crate) fn set_active(active: bool) -> Result<()> {
    let variables = json!({ ACTIVE_VARIABLE: i32::from(active) }).to_string();
    let output = Command::new(CLI_PATH)
        .arg("--set-variables")
        .arg(&variables)
        .output()
        .with_context(|| format!("could not run {CLI_PATH}"))?;
    if !output.status.success() {
        bail!(
            "karabiner_cli --set-variables {variables} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(devices: Value) -> Value {
        json!({ "profiles": [
            { "name": "Other", "selected": false, "devices": [{ "identifiers": { "is_keyboard": true }, "ignore": true }] },
            { "name": "Default profile", "selected": true, "devices": devices }
        ] })
    }

    #[test]
    fn each_missing_piece_names_its_fix() {
        let ready = document(json!([]));
        let cases = [
            (false, Some(7), Some(&ready), "brew install"),
            (
                true,
                None,
                Some(&ready),
                "open Karabiner-Elements and finish",
            ),
            (
                true,
                Some(7),
                None,
                "writes ~/.config/karabiner/karabiner.json",
            ),
        ];
        for (installed, pid, document, fix_part) in cases {
            match availability(installed, pid, document) {
                Availability::Missing { fix, .. } => assert!(fix.contains(fix_part), "{fix}"),
                Availability::Ready => panic!("{installed} {pid:?} should be missing"),
            }
        }
        assert_eq!(
            availability(true, Some(7), Some(&ready)),
            Availability::Ready
        );
    }

    #[test]
    fn an_ignored_built_in_keyboard_in_the_selected_profile_is_not_ready() {
        let ignored = document(json!([{ "identifiers": { "is_keyboard": true }, "ignore": true }]));
        assert!(ignores_built_in_keyboard(&ignored));
        assert!(matches!(
            availability(true, Some(7), Some(&ignored)),
            Availability::Missing { .. }
        ));

        let external = document(json!([{
            "identifiers": { "is_keyboard": true, "product_id": 592, "vendor_id": 1452 },
            "ignore": true
        }]));
        assert!(!ignores_built_in_keyboard(&external));
        assert!(!ignores_built_in_keyboard(&document(json!([]))));
    }
}
