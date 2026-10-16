use anyhow::{bail, Context, Result};
use serde_json::json;

const LAUNCHER: &str = "qol-launcher";
const FLOW_ACTION: &str = "flow";

pub fn open_in_launcher() -> Result<()> {
    let route = qol_conventions::api_routes::plugin_action(LAUNCHER, FLOW_ACTION);
    let body = flow_request(env!("QOL_PLUGIN_ID"));
    let (status, response) = qol_plugin_api::host_exec::post_to_daemon(&route, &body)
        .context("qol-tray did not answer the request to open the launcher")?;
    if !(200..300).contains(&status) {
        bail!("qol-tray refused to open the launcher (HTTP {status}): {response}");
    }
    Ok(())
}

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings_via_tray(env!("QOL_PLUGIN_ID"))
        .context("failed to open the clipboard settings")
}

fn flow_request(plugin_id: &str) -> String {
    json!({ "plugin": plugin_id }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launcher_is_asked_for_this_plugin_flow_through_the_tray() {
        assert_eq!(
            qol_conventions::api_routes::plugin_action(LAUNCHER, FLOW_ACTION),
            "/api/plugins/qol-launcher/actions/flow"
        );
        assert_eq!(
            flow_request("qol-clipboard"),
            r#"{"plugin":"qol-clipboard"}"#
        );
    }
}
