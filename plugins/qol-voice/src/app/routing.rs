use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;

use qol_terminal_sessions::cli::CliToolColor;
use qol_terminal_sessions::{DeliveryMode, SessionBinding, TerminalError};
use serde::{Deserialize, Serialize};

use crate::config::RoutingConfig;
use crate::turn::{ConversationCommand, Effect, Observation, TurnId};
use crate::voice_session::{VoiceSessionCause, VoiceSessionUpdate};

const NO_TARGET: &str = "none";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalTarget {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<CliToolColor>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteState {
    Unselected,
    Ready,
    Delivered,
    TargetUnavailable,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RouteStatus {
    pub state: RouteState,
    pub delivery_mode: DeliveryMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Default for RouteStatus {
    fn default() -> Self {
        Self {
            state: RouteState::Unselected,
            delivery_mode: DeliveryMode::Insert,
            target_label: None,
            error: None,
        }
    }
}

#[derive(Clone)]
enum ConfiguredTarget {
    Unselected,
    Invalid(String),
    Selected {
        binding: SessionBinding,
        label: String,
        availability_error: Option<String>,
    },
}

#[derive(Clone)]
pub(super) struct RouteSelection {
    target: ConfiguredTarget,
    delivery_mode: DeliveryMode,
}

impl Default for RouteSelection {
    fn default() -> Self {
        Self {
            target: ConfiguredTarget::Unselected,
            delivery_mode: DeliveryMode::Insert,
        }
    }
}

impl RouteSelection {
    pub(super) fn resolve(
        config: &RoutingConfig,
        targets: impl FnOnce() -> Result<Vec<TerminalTarget>, TerminalError>,
    ) -> Self {
        let value = config.target.trim();
        let target = if value.is_empty() || value == NO_TARGET {
            ConfiguredTarget::Unselected
        } else {
            match SessionBinding::from_str(value) {
                Ok(binding) => {
                    let (label, availability_error) = match targets() {
                        Ok(targets) => match targets
                            .into_iter()
                            .find(|candidate| candidate.value == value)
                        {
                            Some(target) => (target.label, None),
                            None => (
                                binding.session_id().to_string(),
                                Some("Selected terminal is not currently available".to_owned()),
                            ),
                        },
                        Err(error) => (binding.session_id().to_string(), Some(error.to_string())),
                    };
                    ConfiguredTarget::Selected {
                        binding,
                        label,
                        availability_error,
                    }
                }
                Err(error) => ConfiguredTarget::Invalid(error.to_string()),
            }
        };
        Self {
            target,
            delivery_mode: config.delivery_mode,
        }
    }

    pub(super) fn status(&self) -> RouteStatus {
        match &self.target {
            ConfiguredTarget::Unselected => RouteStatus {
                delivery_mode: self.delivery_mode,
                ..RouteStatus::default()
            },
            ConfiguredTarget::Invalid(error) => RouteStatus {
                state: RouteState::Failed,
                delivery_mode: self.delivery_mode,
                target_label: None,
                error: Some(error.clone()),
            },
            ConfiguredTarget::Selected {
                label,
                availability_error,
                ..
            } => RouteStatus {
                state: if availability_error.is_some() {
                    RouteState::TargetUnavailable
                } else {
                    RouteState::Ready
                },
                delivery_mode: self.delivery_mode,
                target_label: Some(label.clone()),
                error: availability_error.clone(),
            },
        }
    }

    pub(super) fn binding(&self) -> Option<&SessionBinding> {
        match &self.target {
            ConfiguredTarget::Selected { binding, .. } => Some(binding),
            ConfiguredTarget::Unselected | ConfiguredTarget::Invalid(_) => None,
        }
    }
}

#[derive(Clone)]
pub(super) struct RouteDecision {
    pub(super) revision: u64,
    selection: RouteSelection,
}

#[derive(Default)]
pub(super) struct RoutingControl {
    revision: u64,
    selection: RouteSelection,
    status: RouteStatus,
}

impl RoutingControl {
    pub(super) fn update(&mut self, selection: RouteSelection) -> Result<(), &'static str> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("voice routing revision space is exhausted")?;
        self.status = selection.status();
        self.selection = selection;
        Ok(())
    }

    pub(super) fn decision(&self) -> RouteDecision {
        RouteDecision {
            revision: self.revision,
            selection: self.selection.clone(),
        }
    }

    pub(super) fn status(&self) -> RouteStatus {
        self.status.clone()
    }

    pub(super) fn finish_delivery(&mut self, revision: u64, status: RouteStatus) {
        if self.revision == revision {
            self.status = status;
        }
    }
}

pub(super) struct DeliveryIntent {
    pub(super) revision: u64,
    pub(super) target: SessionBinding,
    pub(super) target_label: String,
    pub(super) text: String,
    pub(super) mode: DeliveryMode,
}

pub(super) struct ConversationRouter {
    configured: RouteDecision,
    pinned: HashMap<TurnId, RouteDecision>,
}

impl ConversationRouter {
    pub(super) fn new(configured: RouteDecision) -> Self {
        Self {
            configured,
            pinned: HashMap::new(),
        }
    }

    pub(super) fn configure(&mut self, configured: RouteDecision) {
        self.configured = configured;
    }

    pub(super) fn observe(&mut self, update: &VoiceSessionUpdate) -> Vec<DeliveryIntent> {
        if let VoiceSessionCause::Observation(envelope) = &update.cause {
            if let Observation::VoiceActivityStarted { turn_id } = envelope.observation {
                self.pinned.insert(turn_id, self.configured.clone());
            }
        }
        let intents = update
            .effects
            .effects
            .iter()
            .filter_map(|effect| {
                let Effect::Conversation(ConversationCommand::CommitUserTurn { turn_id, text }) =
                    effect
                else {
                    return None;
                };
                let decision = self
                    .pinned
                    .remove(turn_id)
                    .unwrap_or_else(|| self.configured.clone());
                delivery_intent(decision, text)
            })
            .collect();
        if let VoiceSessionCause::Observation(envelope) = &update.cause {
            if let Observation::TranscriptHypothesis {
                turn_id,
                final_result: true,
                ..
            } = envelope.observation
            {
                self.pinned.remove(&turn_id);
            }
        }
        intents
    }
}

fn delivery_intent(decision: RouteDecision, text: &str) -> Option<DeliveryIntent> {
    let ConfiguredTarget::Selected { binding, label, .. } = decision.selection.target else {
        return None;
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(DeliveryIntent {
        revision: decision.revision,
        target: binding,
        target_label: label,
        text: text.to_owned(),
        mode: decision.selection.delivery_mode,
    })
}

pub(super) fn target_label(display_name: Option<&str>, cwd: &str, tool: &str) -> String {
    let display_name = display_name.map(str::trim).filter(|name| !name.is_empty());
    let project = Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty());
    match (display_name, project) {
        (Some(name), Some(project)) if name != project => format!("{name} — {project} · {tool}"),
        (Some(name), _) => format!("{name} · {tool}"),
        (None, Some(project)) => format!("{project} · {tool}"),
        (None, None) => format!("Terminal · {tool}"),
    }
}

#[cfg(test)]
mod tests {
    use qol_terminal_sessions::cli::CliToolColor;
    use qol_terminal_sessions::{kitty, DeliveryMode, SessionBinding, SessionId};

    use crate::config::RoutingConfig;
    use crate::turn::{
        ConversationCommand, Effect, EffectBatch, Observation, ObservationEnvelope,
        SessionId as VoiceSessionId, TurnId, TurnSnapshot,
    };
    use crate::voice_session::{VoiceSessionCause, VoiceSessionEvidence, VoiceSessionUpdate};

    use super::{
        target_label, ConfiguredTarget, ConversationRouter, RouteDecision, RouteSelection,
        RouteState, RoutingControl, TerminalTarget,
    };

    #[test]
    fn terminal_target_wire_contract_carries_optional_strategy_accent() {
        let target = TerminalTarget {
            value: "kitty:1:42".to_owned(),
            label: "Task · Codex".to_owned(),
            accent: Some(CliToolColor::new(0x82, 0xaa, 0xff)),
        };

        assert_eq!(
            serde_json::to_value(target).unwrap(),
            serde_json::json!({
                "value": "kitty:1:42",
                "label": "Task · Codex",
                "accent": {
                    "red": 130,
                    "green": 170,
                    "blue": 255
                }
            })
        );
        let legacy: TerminalTarget = serde_json::from_value(serde_json::json!({
            "value": "kitty:1:42",
            "label": "Task · Codex"
        }))
        .unwrap();
        assert_eq!(legacy.accent, None);
    }

    #[test]
    fn terminal_target_labels_preserve_semantic_cli_identity() {
        assert_eq!(
            target_label(
                Some("QoL Voice and other improvements"),
                "/work/qol-tts",
                "Codex",
            ),
            "QoL Voice and other improvements — qol-tts · Codex"
        );
        assert_eq!(
            target_label(Some("qol dev"), "/work/qol-monorepo", "CLI"),
            "qol dev — qol-monorepo · CLI"
        );
    }

    #[test]
    fn a_turn_keeps_the_route_selected_when_speech_started() {
        let first = binding(1, 101);
        let second = binding(2, 202);
        let mut control = RoutingControl::default();
        control
            .update(selection(&first, DeliveryMode::Insert))
            .unwrap();
        let mut router = ConversationRouter::new(control.decision());

        assert!(router
            .observe(&update(
                1,
                Observation::VoiceActivityStarted { turn_id: TurnId(7) },
                Vec::new(),
            ))
            .is_empty());
        control
            .update(selection(&second, DeliveryMode::Submit))
            .unwrap();
        router.configure(control.decision());
        let intents = router.observe(&update(
            2,
            Observation::VoiceActivityEnded {
                turn_id: TurnId(7),
                awaiting_transcript: true,
            },
            vec![Effect::Conversation(ConversationCommand::CommitUserTurn {
                turn_id: TurnId(7),
                text: " cargo test ".to_owned(),
            })],
        ));

        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].target, first);
        assert_eq!(intents[0].text, "cargo test");
        assert_eq!(intents[0].mode, DeliveryMode::Insert);
    }

    #[test]
    fn final_empty_transcripts_release_pinned_routes() {
        let target = binding(1, 101);
        let mut router = ConversationRouter::new(RouteDecision {
            revision: 1,
            selection: selection(&target, DeliveryMode::Insert),
        });

        router.observe(&update(
            1,
            Observation::VoiceActivityStarted { turn_id: TurnId(7) },
            Vec::new(),
        ));
        router.observe(&update(
            2,
            Observation::TranscriptHypothesis {
                turn_id: TurnId(7),
                text: String::new(),
                confidence_permille: None,
                final_result: true,
            },
            Vec::new(),
        ));

        assert!(router.pinned.is_empty());
    }

    #[test]
    fn unavailable_targets_remain_explicit_and_safe() {
        let target = binding(1, 101);
        let config = RoutingConfig {
            target: target.token(),
            delivery_mode: DeliveryMode::Insert,
        };

        let selection = RouteSelection::resolve(&config, || Ok(Vec::new()));
        let status = selection.status();

        assert_eq!(status.state, RouteState::TargetUnavailable);
        assert_eq!(status.target_label.as_deref(), Some("kitty:1"));
        assert!(status.error.is_some());
    }

    fn selection(target: &SessionBinding, delivery_mode: DeliveryMode) -> RouteSelection {
        RouteSelection {
            target: ConfiguredTarget::Selected {
                binding: target.clone(),
                label: target.session_id().to_string(),
                availability_error: None,
            },
            delivery_mode,
        }
    }

    fn binding(native: u64, root_pid: i32) -> SessionBinding {
        SessionBinding::new(
            SessionId::new(kitty::backend_id().clone(), native.to_string()).unwrap(),
            root_pid,
        )
        .unwrap()
    }

    fn update(sequence: u64, observation: Observation, effects: Vec<Effect>) -> VoiceSessionUpdate {
        let session_id = VoiceSessionId(1);
        VoiceSessionUpdate {
            cause: VoiceSessionCause::Observation(ObservationEnvelope {
                session_id,
                sequence,
                observed_at_ms: sequence,
                observation,
            }),
            evidence: VoiceSessionEvidence::default(),
            snapshot: TurnSnapshot {
                session_id,
                last_sequence: sequence,
                assistant: crate::turn::AssistantOutputState::Idle,
                user: crate::turn::UserActivityState::Idle,
            },
            effects: EffectBatch {
                schema_version: crate::turn::CONTROL_SCHEMA_VERSION,
                session_id,
                caused_by_sequence: sequence,
                effects,
            },
        }
    }
}
