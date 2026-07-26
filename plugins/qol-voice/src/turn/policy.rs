use super::model::{
    AgentCommand, AssistantOutputState, AssistantTurnRequestEnvelope, ConversationCommand,
    ConversationalSignal, Effect, Observation, ObservationEnvelope, PlaybackCommand,
    RecognitionCommand, ResponseId, SynthesisCommand, TurnDecision, TurnId, TurnSnapshot,
    UserActivityState, UtteranceId,
};

pub trait TurnPolicy {
    fn decide(&self, state: &TurnSnapshot, envelope: &ObservationEnvelope) -> TurnDecision;

    fn request_assistant_turn(
        &self,
        state: &TurnSnapshot,
        envelope: &AssistantTurnRequestEnvelope,
    ) -> TurnDecision;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResponsiveTurnPolicy {
    confirmation_confidence_permille: u16,
    duck_level_permille: u16,
    duck_transition_ms: u16,
    resume_transition_ms: u16,
}

impl Default for ResponsiveTurnPolicy {
    fn default() -> Self {
        Self {
            confirmation_confidence_permille: 650,
            duck_level_permille: 250,
            duck_transition_ms: 80,
            resume_transition_ms: 140,
        }
    }
}

impl TurnPolicy for ResponsiveTurnPolicy {
    fn decide(&self, state: &TurnSnapshot, envelope: &ObservationEnvelope) -> TurnDecision {
        match &envelope.observation {
            Observation::AssistantSpeechStarted {
                response_id,
                utterance_id,
            } => self.assistant_speech_started(
                state,
                envelope.observed_at_ms,
                *response_id,
                *utterance_id,
            ),
            Observation::AssistantSpeechFinished { utterance_id } => {
                self.finish_assistant_speech(state, *utterance_id)
            }
            Observation::VoiceActivityStarted { turn_id } => {
                self.start_voice_activity(state, *turn_id, envelope.observed_at_ms)
            }
            Observation::VoiceActivityEnded {
                turn_id,
                awaiting_transcript,
            } => self.end_voice_activity(state, *turn_id, *awaiting_transcript),
            Observation::TranscriptHypothesis {
                turn_id,
                text,
                confidence_permille,
                final_result,
            } => self.handle_transcript(state, *turn_id, text, *confidence_permille, *final_result),
            Observation::ConversationalSignalDetected {
                signal,
                confidence_permille,
            } => self.handle_conversational_signal(state, signal, *confidence_permille),
        }
    }

    fn request_assistant_turn(
        &self,
        state: &TurnSnapshot,
        envelope: &AssistantTurnRequestEnvelope,
    ) -> TurnDecision {
        self.start_assistant_turn(
            state,
            envelope.requested_at_ms,
            envelope.request.response_id,
            envelope.request.utterance_id,
        )
    }
}

impl ResponsiveTurnPolicy {
    fn start_assistant_turn(
        &self,
        state: &TurnSnapshot,
        requested_at_ms: u64,
        response_id: ResponseId,
        utterance_id: UtteranceId,
    ) -> TurnDecision {
        let (user, mut effects) = self.interrupt_user(state, requested_at_ms, response_id);
        let (assistant, playback_effect) =
            self.claim_assistant_output(state.assistant, response_id, utterance_id);
        if let Some(effect) = playback_effect {
            effects.push(effect);
        }
        TurnDecision {
            assistant,
            user,
            effects,
        }
    }

    fn assistant_speech_started(
        &self,
        state: &TurnSnapshot,
        observed_at_ms: u64,
        response_id: ResponseId,
        utterance_id: UtteranceId,
    ) -> TurnDecision {
        let (user, effects) = self.interrupt_user(state, observed_at_ms, response_id);
        TurnDecision {
            assistant: AssistantOutputState::Playing {
                response_id,
                utterance_id,
            },
            user,
            effects,
        }
    }

