use qol_host_defaults::{Lifetime, Owner, Report, Role, State};
use serde_json::{json, Value};

pub(super) struct ListEntry {
    pub role: Role,
    pub plugins: Vec<String>,
    pub holder: Option<String>,
    pub report: Report,
}

pub(super) fn lifetime_id(lifetime: Lifetime) -> &'static str {
    match lifetime {
        Lifetime::PortableSession => "portable_session",
        Lifetime::ResidentPolicy => "resident_policy",
    }
}

fn state_parts(state: &State) -> (&'static str, Option<String>) {
    match state {
        State::Silent => ("silent", None),
        State::NeedsConfirmation { ask } => ("needs_confirmation", Some(ask.to_string())),
        State::NeedsPermission { ask } => ("needs_permission", Some(ask.to_string())),
        State::Impossible { reason } => ("impossible", Some(reason.to_string())),
        State::Unknown { reason } => ("unknown", Some(reason.to_string())),
    }
}

fn owner_parts(owner: &Owner) -> (&'static str, Option<String>) {
    match owner {
        Owner::Named(name) => ("named", Some(name.clone())),
        Owner::Unreadable => ("unreadable", None),
        Owner::NoHostTool => ("no_host_tool", None),
    }
}

fn state_text(state: &State) -> String {
    let (kind, detail) = state_parts(state);
    match detail {
        Some(detail) => format!("{kind} ({detail})"),
        None => kind.to_string(),
    }
}

fn owner_text(owner: &Owner) -> String {
    let (kind, name) = owner_parts(owner);
    name.unwrap_or_else(|| kind.replace('_', " "))
}

fn lifetimes_text(lifetimes: &[Lifetime]) -> String {
    if lifetimes.is_empty() {
        return "none".to_string();
    }
    lifetimes
        .iter()
        .map(|lifetime| lifetime_id(*lifetime))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn report_json(report: &Report) -> Value {
    let (state, detail) = state_parts(&report.state);
    let (owner, name) = owner_parts(&report.owner);
    json!({
        "role": report.role.id(),
        "state": {"kind": state, "detail": detail},
        "owner": {"kind": owner, "name": name},
        "triggers": report.triggers,
        "kept": report.kept,
        "lifetimes": report
            .lifetimes
            .iter()
            .map(|lifetime| lifetime_id(*lifetime))
            .collect::<Vec<_>>(),
    })
}

pub(super) fn report_text(report: &Report) -> String {
    let mut lines = vec![
        format!("role: {}", report.role.id()),
        format!("state: {}", state_text(&report.state)),
        format!("owner: {}", owner_text(&report.owner)),
        format!("lifetimes: {}", lifetimes_text(&report.lifetimes)),
    ];
    if !report.triggers.is_empty() {
        lines.push(format!("takes: {}", report.triggers.join(", ")));
    }
    if !report.kept.is_empty() {
        lines.push(format!("system keeps: {}", report.kept.join("; ")));
    }
    lines.join("\n")
}

pub(super) fn list_json(entries: &[ListEntry]) -> Value {
    let roles: Vec<Value> = entries
        .iter()
        .map(|entry| {
            let mut value = report_json(&entry.report);
            value["role"] = json!(entry.role.id());
            value["plugins"] = json!(entry.plugins);
            value["holder"] = json!(entry.holder);
            value
        })
        .collect();
    json!({ "roles": roles })
}

pub(super) fn list_text(entries: &[ListEntry]) -> String {
    if entries.is_empty() {
        return "no installed plugin declares a host default".to_string();
    }
    entries
        .iter()
        .map(|entry| {
            format!(
                "{}: {} | owner {} | held by {} | declared by {}",
                entry.role.id(),
                state_text(&entry.report.state),
                owner_text(&entry.report.owner),
                entry.holder.as_deref().unwrap_or("nobody"),
                entry.plugins.join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_and_owners_have_stable_names() {
        assert_eq!(state_parts(&State::Silent), ("silent", None));
        assert_eq!(
            state_parts(&State::Impossible {
                reason: "no".to_string()
            }),
            ("impossible", Some("no".to_string()))
        );
        assert_eq!(
            state_parts(&State::Unknown {
                reason: "x".to_string()
            })
            .0,
            "unknown"
        );
        assert_eq!(
            state_parts(&State::NeedsConfirmation {
                ask: "a".to_string()
            })
            .0,
            "needs_confirmation"
        );
        assert_eq!(
            state_parts(&State::NeedsPermission {
                ask: "a".to_string()
            })
            .0,
            "needs_permission"
        );
        assert_eq!(owner_parts(&Owner::Named("Nemo".to_string())).0, "named");
        assert_eq!(owner_parts(&Owner::Unreadable), ("unreadable", None));
        assert_eq!(owner_parts(&Owner::NoHostTool), ("no_host_tool", None));
    }

    #[test]
    fn text_forms_read_plainly() {
        assert_eq!(state_text(&State::Silent), "silent");
        assert_eq!(
            state_text(&State::Impossible {
                reason: "Finder owns it".to_string()
            }),
            "impossible (Finder owns it)"
        );
        assert_eq!(owner_text(&Owner::NoHostTool), "no host tool");
        assert_eq!(owner_text(&Owner::Named("Nemo".to_string())), "Nemo");
        assert_eq!(lifetimes_text(&[]), "none");
        assert_eq!(
            lifetimes_text(&[Lifetime::PortableSession, Lifetime::ResidentPolicy]),
            "portable_session, resident_policy"
        );
    }

    #[test]
    fn an_empty_list_says_so() {
        assert_eq!(
            list_text(&[]),
            "no installed plugin declares a host default"
        );
        assert_eq!(list_json(&[]), json!({"roles": []}));
    }
}
