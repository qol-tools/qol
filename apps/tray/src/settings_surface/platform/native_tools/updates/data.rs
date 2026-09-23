use anyhow::bail;
use qol_runtime::local_http::Method;
use serde::Deserialize;

use super::super::data::{request_json, request_text, REQUEST_TIMEOUT};
use super::model::UpdatesSnapshot;

const UPDATES_QUERY: &str = "/api/core/queries/updates";
const NO_ANSWER: &str = "qol-tray did not answer. Try again.";

#[derive(Debug, Deserialize)]
struct ActionResponse {
    success: bool,
    message: String,
}

pub(super) fn load() -> anyhow::Result<UpdatesSnapshot> {
    request_json(Method::Get, UPDATES_QUERY, None, REQUEST_TIMEOUT)
}

pub(super) fn action(name: &str, body: Option<&str>) -> anyhow::Result<()> {
    let route = format!("/api/core/actions/{name}");
    match request_text(Method::Post, &route, body, REQUEST_TIMEOUT) {
        Ok(body) => outcome(&body),
        Err(error) => bail!("{}", refusal(&format!("{error:#}"))),
    }
}

fn outcome(body: &str) -> anyhow::Result<()> {
    let Ok(response) = serde_json::from_str::<ActionResponse>(body) else {
        bail!("{NO_ANSWER}");
    };
    if response.success {
        Ok(())
    } else {
        bail!("{}", response.message)
    }
}

fn refusal(body: &str) -> String {
    serde_json::from_str::<ActionResponse>(body)
        .map_or_else(|_| NO_ANSWER.to_string(), |response| response.message)
}

#[cfg(test)]
mod tests {
    use super::{outcome, refusal, NO_ANSWER};

    #[test]
    fn refused_actions_surface_the_tray_message() {
        assert_eq!(
            refusal(r#"{"success":false,"message":"qol-tray is up to date"}"#),
            "qol-tray is up to date"
        );
        assert_eq!(refusal("An update is already running"), NO_ANSWER);
        assert_eq!(refusal(""), NO_ANSWER);
    }

    #[test]
    fn action_responses_parse_both_outcomes() {
        assert!(outcome(r#"{"success":true,"message":"Update started"}"#).is_ok());
        assert_eq!(
            outcome(r#"{"success":false,"message":"Development builds update through Recompile"}"#)
                .unwrap_err()
                .to_string(),
            "Development builds update through Recompile"
        );
        assert_eq!(outcome("not json").unwrap_err().to_string(), NO_ANSWER);
    }
}