    fn interrupt_user(
        &self,
        state: &TurnSnapshot,
        interrupted_at_ms: u64,
        by_response_id: ResponseId,
    ) -> (UserActivityState, Vec<Effect>) {
        let (active_turn_id, started_at_ms, hypothesis) = match &state.user {
            UserActivityState::Idle | UserActivityState::Interrupted { .. } => {
                return (state.user.clone(), Vec::new());
            }
            UserActivityState::Candidate {
                turn_id,
                started_at_ms,
            } => (*turn_id, *started_at_ms, String::new()),
            UserActivityState::Confirmed {
                turn_id,
                started_at_ms,
                hypothesis,
            }
            | UserActivityState::Finalizing {
                turn_id,
                started_at_ms,
                hypothesis,
            } => (*turn_id, *started_at_ms, hypothesis.clone()),
        };
        (
            UserActivityState::Interrupted {
                turn_id: active_turn_id,
                started_at_ms,
                interrupted_at_ms,
                hypothesis,
                by_response_id,
            },
            vec![Effect::Recognition(RecognitionCommand::FinalizeUserTurn {
                turn_id: active_turn_id,
            })],
        )
    }

    fn claim_assistant_output(
        &self,
        assistant: AssistantOutputState,
        response_id: ResponseId,
        utterance_id: UtteranceId,
    ) -> (AssistantOutputState, Option<Effect>) {
        let requested_ids = (response_id, utterance_id);
        if assistant.active_ids() != Some(requested_ids) {
            return (
                AssistantOutputState::Starting {
                    response_id,
                    utterance_id,
                },
                Some(Effect::Playback(PlaybackCommand::Start {
                    response_id,
                    utterance_id,
                })),
            );
        }
        if matches!(assistant, AssistantOutputState::Starting { .. }) {
            return (assistant, None);
        }
        if matches!(assistant, AssistantOutputState::Playing { .. }) {
            return (assistant, None);
        }
        (
            AssistantOutputState::Playing {
                response_id,
                utterance_id,
            },
            Some(Effect::Playback(PlaybackCommand::Resume {
                utterance_id,
                transition_ms: self.resume_transition_ms,
            })),
        )
    }

    fn finish_assistant_speech(
        &self,
        state: &TurnSnapshot,
        utterance_id: super::UtteranceId,
    ) -> TurnDecision {
        let assistant = match state.assistant.active_ids() {
            Some((_, active_utterance_id)) if active_utterance_id == utterance_id => {
                AssistantOutputState::Idle
            }
            _ => state.assistant,
        };
        TurnDecision {
            assistant,
            user: state.user.clone(),
            effects: Vec::new(),
        }
    }

    fn start_voice_activity(
        &self,
        state: &TurnSnapshot,
        turn_id: TurnId,
        observed_at_ms: u64,
    ) -> TurnDecision {
        let user = match &state.user {
            UserActivityState::Idle => UserActivityState::Candidate {
                turn_id,
                started_at_ms: observed_at_ms,
            },
            active => active.clone(),
        };
        let Some((response_id, utterance_id)) = state.assistant.active_ids() else {
            return TurnDecision {
                assistant: state.assistant,
                user,
                effects: Vec::new(),
            };
        };
        if !matches!(state.assistant, AssistantOutputState::Playing { .. }) {
            return TurnDecision {
                assistant: state.assistant,
                user,
                effects: Vec::new(),
            };
        }
        TurnDecision {
            assistant: AssistantOutputState::Ducked {
                response_id,
                utterance_id,
            },
            user,
            effects: vec![Effect::Playback(PlaybackCommand::Duck {
                utterance_id,
                level_permille: self.duck_level_permille,
                transition_ms: self.duck_transition_ms,
            })],
        }
    }

