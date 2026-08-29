use std::time::Duration;

pub use qol_plugin_api::launcher_flows::FlowEntry;

pub const MAX_ROWS: usize = 8;

const FETCH_IO_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, PartialEq)]
pub struct FlowRow {
    pub title: String,
    pub subtitle: Option<String>,
    pub copy: Option<String>,
    pub raw: serde_json::Value,
}

pub fn parse_rows(payload: &serde_json::Value) -> Result<Vec<FlowRow>, String> {
    let Some(rows) = payload.get("rows").and_then(|rows| rows.as_array()) else {
        return Err("flow response has no rows array".to_string());
    };
    Ok(rows
        .iter()
        .filter_map(|item| {
            let title = item.get("title")?.as_str()?;
            Some(FlowRow {
                title: title.to_string(),
                subtitle: item
                    .get("subtitle")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                copy: item
                    .get("copy")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                raw: item.clone(),
            })
        })
        .take(MAX_ROWS)
        .collect())
}

pub fn fetch_rows(entry: &FlowEntry, text: &str) -> Result<Vec<FlowRow>, String> {
    let body = serde_json::json!({ "query": text }).to_string();
    let route = qol_conventions::api_routes::plugin_query(&entry.plugin_id, &entry.query);
    let (status, response) =
        qol_plugin_api::host_exec::post_to_daemon_with_timeout(&route, &body, FETCH_IO_TIMEOUT)
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
    let payload: serde_json::Value =
        serde_json::from_str(&response).map_err(|error| error.to_string())?;
    parse_rows(&payload)
}

pub fn render_action_input(
    action: &qol_config::contract::RowActionSpec,
    row: &FlowRow,
) -> serde_json::Value {
    let Some(input) = action.input.as_ref() else {
        return serde_json::json!({});
    };
    serde_json::Value::Object(
        input
            .iter()
            .map(|(field, template)| (field.clone(), render_template(template, row)))
            .collect(),
    )
}

fn render_template(template: &str, row: &FlowRow) -> serde_json::Value {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(length) = rest[start + 1..].find('}') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + length];
        rendered.push_str(&rest[..start]);
        match row.raw.get(name).and_then(|value| value.as_str()) {
            Some(value) => rendered.push_str(value),
            None => rendered.push_str(&rest[start..=start + length + 1]),
        }
        rest = &rest[start + length + 2..];
    }
    rendered.push_str(rest);
    serde_json::Value::String(rendered)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rows_keeps_titled_objects_and_caps() {
        let payload = serde_json::json!({
            "rows": [
                { "title": "one", "subtitle": "s1", "copy": "c1", "key": "k1" },
                { "nope": true },
                { "title": 3 },
                "str",
                { "title": "two" }
            ]
        });
        let rows = parse_rows(&payload).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].title, "one");
        assert_eq!(rows[0].subtitle.as_deref(), Some("s1"));
        assert_eq!(rows[0].copy.as_deref(), Some("c1"));
        assert_eq!(rows[0].raw["key"], "k1");
        assert_eq!(rows[1].title, "two");
        assert!(rows[1].subtitle.is_none());
        assert!(rows[1].copy.is_none());

        let payload = serde_json::json!({
            "rows": (0..12)
                .map(|index| serde_json::json!({ "title": format!("row {index}") }))
                .collect::<Vec<_>>()
        });
        let rows = parse_rows(&payload).unwrap();
        assert_eq!(rows.len(), MAX_ROWS);
        assert_eq!(rows[MAX_ROWS - 1].title, "row 7");
    }

    #[test]
    fn parse_rows_rejects_missing_array() {
        assert_eq!(
            parse_rows(&serde_json::json!({})),
            Err("flow response has no rows array".to_string())
        );
        assert_eq!(
            parse_rows(&serde_json::json!({ "rows": "nope" })),
            Err("flow response has no rows array".to_string())
        );
        assert_eq!(
            parse_rows(&serde_json::json!({ "rows": [1, 2] })),
            Ok(Vec::new())
        );
    }

    #[test]
    fn render_action_input_substitutes_string_fields() {
        let action = qol_config::contract::RowActionSpec {
            action: "remember".to_string(),
            input: Some(qol_config::contract::IndexMap::from([
                ("note".to_string(), "{text}".to_string()),
                ("prefix".to_string(), "keep {missing} as-is".to_string()),
            ])),
            label: None,
            key: None,
            when: None,
        };
        let row = FlowRow {
            title: "t".to_string(),
            subtitle: None,
            copy: None,
            raw: serde_json::json!({ "text": "hello world" }),
        };

        let input = render_action_input(&action, &row);
        assert_eq!(input["note"], "hello world");
        assert_eq!(input["prefix"], "keep {missing} as-is");

        let action = qol_config::contract::RowActionSpec {
            action: "remember".to_string(),
            input: None,
            label: None,
            key: None,
            when: None,
        };
        assert_eq!(render_action_input(&action, &row), serde_json::json!({}));
    }
}
