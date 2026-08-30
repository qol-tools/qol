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
    let mut rows: Vec<FlowRow> = rows
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
        .collect();
    rows.sort_by(|a, b| row_date_key(a).cmp(&row_date_key(b)));
    Ok(rows)
}

fn row_date_key(row: &FlowRow) -> (bool, std::cmp::Reverse<&str>) {
    let at = row
        .raw
        .get("trail")
        .and_then(|trail| trail.as_array())
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.get("at"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    (at.is_empty(), std::cmp::Reverse(at))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FlowVerdict {
    Answered,
    Vague,
    NoMemory,
}

pub struct FlowFetch {
    pub rows: Vec<FlowRow>,
    pub verdict: FlowVerdict,
}

pub fn parse_verdict(payload: &serde_json::Value) -> FlowVerdict {
    match payload.get("verdict").and_then(|value| value.as_str()) {
        Some("candidates") => FlowVerdict::Vague,
        Some("no-memory") => {
            let rows = payload
                .get("rows")
                .and_then(|rows| rows.as_array())
                .map(|rows| rows.len())
                .unwrap_or(0);
            if rows > 0 {
                FlowVerdict::Vague
            } else {
                FlowVerdict::NoMemory
            }
        }
        _ => FlowVerdict::Answered,
    }
}

pub struct TrailNode {
    pub at: String,
    pub tag: String,
    pub text: String,
    pub struck: bool,
}

pub fn trail_of(raw: &serde_json::Value) -> Vec<TrailNode> {
    let Some(entries) = raw.get("trail").and_then(|trail| trail.as_array()) else {
        return vec![fallback_node(raw)];
    };
    let nodes: Vec<TrailNode> = entries
        .iter()
        .filter_map(|entry| {
            let text = entry.get("text")?.as_str()?;
            if text.is_empty() {
                return None;
            }
            Some(TrailNode {
                at: entry
                    .get("at")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                tag: entry
                    .get("tag")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string(),
                text: text.to_string(),
                struck: entry
                    .get("struck")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false),
            })
        })
        .collect();
    if nodes.is_empty() {
        return vec![fallback_node(raw)];
    }
    nodes
}

fn fallback_node(raw: &serde_json::Value) -> TrailNode {
    TrailNode {
        at: String::new(),
        tag: raw
            .get("subtitle")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
        text: raw
            .get("title")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string(),
        struck: false,
    }
}

pub fn detail_of(raw: &serde_json::Value) -> Vec<(String, String)> {
    let Some(entries) = raw.get("detail").and_then(|detail| detail.as_array()) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let label = entry.get("label")?.as_str()?;
            let value = entry.get("value")?.as_str()?;
            if label.is_empty() || value.is_empty() {
                return None;
            }
            Some((label.to_string(), value.to_string()))
        })
        .collect()
}