    fn end_voice_activity(
        &self,
        state: &TurnSnapshot,
        turn_id: TurnId,
        awaiting_transcript: bool,
    ) -> TurnDecision {
        if active_turn_id(&state.user) != Some(turn_id) {
            return unchanged(state);
        }
        if matches!(state.user, UserActivityState::Interrupted { .. }) {
            return TurnDecision {
                assistant: state.assistant,
                user: state.user.clone(),
                effects: Vec::new(),
            };
        }
        if awaiting_transcript {
            let (started_at_ms, hypothesis) = match &state.user {
                UserActivityState::Candidate { started_at_ms, .. } => {
                    (*started_at_ms, String::new())
                }
                UserActivityState::Confirmed {
                    started_at_ms,
                    hypothesis,
                    ..
                }
                | UserActivityState::Finalizing {
                    started_at_ms,
                    hypothesis,
                    ..
                } => (*started_at_ms, hypothesis.clone()),
                UserActivityState::Idle | UserActivityState::Interrupted { .. } => {
                    return unchanged(state);
                }
            };
            return TurnDecision {
                assistant: state.assistant,
                user: UserActivityState::Finalizing {
                    turn_id,
                    started_at_ms,
                    hypothesis,
                },
                effects: Vec::new(),
            };
        }
        if matches!(state.user, UserActivityState::Confirmed { .. }) {
            return unchanged(state);
        }
        self.resume_assistant(state)
    }

    fn handle_transcript(
        &self,
        state: &TurnSnapshot,
        turn_id: TurnId,
        text: &str,
        confidence_permille: Option<u16>,
        final_result: bool,
    ) -> TurnDecision {
        if active_turn_id(&state.user) != Some(turn_id) {
            return unchanged(state);
        }
        let normalized = text.trim();
        if matches!(state.user, UserActivityState::Interrupted { .. }) {
            return self.handle_interrupted_transcript(
                state,
                normalized,
                confidence_permille,
                final_result,
            );
        }
        if final_result && normalized.is_empty() {
            return self.resume_assistant(state);
        }
        if final_result {
            return self.commit_user_turn(state, turn_id, normalized.to_owned());
        }
        if normalized.is_empty() || !self.is_confident(confidence_permille) {
            return TurnDecision {
                assistant: state.assistant,
                user: state.user.clone(),
                effects: Vec::new(),
            };
        }
        self.confirm_user_turn(state, normalized.to_owned())
    }

    fn handle_interrupted_transcript(
        &self,
        state: &TurnSnapshot,
        text: &str,
        confidence_permille: Option<u16>,
        final_result: bool,
    ) -> TurnDecision {
        let UserActivityState::Interrupted {
            turn_id,
            started_at_ms,
            interrupted_at_ms,
            by_response_id,
            ..
        } = &state.user
        else {
            return TurnDecision {
                assistant: state.assistant,
                user: state.user.clone(),
                effects: Vec::new(),
            };
        };
        if final_result {
            let effects = if text.is_empty() {
                Vec::new()
            } else {
                vec![Effect::Conversation(ConversationCommand::CommitUserTurn {
                    turn_id: *turn_id,
                    text: text.to_owned(),
                })]
            };
            return TurnDecision {
                assistant: state.assistant,
                user: UserActivityState::Idle,
                effects,
            };
        }
        if text.is_empty() || !self.is_confident(confidence_permille) {
            return TurnDecision {
                assistant: state.assistant,
                user: state.user.clone(),
                effects: Vec::new(),
            };
        }
        TurnDecision {
            assistant: state.assistant,
            user: UserActivityState::Interrupted {
                turn_id: *turn_id,
                started_at_ms: *started_at_ms,
                interrupted_at_ms: *interrupted_at_ms,
                hypothesis: text.to_owned(),
                by_response_id: *by_response_id,
            },
            effects: Vec::new(),
        }
    }

    fn handle_conversational_signal(
        &self,
        state: &TurnSnapshot,
        signal: &ConversationalSignal,
        confidence_permille: u16,
    ) -> TurnDecision {
        if confidence_permille < self.confirmation_confidence_permille {
            return TurnDecision {
                assistant: state.assistant,
                user: state.user.clone(),
                effects: Vec::new(),
            };
        }
        match signal {
            ConversationalSignal::TakeTurn => self.confirm_user_turn(state, String::new()),
            ConversationalSignal::Backchannel | ConversationalSignal::Noise => {
                self.resume_assistant(state)
            }
        }
    }

