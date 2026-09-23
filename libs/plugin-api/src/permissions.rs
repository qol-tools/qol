use anyhow::{Context, Result};
use qol_headless::{DoctorCheck, DoctorCheckResult};
use qol_platform::{permission_status, Permission, PermissionState};
use serde_json::json;

use crate::manifest::PluginManifest;

pub const CHECK_ID: &str = "permissions";

const ABOUT: &str = "Report the OS grants this plugin declares, without requesting any of them.";

pub fn declared(raw_manifest: &str) -> Result<Vec<Permission>> {
    let manifest: PluginManifest =
        toml::from_str(raw_manifest).context("parse the plugin manifest")?;
    Ok(manifest.capabilities.permissions)
}

pub fn permissions_check(
    plugin_name: impl Into<String>,
    raw_manifest: &'static str,
) -> DoctorCheck {
    let plugin_name = plugin_name.into();
    DoctorCheck::new(CHECK_ID, ABOUT, move || {
        Ok(evaluate(&plugin_name, &observe(&declared(raw_manifest)?)))
    })
}

fn observe(declared: &[Permission]) -> Vec<(Permission, PermissionState)> {
    declared
        .iter()
        .map(|permission| (*permission, permission_status(*permission)))
        .collect()
}

fn evaluate(plugin_name: &str, observed: &[(Permission, PermissionState)]) -> DoctorCheckResult {
    if observed.is_empty() {
        return DoctorCheckResult::ok(
            CHECK_ID,
            format!("{plugin_name} declares no OS permissions."),
        );
    }

    let details = json!({
        "observed": observed
            .iter()
            .map(|(permission, state)| json!({
                "permission": permission,
                "state": state,
                "granted": state.granted(),
            }))
            .collect::<Vec<_>>(),
        "requested": false,
    });

    let denied = observed.iter().find_map(|(permission, state)| match state {
        PermissionState::Denied { grant_at } => Some((*permission, *grant_at)),
        _ => None,
    });
    if let Some((permission, grant_at)) = denied {
        return DoctorCheckResult::fail(
            CHECK_ID,
            format!("{plugin_name} is not allowed to do {}.", permission.label()),
        )
        .with_fix(format!("Enable {plugin_name} in {grant_at}"))
        .with_details(details);
    }

    let unreadable = observed.iter().find_map(|(permission, state)| match state {
        PermissionState::Unknown { because } => Some((*permission, *because)),
        _ => None,
    });
    if let Some((permission, because)) = unreadable {
        return DoctorCheckResult::warn(
            CHECK_ID,
            format!(
                "Whether {plugin_name} may do {} could not be read: {because}.",
                permission.label()
            ),
        )
        .with_details(details);
    }

    DoctorCheckResult::ok(
        CHECK_ID,
        format!("{plugin_name} has every OS permission it declares."),
    )
    .with_details(details)
}

#[cfg(test)]
mod tests {
    use qol_headless::DoctorStatus;

    use super::*;

    const MANIFEST: &str = r#"
[plugin]
id = "example"
name = "Example"
description = ""
version = "0.0.1"

[menu]
label = "Example"
items = []

[capabilities]
doctor = true
permissions = ["input-capture", "screen-capture"]
"#;
    #[test]
    fn a_manifest_declaration_is_read_back_in_order() {
        assert_eq!(
            declared(MANIFEST).expect("manifest parses"),
            vec![Permission::InputCapture, Permission::ScreenCapture]
        );
    }

    #[test]
    fn an_undeclared_plugin_reads_back_as_needing_nothing() {
        let manifest = MANIFEST.replace(r#"permissions = ["input-capture", "screen-capture"]"#, "");
        assert_eq!(declared(&manifest).expect("manifest parses"), vec![]);
    }

    #[test]
    fn a_misspelled_permission_is_rejected_rather_than_ignored() {
        let manifest = MANIFEST.replace("input-capture", "input_capture");
        assert!(declared(&manifest).is_err());
    }

    #[test]
    fn declaring_nothing_passes_without_touching_the_os() {
        let result = evaluate("Example", &[]);
        assert_eq!(result.status, DoctorStatus::Ok);
    }

    #[test]
    fn a_refusal_fails_and_says_where_to_grant_it() {
        let result = evaluate(
            "Example",
            &[
                (Permission::InputCapture, PermissionState::Granted),
                (
                    Permission::ScreenCapture,
                    PermissionState::Denied {
                        grant_at: "Settings > Recording",
                    },
                ),
            ],
        );

        assert_eq!(result.status, DoctorStatus::Fail);
        assert!(result.message.contains("screen capture"));
        assert_eq!(
            result.fix.as_deref(),
            Some("Enable Example in Settings > Recording")
        );
        assert_eq!(result.details.unwrap()["requested"], false);
    }

    #[test]
    fn an_unreadable_grant_warns_instead_of_passing() {
        let result = evaluate(
            "Example",
            &[(
                Permission::InputCapture,
                PermissionState::Unknown {
                    because: "no way to ask here",
                },
            )],
        );

        assert_eq!(result.status, DoctorStatus::Warn);
        assert!(result.message.contains("no way to ask here"));
    }

    #[test]
    fn an_ungated_platform_passes() {
        let result = evaluate(
            "Example",
            &[(
                Permission::InputCapture,
                PermissionState::NotRequired {
                    because: "nothing gates this",
                },
            )],
        );

        assert_eq!(result.status, DoctorStatus::Ok);
    }
}
