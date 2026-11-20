use std::time::Duration;

use qol_plugin_api::launcher_flows::{FlowDetail, FlowEntry, FlowPanel};

use crate::flow::{
    parse_preview_rows, parse_rows, parse_status, parse_verdict, render_action_input, FlowFetch,
    FlowRow,
};

const FETCH_IO_TIMEOUT: Duration = Duration::from_secs(12);

pub fn fetch_rows(entry: &FlowEntry, text: &str) -> Result<FlowFetch, String> {
    let body = serde_json::json!({ "query": text }).to_string();
    let payload = post_query(entry, &entry.query, &body)?;
    let rows = match entry.panel {
        FlowPanel::Preview => parse_preview_rows(&payload)?,
        FlowPanel::None => parse_rows(&payload)?,
    };
    Ok(FlowFetch {
        rows,
        verdict: parse_verdict(&payload),
        status: parse_status(&payload),
    })
}

pub fn fetch_detail(entry: &FlowEntry, query: &str, id: &str) -> Result<FlowDetail, String> {
    let body = serde_json::json!({ "id": id }).to_string();
    let payload = post_query(entry, query, &body)?;
    serde_json::from_value(payload).map_err(|error| error.to_string())
}

fn post_query(entry: &FlowEntry, query: &str, body: &str) -> Result<serde_json::Value, String> {
    let route = qol_conventions::api_routes::plugin_query(&entry.plugin_id, query);
    let (status, response) =
        qol_plugin_api::host_exec::post_to_daemon_with_timeout(&route, body, FETCH_IO_TIMEOUT)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
                    format!(
                        "host did not answer within {} s",
                        FETCH_IO_TIMEOUT.as_secs()
                    )
                }
                _ => error.to_string(),
            })?;
    if !(200..300).contains(&status) {
        return Err(format!("host {status}: {response}"));
    }
    serde_json::from_str(&response).map_err(|error| error.to_string())
}

pub fn send_feedback(entry: &FlowEntry, query: &str, key: &str) -> Result<(), String> {
    let body = serde_json::json!({ "query": query, "key": key, "vote": -1 }).to_string();
    let route = qol_conventions::api_routes::plugin_query(&entry.plugin_id, "feedback");
    let (status, response) =
        qol_plugin_api::host_exec::post_to_daemon_with_timeout(&route, &body, FETCH_IO_TIMEOUT)
            .map_err(|error| error.to_string())?;
    if !(200..300).contains(&status) {
        return Err(format!("host {status}: {response}"));
    }
    Ok(())
}

pub fn run_row_action(
    entry: &FlowEntry,
    action: &qol_config::contract::RowActionSpec,
    row: &FlowRow,
) -> Result<(), String> {
    let body = render_action_input(action, row).to_string();
    let route = qol_conventions::api_routes::plugin_action(&entry.plugin_id, &action.action);
    let (status, response) = qol_plugin_api::host_exec::post_to_daemon(&route, &body)
        .map_err(|error| error.to_string())?;
    if !(200..300).contains(&status) {
        return Err(format!("host {status}: {response}"));
    }
    Ok(())
}
