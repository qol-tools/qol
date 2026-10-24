use super::{catalog, claim, service, store};
use anyhow::{anyhow, bail, Result};
use qol_conventions::api_routes;
use qol_host_defaults::Role;
use serde_json::{json, Value};
use std::io;
use std::time::Duration;

pub(crate) const CLAIM_ACTION: &str = "host_default_claim";
pub(crate) const RELEASE_ACTION: &str = "host_default_release";

const REFUSED_STATUS: u16 = 409;
const TRAY_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn is_change_action(action: &str) -> bool {
    action == CLAIM_ACTION || action == RELEASE_ACTION
}

pub(crate) fn apply_change(plugin_id: &str, action: &str, input: &Value) -> Result<Value> {
    let role = input
        .get("role")
        .and_then(Value::as_str)
        .and_then(Role::parse)
        .ok_or_else(|| anyhow!("the request names no known role"))?;
    match action {
        CLAIM_ACTION => {
            let declaration = catalog::declared(plugin_id, role)?;
            service::claim_declared(&declaration)?;
            Ok(json!({"role": role.id(), "plugin": plugin_id, "claimed": true}))
        }
        RELEASE_ACTION => {
            let outcome = service::release_role(role)?;
            Ok(json!({"role": role.id(), "outcome": claim::outcome_label(&outcome)}))
        }
        other => bail!("{other} is not a host default change"),
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum Reply {
    Done(Value),
    Refused(String),
    Unreached,
}

pub(super) fn reply_of(response: io::Result<(u16, String)>) -> Reply {
    let (status, body) = match response {
        Ok(response) => response,
        Err(error) => {
            log::debug!("host defaults: no running tray answered: {error}");
            return Reply::Unreached;
        }
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&body) else {
        return Reply::Unreached;
    };
    match status {
        200 => Reply::Done(parsed.get("data").cloned().unwrap_or(Value::Null)),
        REFUSED_STATUS => Reply::Refused(
            parsed
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the running tray refused the change")
                .to_string(),
        ),
        _ => Reply::Unreached,
    }
}

fn ask_tray(plugin_id: &str, action: &str, role: Role) -> Reply {
    let path = api_routes::plugin_action(plugin_id, action);
    let body = json!({"role": role.id()}).to_string();
    reply_of(qol_plugin_api::host_exec::post_to_daemon_with_timeout(
        &path,
        &body,
        TRAY_TIMEOUT,
    ))
}

pub(super) fn settle<T>(
    reply: Reply,
    local: impl FnOnce() -> Result<T>,
    from_tray: impl FnOnce(&Value) -> Result<T>,
) -> Result<T> {
    match reply {
        Reply::Done(data) => from_tray(&data),
        Reply::Refused(message) => Err(anyhow!(message)),
        Reply::Unreached => local(),
    }
}

pub(super) fn claim(plugin_id: &str, role: Role) -> Result<()> {
    let declaration = catalog::declared(plugin_id, role)?;
    settle(
        ask_tray(plugin_id, CLAIM_ACTION, role),
        || service::claim_declared(&declaration),
        |_| Ok(()),
    )
}

pub(super) fn release(role: Role) -> Result<String> {
    let holder = store::ledger()?.holder(role)?;
    let reply = match holder {
        Some(holder) => ask_tray(&holder, RELEASE_ACTION, role),
        None => Reply::Unreached,
    };
    settle(
        reply,
        || Ok(claim::outcome_label(&service::release_role(role)?).to_string()),
        |data| {
            data.get("outcome")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| anyhow!("the running tray answered without an outcome"))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: Value) -> io::Result<(u16, String)> {
        Ok((status, body.to_string()))
    }

    #[test]
    fn a_tray_that_does_not_answer_is_unreached() {
        let refused = io::Error::new(io::ErrorKind::ConnectionRefused, "no tray");
        assert_eq!(reply_of(Err(refused)), Reply::Unreached);
    }

    #[test]
    fn an_answered_change_carries_its_data() {
        let reply = reply_of(answer(
            200,
            json!({"success": true, "message": "ok", "data": {"outcome": "restored"}}),
        ));
        assert_eq!(reply, Reply::Done(json!({"outcome": "restored"})));
    }

    #[test]
    fn a_refusal_carries_the_tray_message() {
        let reply = reply_of(answer(
            409,
            json!({"success": false, "message": "Print already opens X. Change that key in Hotkeys first."}),
        ));
        assert_eq!(
            reply,
            Reply::Refused("Print already opens X. Change that key in Hotkeys first.".to_string())
        );
    }

    #[test]
    fn an_unknown_route_or_a_garbled_body_falls_back_to_local() {
        assert_eq!(
            reply_of(answer(404, json!({"message": "x"}))),
            Reply::Unreached
        );
        assert_eq!(
            reply_of(Ok((200, "not json".to_string()))),
            Reply::Unreached
        );
    }

    #[test]
    fn settling_runs_local_only_when_no_tray_answered() {
        let local = || Ok("local".to_string());
        let tray = |data: &Value| Ok(data.to_string());

        assert_eq!(settle(Reply::Unreached, local, tray).unwrap(), "local");
        assert_eq!(settle(Reply::Done(json!(7)), local, tray).unwrap(), "7");
        let refused = settle(Reply::Refused("no".to_string()), local, tray);
        assert_eq!(refused.unwrap_err().to_string(), "no");
    }

    #[test]
    fn a_change_for_an_unknown_role_is_refused_before_anything_runs() {
        let error = apply_change("qol-shot", CLAIM_ACTION, &json!({"role": "slot.nonsense"}))
            .expect_err("unknown role");
        assert!(error.to_string().contains("no known role"), "{error}");
        assert!(apply_change("qol-shot", CLAIM_ACTION, &json!({})).is_err());
    }

    #[test]
    fn only_the_two_change_actions_are_change_actions() {
        assert!(is_change_action(CLAIM_ACTION) && is_change_action(RELEASE_ACTION));
        assert!(!is_change_action(
            crate::features::host_defaults::ROW_ACTION
        ));
    }
}
