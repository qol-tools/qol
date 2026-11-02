pub(crate) mod compile;
pub(crate) mod document;
mod key_codes;
pub(crate) mod store;

use std::path::Path;

use serde_json::{json, Value};

const CLI_PATH: &str = "/Library/Application Support/org.pqrs/Karabiner-Elements/bin/karabiner_cli";

pub(crate) fn report(compiled: &compile::Compiled) -> Value {
    json!({
        "complex_modifications_rule": compiled.rule,
        "simple_modifications": compiled.simple_modifications,
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
    if synced.edited_by_hand {
        lines.push(
            "The Key Remap block had been edited in Karabiner-Elements; it was replaced."
                .to_string(),
        );
    }
    for key in &synced.conflicts {
        lines.push(format!(
            "Your own Karabiner-Elements change on {key} stays, so Key Remap's modifier key change for {key} is not applied."
        ));
    }
    for gap in &compiled.gaps {
        lines.push(format!("Not compiled: {}: {}", gap.rule, gap.reason));
    }
    lines.join("\n") + "\n"
}

pub(crate) fn remove_summary(removed: bool, path: &Path) -> String {
    let path = path.display();
    if removed {
        format!("Removed Key Remap's rules and modifier key changes from {path}.\n")
    } else {
        format!("{path} has no Key Remap rules.\n")
    }
}
