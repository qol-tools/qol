use std::collections::HashMap;

use qol_terminal_sessions::{SessionId, TerminalSessionService};

use crate::host::Pane;
use crate::session::registry::is_master_key;

pub fn title(key: &str, prs: &[String]) -> Option<String> {
    let numbers = prs
        .iter()
        .filter_map(|pr| pr.rsplit('/').next())
        .map(|number| format!("#{number}"))
        .collect::<Vec<_>>();
    (!numbers.is_empty()).then(|| format!("{} {key}", numbers.join(",")))
}

fn wanted(panes: &[Pane], prs_of: impl Fn(&str) -> Vec<String>) -> Vec<(&Pane, String)> {
    panes
        .iter()
        .filter_map(|pane| {
            let key = pane.spawn_identity.as_ref()?.key.as_str();
            if is_master_key(Some(key)) {
                return None;
            }
            Some((pane, title(key, &prs_of(key))?))
        })
        .collect()
}

pub struct TabTitles {
    terminals: TerminalSessionService,
    applied: HashMap<SessionId, String>,
}

impl TabTitles {
    pub fn system() -> Self {
        Self {
            terminals: TerminalSessionService::system(),
            applied: HashMap::new(),
        }
    }

    pub fn apply(&mut self, panes: &[Pane]) {
        self.applied
            .retain(|id, _| panes.iter().any(|pane| &pane.id == id));
        for (pane, title) in wanted(panes, qol_sessions_engine::pull_requests) {
            if self.applied.get(&pane.id) == Some(&title) {
                continue;
            }
            let Ok(binding) = pane.binding() else {
                continue;
            };
            match self.terminals.set_tab_title(&binding, &title) {
                Ok(()) => qol_runtime::probe!(
                    "CLI_SESSIONS_TAB_TITLE",
                    "event=applied session={binding} title={title}"
                ),
                Err(error) => {
                    log::warn!("[cli-sessions] tab title {title} not set on {binding}: {error}")
                }
            }
            self.applied.insert(pane.id.clone(), title);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::knocks::tests::pane;

    #[test]
    fn a_tab_title_leads_with_every_pull_request_number_then_the_key() {
        let prs = [
            "https://github.com/qol-tools/qol/pull/237".to_owned(),
            "https://github.com/qol-tools/qol/pull/240".to_owned(),
        ];
        assert_eq!(title("ci-speed", &prs[1..]).unwrap(), "#240 ci-speed");
        assert_eq!(title("ci-speed", &prs).unwrap(), "#237,#240 ci-speed");
        assert_eq!(title("ci-speed", &[]), None);
    }

    #[test]
    fn only_keyed_sessions_with_pull_requests_other_than_the_switchboard_are_renamed() {
        let panes = [
            pane(1, Some("ci-speed")),
            pane(2, Some("idle-lane")),
            pane(3, None),
            pane(4, Some(qol_sessions_engine::SWITCHBOARD_KEY)),
        ];
        let renamed = wanted(&panes, |key| match key {
            "idle-lane" => Vec::new(),
            _ => vec!["https://github.com/o/r/pull/239".to_owned()],
        });
        assert_eq!(renamed.len(), 1);
        assert_eq!(renamed[0].0.id, panes[0].id);
        assert_eq!(renamed[0].1, "#239 ci-speed");
    }
}
