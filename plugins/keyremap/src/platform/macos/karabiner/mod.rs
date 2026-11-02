pub(crate) mod backend;
pub(crate) mod compile;
pub(crate) mod document;
mod key_codes;
pub(crate) mod service;
pub(crate) mod store;
pub(crate) mod system_modifier_keys;

use std::path::Path;

use serde_json::{json, Value};

const CLI_PATH: &str = "/Library/Application Support/org.pqrs/Karabiner-Elements/bin/karabiner_cli";

pub(crate) fn report(compiled: &compile::Compiled) -> Value {
    json!({
        "complex_modifications_rule": compiled.rule,
        "simple_modifications": compiled.simple_modifications,
        "system_modifier_keys": compiled.device_modifications,
        "not_compiled": compiled
            .gaps
            .iter()
            .map(|gap| json!({ "rule": gap.rule, "reason": gap.reason }))
            .collect::<Vec<_>>(),
    })
}

pub(crate) fn sync_summary(
    synced: &document::Synced,
    compiled: &compile::Compiled,
    path: &Path,
) -> String {
    let path = path.display();
    let profile = &synced.profile;
    let count = compiled.rule["manipulators"].as_array().map_or(0, Vec::len);
    let mut lines = vec![if synced.changed {
        format!("Wrote {count} Key Remap rules to the \"{profile}\" profile in {path}.")
    } else {
        format!(
            "{path} already has the current {count} Key Remap rules in the \"{profile}\" profile."
        )
    }];
    lines.push(format!(
        "They stay off until Key Remap sets {}; to try them now run: '{CLI_PATH}' --set-variables '{{\"{}\":1}}'",
        compile::ACTIVE_VARIABLE,
        compile::ACTIVE_VARIABLE
    ));
    let carried: usize = synced
        .sidecar
        .device_modifications
        .iter()
        .map(|device| device.simple_modifications.len())
        .sum();
    if carried > 0 {
        lines.push(format!(
            "Carried {carried} System Settings modifier key change(s) to the same keyboards in Karabiner-Elements."
        ));
    }
    if synced.edited_by_hand {
        lines.push(
            "The Key Remap block had been edited in Karabiner-Elements; it was replaced."
                .to_string(),
        );
    }
    for key in &synced.conflicts {
        lines.push(format!(
            "karabiner.json already changes {key}, so Key Remap's modifier key change for {key} is not applied."
        ));
    }
    for gap in &compiled.gaps {
        lines.push(format!("Not compiled: {}: {}", gap.rule, gap.reason));
    }
    lines.join("\n") + "\n"
}

pub(crate) fn remove_summary(removed: &document::Removed, path: &Path) -> String {
    let path = path.display();
    let mut text = if removed.changed {
        format!("Removed Key Remap's rules from {path}.\n")
    } else {
        format!("{path} has no Key Remap rules.\n")
    };
    if removed.untracked {
        text.push_str("Key Remap has no record of the modifier key changes it wrote, so remove any left in simple_modifications in Karabiner-Elements.\n");
    }
    text
}