pub fn fetch_rows(entry: &FlowEntry, text: &str) -> Result<FlowFetch, String> {
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
    let rows = parse_rows(&payload)?;
    Ok(FlowFetch {
        rows,
        verdict: parse_verdict(&payload),
    })
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
    fn parse_verdict_maps_the_wire_values() {
        assert_eq!(
            parse_verdict(&serde_json::json!({ "verdict": "answered", "rows": [] })),
            FlowVerdict::Answered
        );
        assert_eq!(
            parse_verdict(&serde_json::json!({ "verdict": "candidates", "rows": [] })),
            FlowVerdict::Vague
        );
        assert_eq!(
            parse_verdict(&serde_json::json!({
                "verdict": "candidates",
                "rows": [{ "title": "nearby" }]
            })),
            FlowVerdict::Vague
        );
        assert_eq!(
            parse_verdict(&serde_json::json!({
                "verdict": "no-memory",
                "rows": [{ "title": "nearby" }]
            })),
            FlowVerdict::Vague
        );
        assert_eq!(
            parse_verdict(&serde_json::json!({ "verdict": "no-memory", "rows": [] })),
            FlowVerdict::NoMemory
        );
        assert_eq!(
            parse_verdict(&serde_json::json!({ "verdict": "no-memory" })),
            FlowVerdict::NoMemory
        );
        assert_eq!(parse_verdict(&serde_json::json!({})), FlowVerdict::Answered);
        assert_eq!(
            parse_verdict(&serde_json::json!({ "rows": [] })),
            FlowVerdict::Answered
        );
        assert_eq!(
            parse_verdict(&serde_json::json!({ "verdict": 7 })),
            FlowVerdict::Answered
        );
    }

    #[test]
    fn parse_rows_sorts_newest_first_with_undated_last() {
        let payload = serde_json::json!({
            "rows": [
                { "title": "aug 1", "trail": [{ "at": "2026-08-01", "text": "x" }] },
                { "title": "aug 12", "trail": [{ "at": "2026-08-12", "text": "x" }] },
                { "title": "undated" },
                { "title": "aug 5", "trail": [{ "at": "2026-08-05", "text": "x" }] }
            ]
        });
        let rows = parse_rows(&payload).unwrap();
        let titles: Vec<&str> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, ["aug 12", "aug 5", "aug 1", "undated"]);
    }

    #[test]
    fn trail_of_parses_a_well_formed_trail_in_order() {
        let row = serde_json::json!({
            "title": "t",
            "trail": [
                { "at": "now", "tag": "true now", "text": "newest fact" },
                { "at": "then", "tag": "superseded", "text": "older fact", "struck": true },
                { "at": "", "tag": "", "text": "plain entry" }
            ]
        });
        let nodes = trail_of(&row);
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].at, "now");
        assert_eq!(nodes[0].tag, "true now");
        assert_eq!(nodes[0].text, "newest fact");
        assert!(!nodes[0].struck);
        assert_eq!(nodes[1].at, "then");
        assert_eq!(nodes[1].tag, "superseded");
        assert_eq!(nodes[1].text, "older fact");
        assert!(nodes[1].struck);
        assert_eq!(nodes[2].text, "plain entry");
        assert_eq!(nodes[2].at, "");
        assert!(!nodes[2].struck);
    }

    #[test]
    fn trail_of_falls_back_to_title_and_subtitle_without_a_usable_trail() {
        let row = serde_json::json!({
            "title": "the clipboard ring survives restarts",
            "subtitle": "capture 2026-08-02"
        });
        let nodes = trail_of(&row);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].text, "the clipboard ring survives restarts");
        assert_eq!(nodes[0].tag, "capture 2026-08-02");
        assert_eq!(nodes[0].at, "");
        assert!(!nodes[0].struck);

        for trail in [
            serde_json::json!("nope"),
            serde_json::json!([]),
            serde_json::json!([{ "text": "" }, 3]),
        ] {
            let row = serde_json::json!({
                "title": "still divable",
                "subtitle": "skill qol-voice",
                "trail": trail
            });
            let nodes = trail_of(&row);
            assert_eq!(nodes.len(), 1);
            assert_eq!(nodes[0].text, "still divable");
            assert_eq!(nodes[0].tag, "skill qol-voice");
        }
    }

    #[test]
    fn trail_of_keeps_only_entries_with_usable_text() {
        let row = serde_json::json!({
            "title": "t",
            "trail": ["a string", 7, null, { "at": "now", "tag": "true now", "text": "kept" }]
        });
        let nodes = trail_of(&row);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].text, "kept");
        assert_eq!(nodes[0].at, "now");
        assert_eq!(nodes[0].tag, "true now");
        assert!(!nodes[0].struck);
    }

    #[test]
    fn detail_of_keeps_valid_fields_in_order_and_skips_unusable_rows() {
        let row = serde_json::json!({
            "detail": [
                { "label": "verdict", "value": "kept" },
                { "label": "", "value": "skipped empty label" },
                { "label": "score", "value": "0.42" },
                { "label": "skipped empty value", "value": "" }
            ]
        });
        assert_eq!(
            detail_of(&row),
            vec![
                ("verdict".to_string(), "kept".to_string()),
                ("score".to_string(), "0.42".to_string())
            ]
        );

        assert_eq!(detail_of(&serde_json::json!({ "title": "t" })), Vec::new());

        let row = serde_json::json!({
            "detail": ["a string", 7, { "label": "kind", "value": "capture" }]
        });
        assert_eq!(
            detail_of(&row),
            vec![("kind".to_string(), "capture".to_string())]
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
