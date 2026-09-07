use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

pub const CONTROL_SCHEMA_VERSION: u16 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ResponseId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct UtteranceId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct TurnId(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ConversationalSignal {
    TakeTurn,
    Backchannel,
    Noise,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum Observation {
    AssistantSpeechStarted {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
    AssistantSpeechFinished {
        utterance_id: UtteranceId,
    },
    VoiceActivityStarted {
        turn_id: TurnId,
    },
    VoiceActivityEnded {
        turn_id: TurnId,
        #[serde(default)]
        awaiting_transcript: bool,
    },
    TranscriptHypothesis {
        turn_id: TurnId,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence_permille: Option<u16>,
        final_result: bool,
    },
    ConversationalSignalDetected {
        signal: ConversationalSignal,
        confidence_permille: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationEnvelope {
    pub session_id: SessionId,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub observation: Observation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTurnRequest {
    pub response_id: ResponseId,
    pub utterance_id: UtteranceId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantTurnRequestEnvelope {
    pub session_id: SessionId,
    pub sequence: u64,
    pub requested_at_ms: u64,
    pub request: AssistantTurnRequest,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AssistantOutputState {
    Idle,
    Starting {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
    Playing {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
    Ducked {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
    Paused {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
}

impl AssistantOutputState {
    pub(crate) fn active_ids(self) -> Option<(ResponseId, UtteranceId)> {
        match self {
            Self::Idle => None,
            Self::Starting {
                response_id,
                utterance_id,
            }
            | Self::Playing {
                response_id,
                utterance_id,
            }
            | Self::Ducked {
                response_id,
                utterance_id,
            }
            | Self::Paused {
                response_id,
                utterance_id,
            } => Some((response_id, utterance_id)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum UserActivityState {
    Idle,
    Candidate {
        turn_id: TurnId,
        started_at_ms: u64,
    },
    Confirmed {
        turn_id: TurnId,
        started_at_ms: u64,
        hypothesis: String,
    },
    Finalizing {
        turn_id: TurnId,
        started_at_ms: u64,
        hypothesis: String,
    },
    Interrupted {
        turn_id: TurnId,
        started_at_ms: u64,
        interrupted_at_ms: u64,
        hypothesis: String,
        by_response_id: ResponseId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSnapshot {
    pub session_id: SessionId,
    pub last_sequence: u64,
    pub assistant: AssistantOutputState,
    pub user: UserActivityState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum PlaybackCommand {
    Start {
        response_id: ResponseId,
        utterance_id: UtteranceId,
    },
    Duck {
        utterance_id: UtteranceId,
        level_permille: u16,
        transition_ms: u16,
    },
    Pause {
        utterance_id: UtteranceId,
    },
    Resume {
        utterance_id: UtteranceId,
        transition_ms: u16,
    },
    Cancel {
        utterance_id: UtteranceId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SynthesisCommand {
    Cancel { response_id: ResponseId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AgentCommand {
    CancelResponse { response_id: ResponseId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum RecognitionCommand {
    FinalizeUserTurn { turn_id: TurnId },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum ConversationCommand {
    CommitUserTurn { turn_id: TurnId, text: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "target", content = "command")]
pub enum Effect {
    Playback(PlaybackCommand),
    Synthesis(SynthesisCommand),
    Agent(AgentCommand),
    Recognition(RecognitionCommand),
    Conversation(ConversationCommand),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectBatch {
    pub schema_version: u16,
    pub session_id: SessionId,
    pub caused_by_sequence: u64,
    pub effects: Vec<Effect>,
}

impl EffectBatch {
    pub(crate) fn new(
        session_id: SessionId,
        caused_by_sequence: u64,
        effects: Vec<Effect>,
    ) -> Self {
        Self {
            schema_version: CONTROL_SCHEMA_VERSION,
            session_id,
            caused_by_sequence,
            effects,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TurnError {
    SessionMismatch {
        expected: SessionId,
        received: SessionId,
    },
    NonMonotonicSequence {
        previous: u64,
        received: u64,
    },
}

impl Display for TurnError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SessionMismatch { expected, received } => write!(
                formatter,
                "voice session mismatch: expected {}, received {}",
                expected.0, received.0
            ),
            Self::NonMonotonicSequence { previous, received } => write!(
                formatter,
                "observation sequence must increase: previous {previous}, received {received}"
            ),
        }
    }
}

impl Error for TurnError {}

#[derive(Clone, Debug)]
pub(crate) struct TurnState {
    session_id: SessionId,
    last_sequence: u64,
    assistant: AssistantOutputState,
    user: UserActivityState,
}

impl TurnState {
    pub(crate) fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            last_sequence: 0,
            assistant: AssistantOutputState::Idle,
            user: UserActivityState::Idle,
        }
    }

    pub(crate) fn validate(&self, session_id: SessionId, sequence: u64) -> Result<(), TurnError> {
        if session_id != self.session_id {
            return Err(TurnError::SessionMismatch {
                expected: self.session_id,
                received: session_id,
            });
        }
        if sequence <= self.last_sequence {
            return Err(TurnError::NonMonotonicSequence {
                previous: self.last_sequence,
                received: sequence,
            });
        }
        Ok(())
    }

    pub(crate) fn apply(
        &mut self,
        sequence: u64,
        assistant: AssistantOutputState,
        user: UserActivityState,
    ) {
        self.last_sequence = sequence;
        self.assistant = assistant;
        self.user = user;
    }

    pub(crate) fn snapshot(&self) -> TurnSnapshot {
        TurnSnapshot {
            session_id: self.session_id,
            last_sequence: self.last_sequence,
            assistant: self.assistant,
            user: self.user.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TurnDecision {
    pub assistant: AssistantOutputState,
    pub user: UserActivityState,
    pub effects: Vec<Effect>,
}
