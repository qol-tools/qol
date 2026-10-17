use qol_sessions_engine::knocks::{self, PendingKnock};
use qol_terminal_sessions::cli::CliSessionInterpreter;
use qol_terminal_sessions::{DeliveryMode, SessionBinding, TerminalSessionService, TextInput};

use crate::host::Pane;
use crate::session::registry::Registry;
use crate::session::status::Status;
use crate::session::tool::{is_generic, Tool};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub key: String,
    pub binding: SessionBinding,
    pub settled: bool,
}

impl Recipient {
    pub fn of(
        pane: &Pane,
        tool: &Tool,
        status: Status,
        runtime_status: Option<Status>,
    ) -> Option<Self> {
        let key = pane.spawn_identity.as_ref()?.key.as_str().to_owned();
        let binding = pane.binding().ok()?;
        let settled = !is_generic(tool)
            && status != Status::Parked
            && matches!(
                runtime_status.unwrap_or(status),
                Status::YourTurn | Status::Acknowledged | Status::Unknown
            );
        Some(Self {
            key,
            binding,
            settled,
        })
    }
}

pub fn recipients(panes: &[Pane], registry: &Registry) -> Vec<Recipient> {
    panes
        .iter()
        .filter_map(|pane| {
            let state = registry
                .get(&pane.id)
                .filter(|state| state.root_pid == pane.root_pid)?;
            Recipient::of(pane, &state.tool, state.status, state.runtime_status)
        })
        .collect()
}

pub fn next<'a>(
    pending: &'a [PendingKnock],
    recipients: &[Recipient],
    mut prompt_free: impl FnMut(&SessionBinding) -> bool,
) -> Option<(&'a PendingKnock, SessionBinding)> {
    let mut oldest_first = pending.iter().collect::<Vec<_>>();
    oldest_first.sort_by_key(|knock| knock.since_ms);
    oldest_first.into_iter().find_map(|knock| {
        let mut matching = recipients
            .iter()
            .filter(|recipient| recipient.key == knock.recipient_key);
        let recipient = matching.next()?;
        let deliverable =
            matching.next().is_none() && recipient.settled && prompt_free(&recipient.binding);
        deliverable.then(|| (knock, recipient.binding.clone()))
    })
}

pub struct Courier {
    terminals: TerminalSessionService,
}

impl Courier {
    pub fn system() -> Self {
        Self {
            terminals: TerminalSessionService::system(),
        }
    }

    pub fn deliver(&self, interpreter: &CliSessionInterpreter, recipients: &[Recipient]) {
        let pending = knocks::pending();
        let Some((knock, binding)) = next(&pending, recipients, |binding| {
            crate::park::prompt_free(&self.terminals, interpreter, binding)
        }) else {
            return;
        };
        if let Err(error) = self
            .terminals
            .send_text(&binding, &knock.text, DeliveryMode::Submit)
        {
            log::warn!(
                "[cli-sessions] knock for {} not typed into {binding}: {error}",
                knock.recipient_key
            );
            return;
        }
        if let Err(error) = knocks::mark_delivered(&knock.recipient_key) {
            log::warn!(
                "[cli-sessions] knock for {} typed but not marked delivered: {error}",
                knock.recipient_key
            );
        }
        qol_runtime::probe!(
            "CLI_SESSIONS_KNOCK",
            "event=delivered key={} session={binding}",
            knock.recipient_key
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_terminal_sessions::cli::{claude_tool, generic_tool};
    use qol_terminal_sessions::{SpawnIdentity, SpawnKey, SpawnSurface};

    fn pane(window: u64, key: Option<&str>) -> Pane {
        Pane {
            id: crate::host::kitty_session_id(window),
            root_pid: i32::try_from(window).unwrap(),
            cwd: "/project".into(),
            title: "lane".into(),
            at_prompt: false,
            reported_cmd: Some("claude".into()),
            foreground_basenames: vec!["claude".into()],
            foreground_pids: Vec::new(),
            capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
            spawn_identity: key.map(|key| SpawnIdentity {
                key: SpawnKey::new(key).unwrap(),
                tool: claude_tool().id,
                surface: SpawnSurface::Tab,
            }),
        }
    }

    fn knock(key: &str, since_ms: u64) -> PendingKnock {
        PendingKnock {
            recipient_key: key.into(),
            text: "[qol session] 1 new message from parent. Read it with session_inbox.".into(),
            since_ms,
        }
    }

    fn ready(window: u64, key: &str) -> Recipient {
        Recipient::of(
            &pane(window, Some(key)),
            &claude_tool(),
            Status::YourTurn,
            Some(Status::YourTurn),
        )
        .unwrap()
    }

    #[test]
    fn only_a_settled_agent_session_with_a_spawn_key_can_take_a_knock() {
        let cases = [
            (
                Some("lane"),
                claude_tool(),
                Status::YourTurn,
                Some(Status::YourTurn),
                Some(true),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::Acknowledged,
                Some(Status::YourTurn),
                Some(true),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::Coordinating,
                Some(Status::YourTurn),
                Some(true),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::Unknown,
                None,
                Some(true),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::Working,
                Some(Status::Working),
                Some(false),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::NeedsYou,
                Some(Status::NeedsYou),
                Some(false),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::Parked,
                Some(Status::YourTurn),
                Some(false),
            ),
            (
                Some("lane"),
                claude_tool(),
                Status::Parked,
                None,
                Some(false),
            ),
            (
                Some("lane"),
                generic_tool(),
                Status::YourTurn,
                Some(Status::YourTurn),
                Some(false),
            ),
            (
                None,
                claude_tool(),
                Status::YourTurn,
                Some(Status::YourTurn),
                None,
            ),
        ];
        for (key, tool, status, runtime, expected) in cases {
            let recipient = Recipient::of(&pane(7, key), &tool, status, runtime);
            assert_eq!(
                recipient.as_ref().map(|recipient| recipient.settled),
                expected,
                "{key:?} {} {status:?} {runtime:?}",
                tool.id
            );
        }
    }

    #[test]
    fn the_oldest_deliverable_knock_goes_out_alone() {
        let recipients = [ready(1, "lane-a"), ready(2, "lane-b")];
        let pending = [knock("lane-b", 20), knock("lane-a", 10)];
        let (chosen, binding) = next(&pending, &recipients, |_| true).unwrap();
        assert_eq!(chosen.recipient_key, "lane-a");
        assert_eq!(binding, recipients[0].binding);
    }

    #[test]
    fn a_held_prompt_passes_the_turn_to_the_next_knock() {
        let recipients = [ready(1, "lane-a"), ready(2, "lane-b")];
        let pending = [knock("lane-a", 10), knock("lane-b", 20)];
        let busy = recipients[0].binding.clone();
        let (chosen, _) = next(&pending, &recipients, |binding| *binding != busy).unwrap();
        assert_eq!(chosen.recipient_key, "lane-b");
        assert!(next(&pending, &recipients, |_| false).is_none());
    }

    #[test]
    fn absent_unsettled_and_shared_keys_wait_without_reading_the_prompt() {
        let working = Recipient::of(
            &pane(3, Some("busy")),
            &claude_tool(),
            Status::Working,
            Some(Status::Working),
        )
        .unwrap();
        let recipients = [working, ready(4, "twin"), ready(5, "twin")];
        let pending = [knock("gone", 1), knock("busy", 2), knock("twin", 3)];
        let mut checks = 0;
        let chosen = next(&pending, &recipients, |_| {
            checks += 1;
            true
        });
        assert!(chosen.is_none());
        assert_eq!(checks, 0);
    }
}
