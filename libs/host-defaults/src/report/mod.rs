use serde::{Deserialize, Serialize};

use crate::role::Role;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifetime {
    PortableSession,
    ResidentPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Silent,
    NeedsConfirmation { ask: String },
    NeedsPermission { ask: String },
    Impossible { reason: String },
    Unknown { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Restore {
    Exact,
    OnExit,
    NeedsConfirmation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    Named(String),
    Unreadable,
    NoHostTool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub role: Role,
    pub state: State,
    pub owner: Owner,
    pub triggers: Vec<String>,
    pub kept: Vec<String>,
    pub restore: Restore,
    pub lifetimes: Vec<Lifetime>,
}

impl Report {
    pub(crate) fn impossible(role: Role, reason: &str) -> Self {
        Self {
            role,
            state: State::Impossible {
                reason: reason.to_string(),
            },
            owner: Owner::Unreadable,
            triggers: Vec::new(),
            kept: Vec::new(),
            restore: Restore::Exact,
            lifetimes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotRoute {
    pub keys: Vec<String>,
    pub reaches: Vec<String>,
    pub owner: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowControl {
    Switch { on: bool },
    Button { label: String },
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowText {
    pub label: String,
    pub description: String,
    pub control: RowControl,
}

pub fn row_text(report: &Report, held: bool, lifetime: Lifetime) -> Option<RowText> {
    let label = match &report.owner {
        Owner::NoHostTool => return None,
        Owner::Named(name) => format!("Use instead of {name}"),
        Owner::Unreadable => format!("Use instead of the system's {}", report.role.kind_noun()),
    };
    if let Some(reason) = refusal(&report.state) {
        return Some(read_only(label, reason.to_string()));
    }
    if !report.lifetimes.contains(&lifetime) {
        return Some(read_only(label, unsupported_lifetime(lifetime).to_string()));
    }
    Some(RowText {
        label,
        description: description(report, lifetime),
        control: control(&report.state, held),
    })
}

fn read_only(label: String, description: String) -> RowText {
    RowText {
        label,
        description,
        control: RowControl::ReadOnly,
    }
}

fn refusal(state: &State) -> Option<&str> {
    match state {
        State::Impossible { reason } | State::Unknown { reason } => Some(reason),
        State::Silent | State::NeedsConfirmation { .. } | State::NeedsPermission { .. } => None,
    }
}

fn ask_text(state: &State) -> Option<&str> {
    match state {
        State::NeedsConfirmation { ask } | State::NeedsPermission { ask } => Some(ask),
        State::Silent | State::Impossible { .. } | State::Unknown { .. } => None,
    }
}

fn unsupported_lifetime(lifetime: Lifetime) -> &'static str {
    match lifetime {
        Lifetime::PortableSession => "Only when kept on this computer.",
        Lifetime::ResidentPolicy => "Only while qol is running.",
    }
}

fn lifetime_sentence(lifetime: Lifetime) -> &'static str {
    match lifetime {
        Lifetime::PortableSession => "Goes back when qol stops.",
        Lifetime::ResidentPolicy => "Stays while qol is off.",
    }
}

fn description(report: &Report, lifetime: Lifetime) -> String {
    let mut parts = Vec::new();
    if !report.triggers.is_empty() {
        parts.push(format!("Takes over: {}.", report.triggers.join(", ")));
    }
    parts.extend(report.kept.iter().cloned());
    if let Some(ask) = ask_text(&report.state) {
        parts.push(ask.to_string());
    }
    parts.push(lifetime_sentence(lifetime).to_string());
    parts.join(" ")
}

fn control(state: &State, held: bool) -> RowControl {
    match state {
        State::Silent => RowControl::Switch { on: held },
        State::NeedsConfirmation { .. } => approval(held, "Allow"),
        State::NeedsPermission { .. } => approval(held, "Open Settings"),
        State::Impossible { .. } | State::Unknown { .. } => RowControl::ReadOnly,
    }
}

fn approval(held: bool, label: &str) -> RowControl {
    if held {
        return RowControl::Switch { on: true };
    }
    RowControl::Button {
        label: label.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [Lifetime; 2] = [Lifetime::PortableSession, Lifetime::ResidentPolicy];

    fn report(role: Role, state: State, owner: Owner) -> Report {
        Report {
            role,
            state,
            owner,
            triggers: vec!["Print".to_string()],
            kept: Vec::new(),
            restore: Restore::Exact,
            lifetimes: BOTH.to_vec(),
        }
    }

    fn named(name: &str) -> Owner {
        Owner::Named(name.to_string())
    }

    #[test]
    fn labels_name_the_owner_or_fall_back_to_the_kind() {
        let cases = [
            (
                "named",
                Role::SlotScreenshot,
                named("GNOME Screenshot"),
                Some("Use instead of GNOME Screenshot"),
            ),
            (
                "unreadable file manager",
                Role::FolderOpen,
                Owner::Unreadable,
                Some("Use instead of the system's file manager"),
            ),
            (
                "unreadable launcher",
                Role::SlotLauncher,
                Owner::Unreadable,
                Some("Use instead of the system's app launcher"),
            ),
            (
                "unreadable window switcher",
                Role::SlotWindowSwitcher,
                Owner::Unreadable,
                Some("Use instead of the system's window switcher"),
            ),
            (
                "unreadable screenshot",
                Role::SlotScreenshot,
                Owner::Unreadable,
                Some("Use instead of the system's screenshot tool"),
            ),
            (
                "unreadable clipboard",
                Role::SlotClipboardHistory,
                Owner::Unreadable,
                Some("Use instead of the system's clipboard history"),
            ),
            (
                "unreadable dictation",
                Role::SlotDictation,
                Owner::Unreadable,
                Some("Use instead of the system's dictation"),
            ),
            ("no host tool", Role::SlotDictation, Owner::NoHostTool, None),
        ];
        for (label, role, owner, expected) in cases {
            let row = row_text(
                &report(role, State::Silent, owner),
                false,
                Lifetime::PortableSession,
            );
            assert_eq!(
                row.as_ref().map(|row| row.label.as_str()),
                expected,
                "case: {label}"
            );
        }
    }

    #[test]
    fn silent_rows_are_a_switch_that_follows_held() {
        for held in [false, true] {
            let row = row_text(
                &report(Role::SlotScreenshot, State::Silent, named("Tool")),
                held,
                Lifetime::PortableSession,
            )
            .unwrap();
            assert_eq!(row.control, RowControl::Switch { on: held });
            assert_eq!(
                row.description,
                "Takes over: Print. Goes back when qol stops."
            );
        }
    }

    #[test]
    fn description_orders_triggers_kept_ask_then_lifetime() {
        let mut report = report(
            Role::FolderOpen,
            State::NeedsConfirmation {
                ask: "Windows asks you to confirm in Settings.".to_string(),
            },
            named("Nemo"),
        );
        report.triggers = vec!["opening folders".to_string(), "Win+E".to_string()];
        report.kept = vec![
            "Show a file in its folder stays with Nemo.".to_string(),
            "Control Panel keeps its own.".to_string(),
        ];
        let cases = [
            (
                Lifetime::PortableSession,
                "Takes over: opening folders, Win+E. Show a file in its folder stays with Nemo. Control Panel keeps its own. Windows asks you to confirm in Settings. Goes back when qol stops.",
            ),
            (
                Lifetime::ResidentPolicy,
                "Takes over: opening folders, Win+E. Show a file in its folder stays with Nemo. Control Panel keeps its own. Windows asks you to confirm in Settings. Stays while qol is off.",
            ),
        ];
        for (lifetime, expected) in cases {
            let row = row_text(&report, false, lifetime).unwrap();
            assert_eq!(row.description, expected);
        }
    }

    #[test]
    fn approval_states_show_a_button_until_held() {
        let cases = [
            (
                "confirmation open",
                State::NeedsConfirmation {
                    ask: "Confirm.".to_string(),
                },
                false,
                RowControl::Button {
                    label: "Allow".to_string(),
                },
            ),
            (
                "confirmation held",
                State::NeedsConfirmation {
                    ask: "Confirm.".to_string(),
                },
                true,
                RowControl::Switch { on: true },
            ),
            (
                "permission open",
                State::NeedsPermission {
                    ask: "Grant.".to_string(),
                },
                false,
                RowControl::Button {
                    label: "Open Settings".to_string(),
                },
            ),
            (
                "permission held",
                State::NeedsPermission {
                    ask: "Grant.".to_string(),
                },
                true,
                RowControl::Switch { on: true },
            ),
        ];
        for (label, state, held, expected) in cases {
            let row = row_text(
                &report(Role::SlotLauncher, state, named("Spotlight")),
                held,
                Lifetime::ResidentPolicy,
            )
            .unwrap();
            assert_eq!(row.control, expected, "case: {label}");
        }
    }

    #[test]
    fn impossible_and_unknown_rows_are_read_only_with_the_reason() {
        let cases = [
            State::Impossible {
                reason: "Finder owns it.".to_string(),
            },
            State::Unknown {
                reason: "The probe failed.".to_string(),
            },
        ];
        for state in cases {
            let reason = refusal(&state).unwrap().to_string();
            let row = row_text(
                &report(Role::FolderOpen, state, named("Finder")),
                true,
                Lifetime::PortableSession,
            )
            .unwrap();
            assert_eq!(row.control, RowControl::ReadOnly);
            assert_eq!(row.description, reason);
        }
    }

    #[test]
    fn a_lifetime_the_role_cannot_honour_is_read_only() {
        let cases = [
            (
                Lifetime::PortableSession,
                Lifetime::ResidentPolicy,
                "Only when kept on this computer.",
            ),
            (
                Lifetime::ResidentPolicy,
                Lifetime::PortableSession,
                "Only while qol is running.",
            ),
        ];
        for (requested, supported, expected) in cases {
            let mut report = report(Role::FolderOpen, State::Silent, named("Nemo"));
            report.lifetimes = vec![supported];
            let row = row_text(&report, true, requested).unwrap();
            assert_eq!(row.control, RowControl::ReadOnly);
            assert_eq!(row.description, expected);
        }
    }

    #[test]
    fn a_report_without_triggers_skips_the_takes_over_sentence() {
        let mut report = report(Role::FolderOpen, State::Silent, named("Nemo"));
        report.triggers.clear();
        let row = row_text(&report, false, Lifetime::PortableSession).unwrap();
        assert_eq!(row.description, "Goes back when qol stops.");
    }

    #[test]
    fn lifetimes_serialize_in_snake_case() {
        let cases = [
            (Lifetime::PortableSession, "\"portable_session\""),
            (Lifetime::ResidentPolicy, "\"resident_policy\""),
        ];
        for (lifetime, expected) in cases {
            assert_eq!(serde_json::to_string(&lifetime).unwrap(), expected);
        }
    }
}