    fn is_confident(&self, confidence_permille: Option<u16>) -> bool {
        confidence_permille
            .map(|confidence| confidence >= self.confirmation_confidence_permille)
            .unwrap_or(true)
    }

    fn confirm_user_turn(&self, state: &TurnSnapshot, hypothesis: String) -> TurnDecision {
        let Some(turn_id) = active_turn_id(&state.user) else {
            return unchanged(state);
        };
        let started_at_ms = match &state.user {
            UserActivityState::Candidate { started_at_ms, .. }
            | UserActivityState::Confirmed { started_at_ms, .. }
            | UserActivityState::Finalizing { started_at_ms, .. }
            | UserActivityState::Interrupted { started_at_ms, .. } => *started_at_ms,
            UserActivityState::Idle => 0,
        };
        let user = UserActivityState::Confirmed {
            turn_id,
            started_at_ms,
            hypothesis,
        };
        let Some((response_id, utterance_id)) = state.assistant.active_ids() else {
            return TurnDecision {
                assistant: state.assistant,
                user,
                effects: Vec::new(),
            };
        };
        if matches!(state.assistant, AssistantOutputState::Paused { .. }) {
            return TurnDecision {
                assistant: state.assistant,
                user,
                effects: Vec::new(),
            };
        }
        TurnDecision {
            assistant: AssistantOutputState::Paused {
                response_id,
                utterance_id,
            },
            user,
            effects: vec![Effect::Playback(PlaybackCommand::Pause { utterance_id })],
        }
    }

    fn resume_assistant(&self, state: &TurnSnapshot) -> TurnDecision {
        let Some((response_id, utterance_id)) = state.assistant.active_ids() else {
            return TurnDecision {
                assistant: AssistantOutputState::Idle,
                user: UserActivityState::Idle,
                effects: Vec::new(),
            };
        };
        if matches!(state.assistant, AssistantOutputState::Starting { .. }) {
            return TurnDecision {
                assistant: state.assistant,
                user: UserActivityState::Idle,
                effects: Vec::new(),
            };
        }
        if matches!(state.assistant, AssistantOutputState::Playing { .. }) {
            return TurnDecision {
                assistant: state.assistant,
                user: UserActivityState::Idle,
                effects: Vec::new(),
            };
        }
        TurnDecision {
            assistant: AssistantOutputState::Playing {
                response_id,
                utterance_id,
            },
            user: UserActivityState::Idle,
            effects: vec![Effect::Playback(PlaybackCommand::Resume {
                utterance_id,
                transition_ms: self.resume_transition_ms,
            })],
        }
    }

    fn commit_user_turn(
        &self,
        state: &TurnSnapshot,
        turn_id: TurnId,
        text: String,
    ) -> TurnDecision {
        let mut effects = Vec::new();
        if let Some((response_id, utterance_id)) = state.assistant.active_ids() {
            effects.extend([
                Effect::Playback(PlaybackCommand::Cancel { utterance_id }),
                Effect::Synthesis(SynthesisCommand::Cancel { response_id }),
                Effect::Agent(AgentCommand::CancelResponse { response_id }),
            ]);
        }
        effects.push(Effect::Conversation(ConversationCommand::CommitUserTurn {
            turn_id,
            text,
        }));
        TurnDecision {
            assistant: AssistantOutputState::Idle,
            user: UserActivityState::Idle,
            effects,
        }
    }
}

fn active_turn_id(user: &UserActivityState) -> Option<TurnId> {
    match user {
        UserActivityState::Idle => None,
        UserActivityState::Candidate { turn_id, .. }
        | UserActivityState::Confirmed { turn_id, .. }
        | UserActivityState::Finalizing { turn_id, .. }
        | UserActivityState::Interrupted { turn_id, .. } => Some(*turn_id),
    }
}

fn unchanged(state: &TurnSnapshot) -> TurnDecision {
    TurnDecision {
        assistant: state.assistant,
        user: state.user.clone(),
        effects: Vec::new(),
    }
}
