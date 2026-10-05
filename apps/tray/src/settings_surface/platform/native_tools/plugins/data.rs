use qol_runtime::local_http::Method;

use super::super::data::{request_json, REQUEST_TIMEOUT};
use super::super::updates::data::action;
use super::model::{RowAction, Snapshot};

const PLUGINS_QUERY: &str = "/api/core/queries/plugins";

pub(super) fn load() -> anyhow::Result<Snapshot> {
    request_json(Method::Get, PLUGINS_QUERY, None, REQUEST_TIMEOUT)
}

pub(super) fn run(row_action: &RowAction) -> anyhow::Result<()> {
    let (name, body) = request(row_action);
    action(name, Some(&body.to_string()))
}

fn request(row_action: &RowAction) -> (&'static str, serde_json::Value) {
    match row_action {
        RowAction::Install(id) => ("install", serde_json::json!({ "id": id })),
        RowAction::Cancel(id) => ("cancel_install", serde_json::json!({ "id": id })),
        RowAction::Remove(id) => ("uninstall", serde_json::json!({ "id": id })),
        RowAction::AddSource(repo) => ("add_source", serde_json::json!({ "repo": repo })),
        RowAction::RemoveSource(repo) => ("remove_source", serde_json::json!({ "repo": repo })),
    }
}

#[cfg(test)]
mod tests {
    use super::{request, RowAction};

    #[test]
    fn every_action_names_its_core_action_and_body() {
        let cases = [
            (
                RowAction::Install("qol-voice".into()),
                "install",
                r#"{"id":"qol-voice"}"#,
            ),
            (
                RowAction::Cancel("qol-voice".into()),
                "cancel_install",
                r#"{"id":"qol-voice"}"#,
            ),
            (
                RowAction::Remove("qol-voice".into()),
                "uninstall",
                r#"{"id":"qol-voice"}"#,
            ),
            (
                RowAction::AddSource("me/tools".into()),
                "add_source",
                r#"{"repo":"me/tools"}"#,
            ),
            (
                RowAction::RemoveSource("me/tools".into()),
                "remove_source",
                r#"{"repo":"me/tools"}"#,
            ),
        ];
        for (row_action, name, body) in cases {
            let (sent_name, sent_body) = request(&row_action);
            assert_eq!(sent_name, name);
            assert_eq!(sent_body.to_string(), body);
        }
    }
}
