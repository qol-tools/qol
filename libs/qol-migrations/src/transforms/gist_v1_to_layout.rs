//! Transform a legacy `qol-tray-profile.json` gist (schema v1) into the
//! per-file profile layout used by the post-redesign on-disk format.
//!
//! Pure function: no I/O. Output is a `HashMap<PathBuf, Vec<u8>>` where keys
//! are paths RELATIVE TO THE PROFILE DIR and values are pretty-printed JSON
//! bytes. The caller decides where the profile dir lives.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

const SUPPORTED_TARGET_OS: &[&str] = &["linux", "macos", "windows"];
const SUPPORTED_GIST_VERSION: u64 = 1;
const REQUIRED_FIELDS: &[&str] = &["hotkeys", "shortcuts", "plugin_configs", "plugins"];

pub fn transform_gist_v1_to_layout(
    gist_json: &Value,
    target_os: &str,
) -> Result<HashMap<PathBuf, Vec<u8>>> {
    if !SUPPORTED_TARGET_OS.contains(&target_os) {
        return Err(anyhow!(
            "unsupported target_os {:?}: expected one of {:?}",
            target_os,
            SUPPORTED_TARGET_OS
        ));
    }

    let root = gist_json
        .as_object()
        .ok_or_else(|| anyhow!("gist v1 root must be a JSON object"))?;

    if let Some(version) = root.get("version") {
        let v = version
            .as_u64()
            .ok_or_else(|| anyhow!("gist v1 'version' must be an integer"))?;
        if v != SUPPORTED_GIST_VERSION {
            return Err(anyhow!(
                "gist v1 transform only supports version {}, got {}",
                SUPPORTED_GIST_VERSION,
                v
            ));
        }
    }

    for field in REQUIRED_FIELDS {
        if !root.contains_key(*field) {
            return Err(anyhow!("gist v1 missing required field: {}", field));
        }
    }

    let hotkeys = &root["hotkeys"];
    let shortcuts = &root["shortcuts"];
    let plugins = &root["plugins"];
    let plugin_configs = root["plugin_configs"]
        .as_object()
        .ok_or_else(|| anyhow!("gist v1 'plugin_configs' must be a JSON object"))?;
    let default_task_runner = json!({ "actions": {} });
    let task_runner = root.get("task_runner").unwrap_or(&default_task_runner);

    let mut out: HashMap<PathBuf, Vec<u8>> = HashMap::new();

    out.insert(
        PathBuf::from("manifest.json"),
        serde_json::to_vec_pretty(&json!({ "version": 1 }))?,
    );

    out.insert(
        PathBuf::from("core/plugins.lock.json"),
        serde_json::to_vec_pretty(&json!({ "version": 1, "plugins": plugins }))?,
    );

    out.insert(
        PathBuf::from(format!("os/{}/hotkeys.json", target_os)),
        serde_json::to_vec_pretty(&json!({ "hotkeys": hotkeys }))?,
    );

    out.insert(
        PathBuf::from("device/shortcuts.json"),
        serde_json::to_vec_pretty(&json!({ "shortcuts": shortcuts }))?,
    );

    out.insert(
        PathBuf::from("device/task-runner.json"),
        serde_json::to_vec_pretty(task_runner)?,
    );

    for (plugin_id, config) in plugin_configs {
        out.insert(
            PathBuf::from(format!("core/plugin-configs/{}.json", plugin_id)),
            serde_json::to_vec_pretty(config)?,
        );
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn full_gist() -> Value {
        let mut hotkeys = Vec::new();
        for i in 0..13 {
            hotkeys.push(json!({
                "id": format!("hk-{i}"),
                "action": "toggle",
                "enabled": true,
                "key": format!("ctrl+{i}"),
                "plugin_id": "plugin-alt-tab",
            }));
        }
        let shortcuts = vec![
            json!({
                "id": "sc-1",
                "name": "Docs",
                "action": { "type": "open_url", "url": "https://example.test/1" },
                "enabled": true,
                "export_to_launcher": true,
            }),
            json!({
                "id": "sc-2",
                "name": "Inbox",
                "action": { "type": "open_url", "url": "https://example.test/2" },
                "enabled": true,
                "export_to_launcher": false,
            }),
            json!({
                "id": "sc-3",
                "name": "Calendar",
                "action": { "type": "open_url", "url": "https://example.test/3" },
                "enabled": false,
                "export_to_launcher": true,
            }),
        ];
        json!({
            "version": 1,
            "hotkeys": hotkeys,
            "shortcuts": shortcuts,
            "task_runner": { "actions": { "run-foo": { "cmd": "foo" } } },
            "plugin_configs": {
                "plugin-alt-tab": { "preview_size": 320 },
                "plugin-launcher": { "max_results": 50 },
                "plugin-lights": { "bridge": "zigbee2mqtt" },
                "plugin-window-actions": { "snap": true },
                "plugin-os-themes": { "dark": true },
                "plugin-screen-recorder": { "fps": 30 },
            },
            "plugins": [
                {
                    "id": "plugin-alt-tab",
                    "repo_url": "https://example.test/alt-tab",
                    "version": "1.2.3",
                    "platforms": ["linux"],
                }
            ],
        })
    }

    fn parse(bytes: &[u8]) -> Value {
        serde_json::from_slice(bytes).expect("output should be valid JSON")
    }

    #[test]
    fn round_trip_full_gist_produces_expected_file_set_and_contents() {
        let gist = full_gist();
        let out = transform_gist_v1_to_layout(&gist, "linux").unwrap();

        let expected_keys: Vec<PathBuf> = [
            "manifest.json",
            "core/plugins.lock.json",
            "os/linux/hotkeys.json",
            "device/shortcuts.json",
            "device/task-runner.json",
            "core/plugin-configs/plugin-alt-tab.json",
            "core/plugin-configs/plugin-launcher.json",
            "core/plugin-configs/plugin-lights.json",
            "core/plugin-configs/plugin-window-actions.json",
            "core/plugin-configs/plugin-os-themes.json",
            "core/plugin-configs/plugin-screen-recorder.json",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();

        let mut actual: Vec<PathBuf> = out.keys().cloned().collect();
        let mut expected = expected_keys.clone();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected, "exact key set mismatch");

        let manifest = parse(&out[&PathBuf::from("manifest.json")]);
        assert_eq!(manifest, json!({ "version": 1 }));

        let lock = parse(&out[&PathBuf::from("core/plugins.lock.json")]);
        assert_eq!(lock["version"], json!(1));
        assert_eq!(lock["plugins"].as_array().unwrap().len(), 1);
        assert_eq!(lock["plugins"][0]["id"], json!("plugin-alt-tab"));

        let hk = parse(&out[&PathBuf::from("os/linux/hotkeys.json")]);
        assert_eq!(hk["hotkeys"].as_array().unwrap().len(), 13);

        let sc = parse(&out[&PathBuf::from("device/shortcuts.json")]);
        assert_eq!(sc["shortcuts"].as_array().unwrap().len(), 3);

        let tr = parse(&out[&PathBuf::from("device/task-runner.json")]);
        assert_eq!(tr["actions"]["run-foo"]["cmd"], json!("foo"));

        let alt_tab = parse(&out[&PathBuf::from("core/plugin-configs/plugin-alt-tab.json")]);
        assert_eq!(alt_tab, json!({ "preview_size": 320 }), "plugin config unwrapped");
    }

    #[test]
    fn output_is_pretty_printed_with_two_space_indent() {
        let gist = full_gist();
        let out = transform_gist_v1_to_layout(&gist, "linux").unwrap();
        let manifest = std::str::from_utf8(&out[&PathBuf::from("manifest.json")]).unwrap();
        assert!(
            manifest.contains("\n  \"version\""),
            "expected 2-space indent, got: {manifest:?}"
        );
    }

    #[test]
    fn empty_collections_still_produce_wrapped_files() {
        let gist = json!({
            "version": 1,
            "hotkeys": [],
            "shortcuts": [],
            "task_runner": { "actions": {} },
            "plugin_configs": {},
            "plugins": [],
        });
        let out = transform_gist_v1_to_layout(&gist, "linux").unwrap();

        let cases: &[(&str, Value)] = &[
            ("os/linux/hotkeys.json", json!({ "hotkeys": [] })),
            ("device/shortcuts.json", json!({ "shortcuts": [] })),
            (
                "core/plugins.lock.json",
                json!({ "version": 1, "plugins": [] }),
            ),
            ("device/task-runner.json", json!({ "actions": {} })),
            ("manifest.json", json!({ "version": 1 })),
        ];
        for (path, expected) in cases {
            let actual = parse(&out[&PathBuf::from(*path)]);
            assert_eq!(&actual, expected, "path: {path}");
        }

        let plugin_config_files: Vec<_> = out
            .keys()
            .filter(|k| k.starts_with("core/plugin-configs"))
            .collect();
        assert!(
            plugin_config_files.is_empty(),
            "empty plugin_configs should produce no per-plugin files, got {plugin_config_files:?}"
        );
    }

    #[test]
    fn target_os_places_hotkeys_in_correct_subdir() {
        let gist = full_gist();
        let cases = ["linux", "macos", "windows"];
        for os in cases {
            let out = transform_gist_v1_to_layout(&gist, os).unwrap();
            let expected = PathBuf::from(format!("os/{os}/hotkeys.json"));
            assert!(out.contains_key(&expected), "missing {expected:?} for os {os}");
            let other_os_keys: Vec<_> = out
                .keys()
                .filter(|k| k.starts_with("os/") && **k != expected)
                .collect();
            assert!(
                other_os_keys.is_empty(),
                "os {os}: unexpected other-os keys {other_os_keys:?}"
            );
        }
    }

    #[test]
    fn invalid_target_os_errors_with_clear_message() {
        let gist = full_gist();
        let cases = ["", "freebsd", "Linux", "LINUX", "ios"];
        for os in cases {
            let err = transform_gist_v1_to_layout(&gist, os).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("unsupported target_os") && msg.contains(os),
                "os {os:?}: unexpected message {msg:?}"
            );
        }
    }

    #[test]
    fn missing_required_field_errors_with_field_name() {
        let cases = ["hotkeys", "shortcuts", "plugin_configs", "plugins"];
        for field in cases {
            let mut gist = full_gist();
            gist.as_object_mut().unwrap().remove(field);
            let err = transform_gist_v1_to_layout(&gist, "linux").unwrap_err();
            let msg = format!("{err}");
            assert_eq!(
                msg,
                format!("gist v1 missing required field: {field}"),
                "field {field}"
            );
        }
    }

    #[test]
    fn missing_task_runner_uses_default_empty_actions() {
        let mut gist = full_gist();
        gist.as_object_mut().unwrap().remove("task_runner");
        let out = transform_gist_v1_to_layout(&gist, "linux").unwrap();
        let tr = parse(&out[&PathBuf::from("device/task-runner.json")]);
        assert_eq!(tr, json!({ "actions": {} }));
    }

    #[test]
    fn unsupported_gist_version_errors() {
        let mut gist = full_gist();
        gist["version"] = json!(2);
        let err = transform_gist_v1_to_layout(&gist, "linux").unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("version"), "msg: {msg}");
    }

    #[test]
    fn non_object_plugin_configs_errors() {
        let mut gist = full_gist();
        gist["plugin_configs"] = json!([]);
        let err = transform_gist_v1_to_layout(&gist, "linux").unwrap_err();
        assert!(format!("{err}").contains("plugin_configs"));
    }

    #[test]
    fn non_object_root_errors() {
        let gist = json!([1, 2, 3]);
        let err = transform_gist_v1_to_layout(&gist, "linux").unwrap_err();
        assert!(format!("{err}").contains("root"));
    }
}
